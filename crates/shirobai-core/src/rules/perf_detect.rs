//! `Performance/Detect` (rubocop-performance): flags `first` / `last` / `[0]`
//! / `[-1]` chained onto `select` / `find_all` / `filter` and rewrites the
//! chain to the preferred single-pass method (`detect` by default, `find`
//! under RuboCop's default `Style/CollectionMethods` `PreferredMethods`).
//!
//! Mirrors `vendor/rubocop-performance/lib/rubocop/cop/performance/detect.rb`
//! (v1.26.1). The stock node pattern has four branches; in prism terms
//! (a block is a field of `CallNode`, not a wrapper node):
//!
//! - outer node must be a parser `send`: method `first` / `last` / `[]`,
//!   call operator `.` / `::` / none — never `&.` (`on_csend` is aliased but
//!   the pattern only matches `(send ...)`, so `&.first` never fires).
//! - `first` / `last` take no parser args (a block-pass IS a parser arg, a
//!   literal block is not). `[]` takes exactly one integer literal `0` / `-1`
//!   (by value, so `0x0` counts) and no block-pass.
//! - inner receiver, block form (`foo.select { ... }.first`): parser wraps
//!   the inner call in `(block ...)`. The inner call must have NO args; for
//!   `first`/`last` it may be send or csend (`call`), for `[]` send only.
//!   Parser `numblock` (`_1`) and `itblock` (`it`) are DIFFERENT node types
//!   and never match — so numbered-parameter blocks are excluded, and
//!   it-blocks follow prism Latest semantics (`ItParametersNode`, excluded).
//!   Stock on whitequark with `TargetRubyVersion <= 3.3` parses `it` as a
//!   plain method call and WOULD flag it; this port follows the engine
//!   RuboCop itself uses for 3.3+ targets (prism translation, `itblock`),
//!   same family as the documented `TargetRubyVersion` limitation.
//! - inner receiver, plain form (`foo.select(&:odd?).first`): any args; send
//!   or csend for `first`/`last`, send only for `[]`.
//! - `accept_first_call?`: accept (no offense) when the block body is empty
//!   AND the parser first-arg is not a block-pass; otherwise accept only a
//!   `lazy` chain (inner receiver is a receiver-carrying `lazy` call without
//!   a literal block). So `foo.select { }.first`, `foo.select(:x).first`,
//!   `foo.select(1, &:b).first`, `foo.select.first` and
//!   `foo.lazy.select { ... }.first` are all accepted.
//!
//! Offense range = inner selector start .. outer selector end, where the
//! outer selector follows parser semantics: `first` / `last` / explicit
//! `.[]` use the method-name token, sugar `x[0]` uses `[0]` (opening
//! bracket through closing bracket). Autocorrect removes
//! `receiver.source_range.end .. outer selector end` (the `.first` /
//! `[0]` tail) and replaces the inner selector with the replacement —
//! including stock's broken rewrite for the explicit `.[](0)` form, which
//! leaves the `(0)` argument list behind. Byte parity beats prettiness.

use ruby_prism::{CallNode, Node, Visit};

#[derive(Debug, Clone)]
pub struct PerfDetectOffense {
    /// Inner selector (`select` / `find_all` / `filter`) byte range: the
    /// offense highlight start and the autocorrect replace target.
    pub sel_start: usize,
    pub sel_end: usize,
    /// End byte of the outer node's receiver (block end or inner call end):
    /// the autocorrect removal starts here.
    pub recv_end: usize,
    /// End byte of the outer selector: offense highlight end and autocorrect
    /// removal end.
    pub outer_end: usize,
    pub message: String,
    /// `preferred` or `reverse.preferred`.
    pub replacement: String,
}

