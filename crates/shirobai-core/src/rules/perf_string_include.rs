//! `Performance/StringInclude` (rubocop-performance): flags regex matches
//! whose pattern is literal-only (`str.match?(/ab/)`, `/ab/ =~ str`, ...)
//! and rewrites them to `String#include?`.
//!
//! Mirrors
//! `vendor/rubocop-performance/lib/rubocop/cop/performance/string_include.rb`
//! (v1.26.1). The stock pattern union, in document order (order matters for
//! which side becomes the rewrite receiver):
//!
//! 1. `(call $!nil? {:match :=~ :!~ :match?} (regexp (str $#literal?) (regopt)))`
//!    — send or csend with a receiver, exactly one literal-regexp argument.
//! 2. `(send (regexp ...) {:match :match? :===} $_)` — regexp-literal
//!    receiver, plain send only (`/ab/&.match?(str)` never matches).
//! 3. `(match-with-lvasgn (regexp ...) $_)` — `/re/ =~ str` with named
//!    captures. A named capture needs `(?<`, which can never be
//!    literal-only, so this branch never produces an offense; prism's
//!    `MatchWriteNode` is deliberately ignored.
//! 4. `(send (regexp ...) :=~ $_)` — regexp-literal receiver, `=~`.
//!
//! A literal regexp is a non-interpolated `RegularExpressionNode` with
//! exactly one content part, NO flags (the parser `(regopt)` must be empty
//! — prism's `closing_loc` carries the flag characters after the 1-byte
//! delimiter), and a content that matches
//! `\A(?:LITERAL_REGEX)+\z` (RuboCop `Util::LITERAL_REGEX`:
//! `[\w\s\-,"'!#%&<>=;:`~/]|\\[^AbBdDgGhHkpPRwWXsSzZ0-9]`, ASCII `\w`/`\s`).
//!
//! `negation` is stock's `node.send_type? && node.method?(:!~)` — true only
//! for a plain-send `!~` (branch 1); `/ab/ !~ str` never matches any branch.
//!
//! The offense covers the whole node. The wrapper builds the replacement
//! (`!recv.include?('ab')`) on the Ruby side with stock's own
//! `interpret_string_escapes` / `to_string_literal` helpers, from the raw
//! content bytes, the rewrite-receiver range and the dot token returned
//! here (`.` when the node has no dot, e.g. operator sends).

use ruby_prism::{CallNode, Node, RegularExpressionNode, Visit};

#[derive(Debug, Clone)]
pub struct PerfStringIncludeOffense {
    /// Whole-node byte range: offense highlight and autocorrect replace
    /// target.
    pub start: usize,
    pub end: usize,
    /// Stock `negation` (`!~` plain send): prefixes the rewrite with `!`
    /// and switches the message.
    pub negation: bool,
    /// Byte range of the node that becomes the `include?` receiver (the
    /// string side after stock's swap).
    pub recv_start: usize,
    pub recv_end: usize,
    /// The dot token (`.`, `&.`, `::`) or `.` when the node has none.
    pub dot: String,
    /// Raw regexp content bytes (escapes NOT interpreted; the wrapper runs
    /// stock's `interpret_string_escapes`).
    pub content: String,
}

/// Standalone entry point used by the per-cop fallback.
pub fn check_perf_string_include(source: &[u8]) -> Vec<PerfStringIncludeOffense> {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in the shared-walk bundle.
/// Config-less: the cop has no options beyond Enabled/Safe*.
pub(crate) fn build_rule() -> PerfStringIncludeVisitor {
    PerfStringIncludeVisitor {
        offenses: Vec::new(),
    }
}

pub(crate) struct PerfStringIncludeVisitor {
    pub(crate) offenses: Vec<PerfStringIncludeOffense>,
}

/// RuboCop `Util::LITERAL_REGEX` over the raw regexp source, anchored and
/// repeated (`\A(?:...)+\z`): ASCII word/space chars, the listed literal
/// punctuation, or a backslash escape whose next CHAR is not one of the
/// regexp metacharacter escapes (`\A \b \B \d \D \g \G \h \H \k \p \P \R
/// \w \W \X \s \S \z \Z` or a back-reference digit). The negated escape
/// class matches any other char, multibyte included.
pub(crate) fn literal_only(content: &str) -> bool {
    if content.is_empty() {
        return false;
    }
    let mut chars = content.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let Some(next) = chars.next() else {
                return false;
            };
            if matches!(
                next,
                'A' | 'b' | 'B' | 'd' | 'D' | 'g' | 'G' | 'h' | 'H' | 'k' | 'p' | 'P' | 'R'
                    | 'w' | 'W' | 'X' | 's' | 'S' | 'z' | 'Z' | '0'..='9'
            ) {
                return false;
            }
        } else {
            let literal_plain = c.is_ascii_alphanumeric()
                || c == '_'
                || matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0c' | '\x0b')
                || matches!(
                    c,
                    '-' | ',' | '"' | '\'' | '!' | '#' | '%' | '&' | '<' | '>' | '=' | ';'
                        | ':' | '`' | '~' | '/'
                );
            if !literal_plain {
                return false;
            }
        }
    }
    true
}

