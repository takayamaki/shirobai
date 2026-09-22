//! `Lint/MisplacedMagicComment`.
//!
//! Stock's `on_new_investigation`
//! (`vendor/rubocop/lib/rubocop/cop/lint/misplaced_magic_comment.rb`):
//!
//! ```ruby
//! def on_new_investigation
//!   return if processed_source.buffer.source.empty?
//!   check_magic_comment_above_shebang
//!   processed_source.comments.each { |comment| check_comment(comment) }
//! end
//!
//! def check_comment(comment)
//!   return unless magic_comment_shaped?(comment.text)   # /\A#(?![^#]*#)/
//!   magic_comment = MagicComment.parse(comment.text)
//!   return unless magic_comment.valid?
//!   if magic_comment.encoding_specified?
//!     ...check_encoding_comment(comment)
//!   elsif magic_comment.frozen_string_literal_specified?
//!     check_top_block_comment(comment, 'frozen_string_literal')
//!   end
//! end
//!
//! def first_code_token
//!   @first_code_token ||= processed_source.sorted_tokens.find { |token| !token.comment? }
//! end
//! ```
//!
//! Two costs: every comment pays `magic_comment_shaped?` + `MagicComment.parse`
//! (three regexp formats), and `check_top_block_comment` materializes the
//! parser-gem token stream (`sorted_tokens`) on every file with a
//! `frozen_string_literal` comment — nearly every file in a modern codebase.
//!
//! The Rust side returns:
//!
//! - the `(index, line)` of the first comment stock's
//!   `check_magic_comment_above_shebang` would find (line > 1, column 0,
//!   text starting with `#!`), or `None`;
//! - for every comment that is magic-comment shaped (starts with `#`, no
//!   second `#`) AND mentions `coding` or `frozen` (ASCII case-insensitive):
//!   `(index, line, after_code)`, where `after_code` is stock's
//!   `comment.source_range.begin_pos > first_code_token.begin_pos` computed on
//!   byte offsets with the shared leading-comment front scan
//!   ([`super::duplicate_magic_comment::scan_front`]) — `false` when the file
//!   has no code token.
//!
//! The candidate filter is a superset of the comments stock acts on: every
//! `encoding` form (`encoding:` / `coding:` / emacs `coding:` / vim
//! `fileencoding=`) contains `coding`, and every `frozen_string_literal` form
//! contains `frozen`. The wrapper runs stock's `check_comment` verbatim on the
//! candidates (so the regexps still decide), with `first_code_token` replaced
//! by the `after_code` flag. Everything else — encoding validity, the
//! shebang rule, `preceded_only_by_magic_comments?`, messages, `move_comment`
//! — is stock's own Ruby.

use super::duplicate_magic_comment::{scan_front, ScanEnd};
use super::line_index;
use super::parse_cache;

/// `(comment index, 1-based line, comment starts after the first code token)`.
pub type MagicCandidate = (usize, usize, bool);

/// `(shebang candidate, magic-comment candidates)`.
pub type MisplacedMagicScan = (Option<(usize, usize)>, Vec<MagicCandidate>);

fn contains_ci(text: &[u8], needle: &[u8]) -> bool {
    text.len() >= needle.len()
        && text
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
}

/// Stock's `magic_comment_shaped?` (`/\A#(?![^#]*#)/`: starts with `#` and
/// has no second `#`) narrowed to comments that can carry an `encoding` or
/// `frozen_string_literal` directive.
fn magic_candidate(text: &[u8]) -> bool {
    text.first() == Some(&b'#')
        && !text[1..].contains(&b'#')
        && (contains_ci(text, b"coding") || contains_ci(text, b"frozen"))
}

/// Stock's shebang finder: line > 1 (so not at byte 0), column 0 (the
/// previous byte is the newline), text starting with `#!`.
fn shebang_candidate(source: &[u8], start: usize, end: usize) -> bool {
    start > 0 && source[start - 1] == b'\n' && source[start..end].starts_with(b"#!")
}

