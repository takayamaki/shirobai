//! `Layout/LeadingEmptyLines`.
//!
//! Stock's `on_new_investigation`:
//!
//! ```text
//! token = processed_source.tokens[0]
//! return unless token && token.line > 1
//! add_offense(token.pos) do |corrector|
//!   range = Parser::Source::Range.new(buffer, 0, token.begin_pos)
//!   corrector.remove(range)
//! end
//! ```
//!
//! The first token is whatever parser-gem's tokenizer emits first: a keyword /
//! identifier (e.g. `class`, `puts`), an inline `#` comment (the whole text
//! without the trailing `\n`), or a `=begin/=end` block comment (the whole
//! block including the trailing `\n`). Files starting with `__END__` carry
//! NO tokens at all (parser stops), so they never fire — even with leading
//! blank lines before the marker.
//!
//! The Rust side has no parser-gem token list, so it reconstructs the "first
//! token" from prism instead:
//!
//! - The first thing in the file is whichever comes first in document order
//!   between (a) the earliest comment (inline `#` or `=begin/=end` — prism
//!   returns both in document order) and (b) the first AST statement.
//! - Both kinds carry the same offsets prism reports as parser-gem's token:
//!   inline comment `start..end` excludes the trailing `\n`; block comment
//!   `start..end` includes the trailing `\n`; an AST first statement's start
//!   is the byte the first lexical token would land on, and its end byte is
//!   computed by [`first_token_end`] (same identifier / keyword / heredoc /
//!   percent-literal scan as `Layout/InitialIndentation`).
//! - When neither (a) nor (b) exist the file has no tokens — matching stock's
//!   `tokens[0].nil?` exit (covers empty files, whitespace-only files, and
//!   files where `__END__` precedes any code).
//!
//! Line numbers come from the shared `LineIndex`. The "line > 1" test
//! compares the 1-based line of the anchor's byte offset to 1; CRLF / BOM
//! normalization is handled by the wrapper's `bundle_eligible?` (the
//! standalone path scans `buffer.source` directly when they differ).
//!
//! The offense range is `[anchor_off, token_end)`; the autocorrect range is
//! `[0, anchor_off)`. The Ruby wrapper turns both into char offsets through
//! `SourceOffsets` (BOM = 1 char, ASCII byte = 1 char).

use super::line_index;
use super::parse_cache;

/// One offense. All offsets are raw-source bytes.
///
/// - `[start, end)` is the offense range stock yields to `add_offense`,
///   covering the first lexical token / comment exactly the way parser-gem's
///   tokenizer reports it.
/// - `[ac_start, ac_end)` is the leading-blank range stock's corrector
///   removes (`[0, token.begin_pos)`).
pub struct LeadingEmptyLinesOffense {
    pub start: usize,
    pub end: usize,
    pub ac_start: usize,
    pub ac_end: usize,
}

pub fn check_leading_empty_lines(source: &[u8]) -> Option<LeadingEmptyLinesOffense> {
    let line_index = line_index::with_line_index(source, |li| li.clone());
    parse_cache::with_parsed_and_comments(source, |_owner, root, comments| {
        // First AST statement's start byte (the byte the first lexical token
        // would land on).
        let first_code_start: Option<usize> =
            root.as_program_node()
                .and_then(|p| p.statements().body().iter().next())
                .map(|n| n.location().start_offset());

        // First comment in document order (inline `#` OR `=begin/=end`); both
        // count as parser-gem tokens. `comments` is already in document order.
        let first_comment: Option<(usize, usize)> = comments.first().copied();

        // Anchor: earliest of the two by start byte.
        let (anchor_start, anchor_end) = match (first_comment, first_code_start) {
            (Some((cs, ce)), Some(code)) => {
                if cs <= code {
                    (cs, ce)
                } else {
                    (code, first_token_end(source, code))
                }
            }
            (Some((cs, ce)), None) => (cs, ce),
            (None, Some(code)) => (code, first_token_end(source, code)),
            (None, None) => return None,
        };

        // `token.line > 1`: line is 1-based.
        if line_index.line_of(anchor_start) <= 1 {
            return None;
        }

        Some(LeadingEmptyLinesOffense {
            start: anchor_start,
            end: anchor_end,
            ac_start: 0,
            ac_end: anchor_start,
        })
    })
}

