//! `Style/RedundantFreeze`: flags `receiver.freeze` when the receiver is an
//! immutable object, and removes the `.freeze` (leaving the receiver bytes
//! untouched).
//!
//! Mirrors `vendor/rubocop/lib/rubocop/cop/style/redundant_freeze.rb` and the
//! `RuboCop::Cop::FrozenStringLiteral` mixin.
//!
//! `on_send` fires for `:freeze` sends (RESTRICT_ON_SEND). There is no
//! `on_csend`, so a safe-navigation `x&.freeze` is never flagged. The receiver
//! must be present and pass one of:
//!
//! - `immutable_literal?(receiver)` — after stripping ONE layer of parentheses
//!   (a prism `ParenthesesNode`, the parser-gem `begin`), the node is:
//!   * an immutable literal node (int / float / rational / imaginary / symbol /
//!     interpolated-symbol / true / false / nil — the rubocop-ast
//!     `IMMUTABLE_LITERALS` set), OR
//!   * a frozen-string-literal candidate (`frozen_string_literal?`), OR
//!   * (target Ruby >= 3.0) a regexp or range literal.
//! - `operation_produces_immutable_object?(receiver)` — the raw (un-stripped)
//!   receiver matches one of the node-pattern arms (see `operation_*` below).
//!
//! Autocorrect removes `node.loc.dot` (the `call_operator_loc`) and
//! `node.loc.selector` (the `message_loc`), leaving everything else — including
//! any parentheses around the receiver, and a trailing block — in place.
//!
//! Offense highlight = the send node's range = `[receiver_start, selector_end)`.
//!
//! # Version and config plumbing
//!
//! Two config-derived booleans travel from the Ruby wrapper:
//!
//! - `target_ruby_30_plus` (`AllCops/TargetRubyVersion >= 3.0`): gates the
//!   regexp/range branch AND the frozen-string candidate set (uninterpolated
//!   only on >= 3.0; any str/dstr on < 3.0).
//! - `string_literals_frozen_by_default` (`AllCops/StringLiteralsFrozenByDefault`
//!   is literally `true`): the fallback for `frozen_string_literals_enabled?`
//!   when no leading `# frozen_string_literal:` comment specifies a value.
//!
//! The `frozen_string_literals_enabled?` decision itself (leading-comment scan
//! plus that fallback) is computed once per file — on raw bytes, reusing
//! `duplicate_magic_comment::frozen_string_literals_enabled` — and folded in
//! during [`RedundantFreezeVisitor::finalize`]. String-receiver offenses are
//! kept `conditional` until then, so a file with no `string.freeze` never pays
//! for the comment scan.

use ruby_prism::{CallNode, Node, Visit};

use super::duplicate_magic_comment::frozen_string_literals_enabled;

/// One offense. `[off_start, off_end)` is the highlight (the send node); the
/// two remove ranges reproduce stock's `corrector.remove(node.loc.dot)` +
/// `corrector.remove(node.loc.selector)`.
pub struct RedundantFreezeOffense {
    pub off_start: usize,
    pub off_end: usize,
    pub dot_start: usize,
    pub dot_end: usize,
    pub selector_start: usize,
    pub selector_end: usize,
}

/// Internal record: the offense plus whether it depends on
/// `frozen_string_literals_enabled?` (a string-literal receiver). Conditional
/// records are dropped in `finalize` when the file is not frozen-string
/// enabled.
struct Record {
    off: RedundantFreezeOffense,
    conditional: bool,
}

/// Standalone entry point used by the per-cop fallback (the bundle is the
/// usual path). `target_ruby_30_plus` and `string_literals_frozen_by_default`
/// come from `AllCops` config; the fsl-enabled decision is computed here.
pub fn check_redundant_freeze(
    source: &[u8],
    target_ruby_30_plus: bool,
    string_literals_frozen_by_default: bool,
) -> Vec<RedundantFreezeOffense> {
    let mut visitor = build_rule(source, target_ruby_30_plus);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.finalize(string_literals_frozen_by_default)
}

