//! `Style/FileNull`.
//!
//! Flags string literals that are exactly the hardcoded null device
//! (`/dev/null`, `NUL`, or `NUL:`, case-insensitive, full match) and rewrites
//! them to `File::NULL`. Two stock quirks drive the shape:
//!
//! 1. **File-level gate for `NUL` / `nul`.** A bare `nul` (case-insensitive) is
//!    only flagged when the file ALSO contains a `/dev/null` string literal
//!    somewhere (`@contain_dev_null_string_in_file`, computed by stock's
//!    `on_new_investigation` over every `:str` descendant). `NUL:` has no such
//!    gate. Because a gated `nul` can appear before the `/dev/null` that
//!    unlocks it, offenses are collected during the walk and emitted after it.
//! 2. **Which prism nodes are parser `:str`.** Stock's `on_str` /
//!    `each_descendant(:str)` fire for every parser `:str` node. In prism these
//!    are `StringNode` (plain strings, `dstr` parts, and the `str` parts inside
//!    interpolated regexp / xstr / dsym) PLUS the whole-body of a
//!    non-interpolated `RegularExpressionNode` / `XStringNode` (parser models
//!    those with a single `:str` child, prism keeps them as leaf nodes without
//!    a child). A plain `SymbolNode` is `:sym`, never `:str`, so it is ignored.
//!    The `str` value is prism's `unescaped` (matches parser's `str_content` /
//!    `node.value`); the offense/replace range is the node location for a
//!    `StringNode`, and the `content_loc` (between the delimiters) for a
//!    regexp / xstr body — mirroring parser's `:str`-child location.
//!
//! `acceptable?` exempts a `:str` whose direct parent is an array, a hash pair,
//! or a `:dstr` (prism `ArrayNode` / `AssocNode` / `InterpolatedStringNode` —
//! the last covers both interpolation and adjacent-literal concatenation; added
//! in rubocop#15333 so an interpolated/concatenated `str` is not rewritten in
//! isolation). A regexp / xstr body's parser parent is the regexp / xstr node
//! itself (never an array / pair / dstr), so those bodies are never exempt
//! regardless of what encloses the literal.

use ruby_prism::{Location, Node};

/// One offense: `add_offense(range, message)` with
/// `corrector.replace(range, "File::NULL")` (offense range == replace range).
pub struct FileNullOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: String,
}

/// A collected `:str`-equivalent that passed `valid_string?`, `acceptable?`,
/// and the `REGEXP` full match. `is_bare_nul` marks the `nul`
/// (case-insensitive) form that is gated on a `/dev/null` somewhere in the file.
struct Candidate {
    start: usize,
    end: usize,
    /// Original-case unescaped value, for the `%<source>s` message field.
    value: String,
    is_bare_nul: bool,
}

pub fn check_file_null(source: &[u8]) -> Vec<FileNullOffense> {
    let mut rule = build_rule();
    super::dispatch::run(source, &mut [&mut rule]);
    rule.into_offenses()
}

pub(crate) fn build_rule() -> Visitor {
    Visitor {
        parent_acceptable: Vec::new(),
        candidates: Vec::new(),
        contains_dev_null: false,
    }
}

pub(crate) struct Visitor {
    /// One entry per open branch node: `true` when it is an `ArrayNode` or
    /// `AssocNode` (a parser `:array` / `:pair` — the `acceptable?` parents).
    /// The top of the stack is the immediate parent of the node being visited.
    parent_acceptable: Vec<bool>,
    candidates: Vec<Candidate>,
    contains_dev_null: bool,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

impl Visitor {
    pub(crate) fn into_offenses(self) -> Vec<FileNullOffense> {
        let contains_dev_null = self.contains_dev_null;
        self.candidates
            .into_iter()
            .filter(|c| !c.is_bare_nul || contains_dev_null)
            .map(|c| FileNullOffense {
                start_offset: c.start,
                end_offset: c.end,
                message: format!("Use `File::NULL` instead of `{}`.", c.value),
            })
            .collect()
    }