/// Standalone entry point used by the per-cop fallback.
pub fn check_perf_detect(source: &[u8], preferred: &str) -> Vec<PerfDetectOffense> {
    let mut visitor = build_rule(preferred);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in the shared-walk bundle.
pub(crate) fn build_rule(preferred: &str) -> PerfDetectVisitor {
    PerfDetectVisitor {
        preferred: preferred.to_string(),
        offenses: Vec::new(),
    }
}

pub(crate) struct PerfDetectVisitor {
    preferred: String,
    pub(crate) offenses: Vec<PerfDetectOffense>,
}

#[derive(Clone, Copy, PartialEq)]
enum SecondMethod {
    First,
    Last,
    /// `[]` with the literal index value (0 or -1).
    Index(i32),
}

fn is_csend(call: &CallNode<'_>) -> bool {
    call.call_operator_loc()
        .is_some_and(|l| l.as_slice() == b"&.")
}

/// Stock `lazy?(caller)`: parser-destructures the inner receiver and checks
/// `method == :lazy && receiver` — true only for a receiver-carrying
/// `lazy` send/csend without a literal block (a `(block ...)` caller
/// destructures its args slot into the method position and never matches).
fn lazy_chain(caller: Option<Node<'_>>) -> bool {
    let Some(c) = caller else { return false };
    let Some(call) = c.as_call_node() else {
        return false;
    };
    if call.name().as_slice() != b"lazy" || call.receiver().is_none() {
        return false;
    }
    // A literal block turns the parser node into `(block ...)`; a block-pass
    // stays a plain send and still matches stock's destructure.
    !matches!(call.block(), Some(b) if b.as_block_node().is_some())
}

impl PerfDetectVisitor {
    fn check_call(&mut self, node: &CallNode<'_>) {
        // Outer must be a parser `send` — `&.first` is a csend and the stock
        // pattern head `(send ...)` never matches it.
        if is_csend(node) {
            return;
        }
        let name = node.name();
        let second = match name.as_slice() {
            b"first" => SecondMethod::First,
            b"last" => SecondMethod::Last,
            b"[]" => {
                let Some(index) = index_arg_value(node) else {
                    return;
                };
                SecondMethod::Index(index)
            }
            _ => return,
        };
        // A block-pass is a parser argument: `.first(&:b)` / `.[](0, &b)`
        // break the exact-arg patterns. A literal block does not.
        let has_block_pass = matches!(node.block(), Some(b) if b.as_block_argument_node().is_some());
        if has_block_pass {
            return;
        }
        if matches!(second, SecondMethod::First | SecondMethod::Last)
            && node.arguments().is_some_and(|a| !a.arguments().is_empty())
        {
            return;
        }

        let Some(receiver) = node.receiver() else {
            return;
        };
        let Some(inner) = receiver.as_call_node() else {
            return;
        };
        if !matches!(
            inner.name().as_slice(),
            b"select" | b"find_all" | b"filter"
        ) {
            return;
        }

        // Split the parser view: a literal block wraps the inner call in
        // `(block ...)`; numblock / itblock are different parser node types
        // and never match the stock pattern.
        let inner_block = inner.block();
        let block_node = inner_block.as_ref().and_then(|b| b.as_block_node());
        let inner_has_block_pass =
            matches!(&inner_block, Some(b) if b.as_block_argument_node().is_some());
        let inner_has_args = inner
            .arguments()
            .is_some_and(|a| !a.arguments().is_empty());
        let body = if let Some(bn) = &block_node {
            match bn.parameters() {
                Some(p)
                    if p.as_numbered_parameters_node().is_some()
                        || p.as_it_parameters_node().is_some() =>
                {
                    return;
                }
                _ => {}
            }
            // Block-form branches require an argless inner call.
            if inner_has_args {
                return;
            }
            bn.body()
        } else {
            None
        };
        // The `[]` branches only accept a parser `send` inner (`&.select`
        // stays unflagged there, while `first` / `last` accept `call`).
        if matches!(second, SecondMethod::Index(_)) && is_csend(&inner) {
            return;
        }

        // accept_first_call?: accept (no offense) when the block body is
        // empty AND the parser first-arg is not a block-pass (parser puts a
        // sole block-pass in the first arg slot); otherwise only a lazy
        // chain is accepted.
        let first_arg_is_block_pass =
            block_node.is_none() && !inner_has_args && inner_has_block_pass;
        if body.is_none() && !first_arg_is_block_pass {
            return;
        }
        if lazy_chain(inner.receiver()) {
            return;
        }

        let Some(inner_sel) = inner.message_loc() else {
            return;
        };
        let Some(outer_end) = outer_selector_end(node) else {
            return;
        };
        let first_method = String::from_utf8_lossy(inner.name().as_slice()).into_owned();
        let preferred = &self.preferred;
        let (message, replacement) = match second {
            SecondMethod::First => (
                format!("Use `{preferred}` instead of `{first_method}.first`."),
                preferred.clone(),
            ),
            SecondMethod::Last => (
                format!("Use `reverse.{preferred}` instead of `{first_method}.last`."),
                format!("reverse.{preferred}"),
            ),
            SecondMethod::Index(i) => {
                let msg = if i == -1 {
                    format!("Use `reverse.{preferred}` instead of `{first_method}[-1]`.")
                } else {
                    format!("Use `{preferred}` instead of `{first_method}[{i}]`.")
                };
                let rep = if i == -1 {
                    format!("reverse.{preferred}")
                } else {
                    preferred.clone()
                };
                (msg, rep)
            }
        };

        self.offenses.push(PerfDetectOffense {
            sel_start: inner_sel.start_offset(),
            sel_end: inner_sel.end_offset(),
            recv_end: inner.location().end_offset(),
            outer_end,
            message,
            replacement,
        });
    }
}

/// The literal index for the `[]` branch: exactly one integer argument whose
/// VALUE is `0` or `-1` (stock matches `(int {0 -1})` by value, so `0x0`
/// also counts). Anything else disqualifies the node.
fn index_arg_value(node: &CallNode<'_>) -> Option<i32> {
    let args = node.arguments()?;
    let args = args.arguments();
    if args.iter().count() != 1 {
        return None;
    }
    let arg = args.iter().next()?;
    let int = arg.as_integer_node()?;
    let value: i32 = int.value().try_into().ok()?;
    if value == 0 || value == -1 {
        Some(value)
    } else {
        None
    }
}

/// End byte of the parser `loc.selector` for the outer node: the method-name
/// token for `first` / `last` / explicit `.[]`, the bracket construct
/// (`[0]`) for the sugar index form.
fn outer_selector_end(node: &CallNode<'_>) -> Option<usize> {
    if node.name().as_slice() == b"[]" && node.call_operator_loc().is_none() {
        return Some(node.closing_loc()?.end_offset());
    }
    Some(node.message_loc()?.end_offset())
}

impl<'pr> Visit<'pr> for PerfDetectVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        ruby_prism::visit_call_node(self, node);
    }
}