pub(crate) fn build_rule(source: &[u8], target_ruby_30_plus: bool) -> RedundantFreezeVisitor<'_> {
    RedundantFreezeVisitor {
        source,
        target_ruby_30_plus,
        records: Vec::new(),
    }
}

pub(crate) struct RedundantFreezeVisitor<'s> {
    source: &'s [u8],
    target_ruby_30_plus: bool,
    records: Vec<Record>,
}

impl<'s> RedundantFreezeVisitor<'s> {
    /// Fold in the `frozen_string_literals_enabled?` decision and return the
    /// final offense list. The comment scan runs only when at least one
    /// string-receiver candidate was recorded.
    pub(crate) fn finalize(
        self,
        string_literals_frozen_by_default: bool,
    ) -> Vec<RedundantFreezeOffense> {
        let has_conditional = self.records.iter().any(|r| r.conditional);
        let enabled = has_conditional
            && frozen_string_literals_enabled(self.source, string_literals_frozen_by_default);
        self.records
            .into_iter()
            .filter(|r| !r.conditional || enabled)
            .map(|r| r.off)
            .collect()
    }

    fn check_call(&mut self, call: &CallNode<'_>) {
        // RESTRICT_ON_SEND = [:freeze]; there is no `on_csend`.
        if call.name().as_slice() != b"freeze" {
            return;
        }
        if call.is_safe_navigation() {
            return;
        }
        // `return unless node.receiver`.
        let Some(receiver) = call.receiver() else {
            return;
        };

        // Decide immutability. `conditional` is true only for the
        // frozen-string-literal branch, whose result depends on
        // `frozen_string_literals_enabled?`.
        let mut conditional = false;
        let immutable = self.immutable_literal(&receiver, &mut conditional)
            || self.operation_produces_immutable_object(&receiver);
        if !immutable {
            return;
        }
        // `conditional` is set true only when the match came via the
        // frozen-string-literal branch (which short-circuits before the
        // operation check); every other match leaves it false.

        let Some(dot_loc) = call.call_operator_loc() else {
            return;
        };
        let Some(sel_loc) = call.message_loc() else {
            return;
        };
        let recv_loc = receiver.location();
        self.records.push(Record {
            off: RedundantFreezeOffense {
                off_start: recv_loc.start_offset(),
                off_end: sel_loc.end_offset(),
                dot_start: dot_loc.start_offset(),
                dot_end: dot_loc.end_offset(),
                selector_start: sel_loc.start_offset(),
                selector_end: sel_loc.end_offset(),
            },
            conditional,
        });
    }

    /// `immutable_literal?(node)` after `strip_parenthesis`. Sets `conditional`
    /// when the match is *only* via the frozen-string-literal branch.
    fn immutable_literal(&self, node: &Node<'_>, conditional: &mut bool) -> bool {
        let stripped_child = strip_parenthesis_child(node);
        let stripped = stripped_child.as_ref().unwrap_or(node);

        if is_immutable_literal_node(stripped) {
            return true;
        }
        if self.frozen_string_candidate(stripped) {
            *conditional = true;
            return true;
        }
        // `target_ruby_version >= 3.0 && node.type?(:regexp, :range)`.
        if self.target_ruby_30_plus && is_regexp_or_range(stripped) {
            return true;
        }
        false
    }

    /// `frozen_string_literal?(node)` candidate check (the "is this a string
    /// literal that would be frozen" half; the enabled half is folded in at
    /// `finalize`). On target >= 3.0 the candidate must be uninterpolated; on
    /// target < 3.0 any str/dstr counts (`FROZEN_STRING_LITERAL_TYPES_RUBY27`).
    fn frozen_string_candidate(&self, node: &Node<'_>) -> bool {
        if node.as_string_node().is_some() {
            return true;
        }
        if let Some(istr) = node.as_interpolated_string_node() {
            if self.target_ruby_30_plus {
                // `uninterpolated_string?` / `uninterpolated_heredoc?`: every
                // part is a plain string (no `#{}` / `#@ivar` interpolation).
                return istr.parts().iter().all(|p| p.as_string_node().is_some());
            }
            // target < 3.0: dstr type counts regardless of interpolation.
            return true;
        }
        false
    }
}

