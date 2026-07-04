//! `Performance/StartWith` (rubocop-performance): flags regex matches whose
//! pattern is literal-only and anchored at the START of the string
//! (`str.match?(/\Aab/)`, `/\Aab/ =~ str`, and `^` when `SafeMultiline` is
//! false) and rewrites them to `String#start_with?`.
//!
//! Mirror image of [`super::perf_end_with`]: same pattern union and method
//! set (`[match, =~, match?]`, no `!~` / `===`, no negation, regexp-receiver
//! branches are `send`-only), but the gate is `literal_at_start?` and the
//! anchor sits at the FRONT of the content. The wrapper drops the anchor with
//! stock's `drop_start_metacharacter` and rebuilds
//! `recv.start_with?('ab')` with `interpret_string_escapes` /
//! `to_string_literal`; the RAW content bytes (anchor attached) are carried
//! on the wire.

use ruby_prism::{CallNode, Node, Visit};

use super::perf_string_include::literal_only;

#[derive(Debug, Clone)]
pub struct PerfStartWithOffense {
    /// Whole-node byte range: offense highlight and autocorrect replace
    /// target.
    pub start: usize,
    pub end: usize,
    /// Byte range of the node that becomes the `start_with?` receiver.
    pub recv_start: usize,
    pub recv_end: usize,
    /// The dot token (`.`, `&.`, `::`) or `.` when the node has none.
    pub dot: String,
    /// Raw regexp content bytes, anchor still attached (the wrapper runs
    /// stock's `drop_start_metacharacter` + `interpret_string_escapes`).
    pub content: String,
}

/// Standalone entry point used by the per-cop fallback.
pub fn check_perf_start_with(source: &[u8], safe_multiline: bool) -> Vec<PerfStartWithOffense> {
    let mut visitor = build_rule(safe_multiline);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in the shared-walk bundle.
/// `safe_multiline` is `Performance/StartWith` `SafeMultiline`: when true
/// (default) only `\A` anchors count; when false `^` counts too.
pub(crate) fn build_rule(safe_multiline: bool) -> PerfStartWithVisitor {
    PerfStartWithVisitor {
        safe_multiline,
        offenses: Vec::new(),
    }
}

pub(crate) struct PerfStartWithVisitor {
    safe_multiline: bool,
    pub(crate) offenses: Vec<PerfStartWithOffense>,
}

/// Raw regexp source content when `node` is a non-interpolated regexp literal
/// with exactly one content part and NO flags (empty `(regopt)`; prism's
/// `closing_loc` is the 1-byte delimiter plus any flag characters). The
/// anchor/literal gate is applied by the caller, so the anchor bytes survive.
fn regexp_raw_content(node: &Node<'_>) -> Option<String> {
    let regexp = node.as_regular_expression_node()?;
    if regexp.closing_loc().as_slice().len() != 1 {
        return None;
    }
    Some(String::from_utf8_lossy(regexp.content_loc().as_slice()).into_owned())
}

/// Stock `literal_at_start?`: literal-only content anchored with `\A`, or
/// with `^` when `SafeMultiline` is false.
///
/// The stock regexps are `\A\\A(?:LITERAL)+\z` and `\A\^(?:LITERAL)+\z`.
/// Both pin the `(?:LITERAL)+` run between a fixed leading anchor (literal
/// `\A`, i.e. 2 bytes backslash-`A`, or a literal `^`) and the absolute end
/// anchor `\z`, so the match is unique: the anchor must be the first byte(s)
/// and the rest must be a non-empty `LITERAL_REGEX` run. That is exactly
/// "strip the leading anchor, then `literal_only` the remainder". An escaped
/// backslash start (`/\\Aabc/`, bytes backslash-backslash-`A`) never strips:
/// the second byte is a backslash, not `A`, so the arm fails — matching
/// stock, whose pattern needs the literal 2-byte `\A` anchor first.
fn literal_at_start(content: &str, safe_multiline: bool) -> bool {
    content.strip_prefix("\\A").is_some_and(literal_only)
        || (!safe_multiline && content.strip_prefix('^').is_some_and(literal_only))
}

fn is_csend(call: &CallNode<'_>) -> bool {
    call.call_operator_loc()
        .is_some_and(|l| l.as_slice() == b"&.")
}

/// Exactly one positional argument and no block-pass (a parser block-pass is
/// an argument and breaks the one-arg patterns; a literal block is not).
fn sole_argument<'pr>(call: &CallNode<'pr>) -> Option<Node<'pr>> {
    if matches!(call.block(), Some(b) if b.as_block_argument_node().is_some()) {
        return None;
    }
    let args = call.arguments()?;
    let args = args.arguments();
    if args.iter().count() != 1 {
        return None;
    }
    args.iter().next()
}