/// The regexp side of every branch: a non-interpolated regexp literal with
/// one content part, no flags, and literal-only content. Returns the raw
/// content when it qualifies.
pub(crate) fn literal_regexp_content(node: &Node<'_>) -> Option<String> {
    let regexp = node.as_regular_expression_node()?;
    regexp_content(&regexp)
}

pub(crate) fn regexp_content(regexp: &RegularExpressionNode<'_>) -> Option<String> {
    // `(regopt)` must be empty: prism's closing_loc is the 1-byte delimiter
    // plus any flag characters.
    if regexp.closing_loc().as_slice().len() != 1 {
        return None;
    }
    let content = regexp.content_loc().as_slice();
    let content = String::from_utf8_lossy(content).into_owned();
    if literal_only(&content) {
        Some(content)
    } else {
        None
    }
}

fn is_csend(call: &CallNode<'_>) -> bool {
    call.call_operator_loc()
        .is_some_and(|l| l.as_slice() == b"&.")
}

/// Exactly one positional argument and no block-pass (a parser block-pass
/// is an argument and breaks the one-arg patterns; a literal block is not).
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

impl PerfStringIncludeVisitor {
    fn check_call(&mut self, node: &CallNode<'_>) {
        let name = node.name();
        let name = name.as_slice();
        if !matches!(name, b"match" | b"=~" | b"!~" | b"match?" | b"===") {
            return;
        }
        let Some(receiver) = node.receiver() else {
            return;
        };
        let Some(arg) = sole_argument(node) else {
            return;
        };

        // Branch 1 first (stock union order): any receiver, the ARGUMENT is
        // the literal regexp, send or csend, method != `===`. Otherwise
        // branches 2 / 4: the RECEIVER is the literal regexp (plain send
        // only — `/ab/&.match?(str)` never matches — and `!~` has no
        // regexp-receiver branch), and the argument becomes the rewrite
        // receiver.
        let branch1 = if matches!(name, b"match" | b"=~" | b"!~" | b"match?") {
            literal_regexp_content(&arg)
        } else {
            None
        };
        let (recv_loc, content) = if let Some(content) = branch1 {
            (receiver.location(), content)
        } else {
            if is_csend(node) || name == b"!~" {
                return;
            }
            let Some(content) = literal_regexp_content(&receiver) else {
                return;
            };
            (arg.location(), content)
        };

        let negation = !is_csend(node) && name == b"!~";
        let dot = node
            .call_operator_loc()
            .map(|l| String::from_utf8_lossy(l.as_slice()).into_owned())
            .unwrap_or_else(|| ".".to_string());

        let loc = node.location();
        self.offenses.push(PerfStringIncludeOffense {
            start: loc.start_offset(),
            end: loc.end_offset(),
            negation,
            recv_start: recv_loc.start_offset(),
            recv_end: recv_loc.end_offset(),
            dot,
            content,
        });
    }
}

impl<'pr> Visit<'pr> for PerfStringIncludeVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        ruby_prism::visit_call_node(self, node);
    }
}

impl super::dispatch::Rule for PerfStringIncludeVisitor {
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

    fn detect(src: &str) -> Vec<PerfStringIncludeOffense> {
        check_perf_string_include(src.as_bytes())
    }

    fn spans(src: &str) -> Vec<(usize, usize)> {
        detect(src).into_iter().map(|o| (o.start, o.end)).collect()
    }

    // Expectations are stock-derived (rubocop-performance 1.26.1 probed via
    // .tmp/2026-07-05/probe-perf).