/// `strip_parenthesis`: for a `begin` (prism `ParenthesesNode`) with a truthy
/// first child, return `Some(child)`; otherwise `None` (the caller keeps the
/// original node, matching stock's `else` branch). Prism `Node` is not
/// cloneable, so the "keep original" case is expressed as `None` rather than a
/// clone.
fn strip_parenthesis_child<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let parens = node.as_parentheses_node()?;
    let body = parens.body()?;
    if let Some(stmts) = body.as_statements_node() {
        // `None` (empty statements) falls back to the node, matching stock's
        // `children.first` being nil.
        stmts.body().iter().next()
    } else {
        // Single non-`StatementsNode` body = the sole child.
        Some(body)
    }
}

/// `node.immutable_literal?` — rubocop-ast `IMMUTABLE_LITERALS`
/// (`LITERALS - MUTABLE_LITERALS`): int / float / sym / dsym / true / false /
/// nil / complex / rational / regopt. `regopt` never appears as a freeze
/// receiver; `complex`/`rational` are prism `ImaginaryNode` / `RationalNode`.
fn is_immutable_literal_node(node: &Node<'_>) -> bool {
    node.as_integer_node().is_some()
        || node.as_float_node().is_some()
        || node.as_rational_node().is_some()
        || node.as_imaginary_node().is_some()
        || node.as_symbol_node().is_some()
        || node.as_interpolated_symbol_node().is_some()
        || node.as_true_node().is_some()
        || node.as_false_node().is_some()
        || node.as_nil_node().is_some()
}

/// `node.type?(:regexp, :range)`: `:regexp` covers both plain and interpolated
/// regexp; `:range` covers `irange` / `erange` (prism `RangeNode`).
fn is_regexp_or_range(node: &Node<'_>) -> bool {
    node.as_regular_expression_node().is_some()
        || node.as_interpolated_regular_expression_node().is_some()
        || node.as_range_node().is_some()
}

impl RedundantFreezeVisitor<'_> {
    /// `operation_produces_immutable_object?` — translated arm by arm. The
    /// `begin`-wrapped arms require a *parenthesized* receiver holding exactly
    /// one child; the count/length/size arms match a bare send or a
    /// send-with-block (parser-gem `send` / `any_block`, both `CallNode` in
    /// prism).
    fn operation_produces_immutable_object(&self, receiver: &Node<'_>) -> bool {
        // Arms 4 & 5: `(send _ {count length size} ...)` and
        // `(any_block (send _ {count length size} ...) ...)`. In prism both are
        // a bare `CallNode` (block attached or not); a parenthesized call is a
        // `ParenthesesNode` and does not match.
        if let Some(call) = receiver.as_call_node() {
            if is_size_method(call.name().as_slice()) {
                return true;
            }
            return false;
        }

        // Arms 1-3 need a parenthesized single-statement receiver whose child
        // is a plain call (no block).
        let Some(child) = parens_single_child(receiver) else {
            return false;
        };
        let Some(call) = child.as_call_node() else {
            return false;
        };
        if call.block().is_some() {
            return false;
        }
        let name = call.name();
        let name = name.as_slice();
        let args: Vec<Node<'_>> = call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        let recv = call.receiver();

        // Arm 1: `(send {float int} {:+ :- :* :** :/ :% :<<} _)` — one argument.
        if args.len() == 1
            && is_arm1_op(name)
            && recv
                .as_ref()
                .is_some_and(|r| r.as_integer_node().is_some() || r.as_float_node().is_some())
        {
            return true;
        }
        // Arm 2: `(send !{(str _) array} {:+ :- :* :** :/ :%} {float int})` —
        // receiver is not a string literal and not an array literal; one
        // numeric argument.
        if args.len() == 1 && is_arm2_op(name) {
            let recv_ok = match &recv {
                Some(r) => r.as_string_node().is_none() && r.as_array_node().is_none(),
                None => true,
            };
            let arg = &args[0];
            if recv_ok && (arg.as_integer_node().is_some() || arg.as_float_node().is_some()) {
                return true;
            }
        }
        // Arm 3: `(send _ {:== :=== :!= :<= :>= :< :>} _)` — one argument, any
        // receiver.
        if args.len() == 1 && is_comparison_op(name) {
            return true;
        }
        false
    }
}