impl PerfStartWithVisitor {
    fn check_call(&mut self, node: &CallNode<'_>) {
        let name = node.name();
        let name = name.as_slice();
        if !matches!(name, b"match" | b"=~" | b"match?") {
            return;
        }
        let Some(receiver) = node.receiver() else {
            return;
        };
        let Some(arg) = sole_argument(node) else {
            return;
        };

        // Branch 1 first (stock union order): any non-nil receiver, the
        // ARGUMENT is the anchored literal regexp, send or csend. Otherwise
        // branches 2 / 4: the RECEIVER is the anchored literal regexp (plain
        // send only — `/\Aab/&.match?(str)` never matches), and the argument
        // becomes the rewrite receiver.
        let branch1 = regexp_raw_content(&arg).filter(|c| literal_at_start(c, self.safe_multiline));
        let (recv_loc, content) = if let Some(content) = branch1 {
            (receiver.location(), content)
        } else {
            if is_csend(node) {
                return;
            }
            let Some(content) =
                regexp_raw_content(&receiver).filter(|c| literal_at_start(c, self.safe_multiline))
            else {
                return;
            };
            (arg.location(), content)
        };

        let dot = node
            .call_operator_loc()
            .map(|l| String::from_utf8_lossy(l.as_slice()).into_owned())
            .unwrap_or_else(|| ".".to_string());

        let loc = node.location();
        self.offenses.push(PerfStartWithOffense {
            start: loc.start_offset(),
            end: loc.end_offset(),
            recv_start: recv_loc.start_offset(),
            recv_end: recv_loc.end_offset(),
            dot,
            content,
        });
    }
}

impl<'pr> Visit<'pr> for PerfStartWithVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        ruby_prism::visit_call_node(self, node);
    }
}

impl super::dispatch::Rule for PerfStartWithVisitor {
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

    fn detect(src: &str, safe_multiline: bool) -> Vec<PerfStartWithOffense> {
        check_perf_start_with(src.as_bytes(), safe_multiline)
    }

    fn spans(src: &str, safe_multiline: bool) -> Vec<(usize, usize)> {
        detect(src, safe_multiline)
            .into_iter()
            .map(|o| (o.start, o.end))
            .collect()
    }

    // Expectations are stock-derived (rubocop-performance 1.26.1 probed via
    // .tmp/2026-07-05/probe-perf/startwith_sm_{true,false}.rb).

