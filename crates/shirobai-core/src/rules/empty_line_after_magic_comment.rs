//! `Layout/EmptyLineAfterMagicComment`.
//!
//! Stock's `on_new_investigation`:
//!
//! 1. `comments_before_code(source)`: every comment with `loc.line <
//!    source.ast.loc.line` (or every comment when `source.ast` is nil).
//! 2. Reverse and `find { |c| MagicComment.parse(c.text).any? }` — the
//!    LAST magic comment in the prefix.
//! 3. `next_line = processed_source[last.loc.line]` (the line just below the
//!    magic, 0-indexed via `processed_source[]` which subtracts 1). When the
//!    magic comment is the file's final line, `next_line` is `nil` and the
//!    check exits (the `return unless` guard).
//! 4. When `next_line.strip.empty?`, no offense; otherwise emit one at
//!    `source_range(buffer, last.loc.line + 1, 0)` (a 1-byte range at column
//!    0 of the line below) with `corrector.insert_before(range, "\n")`.
//!
//! The Rust side does step 1: it pulls every comment from the shared parse
//! cache, finds the line of the first AST statement (if any), and returns the
//! byte ranges + 1-based line of every comment that comes before that line
//! (or all of them when the AST has no statements). It does not parse the
//! magic-comment regex itself — that stays on the Ruby side
//! (`MagicComment.parse`), which handles the SimpleComment / EmacsComment /
//! VimComment variants byte-for-byte.

use super::line_index;
use super::parse_cache;

/// One candidate comment before the file's first code line. The Ruby wrapper
/// runs `MagicComment.parse(text).any?` on the slice `[start, end)` and picks
/// the latest matching candidate.
pub struct MagicCommentCandidate {
    /// Comment `source_range` (matches parser-gem `comment.source_range`).
    /// Prism's location ends past a trailing `\r` for CRLF endings, while
    /// parser-gem's source_range stops at the `#`; the end is snapped back so
    /// the two match.
    pub start: usize,
    pub end: usize,
    /// 1-based comment line. The Ruby wrapper uses this to find the
    /// `next_line` content via `processed_source[line]` (0-indexed access ==
    /// `lines[line]`) and to build the `source_range(buffer, line + 1, 0)`
    /// offense range.
    pub line: usize,
}

pub fn check_empty_line_after_magic_comment(source: &[u8]) -> Vec<MagicCommentCandidate> {
    let line_index = line_index::with_line_index(source, |li| li.clone());
    parse_cache::with_parsed_and_comments(source, |_owner, root, comments| {
        // `source.ast.loc.line` — the 1-based line of the first AST
        // statement, or None when the program has no statements
        // (`source.ast` would be `nil` in parser-gem).
        let ast_first_line = root
            .as_program_node()
            .and_then(|p| p.statements().body().iter().next())
            .map(|n| line_index.line_of(n.location().start_offset()));

        // Walk every prism comment in document order. Stock's
        // `take_while { |c| c.loc.line < ast.loc.line }` stops at the first
        // comment AT OR AFTER the first code line, so we apply the same
        // condition here. When there is no AST, every comment is a
        // candidate.
        let mut out = Vec::new();
        for (s, e_raw) in comments {
            // Snap trailing `\r` (CRLF) — parser-gem's `comment.source_range`
            // does not include it.
            let e = if e_raw > s && source.get(e_raw - 1) == Some(&b'\r') {
                e_raw - 1
            } else {
                e_raw
            };
            let line = line_index.line_of(s);
            if let Some(first) = ast_first_line
                && line >= first
            {
                break;
            }
            out.push(MagicCommentCandidate {
                start: s,
                end: e,
                line,
            });
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<MagicCommentCandidate> {
        check_empty_line_after_magic_comment(src.as_bytes())
    }

    // Typical: one magic-shaped comment followed by code on the next line.
    #[test]
    fn one_comment_before_code() {
        let got = run("# frozen_string_literal: true\nclass Foo; end\n");
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].start, got[0].end), (0, 29));
        assert_eq!(got[0].line, 1);
    }

    // Two comments before code: both are candidates (Ruby picks the LAST
    // magic-shaped one).
    #[test]
    fn two_comments_before_code() {
        let got = run("# encoding: utf-8\n# frozen_string_literal: true\nclass Foo; end\n");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].line, 1);
        assert_eq!(got[1].line, 2);
    }

    // No AST (comments only): every comment is a candidate.
    #[test]
    fn comments_only_no_ast() {
        let got = run("# frozen_string_literal: true\n# Hello\n");
        assert_eq!(got.len(), 2);
    }

    // Comment after code is NOT a candidate (line >= ast_first_line).
    #[test]
    fn comment_after_code_excluded() {
        let got = run("puts 'hi'\n# frozen_string_literal: true\nfoo\n");
        assert!(got.is_empty());
    }

    // CRLF: prism's comment end includes the trailing `\r`; we snap it back.
    #[test]
    fn crlf_snaps_trailing_cr() {
        let got = run("# frozen_string_literal: true\r\nclass Foo; end\n");
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].start, got[0].end), (0, 29));
    }

    // Empty source: no comments, no candidates.
    #[test]
    fn empty_source() {
        assert!(run("").is_empty());
    }

    // Shebang is a comment too (prism reports it). It comes before the magic,
    // so both are candidates.
    #[test]
    fn shebang_then_magic() {
        let got = run("#!/usr/bin/env ruby\n# frozen_string_literal: true\nclass Foo; end\n");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].line, 1);
        assert_eq!(got[1].line, 2);
    }

    // `=begin/=end` block comment then a magic comment then code: both
    // comments are candidates (block comment line is 1, magic is line 4).
    #[test]
    fn block_comment_then_magic() {
        let got = run("=begin\nmagic\n=end\n# frozen_string_literal: true\nclass Foo; end\n");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].line, 1);
        assert_eq!(got[1].line, 4);
    }

    // Blank line at the top: no comments at all.
    #[test]
    fn blank_only() {
        assert!(run("\n").is_empty());
    }
}