pub fn check_misplaced_magic_comment(source: &[u8]) -> MisplacedMagicScan {
    if source.is_empty() {
        return (None, Vec::new());
    }

    let (comments, scan) =
        parse_cache::with_parsed_and_comments(source, |_owner, _root, comments| {
            let (scan, _last_comment_start) = scan_front(source, &comments);
            (comments, scan)
        });
    let first_code = match scan {
        ScanEnd::Token(pos) => Some(pos),
        ScanEnd::NoToken { .. } => None,
    };

    let shebang = comments
        .iter()
        .enumerate()
        .find(|&(_, &(start, end))| shebang_candidate(source, start, end))
        .map(|(index, &(start, _))| (index, start));
    let mut candidates: Vec<MagicCandidate> = comments
        .iter()
        .enumerate()
        .filter(|&(_, &(start, end))| magic_candidate(&source[start..end]))
        .map(|(index, &(start, _))| {
            let after_code = first_code.is_some_and(|pos| start > pos);
            (index, start, after_code)
        })
        .collect();

    if shebang.is_none() && candidates.is_empty() {
        return (None, candidates);
    }
    line_index::with_line_index(source, |li| {
        for candidate in &mut candidates {
            candidate.1 = li.line_of(candidate.1);
        }
        (
            shebang.map(|(index, start)| (index, li.line_of(start))),
            candidates,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> MisplacedMagicScan {
        check_misplaced_magic_comment(src.as_bytes())
    }

    // Typical: a leading frozen_string_literal comment above code -> one
    // candidate, not after code.
    #[test]
    fn leading_fsl() {
        assert_eq!(
            run("# frozen_string_literal: true\nputs 1\n"),
            (None, vec![(0, 1, false)])
        );
    }

    // The misplaced case: the magic comment comes after code.
    #[test]
    fn fsl_after_code() {
        assert_eq!(
            run("require 'foo'\n# frozen_string_literal: true\n"),
            (None, vec![(0, 2, true)])
        );
    }

    // Plain comments (no `coding` / `frozen`) are never candidates; the
    // index still counts them.
    #[test]
    fn plain_comments_skipped_but_counted() {
        assert_eq!(
            run("# doc\nputs 1\n# encoding: utf-8\n"),
            (None, vec![(1, 3, true)])
        );
    }

    // A second `#` makes the comment not magic-comment shaped (stock's
    // `magic_comment_shaped?`), e.g. documentation quoting a magic comment.
    #[test]
    fn quoted_magic_comment_is_not_shaped() {
        assert_eq!(run("#   # -*- coding: UTF-8 -*-\nputs 1\n"), (None, vec![]));
    }

    // Every encoding / fsl spelling is caught, case-insensitively: emacs,
    // vim `fileencoding`, kebab fsl, upper-case.
    #[test]
    fn all_spellings_are_candidates() {
        let src = "# -*- coding: utf-8 -*-\n# vim: set fileencoding=utf-8 :\n# frozen-string-literal: true\n# FROZEN_STRING_LITERAL: TRUE\nputs 1\n";
        assert_eq!(
            run(src),
            (
                None,
                vec![(0, 1, false), (1, 2, false), (2, 3, false), (3, 4, false)]
            )
        );
    }

    // No code token at all (comment-only file): `after_code` is false.
    #[test]
    fn no_code_token() {
        assert_eq!(
            run("# frozen_string_literal: true\n"),
            (None, vec![(0, 1, false)])
        );
    }

    // Empty source: stock returns before doing anything.
    #[test]
    fn empty_source() {
        assert_eq!(run(""), (None, vec![]));
    }

    // A shebang below a magic comment (line > 1, column 0) is the shebang
    // candidate; the shebang text itself is not a magic candidate.
    #[test]
    fn shebang_below_magic_comment() {
        assert_eq!(
            run("# frozen_string_literal: true\n#!/usr/bin/env ruby\nputs 1\n"),
            (Some((1, 2)), vec![(0, 1, false)])
        );
    }

    // A shebang on line 1 is not a candidate; an indented `#!` is not at
    // column 0.
    #[test]
    fn shebang_on_first_line_or_indented() {
        assert_eq!(run("#!/usr/bin/env ruby\nputs 1\n"), (None, vec![]));
        assert_eq!(run("puts 1\n  #! not a shebang\n"), (None, vec![]));
    }

    // `__END__` stops the front scan with no token: a magic comment before
    // it is not after code.
    #[test]
    fn end_marker() {
        assert_eq!(
            run("# frozen_string_literal: true\n__END__\ndata\n"),
            (None, vec![(0, 1, false)])
        );
    }
}
