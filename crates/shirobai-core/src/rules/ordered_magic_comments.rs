//! `Lint/OrderedMagicComments`.
//!
//! Stock's `on_new_investigation`
//! (`vendor/rubocop/lib/rubocop/cop/lint/ordered_magic_comments.rb`):
//!
//! 1. `return if processed_source.buffer.source.empty?`.
//! 2. `magic_comment_lines` walks `leading_magic_comments` — the same leading
//!    line slice as `Lint/DuplicateMagicComment` (`leading_comment_lines` from
//!    the `FrozenStringLiteral` mixin), each mapped through `MagicComment.parse`
//!    — and records two 0-based indices into that slice: `lines[0]` (encoding)
//!    is updated on every `encoding_specified?` line, `lines[1]` (other) on
//!    every `elsif valid?` line (a `#`-prefixed comment specifying some
//!    non-encoding magic kind). The scan returns as soon as both are set
//!    (stock's `return lines if lines[0] && lines[1]`).
//! 3. Offense only when both are set and `encoding_line >= other_line` (the
//!    encoding comment does NOT precede the other magic comment; the two are
//!    always distinct because a line falls in at most one bucket).
//! 4. The offense highlight is `buffer.line_range(encoding_line + 1)` (the whole
//!    encoding line, no trailing newline). Autocorrect swaps that line with
//!    `buffer.line_range(other_line + 1)` (each replaced by the other's source).
//!
//! Because `leading_comment_lines` is a contiguous slice starting at line 1, a
//! 0-based slice index `i` is exactly the 1-based buffer line `i + 1`. The Rust
//! side reproduces the scan and returns the two 1-based line numbers; the Ruby
//! wrapper rebuilds the offense range and the swap with stock's own
//! `buffer.line_range`, so offense and autocorrect bytes match by construction.
//! Line numbers are byte/char agnostic, so no `SourceOffsets` conversion is
//! needed.
//!
//! The leading-line scan (first non-comment token, `__END__` / phantom-line
//! semantics) and the per-line magic classification are shared with
//! `Lint/DuplicateMagicComment` — see [`super::duplicate_magic_comment`].

use super::duplicate_magic_comment::{
    leading_line_count, line_slice, ordered_bucket, scan_front, OrderedBucket,
};
use super::line_index;
use super::parse_cache;

/// The two 1-based line numbers of an offense: `(encoding_line, other_line)`.
/// The offense highlight is the encoding line; autocorrect swaps the two.
pub type OrderedOffense = (usize, usize);