    /// `valid_string?` + the file-level `/dev/null` tally + the offense test,
    /// for one `:str`-equivalent node. `acceptable` is the parser
    /// `node.parent.type?(:array, :pair)` verdict (always `false` for a
    /// regexp / xstr body).
    fn consider(&mut self, value: &[u8], start: usize, end: usize, acceptable: bool) {
        // `valid_string?`: `!value.empty? && value.valid_encoding?`.
        if value.is_empty() {
            return;
        }
        let Ok(text) = std::str::from_utf8(value) else {
            return;
        };

        // `@contain_dev_null_string_in_file`: any valid `:str` descendant whose
        // downcased content is `/dev/null` (array / hash members count too).
        if text.eq_ignore_ascii_case("/dev/null") {
            self.contains_dev_null = true;
        }

        // `acceptable?`: inside an array or a hash pair.
        if acceptable {
            return;
        }

        // `REGEXP = %r{\A(/dev/null|NUL:?)\z}i` — a case-insensitive full match.
        let is_bare_nul = text.eq_ignore_ascii_case("nul");
        let matched = text.eq_ignore_ascii_case("/dev/null")
            || is_bare_nul
            || text.eq_ignore_ascii_case("nul:");
        if !matched {
            return;
        }

        self.candidates.push(Candidate {
            start,
            end,
            value: text.to_owned(),
            is_bare_nul,
        });
    }
}

impl super::dispatch::Rule for Visitor {
    fn enter(&mut self, node: &Node<'_>) {
        // A `:str` whose parent is a parser `:array` / `:pair` / `:dstr` is
        // exempt (rubocop#15333: a `str` that is part of an interpolated or
        // concatenated string must not be rewritten in isolation). prism models
        // both interpolation and adjacent-literal concatenation as
        // `InterpolatedStringNode` (= parser `:dstr`). A `str` part inside an
        // interpolated regexp / xstr / dsym has a `:regexp` / `:xstr` / `:dsym`
        // parent, not `:dstr`, so those stay flagged.
        let acceptable = matches!(
            node,
            Node::ArrayNode { .. } | Node::AssocNode { .. } | Node::InterpolatedStringNode { .. }
        );
        self.parent_acceptable.push(acceptable);
    }

    fn leave(&mut self) {
        self.parent_acceptable.pop();
    }

    fn enter_leaf(&mut self, node: &Node<'_>) {
        match node {
            Node::StringNode { .. } => {
                let n = node.as_string_node().unwrap();
                let (start, end) = loc(&n.location());
                let acceptable = *self.parent_acceptable.last().unwrap_or(&false);
                self.consider(n.unescaped(), start, end, acceptable);
            }
            Node::RegularExpressionNode { .. } => {
                let n = node.as_regular_expression_node().unwrap();
                let (start, end) = loc(&n.content_loc());
                // A regexp body's parser parent is the `:regexp`, never an
                // array / pair.
                self.consider(n.unescaped(), start, end, false);
            }
            Node::XStringNode { .. } => {
                let n = node.as_x_string_node().unwrap();
                let (start, end) = loc(&n.content_loc());
                self.consider(n.unescaped(), start, end, false);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<FileNullOffense> {
        check_file_null(src.as_bytes())
    }

    fn apply(src: &str) -> String {
        let mut out = src.as_bytes().to_vec();
        let mut edits: Vec<(usize, usize)> = run(src)
            .iter()
            .map(|o| (o.start_offset, o.end_offset))
            .collect();
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        for (start, end) in edits {
            out.splice(start..end, b"File::NULL".iter().copied());
        }
        String::from_utf8(out).unwrap()
    }

    // Typical: a plain `/dev/null` literal is flagged and rewritten.
    #[test]
    fn plain_dev_null() {
        let src = "x = '/dev/null'";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(&src[got[0].start_offset..got[0].end_offset], "'/dev/null'");
        assert_eq!(got[0].message, "Use `File::NULL` instead of `/dev/null`.");
        assert_eq!(apply(src), "x = File::NULL");
    }

    // Bare `NUL` is inert without a `/dev/null` in the file...
    #[test]
    fn bare_nul_without_dev_null() {
        assert!(run("x = 'NUL'").is_empty());
        assert!(run("x = 'nul'").is_empty());
    }

    // ...but fires once the file also contains a `/dev/null` literal.
    #[test]
    fn bare_nul_gated_on_dev_null() {
        let src = "a = '/dev/null'\nx = 'NUL'";
        let got = run(src);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].message, "Use `File::NULL` instead of `NUL`.");
    }

    // The gate is unlocked even by a `/dev/null` that appears AFTER the `nul`
    // (collection happens during the walk, emission after it).
    #[test]
    fn bare_nul_gate_from_later_dev_null() {
        let got = run("x = 'nul'\na = '/dev/null'");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].message, "Use `File::NULL` instead of `nul`.");
    }

