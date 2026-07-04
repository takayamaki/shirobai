//! `Performance/EndWith` (rubocop-performance): flags regex matches whose
//! pattern is literal-only and anchored at the END of the string
//! (`str.match?(/bc\z/)`, `/bc\z/ =~ str`, and `$` when `SafeMultiline` is
//! false) and rewrites them to `String#end_with?`.
//!
//! Mirrors
//! `vendor/rubocop-performance/lib/rubocop/cop/performance/end_with.rb`
//! (v1.26.1). Same pattern union as `Performance/StringInclude`, in document
//! order (order picks the rewrite receiver), but with a smaller method set
//! and an anchor gate instead of the plain literal-only gate:
//!
//! 1. `(call $!nil? {:match :=~ :match?} (regexp (str $#literal_at_end?) (regopt)))`
//!    — send or csend with a receiver, the ARGUMENT is the anchored regexp.
//! 2. `(send (regexp (str $#literal_at_end?) (regopt)) {:match :match?} $_)`
//!    — regexp-literal RECEIVER, plain send only (`/bc\z/&.match?(str)` never
//!    matches). The argument becomes the rewrite receiver.
//! 3. `({send match-with-lvasgn} (regexp ...) $_)` — a named-capture
//!    `=~` write. A named capture needs `(?<`, which can never be anchored
//!    literal-only, so this branch never fires; prism's `MatchWriteNode` is
//!    deliberately ignored.
//! 4. `(send (regexp (str $#literal_at_end?) (regopt)) :=~ $_)` — regexp
//!    receiver, `=~`.
//!
//! `RESTRICT_ON_SEND` is `[match, =~, match?]` — no `!~`, no `===` (the two
//! differences from `Performance/StringInclude`), and there is no negation.
//! Both sides accept exactly those three methods.
//!
//! `literal_at_end?` is stock's `\A(?:LITERAL_REGEX)+\z\z` (a `\z` anchor) or
//! `\A(?:LITERAL_REGEX)+$\z` (a `$` anchor, only when `SafeMultiline` is
//! false). Because the literal run is pinned between `\A` and the fixed
//! trailing anchor, this reduces exactly to "strip the trailing anchor, the
//! remainder is a non-empty `LITERAL_REGEX` run" — see [`literal_at_end`].
//!
//! The offense covers the whole node. The wrapper builds the replacement
//! (`recv.end_with?('bc')`) on the Ruby side with stock's own
//! `drop_end_metacharacter` / `interpret_string_escapes` / `to_string_literal`
//! helpers, from the RAW content bytes (anchor still attached), the
//! rewrite-receiver range and the dot token returned here.

use ruby_prism::{CallNode, Node, Visit};

use super::perf_string_include::literal_only;

#[derive(Debug, Clone)]
pub struct PerfEndWithOffense {
    /// Whole-node byte range: offense highlight and autocorrect replace
    /// target.
    pub start: usize,
    pub end: usize,
    /// Byte range of the node that becomes the `end_with?` receiver.
    pub recv_start: usize,
    pub recv_end: usize,
    /// The dot token (`.`, `&.`, `::`) or `.` when the node has none.
    pub dot: String,
    /// Raw regexp content bytes, anchor still attached (the wrapper runs
    /// stock's `drop_end_metacharacter` + `interpret_string_escapes`).
    pub content: String,
}