impl super::dispatch::Rule for PerfDetectVisitor {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(Interest::ENTER_CALL)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.check_call(&call);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(src: &str) -> Vec<PerfDetectOffense> {
        check_perf_detect(src.as_bytes(), "find")
    }

    fn spans(src: &str) -> Vec<(usize, usize)> {
        detect(src)
            .into_iter()
            .map(|o| (o.sel_start, o.outer_end))
            .collect()
    }

    // Expectations below are stock-derived: every offense range, message and
    // correction was probed against rubocop-performance 1.26.1 with rubocop
    // 1.88.0 (.tmp/2026-07-05/probe-perf).

    #[test]
    fn flags_select_first_block() {
        let off = detect("[1, 2].select { |i| i.odd? }.first\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.sel_start, o.outer_end), (7, 34));
        assert_eq!(o.message, "Use `find` instead of `select.first`.");
        assert_eq!(o.replacement, "find");
        assert_eq!((o.sel_start, o.sel_end), (7, 13));
        assert_eq!(o.recv_end, 28); // after `}`
    }

    #[test]
    fn flags_select_last_block() {
        let off = detect("[1, 2].select { |i| i.odd? }.last\n");
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].message, "Use `reverse.find` instead of `select.last`.");
        assert_eq!(off[0].replacement, "reverse.find");
        assert_eq!((off[0].sel_start, off[0].outer_end), (7, 33));
    }

    #[test]
    fn flags_filter_index_zero() {
        let off = detect("[1, 2].filter { |i| i.odd? }[0]\n");
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].sel_start, off[0].outer_end), (7, 31));
        assert_eq!(off[0].message, "Use `find` instead of `filter[0]`.");
        assert_eq!(off[0].replacement, "find");
        assert_eq!(off[0].recv_end, 28);
    }

    #[test]
    fn flags_filter_index_minus_one() {
        let off = detect("[1, 2].filter { |i| i.odd? }[-1]\n");
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].sel_start, off[0].outer_end), (7, 32));
        assert_eq!(off[0].message, "Use `reverse.find` instead of `filter[-1]`.");
        assert_eq!(off[0].replacement, "reverse.find");
    }

    #[test]
    fn accepts_other_index() {
        assert!(spans("[1, 2].filter { |i| i.odd? }[1]\n").is_empty());
    }

    #[test]
    fn flags_find_all_first() {
        let off = detect("[1, 2].find_all { |i| i.odd? }.first\n");
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].message, "Use `find` instead of `find_all.first`.");
    }

    #[test]
    fn accepts_first_with_args() {
        assert!(spans("[1, 2].select { |i| i.odd? }.first(2)\n").is_empty());
    }

    #[test]
    fn accepts_empty_block_body() {
        assert!(spans("foo.select { }.first\n").is_empty());
        assert!(spans("foo.select { |i| }.first\n").is_empty());
    }

    #[test]
    fn flags_block_pass_select() {
        let off = detect("foo.select(&:odd?).first\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.sel_start, o.outer_end), (4, 24));
        assert_eq!(o.recv_end, 18); // after `)`
        assert_eq!(o.message, "Use `find` instead of `select.first`.");
    }

    #[test]
    fn accepts_select_with_plain_arg() {
        assert!(spans("foo.select(:x).first\n").is_empty());
    }

    #[test]
    fn accepts_select_with_arg_and_block_pass() {
        assert!(spans("foo.select(1, &:odd?).first\n").is_empty());
    }

    #[test]
    fn accepts_bare_select() {
        assert!(spans("foo.select.first\n").is_empty());
    }

    #[test]
    fn accepts_lazy_chain() {
        assert!(spans("foo.lazy.select { |i| i.odd? }.first\n").is_empty());
    }

    #[test]
    fn flags_receiverless_lazy() {
        // `lazy` without a receiver does not satisfy stock's `lazy?`.
        let off = spans("lazy.select { |i| i.odd? }.first\n");
        assert_eq!(off, vec![(5, 32)]);
    }

    #[test]
    fn accepts_csend_outer() {
        assert!(spans("foo.select { |i| i.odd? }&.first\n").is_empty());
    }

    #[test]
    fn flags_csend_inner_first() {
        let off = spans("foo&.select { |i| i.odd? }.first\n");
        assert_eq!(off, vec![(5, 32)]);
    }

    #[test]
    fn accepts_csend_inner_index() {
        assert!(spans("foo&.select { |i| i.odd? }[0]\n").is_empty());
    }

    #[test]
    fn flags_explicit_index_call() {
        // Stock's autocorrect is knowingly broken here (leaves `(0)` behind);
        // the ranges must still match byte for byte.
        let off = detect("foo.select { |i| i.odd? }.[](0)\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.sel_start, o.outer_end), (4, 28));
        assert_eq!(o.message, "Use `find` instead of `select[0]`.");
        assert_eq!(o.recv_end, 25);
    }

    #[test]
    fn flags_explicit_index_call_no_parens() {
        let off = spans("foo.select { |i| i.odd? }.[] 0\n");
        assert_eq!(off, vec![(4, 28)]);
    }

    #[test]
    fn flags_implicit_receiver_select() {
        let off = spans("select { |i| i.odd? }.first\n");
        assert_eq!(off, vec![(0, 27)]);
    }

    #[test]
    fn accepts_numbered_parameter_block() {
        assert!(spans("foo.select { _1.odd? }.first\n").is_empty());
    }

    #[test]
    fn accepts_it_parameter_block() {
        // prism Latest semantics: an `it` block is an ItParametersNode, which
        // the parser-side pattern (`block` only) does not match. Stock on
        // whitequark (TargetRubyVersion <= 3.3) flags this; see the module
        // docs for the deliberate divergence.
        assert!(spans("foo.select { it.odd? }.first\n").is_empty());
    }

    #[test]
    fn flags_multiline_do_end() {
        let off = detect("foo.select do |i|\n  i.odd?\nend.first\n");
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].sel_start, off[0].outer_end), (4, 36));
        assert_eq!(off[0].recv_end, 30); // after `end`
    }

    #[test]
    fn flags_double_colon_first() {
        let off = detect("foo.select { |i| i.odd? }::first\n");
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].sel_start, off[0].outer_end), (4, 32));
        assert_eq!(off[0].recv_end, 25);
    }

    #[test]
    fn flags_inside_longer_chain() {
        let off = spans("foo.select { |i| i.odd? }.first.to_s\n");
        assert_eq!(off, vec![(4, 31)]);
    }

    #[test]
    fn accepts_select_with_args_and_block() {
        assert!(spans("foo.select(x) { |i| i.odd? }.first\n").is_empty());
    }

    #[test]
    fn respects_preferred_method() {
        let off = check_perf_detect("foo.select { |i| i.odd? }.first\n".as_bytes(), "detect");
        assert_eq!(off[0].message, "Use `detect` instead of `select.first`.");
        assert_eq!(off[0].replacement, "detect");
    }

    #[test]
    fn flags_hexadecimal_zero_index() {
        // Stock matches `(int 0)` by VALUE: `[0x0]` counts.
        let off = spans("foo.select { |i| i.odd? }[0x0]\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_first_with_literal_block() {
        // `.first { ... }` — the parser send under the outer block wrapper
        // still matches (args are empty); stock flags it.
        let off = spans("foo.select { |i| i.odd? }.first { bar }\n");
        assert_eq!(off, vec![(4, 31)]);
    }

    #[test]
    fn accepts_first_with_block_pass() {
        assert!(spans("foo.select { |i| i.odd? }.first(&:blk)\n").is_empty());
    }
}