    // `NUL:` has no gate and is always flagged.
    #[test]
    fn nul_colon_always_flagged() {
        let got = run("x = 'NUL:'");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].message, "Use `File::NULL` instead of `NUL:`.");
    }

    // Case-insensitive detection, original-case message.
    #[test]
    fn case_insensitive() {
        let got = run("x = \"/DEV/NULL\"");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].message, "Use `File::NULL` instead of `/DEV/NULL`.");
    }

    // Arrays and hash pairs are exempt (but still feed the `/dev/null` gate).
    #[test]
    fn array_and_hash_exempt() {
        assert!(run("['/dev/null', 'NUL']").is_empty());
        assert!(run("%w[/dev/null NUL]").is_empty());
        assert!(run("{ unix: \"/dev/null\", windows: \"nul\" }").is_empty());
        assert!(run("{ \"/dev/null\" => 1, \"nul\" => 2 }").is_empty());
    }

    // A `%w` member unlocks the bare-`nul` gate for a NON-member `nul`.
    #[test]
    fn percent_w_member_feeds_gate() {
        let got = run("a = %w[/dev/null]\nx = 'NUL'");
        assert_eq!(got.len(), 1);
        assert_eq!(&run("a = %w[/dev/null]\nx = 'NUL'")[0].message,
                   "Use `File::NULL` instead of `NUL`.");
        assert_eq!(got[0].message, "Use `File::NULL` instead of `NUL`.");
    }

    // A substring is not a full match.
    #[test]
    fn substring_ignored() {
        assert!(run("'the /dev/null device and NUL'").is_empty());
    }

    // A `str` part inside an interpolated string has a `:dstr` parent
    // (rubocop#15333): exempt, not rewritten in isolation. But it still feeds
    // the `/dev/null` gate, so a following bare `nul` becomes flaggable.
    #[test]
    fn interpolated_string_part() {
        assert!(run("x = \"#{y}/dev/null\"").is_empty());
        assert_eq!(apply("x = \"#{y}/dev/null\""), "x = \"#{y}/dev/null\"");
        // The exempt `/dev/null` part still unlocks the bare-`nul` gate.
        let got = run("x = \"#{y}/dev/null\"\nz = 'NUL'");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].message, "Use `File::NULL` instead of `NUL`.");
    }

    // Adjacent string literal concatenation is also `:dstr`: exempt.
    #[test]
    fn adjacent_concatenation_part() {
        assert!(run("x = '/dev/null' '/dev/null'").is_empty());
    }

    // A non-interpolated regexp / xstr body is a parser `:str` child: flagged,
    // ranged on the content between the delimiters, and it feeds the gate.
    #[test]
    fn regexp_and_xstr_body() {
        assert_eq!(apply("x = %r{/dev/null}"), "x = %r{File::NULL}");
        assert_eq!(apply("x = /\\/dev\\/null/"), "x = /File::NULL/");
        assert_eq!(apply("x = `/dev/null`"), "x = `File::NULL`");
        // The regexp body also unlocks the bare-`nul` gate.
        let got = run("r = %r{/dev/null}\nx = 'NUL'");
        assert_eq!(got.len(), 2);
    }

    // A plain symbol is `:sym`, not `:str`: ignored, and does not feed the gate.
    #[test]
    fn plain_symbol_ignored() {
        assert!(run("x = :\"/dev/null\"").is_empty());
        assert!(run("a = :\"/dev/null\"\nx = 'NUL'").is_empty());
    }

    // A heredoc body of `/dev/null` carries a trailing newline, so it is NOT a
    // full match and does not fire or feed the gate.
    #[test]
    fn heredoc_body_not_matched() {
        assert!(run("x = <<~H\n/dev/null\nH\n").is_empty());
        assert!(run("x = <<~H\n/dev/null\nH\ny = 'NUL'").is_empty());
    }

    // Empty and invalid-encoding strings are skipped by `valid_string?`.
    #[test]
    fn empty_and_invalid_skipped() {
        assert!(run("x = ''").is_empty());
        assert!(run("x = \"\\xa4\"").is_empty());
    }

    // Two branches of a ternary, both flagged in source order.
    #[test]
    fn ternary_two_offenses() {
        let src = "path = cond ? '/dev/null' : 'NUL:'";
        let got = run(src);
        assert_eq!(got.len(), 2);
        assert_eq!(apply(src), "path = cond ? File::NULL : File::NULL");
    }
}