/// Standalone entry point used by the per-cop fallback.
pub fn check_perf_end_with(source: &[u8], safe_multiline: bool) -> Vec<PerfEndWithOffense> {
    let mut visitor = build_rule(safe_multiline);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in the shared-walk bundle.
/// `safe_multiline` is `Performance/EndWith` `SafeMultiline`: when true
/// (default) only `\z` anchors count; when false `$` counts too.
pub(crate) fn build_rule(safe_multiline: bool) -> PerfEndWithVisitor {
    PerfEndWithVisitor {
        safe_multiline,
        offenses: Vec::new(),
    }
}

pub(crate) struct PerfEndWithVisitor {
    safe_multiline: bool,
    pub(crate) offenses: Vec<PerfEndWithOffense>,
}

/// Raw regexp source content when `node` is a non-interpolated regexp literal
/// with exactly one content part and NO flags (empty `(regopt)`; prism's
/// `closing_loc` is the 1-byte delimiter plus any flag characters). Unlike
/// `perf_string_include::regexp_content`, the anchor/literal gate is applied
/// by the caller, so the anchor bytes survive here.
fn regexp_raw_content(node: &Node<'_>) -> Option<String> {
    let regexp = node.as_regular_expression_node()?;
    if regexp.closing_loc().as_slice().len() != 1 {
        return None;
    }
    Some(String::from_utf8_lossy(regexp.content_loc().as_slice()).into_owned())
}

/// Stock `literal_at_end?`: literal-only content anchored with `\z`, or with
/// `$` when `SafeMultiline` is false.
///
/// The stock regexps are `\A(?:LITERAL)+\\z\z` and `\A(?:LITERAL)+\$\z`.
/// Both pin the `(?:LITERAL)+` run between the absolute start anchor `\A` and
/// a fixed trailing anchor (literal `\z`, i.e. 2 bytes backslash-`z`, or a
/// literal `$`), so the match is unique: the anchor must be the final byte(s)
/// and the rest must be a non-empty `LITERAL_REGEX` run. That is exactly
/// "strip the trailing anchor, then `literal_only` the remainder". An escaped
/// backslash before `z` (e.g. `a\\z`) leaves a dangling backslash in the
/// prefix, which `literal_only` rejects — matching stock.
fn literal_at_end(content: &str, safe_multiline: bool) -> bool {
    content.strip_suffix("\\z").is_some_and(literal_only)
        || (!safe_multiline && content.strip_suffix('$').is_some_and(literal_only))
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

impl PerfEndWithVisitor {
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
        // send only — `/bc\z/&.match?(str)` never matches), and the argument
        // becomes the rewrite receiver.
        let branch1 = regexp_raw_content(&arg).filter(|c| literal_at_end(c, self.safe_multiline));
        let (recv_loc, content) = if let Some(content) = branch1 {
            (receiver.location(), content)
        } else {
            if is_csend(node) {
                return;
            }
            let Some(content) =
                regexp_raw_content(&receiver).filter(|c| literal_at_end(c, self.safe_multiline))
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
        self.offenses.push(PerfEndWithOffense {
            start: loc.start_offset(),
            end: loc.end_offset(),
            recv_start: recv_loc.start_offset(),
            recv_end: recv_loc.end_offset(),
            dot,
            content,
        });
    }
}

impl<'pr> Visit<'pr> for PerfEndWithVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        ruby_prism::visit_call_node(self, node);
    }
}

impl super::dispatch::Rule for PerfEndWithVisitor {
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

    fn detect(src: &str, safe_multiline: bool) -> Vec<PerfEndWithOffense> {
        check_perf_end_with(src.as_bytes(), safe_multiline)
    }

    fn spans(src: &str, safe_multiline: bool) -> Vec<(usize, usize)> {
        detect(src, safe_multiline)
            .into_iter()
            .map(|o| (o.start, o.end))
            .collect()
    }

    // Expectations are stock-derived (rubocop-performance 1.26.1 probed via
    // .tmp/2026-07-05/probe-perf/endwith_sm_{true,false}.rb).

