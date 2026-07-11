//! `Style/MagicCommentFormat`.
//!
//! Stock's `on_new_investigation`
//! (`vendor/rubocop/lib/rubocop/cop/style/magic_comment_format.rb`):
//!
//! ```ruby
//! def on_new_investigation
//!   return unless processed_source.ast
//!   magic_comments.each do |comment|
//!     issues = find_issues(comment)
//!     register_offenses(issues) if issues.any?
//!   end
//! end
//!
//! def magic_comments
//!   processed_source.each_comment_in_lines(leading_comment_lines)
//!                   .select { |comment| MagicComment.parse(comment.text).valid? }
//!                   .map { |comment| CommentRange.new(comment) }
//! end
//!
//! def leading_comment_lines
//!   first_non_comment_token = processed_source.tokens.find { |t| !t.comment? }
//!   first_non_comment_token ? (0...first_non_comment_token.line) : (0..)
//! end
//! ```
//!
//! Everything after `leading_comment_lines` runs on the parser-gem *comment*
//! objects (`each_comment_in_lines`, which reads `processed_source.comments` —
//! from the parse, not the token stream) and stock's own `CommentRange` regex
//! extraction, offense predicates, messages and corrections. The ONLY place the
//! cop materializes the parser-gem token stream is `leading_comment_lines`'
//! `processed_source.tokens.find` — the "toucher" cost this program targets.
//!
//! So the Rust side reproduces ONLY the leading-line boundary: the 1-based line
//! of the first non-comment token (stock's `first_non_comment_token.line`), or
//! `0` when the file has no non-comment token (stock's endless `0..`). The
//! wrapper builds the `leading_comment_lines` range from that number and runs
//! stock's `magic_comments` and the rest unchanged, so detection, messages, and
//! autocorrect are stock's own code — byte-identical by construction, including
//! non-ASCII offsets (the offense ranges come from `CommentRange`'s
//! `loc.expression`, char offsets, never through Rust).
//!
//! The first-non-comment-token scan is the shared leading-comment front
//! ([`super::duplicate_magic_comment::scan_front`]): a byte scan that skips
//! whitespace, comments, `\`-newline continuations and a leading BOM, and stops
//! at the first real token, EOF, a `__END__` data marker, or a NUL-family byte.
//! It is the same front `Lint/DuplicateMagicComment` / `Lint/OrderedMagicComments`
//! use, so its parser-gem-first-token semantics are already pinned there.

use super::duplicate_magic_comment::{scan_front, ScanEnd};
use super::line_index;
use super::parse_cache;

/// Stock's `leading_comment_lines` boundary: the 1-based line of the first
/// non-comment token, or `0` when the file has no non-comment token (stock's
/// endless `0..` range — the wrapper turns `0` into an all-lines range).
pub fn check_magic_comment_format(source: &[u8]) -> usize {
    let (scan, _last_comment_start) = parse_cache::with_parsed_and_comments(
        source,
        |_owner, _root, comments| scan_front(source, &comments),
    );

    match scan {
        ScanEnd::Token(pos) => line_index::with_line_index(source, |li| li.line_of(pos)),
        ScanEnd::NoToken { .. } => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> usize {
        check_magic_comment_format(src.as_bytes())
    }

    // Typical: a magic comment then code on line 2 -> boundary line 2.
    #[test]
    fn magic_then_code() {
        assert_eq!(run("# frozen_string_literal: true\nputs 1\n"), 2);
    }

    // Code on line 1: boundary line 1 (no leading comments).
    #[test]
    fn code_first() {
        assert_eq!(run("puts 1\n# frozen_string_literal: true\n"), 1);
    }

    // Shebang plus a magic comment, code on line 3 -> boundary line 3.
    #[test]
    fn shebang_and_magic() {
        assert_eq!(run("#!/usr/bin/env ruby\n# encoding: utf-8\nputs 1\n"), 3);
    }

    // No non-comment token (comment-only file) -> 0 (stock's `0..`). In the cop
    // this never reaches offense processing because `processed_source.ast` is
    // nil there and the wrapper returns first.
    #[test]
    fn comment_only() {
        assert_eq!(run("# frozen_string_literal: true\n"), 0);
        assert_eq!(run(""), 0);
        assert_eq!(run("   \n"), 0);
    }

    // A `;` is a non-comment token: boundary is its line.
    #[test]
    fn semicolon_is_a_token() {
        assert_eq!(run("# encoding: utf-8\n;\nputs 1\n"), 2);
    }

    // A leading UTF-8 BOM is skipped: the first real token is `puts` on line 3,
    // so both leading magic-comment lines are inside the boundary. Without the
    // BOM skip the scan would stop at byte 0 and report line 1, hiding the
    // leading comments from stock's `each_comment_in_lines`.
    #[test]
    fn leading_bom_skipped() {
        assert_eq!(
            run("\u{feff}# frozen-string-literal: true\n# encoding: utf-8\nputs 1\n"),
            3
        );
    }

    // `__END__` at column 0 stops the scan with no token -> 0.
    #[test]
    fn end_marker_no_token() {
        assert_eq!(run("# frozen_string_literal: true\n__END__\ndata\n"), 0);
    }
}