/// `ParenthesesNode` holding exactly one child; `None` otherwise (zero or
/// multiple children, or not parenthesized).
fn parens_single_child<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let parens = node.as_parentheses_node()?;
    let body = parens.body()?;
    if let Some(stmts) = body.as_statements_node() {
        let mut it = stmts.body().iter();
        let first = it.next()?;
        if it.next().is_some() {
            return None;
        }
        Some(first)
    } else {
        Some(body)
    }
}

fn is_size_method(name: &[u8]) -> bool {
    matches!(name, b"count" | b"length" | b"size")
}

fn is_arm1_op(name: &[u8]) -> bool {
    matches!(
        name,
        b"+" | b"-" | b"*" | b"**" | b"/" | b"%" | b"<<"
    )
}

fn is_arm2_op(name: &[u8]) -> bool {
    matches!(name, b"+" | b"-" | b"*" | b"**" | b"/" | b"%")
}

fn is_comparison_op(name: &[u8]) -> bool {
    matches!(
        name,
        b"==" | b"===" | b"!=" | b"<=" | b">=" | b"<" | b">"
    )
}

impl<'pr, 's> Visit<'pr> for RedundantFreezeVisitor<'s> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        self.check_call(node);
        ruby_prism::visit_call_node(self, node);
    }
}