    #[test]
    fn flags_match_q_with_backslash_a() {
        let off = detect("str.match?(/\\Aabc/)\n", true);
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 19));
        assert_eq!((o.recv_start, o.recv_end), (0, 3)); // `str`
        assert_eq!(o.dot, ".");
        assert_eq!(o.content, "\\Aabc"); // raw, anchor attached
    }

    #[test]
    fn flags_regexp_receiver_match_q() {
        let off = detect("/\\Aabc/.match?(str)\n", true);
        assert_eq!(off.len(), 1);
        let o = &off[0];
        // The rewrite receiver is the ARGUMENT (`str`).
        assert_eq!((o.recv_start, o.recv_end), (15, 18));
        assert_eq!(o.dot, ".");
        assert_eq!(o.content, "\\Aabc");
    }

    #[test]
    fn flags_operator_match_and_regexp_receiver() {
        let off = detect("str =~ /\\Aabc/\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].start, off[0].end), (0, 14));
        assert_eq!((off[0].recv_start, off[0].recv_end), (0, 3));
        assert_eq!(off[0].dot, ".");

        let off = detect("/\\Aabc/ =~ str\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].start, off[0].end), (0, 14));
        assert_eq!((off[0].recv_start, off[0].recv_end), (11, 14));
    }

    #[test]
    fn flags_match_send() {
        assert_eq!(spans("str.match(/\\Aabc/)\n", true), vec![(0, 18)]);
    }

    #[test]
    fn flags_csend_argument_side() {
        let off = detect("str&.match /\\Aabc/\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].dot, "&.");
        assert_eq!(off[0].content, "\\Aabc");
    }

    #[test]
    fn accepts_csend_on_regexp_receiver() {
        assert!(spans("/\\Aabc/&.match?(str)\n", true).is_empty());
    }

    #[test]
    fn accepts_triple_equals_and_negation() {
        assert!(spans("/\\Aabc/ === str\n", true).is_empty());
        assert!(spans("str !~ /\\Aabc/\n", true).is_empty());
    }

    #[test]
    fn accepts_receiverless_call() {
        assert!(spans("match(/\\Aabc/)\n", true).is_empty());
    }

    #[test]
    fn caret_anchor_is_gated_by_safe_multiline() {
        assert!(spans("str.match?(/^abc/)\n", true).is_empty());
        assert_eq!(spans("str.match?(/^abc/)\n", false), vec![(0, 18)]);
        assert_eq!(spans("/^abc/ =~ str\n", false), vec![(0, 13)]);
        // A `\A` anchor still counts under SafeMultiline false.
        assert_eq!(spans("str.match?(/\\Aabc/)\n", false), vec![(0, 19)]);
    }

    #[test]
    fn accepts_escaped_backslash_before_a() {
        // `/\\Aabc/` is an escaped backslash then a literal `A`, NOT a `\A`
        // anchor: the remainder starts with a bare `A` after the backslash.
        assert!(spans("str.match?(/\\\\Aabc/)\n", true).is_empty());
    }

    #[test]
    fn accepts_bare_anchor_without_suffix() {
        assert!(spans("str.match?(/\\A/)\n", true).is_empty());
        assert!(spans("str.match?(/^/)\n", false).is_empty());
    }

    #[test]
    fn accepts_non_initial_caret() {
        assert!(spans("str.match?(/\\Aa^b/)\n", true).is_empty());
        assert!(spans("str.match?(/a^b/)\n", false).is_empty());
    }

    #[test]
    fn flags_escaped_metachar_after_anchor() {
        // `/\A\^/`: escaped caret is a literal, so `\^` after the anchor is
        // literal-only. Raw content keeps the anchor for the wrapper.
        let off = detect("str.match?(/\\A\\^/)\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].start, off[0].end), (0, 18));
        assert_eq!(off[0].content, "\\A\\^");
    }

    #[test]
    fn branch_order_prefers_argument_regexp() {
        // `/\Ax/.match?(/\Aab/)`: branch 1 (arg) wins — receiver stays
        // `/\Ax/` and the content comes from the argument.
        let off = detect("/\\Ax/.match?(/\\Aab/)\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].recv_start, off[0].recv_end), (0, 5)); // `/\Ax/`
        assert_eq!(off[0].content, "\\Aab");

        // `/\Aab/.match?(/xy/)`: argument not anchored, so branch 2 fires.
        let off = detect("/\\Aab/.match?(/xy/)\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].recv_start, off[0].recv_end), (14, 18)); // `/xy/`
        assert_eq!(off[0].content, "\\Aab");
    }

    #[test]
    fn literal_at_start_equivalence() {
        assert!(literal_at_start("\\Aabc", true));
        assert!(literal_at_start("\\A\\n", true)); // \n escape is a literal
        assert!(literal_at_start("\\A\\^", true)); // escaped metachar literal
        assert!(literal_at_start("\\A\\\\", true)); // anchor + escaped backslash
        // Escaped backslash then a bare `A` is not the `\A` anchor.
        assert!(!literal_at_start("\\\\Aabc", true));
        // No suffix after the anchor.
        assert!(!literal_at_start("\\A", true));
        // Bare multibyte is not an ASCII literal.
        assert!(!literal_at_start("\\A\u{3042}", true));
        // `^` only under SafeMultiline false.
        assert!(!literal_at_start("^abc", true));
        assert!(literal_at_start("^abc", false));
        assert!(!literal_at_start("^", false)); // no suffix
        assert!(!literal_at_start("a^b", false)); // `^` not initial
    }
}