    #[test]
    fn flags_match_q_with_literal_regexp() {
        let off = detect("str.match?(/ab/)\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 16));
        assert!(!o.negation);
        assert_eq!((o.recv_start, o.recv_end), (0, 3)); // `str`
        assert_eq!(o.dot, ".");
        assert_eq!(o.content, "ab");
    }

    #[test]
    fn flags_regexp_receiver_match_q() {
        let off = detect("/ab/.match?(str)\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        // The rewrite receiver is the ARGUMENT (`str`).
        assert_eq!((o.recv_start, o.recv_end), (12, 15));
        assert_eq!(o.dot, ".");
        assert_eq!(o.content, "ab");
    }

    #[test]
    fn flags_operator_match() {
        let off = detect("str =~ /ab/\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 11));
        assert_eq!((o.recv_start, o.recv_end), (0, 3));
        assert_eq!(o.dot, "."); // no dot token on operator sends
    }

    #[test]
    fn flags_regexp_receiver_operator_match() {
        let off = detect("/ab/ =~ str\n");
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].recv_start, off[0].recv_end), (8, 11));
    }

    #[test]
    fn flags_match_and_triple_equals() {
        assert_eq!(spans("str.match(/ab/)\n"), vec![(0, 15)]);
        assert_eq!(spans("/ab/ === str\n"), vec![(0, 12)]);
    }

    #[test]
    fn flags_negation() {
        let off = detect("str !~ /ab/\n");
        assert_eq!(off.len(), 1);
        assert!(off[0].negation);
    }

    #[test]
    fn accepts_regexp_receiver_negation() {
        // `!~` has no regexp-receiver branch in the stock pattern.
        assert!(spans("/ab/ !~ str\n").is_empty());
    }

    #[test]
    fn flags_csend_with_amp_dot() {
        let off = detect("str&.match?(/ab/)\n");
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].dot, "&.");
        assert!(!off[0].negation);
    }

    #[test]
    fn accepts_csend_on_regexp_receiver() {
        assert!(spans("/ab/&.match?(str)\n").is_empty());
        assert!(spans("/ab/&.match(str)\n").is_empty());
    }

    #[test]
    fn accepts_non_literal_patterns() {
        assert!(spans("str.match?(/a.b/)\n").is_empty());
        assert!(spans("str.match?(/ab/i)\n").is_empty());
        assert!(spans("str.match?(/a\u{3042}b/)\n").is_empty()); // multibyte
        assert!(spans("str.match?(/a\\db/)\n").is_empty()); // \d metachar
        assert!(spans("str.match?(//)\n").is_empty()); // empty
    }

    #[test]
    fn accepts_interpolated_regexp() {
        assert!(spans("str.match?(/a#{x}b/)\n").is_empty());
    }

    #[test]
    fn flags_escaped_literal_content() {
        let off = detect("str.match?(/a\\.b/)\n");
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].content, "a\\.b"); // raw bytes, uninterpreted
    }

    #[test]
    fn flags_percent_r_forms() {
        let off = detect("str.match?(%r{a/b})\n");
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].content, "a/b");
    }

    #[test]
    fn accepts_extra_arguments() {
        assert!(spans("str.match(/ab/, 1)\n").is_empty());
        assert!(spans("/ab/.match(str, 1)\n").is_empty());
    }

    #[test]
    fn accepts_receiverless_call() {
        assert!(spans("match?(/ab/)\n").is_empty());
    }

    #[test]
    fn flags_symbol_receiver() {
        // Stock flags any non-nil receiver; the rewrite is semantically wrong
        // for symbols (SafeAutoCorrect: false covers it) but must byte-match.
        assert_eq!(spans(":sym.match?(/ab/)\n"), vec![(0, 17)]);
    }

    #[test]
    fn branch_order_prefers_argument_regexp() {
        // `/xy/.match?(/ab/)`: branch 1 wins (stock union order) — the
        // receiver stays `/xy/` and the content comes from the argument.
        let off = detect("/xy/.match?(/ab/)\n");
        assert_eq!(off.len(), 1);
        assert_eq!((off[0].recv_start, off[0].recv_end), (0, 4));
        assert_eq!(off[0].content, "ab");
    }

    #[test]
    fn literal_only_matches_stock_util_regex() {
        assert!(literal_only("ab"));
        assert!(literal_only("a b"));
        assert!(literal_only("a-b,c\"d'e"));
        assert!(literal_only("a`~b"));
        assert!(literal_only("a\\nb")); // \n escape → escape alternative
        assert!(literal_only("a\\\\b"));
        assert!(literal_only("a\\/b"));
        assert!(literal_only("a\\\u{3042}b")); // escaped multibyte char
        assert!(!literal_only("a\u{3042}b")); // bare multibyte
        assert!(!literal_only("a.b"));
        assert!(!literal_only("a\\db"));
        assert!(!literal_only("a\\1b")); // back-reference
        assert!(!literal_only(""));
        assert!(!literal_only("ab\\")); // dangling backslash
        assert!(!literal_only("(?<x>ab)"));
    }
}