pub fn check_ordered_magic_comments(source: &[u8]) -> Option<OrderedOffense> {
    if source.is_empty() {
        return None;
    }

    let (scan, last_comment_start) = parse_cache::with_parsed_and_comments(
        source,
        |_owner, _root, comments| scan_front(source, &comments),
    );

    line_index::with_line_index(source, |li| {
        let leading = leading_line_count(source, li, &scan, last_comment_start);

        // Stock's `magic_comment_lines` loop: `encoding` tracks the latest
        // `encoding_specified?` index, `other` the latest `elsif valid?` index;
        // both are 0-based indices into the leading slice. Stop as soon as both
        // are set (`return lines if lines[0] && lines[1]`).
        let mut encoding: Option<usize> = None;
        let mut other: Option<usize> = None;
        for (i, &start) in li.line_starts().iter().take(leading).enumerate() {
            match ordered_bucket(line_slice(source, start)) {
                OrderedBucket::Encoding => encoding = Some(i),
                OrderedBucket::OtherValid => other = Some(i),
                OrderedBucket::None => {}
            }
            if encoding.is_some() && other.is_some() {
                break;
            }
        }

        match (encoding, other) {
            // `return if encoding_line < other_line`: offense when encoding does
            // not precede the other magic comment. The two indices are distinct
            // (a line is at most one bucket), so `>=` is effectively `>`.
            (Some(e), Some(o)) if e >= o => Some((e + 1, o + 1)),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Option<(usize, usize)> {
        check_ordered_magic_comments(src.as_bytes())
    }

    // Typical: encoding after frozen_string_literal -> offense swapping the two.
    #[test]
    fn encoding_after_fsl() {
        assert_eq!(run("# frozen_string_literal: true\n# encoding: ascii\n"), Some((2, 1)));
    }

    // `coding` prefix variant counts as encoding.
    #[test]
    fn coding_after_fsl() {
        assert_eq!(run("# frozen_string_literal: true\n# coding: ascii\n"), Some((2, 1)));
    }

    // Encoding first -> no offense.
    #[test]
    fn encoding_first_ok() {
        assert_eq!(run("# encoding: ascii\n# frozen_string_literal: true\n"), None);
    }

    // Encoding right after a shebang, before fsl -> no offense.
    #[test]
    fn shebang_then_encoding_ok() {
        assert_eq!(
            run("#!/usr/bin/env ruby\n# encoding: ascii\n# frozen_string_literal: true\n"),
            None
        );
    }

    // Shebang does not stop the leading prefix: fsl then encoding -> offense on
    // the encoding line (line 3), swapping with the fsl line (line 2).
    #[test]
    fn shebang_then_fsl_then_encoding() {
        assert_eq!(
            run("#!/usr/bin/env ruby\n# frozen_string_literal: true\n# encoding: ascii\n"),
            Some((3, 2))
        );
    }

    // shareable_constant_value is a valid non-encoding magic comment.
    #[test]
    fn shareable_then_encoding() {
        assert_eq!(
            run("# shareable_constant_value: literal\n# encoding: ascii\n"),
            Some((2, 1))
        );
    }

    #[test]
    fn encoding_then_shareable_ok() {
        assert_eq!(run("# encoding: ascii\n# shareable_constant_value: literal\n"), None);
    }

    // Only one magic kind -> no pair -> no offense.
    #[test]
    fn encoding_only() {
        assert_eq!(run("# encoding: ascii\n"), None);
    }

    #[test]
    fn fsl_only() {
        assert_eq!(run("# frozen_string_literal: true\n"), None);
    }

    // Emacs form `-*- encoding : ascii-8bit -*-` after fsl -> offense.
    #[test]
    fn emacs_encoding_after_fsl() {
        assert_eq!(
            run("# frozen_string_literal: true\n# -*- encoding : ascii-8bit -*-\n"),
            Some((2, 1))
        );
    }

    // A magic-shaped hash-literal line further down is not a leading comment
    // (a non-comment token on line 3 ends the prefix), so no offense.
    #[test]
    fn magic_shaped_code_not_leading() {
        assert_eq!(
            run("# frozen_string_literal: true\n\nx = { encoding: Encoding::SJIS }\n"),
            None
        );
    }

    // Empty source -> nothing (stock returns early).
    #[test]
    fn empty_source() {
        assert_eq!(run(""), None);
    }

    // typed is a valid non-encoding magic kind (Sorbet sigil).
    #[test]
    fn typed_then_encoding() {
        assert_eq!(run("# typed: true\n# encoding: ascii\n"), Some((2, 1)));
    }

    // rbs_inline only counts with an enabled/disabled value.
    #[test]
    fn rbs_inline_enabled_then_encoding() {
        assert_eq!(run("# rbs_inline: enabled\n# encoding: ascii\n"), Some((2, 1)));
        // A non-enabled/disabled value is NOT a valid rbs_inline magic comment.
        assert_eq!(run("# rbs_inline: yes\n# encoding: ascii\n"), None);
    }

    // A leading-space `# frozen...` line is NOT `start_with?('#')`, so it is not
    // a valid "other" magic comment; the encoding line then has no partner.
    #[test]
    fn leading_space_fsl_not_other() {
        assert_eq!(run("  # frozen_string_literal: true\n# encoding: ascii\n"), None);
    }

    // Three leading magic comments: the `other` bucket keeps the LATEST valid
    // non-encoding line seen before encoding is found (stock overwrites
    // `lines[1]` each time), so the encoding line (3) pairs with the shareable
    // line (2), not the fsl line (1).
    #[test]
    fn other_bucket_tracks_latest_before_encoding() {
        assert_eq!(
            run("# frozen_string_literal: true\n# shareable_constant_value: literal\n# encoding: ascii\n"),
            Some((3, 2))
        );
    }
}