    #[test]
    fn flags_match_q_with_backslash_z() {
        let off = detect("str.match?(/abc\\z/)\n", true);
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 19));
        assert_eq!((o.recv_start, o.recv_end), (0, 3)); // `str`
        assert_eq!(o.dot, ".");
        assert_eq!(o.content, "abc\\z"); // raw, anchor attached
    }

    #[test]
    fn flags_regexp_receiver_match_q() {
        let off = detect("/abc\\z/.match?(str)\n", true);
        assert_eq!(off.len(), 1);
        let o = &off[0];
        // The rewrite receiver is the ARGUMENT (`str`).
        assert_eq!((o.recv_start, o.recv_end), (15, 18));
        assert_eq!(o.dot, ".");
        assert_eq!(o.content, "abc\\z");
    }

    #[test]
    fn flags_operator_match_and_regexp_receiver() {
        let off = detect("str =~ /abc\\z/\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].start, off[0].end), (0, 14));
        assert_eq!((off[0].recv_start, off[0].recv_end), (0, 3));
        assert_eq!(off[0].dot, "."); // no dot token on operator sends

        let off = detect("/abc\\z/ =~ str\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].start, off[0].end), (0, 14));
        assert_eq!((off[0].recv_start, off[0].recv_end), (11, 14));
    }

    #[test]
    fn flags_match_send() {
        assert_eq!(spans("str.match(/abc\\z/)\n", true), vec![(0, 18)]);
    }

    #[test]
    fn flags_csend_argument_side() {
        let off = detect("str&.match /abc\\z/\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].dot, "&.");
        assert_eq!(off[0].content, "abc\\z");
    }

    #[test]
    fn accepts_csend_on_regexp_receiver() {
        // Branches 2/4 are `send`-only; a csend receiver never matches.
        assert!(spans("/abc\\z/&.match?(str)\n", true).is_empty());
    }

    #[test]
    fn accepts_triple_equals_and_negation() {
        // `===` and `!~` are not in `RESTRICT_ON_SEND`.
        assert!(spans("/abc\\z/ === str\n", true).is_empty());
        assert!(spans("str !~ /abc\\z/\n", true).is_empty());
    }

    #[test]
    fn accepts_receiverless_call() {
        assert!(spans("match(/abc\\z/)\n", true).is_empty());
    }

    #[test]
    fn dollar_anchor_is_gated_by_safe_multiline() {
        // `$` counts only when SafeMultiline is false.
        assert!(spans("str.match?(/abc$/)\n", true).is_empty());
        assert_eq!(spans("str.match?(/abc$/)\n", false), vec![(0, 18)]);
        assert_eq!(spans("/abc$/ =~ str\n", false), vec![(0, 13)]);
        // A `\z` anchor still counts under SafeMultiline false.
        assert_eq!(spans("str.match?(/abc\\z/)\n", false), vec![(0, 19)]);
    }

    #[test]
    fn accepts_escaped_backslash_before_z() {
        // `/a\\z/` is an escaped backslash then a literal `z`, NOT a `\z`
        // anchor: the prefix `a\` is a dangling backslash, rejected.
        assert!(spans("str.match?(/a\\\\z/)\n", true).is_empty());
    }

    #[test]
    fn accepts_bare_anchor_without_prefix() {
        // `(?:LITERAL)+` needs at least one char before the anchor.
        assert!(spans("str.match?(/\\z/)\n", true).is_empty());
        assert!(spans("str.match?(/$/)\n", false).is_empty());
    }

    #[test]
    fn accepts_non_terminal_dollar() {
        // A `$` not at the very end is not an anchor, and is not a literal.
        assert!(spans("str.match?(/a$b\\z/)\n", true).is_empty());
        assert!(spans("str.match?(/a$b/)\n", false).is_empty());
    }

    #[test]
    fn flags_escaped_metachar_before_anchor() {
        // `/a\$\z/`: the escaped `$` is a literal, so the prefix `a\$` is
        // literal-only and the trailing `\z` anchors it. Raw content keeps
        // the anchor for the wrapper to drop.
        let off = detect("str.match?(/a\\$\\z/)\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].start, off[0].end), (0, 19));
        assert_eq!(off[0].content, "a\\$\\z");
    }

    #[test]
    fn branch_order_prefers_argument_regexp() {
        // `/x\z/.match?(/ab\z/)`: branch 1 (arg) wins — receiver stays
        // `/x\z/` and the content comes from the argument.
        let off = detect("/x\\z/.match?(/ab\\z/)\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].recv_start, off[0].recv_end), (0, 5)); // `/x\z/`
        assert_eq!(off[0].content, "ab\\z");

        // `/ab\z/.match?(/xy/)`: the argument is not anchored, so branch 2
        // fires — content from the receiver, rewrite receiver is the arg.
        let off = detect("/ab\\z/.match?(/xy/)\n", true);
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].recv_start, off[0].recv_end), (14, 18)); // `/xy/`
        assert_eq!(off[0].content, "ab\\z");
    }

    #[test]
    fn literal_at_end_equivalence() {
        // \z anchor with a literal run.
        assert!(literal_at_end("abc\\z", true));
        assert!(literal_at_end("\\n\\z", true)); // \n escape is a literal
        assert!(literal_at_end("a\\$\\z", true)); // escaped metachar literal
        assert!(literal_at_end("\\\\\\z", true)); // escaped backslash + anchor
        // Escaped backslash before z leaves a dangling backslash prefix.
        assert!(!literal_at_end("a\\\\z", true));
        // No prefix before the anchor.
        assert!(!literal_at_end("\\z", true));
        // `\Z` (capital) is a different anchor and not literal.
        assert!(!literal_at_end("abc\\Z", true));
        // Bare multibyte is not an ASCII literal.
        assert!(!literal_at_end("\u{3042}\\z", true));
        // `$` only under SafeMultiline false.
        assert!(!literal_at_end("abc$", true));
        assert!(literal_at_end("abc$", false));
        assert!(!literal_at_end("$", false)); // no prefix
        assert!(!literal_at_end("a$b", false)); // `$` not terminal
    }
}