impl super::dispatch::Rule for RedundantFreezeVisitor<'_> {
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

    fn detect(src: &str, t30: bool, sfbd: bool) -> Vec<(usize, usize)> {
        check_redundant_freeze(src.as_bytes(), t30, sfbd)
            .into_iter()
            .map(|o| (o.off_start, o.off_end))
            .collect()
    }

    fn count(src: &str) -> usize {
        detect(src, true, false).len()
    }

    #[test]
    fn flags_immutable_literals() {
        for lit in ["1", "1.5", "1r", "1i", "1ri", ":sym", ":\"a\"", "true", "false", "nil"] {
            assert_eq!(count(&format!("{lit}.freeze\n")), 1, "{lit}");
        }
    }

    #[test]
    fn flags_interpolated_symbol() {
        assert_eq!(count(":\"a#{b}\".freeze\n"), 1);
    }

    #[test]
    fn does_not_flag_safe_navigation() {
        assert_eq!(count("x&.freeze\n"), 0);
    }

    #[test]
    fn does_not_flag_receiverless() {
        assert_eq!(count("freeze\n"), 0);
    }

    #[test]
    fn does_not_flag_method_call_receiver() {
        assert_eq!(count("Something.new.freeze\n"), 0);
        assert_eq!(count("ENV['foo'].freeze\n"), 0);
    }

    #[test]
    fn string_needs_frozen_enabled() {
        // No magic comment, sfbd=false -> not flagged.
        assert_eq!(detect("'str'.freeze\n", true, false).len(), 0);
        // sfbd=true -> flagged.
        assert_eq!(detect("\"\".freeze\n", true, true).len(), 1);
        // Magic comment true -> flagged even with sfbd=false.
        assert_eq!(
            detect("# frozen_string_literal: true\n'str'.freeze\n", true, false).len(),
            1
        );
        // Magic comment false wins over sfbd=true.
        assert_eq!(
            detect("# frozen_string_literal: false\n\"\".freeze\n", true, true).len(),
            0
        );
    }

    #[test]
    fn interpolated_string_target_gate() {
        let src = "# frozen_string_literal: true\n\"a#{x}\".freeze\n";
        // target >= 3.0: interpolated string is not a candidate.
        assert_eq!(detect(src, true, false).len(), 0);
        // target < 3.0: dstr counts.
        assert_eq!(detect(src, false, false).len(), 1);
    }

    #[test]
    fn adjacent_strings_uninterpolated() {
        // "a" "b" is dstr-with-all-str-parts -> candidate; flagged when enabled.
        assert_eq!(detect("\"a\" \"b\".freeze\n", true, true).len(), 1);
    }

    #[test]
    fn heredoc_plain_is_candidate() {
        assert_eq!(
            detect("# frozen_string_literal: true\n<<~HD.freeze\n  plain\nHD\n", true, false).len(),
            1
        );
    }

    #[test]
    fn heredoc_interpolated_not_candidate() {
        assert_eq!(
            detect("<<~HD.freeze\n  a#{x}\nHD\n", true, true).len(),
            0
        );
    }

    #[test]
    fn regexp_and_range_version_gate() {
        assert_eq!(detect("/re/.freeze\n", true, false).len(), 1);
        assert_eq!(detect("/re/.freeze\n", false, false).len(), 0);
        assert_eq!(detect("(1..2).freeze\n", true, false).len(), 1);
        assert_eq!(detect("(1..2).freeze\n", false, false).len(), 0);
        assert_eq!(detect("(1...2).freeze\n", true, false).len(), 1);
    }

    #[test]
    fn operation_arms() {
        for src in [
            "(1 + 2).freeze\n",
            "(1.5 * 2).freeze\n",
            "(1 << 2).freeze\n",
            "(2 > 1).freeze\n",
            "('a' > 'b').freeze\n",
            "(a > b).freeze\n",
            "(a + 1).freeze\n",
            "(1 + b).freeze\n",
            "(\"x#{y}\" + 1).freeze\n",
            "[1,2].count.freeze\n",
            "[1,2].size.freeze\n",
            "x.length.freeze\n",
            "x.count { it }.freeze\n",
            "x.count { _1 }.freeze\n",
            "x.size(arg).freeze\n",
            "count.freeze\n",
        ] {
            assert_eq!(count(src), 1, "{src}");
        }
    }

    #[test]
    fn operation_non_matches() {
        for src in [
            "('a' + 'b').freeze\n",
            "('a' * 20).freeze\n",
            "(a + b).freeze\n",
            "([42] * 42).freeze\n",
            "(a << 2).freeze\n",
            "(\"x\" + 1).freeze\n",
            "([1] + 1).freeze\n",
            "(a % b).freeze\n",
            "([1,2].count).freeze\n",
            "foo.count.bar.freeze\n",
        ] {
            assert_eq!(count(src), 0, "{src}");
        }
    }

    #[test]
    fn begin_strip_quirks() {
        // Empty parens: no offense.
        assert_eq!(count("().freeze\n"), 0);
        // Multi-statement begin strips to first child: int -> flagged.
        assert_eq!(count("(1; 2).freeze\n"), 1);
        // First child not immutable -> not flagged.
        assert_eq!(count("(a; b).freeze\n"), 0);
        // Double parens: outer child is a begin, not a send/literal.
        assert_eq!(count("((1+1)).freeze\n"), 0);
    }

    #[test]
    fn autocorrect_ranges_remove_dot_and_selector() {
        // `(1 + 2).freeze` -> dot at 7..8, selector 8..14, highlight 0..14.
        let offs = check_redundant_freeze("(1 + 2).freeze\n".as_bytes(), true, false);
        assert_eq!(offs.len(), 1);
        let o = &offs[0];
        assert_eq!((o.off_start, o.off_end), (0, 14));
        assert_eq!(&"(1 + 2).freeze\n".as_bytes()[o.dot_start..o.dot_end], b".");
        assert_eq!(
            &"(1 + 2).freeze\n".as_bytes()[o.selector_start..o.selector_end],
            b"freeze"
        );
    }

    #[test]
    fn freeze_with_block_highlight_excludes_block() {
        // `(1 + 2).freeze { }` -> highlight ends at selector, not the block.
        let offs = check_redundant_freeze("(1 + 2).freeze { }\n".as_bytes(), true, false);
        assert_eq!(offs.len(), 1);
        assert_eq!(offs[0].off_end, 14);
    }
}