/// End byte (exclusive) of the first lexical token at `start`. Mirrors
/// parser-gem's tokenization for the shapes that can be the FIRST token in a
/// file. See `initial_indentation::first_token_end` for the equivalent logic
/// used by `Layout/InitialIndentation` — both cops key on the same notion of
/// "first token" and the offense oracle scores on `line:column:message`, so
/// the long tail of less-common starters can safely fall back to a 1-byte
/// end without breaking drop-in compatibility.
fn first_token_end(source: &[u8], start: usize) -> usize {
    if start >= source.len() {
        return start;
    }
    let bytes = &source[start..];
    let first = bytes[0];

    // Identifier / keyword.
    if is_ident_start(first) {
        let mut i = 1;
        while i < bytes.len() && is_ident_continue(bytes[i]) {
            i += 1;
        }
        if i < bytes.len() && (bytes[i] == b'?' || bytes[i] == b'!') {
            i += 1;
        }
        return start + i;
    }

    // `@`/`@@` ivar/cvar.
    if first == b'@' {
        let mut i = 1;
        if bytes.len() > 1 && bytes[1] == b'@' {
            i = 2;
        }
        if i < bytes.len() && is_ident_start(bytes[i]) {
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            return start + i;
        }
        return start + i.max(1);
    }

    // `$` global var.
    if first == b'$' {
        let mut i = 1;
        if i < bytes.len() && is_ident_start(bytes[i]) {
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            return start + i;
        }
        if i < bytes.len() {
            return start + 2;
        }
        return start + 1;
    }

    // Heredoc opener `<<`, `<<-`, `<<~` + marker (bare identifier form).
    if first == b'<' && bytes.len() > 1 && bytes[1] == b'<' {
        let mut i = 2;
        if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'~') {
            i += 1;
        }
        if i < bytes.len() && is_ident_start(bytes[i]) {
            i += 1;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
        }
        return start + i;
    }

    // Percent literal opener with a type letter (`%w[`, `%i(`, `%r{`, ...).
    if first == b'%' && bytes.len() >= 3 {
        let second = bytes[1];
        if matches!(
            second,
            b'i' | b'I' | b'q' | b'Q' | b'r' | b's' | b'w' | b'W' | b'x'
        ) {
            return start + 3;
        }
    }

    // Common multi-char operators that can lead a statement.
    if bytes.len() >= 3 && &bytes[..3] == b"..." {
        return start + 3;
    }
    if bytes.len() >= 2 {
        let pair = &bytes[..2];
        if matches!(pair, b"::" | b"->" | b"&." | b"**" | b"..") {
            return start + 2;
        }
    }

    start + 1
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Option<(usize, usize, usize, usize)> {
        check_leading_empty_lines(src.as_bytes())
            .map(|o| (o.start, o.end, o.ac_start, o.ac_end))
    }

    // Typical: one leading blank line before a class.
    #[test]
    fn one_blank_before_class() {
        // "\nclass Foo\nend\n": `class` at byte 1..6, ac [0, 1).
        assert_eq!(run("\nclass Foo\nend\n"), Some((1, 6, 0, 1)));
    }

    // Two leading blank lines before a call.
    #[test]
    fn two_blanks_before_puts() {
        // "\n\nputs 1\n": `puts` at byte 2..6, ac [0, 2).
        assert_eq!(run("\n\nputs 1\n"), Some((2, 6, 0, 2)));
    }

    // No leading blank: clean.
    #[test]
    fn no_leading_blank() {
        assert_eq!(run("class Foo\nend\n"), None);
    }

    // Empty file: no token, no offense.
    #[test]
    fn empty_source() {
        assert_eq!(run(""), None);
    }

    // Whitespace-only: no token.
    #[test]
    fn only_newline() {
        assert_eq!(run("\n"), None);
    }

    // Multiple newlines, still no code/comment: no token.
    #[test]
    fn only_blanks() {
        assert_eq!(run("\n\n\n\n"), None);
    }

    // Leading blank before an inline `#` comment: the comment IS a token.
    #[test]
    fn blank_before_comment() {
        // "\n# comment\n": comment at byte 1..10, ac [0, 1).
        assert_eq!(run("\n# comment\n"), Some((1, 10, 0, 1)));
    }

    // Leading blank before `=begin/=end` block comment.
    #[test]
    fn blank_before_block_comment() {
        // "\n=begin\nbody\n=end\nclass A; end\n": comment 1..18.
        let src = "\n=begin\nbody\n=end\nclass A; end\n";
        assert_eq!(run(src), Some((1, 18, 0, 1)));
    }

    // `\n__END__\n...`: parser cuts off, no tokens, no offense.
    #[test]
    fn blank_before_end_marker() {
        assert_eq!(run("\n__END__\nfoo\n"), None);
    }

    // `__END__` first thing: no tokens.
    #[test]
    fn end_marker_first() {
        assert_eq!(run("__END__\nfoo\n"), None);
    }

    // BOM + blank + code: anchor is the call at byte 4 (line 2); offense range
    // is byte 4..8 (`puts`), ac range 0..4. The wrapper converts to char
    // offsets, where BOM = 1 char.
    #[test]
    fn bom_then_blank_then_code() {
        assert_eq!(run("\u{FEFF}\nputs 1\n"), Some((4, 8, 0, 4)));
    }

    // BOM + code (no blank): code at byte 3, line 1 → no offense.
    #[test]
    fn bom_no_blank() {
        assert_eq!(run("\u{FEFF}puts 1\n"), None);
    }

    // CRLF blank line + code: `\r\n\r\n` is two blank lines; `puts` is at
    // line 3, byte 4.
    #[test]
    fn crlf_blank_then_code() {
        // "\r\n\r\nputs 1\n": puts at 4..8.
        assert_eq!(run("\r\n\r\nputs 1\n"), Some((4, 8, 0, 4)));
    }

    // Three blank lines before a class.
    #[test]
    fn three_blanks_before_class() {
        // "\n\n\nclass A; end\n": class at 3..8.
        assert_eq!(run("\n\n\nclass A; end\n"), Some((3, 8, 0, 3)));
    }

    // Blank then indented code: still fires (the offense ANCHOR is the code's
    // first byte; `Layout/InitialIndentation` handles the indentation in a
    // separate pass).
    #[test]
    fn blank_then_indented_code() {
        // "\n   class A; end\n": class at byte 4..9.
        assert_eq!(run("\n   class A; end\n"), Some((4, 9, 0, 4)));
    }

    // Blank then comment-only file: the comment is a token.
    #[test]
    fn blank_then_comment_only() {
        // "\n# c\n# d\n": # c at byte 1..4.
        assert_eq!(run("\n# c\n# d\n"), Some((1, 4, 0, 1)));
    }

    // Leading whitespace on the first line followed by blank + code: the
    // first line is still line 1, so the code on line 3 has line > 1 → fires.
    #[test]
    fn leading_space_then_blank() {
        // " \n\nputs 1\n": puts at byte 3..7.
        assert_eq!(run(" \n\nputs 1\n"), Some((3, 7, 0, 3)));
    }

    // No blank, comment first: line 1, no offense.
    #[test]
    fn comment_first_line() {
        assert_eq!(run("# c\nclass A; end\n"), None);
    }

    // First line is shebang (an inline `#` comment); next line is blank then
    // class: shebang is on line 1 so it's the first token → no offense.
    #[test]
    fn shebang_then_blank_then_class() {
        let src = "#!/usr/bin/env ruby\n\nclass Foo\nend\n";
        assert_eq!(run(src), None);
    }

    // Single-byte first token (identifier `x`): caret is 1 byte wide.
    #[test]
    fn blank_then_single_char_var() {
        // "\nx = 1\n": x at byte 1..2.
        assert_eq!(run("\nx = 1\n"), Some((1, 2, 0, 1)));
    }

    // Tab-only blank line: `\t\n` is still a blank line → code at line 2.
    #[test]
    fn tab_blank_then_code() {
        // "\t\nputs 1\n": puts at byte 2..6.
        assert_eq!(run("\t\nputs 1\n"), Some((2, 6, 0, 2)));
    }
}
