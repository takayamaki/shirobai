//! `Style/FrozenStringLiteralComment`.
//!
//! Stock's `on_new_investigation` (paraphrased):
//!
//! 1. `return if processed_source.tokens.empty?` — a file with only
//!    whitespace (or truly empty) has no tokens; a comment (including a
//!    shebang or a `=begin/=end` block) is a token, so it does NOT gate.
//! 2. `always`  → offense unless `frozen_string_literal_comment_exists?`.
//!    `never`  → offense when it exists.
//!    `always_true` → offense unless the comment is present AND set to
//!    `true`.
//!
//! Everything the cop reads lives in the file's LEADING bytes plus a scan of
//! the comment tokens, so this rule never needs the parser-gem token list.
//! That is the whole point of replacing this cop: it is one of the last
//! enabled cops that forces RuboCop to build the lazily-converted
//! rubocop-ast 1.50 token list. The Rust side reads prism comments and a
//! byte scan instead.
//!
//! The leading-comment machinery (token scan, `leading_comment_lines` count,
//! `MagicComment` fsl-value classification) is SHARED with
//! `Lint/DuplicateMagicComment` and `Style/RedundantFreeze` — see the
//! `pub(crate)` helpers in [`super::duplicate_magic_comment`]. This module
//! only adds what is unique to this cop (the style dispatch, the
//! comment-token offense location, and the insertion-point search).
//!
//! # The two detection surfaces (probed against the real CLI)
//!
//! The mixin methods (`frozen_string_literal_comment_exists?`,
//! `frozen_string_literal_specified?`, `frozen_string_literals_enabled?`)
//! all read `leading_comment_lines`: the raw LINE slice
//! `lines[0...(first_non_comment_token.line - 1)]` (or every line when the
//! file has no non-comment token), each run through `MagicComment.parse`.
//! This is line based — a magic-comment-looking line INSIDE a `=begin/=end`
//! block counts, exactly like `Lint/DuplicateMagicComment`.
//!
//! The OFFENSE LOCATION for `never` / `always_true`
//! (`frozen_string_literal_comment`) instead does
//! `tokens.find { MagicComment.parse(token.text).frozen_string_literal_specified? }`
//! over the comment TOKENS. When the leading-line gate fires but no comment
//! TOKEN carries a frozen-string-literal setting (the `=begin` case above),
//! stock calls `nil.pos` and RAISES; RuboCop swallows the per-file error and
//! reports NO offense. This rule reproduces that by emitting nothing when the
//! gate fires but the token search finds no comment.
//!
//! # Insertion point (`last_special_comment` / `insert_comment`)
//!
//! For a missing comment the fix is inserted after the last "special"
//! leading comment:
//!
//! - `tokens[0]` counts as a shebang only when its text starts with `#!`.
//! - The token right after the shebang (or `tokens[0]` when there is no
//!   shebang) counts as an encoding comment when
//!   `MagicComment.parse(text).encoding_specified?` AND
//!   `text.valid_encoding?` (1.92, rubocop#15799; any encoding, not only
//!   UTF-8).
//! - Found → insert `"\n# frozen_string_literal: true"` after that line.
//!   None → prepend `"# frozen_string_literal: true\n"` at the file start.
//!
//! Only `tokens[0]` and `tokens[1]` are ever inspected. Besides comments,
//! only a plain `'...'` / `"..."` string (one `tSTRING` token whose text is
//! the content) can pass the encoding check, through the Emacs form.

use super::duplicate_magic_comment::{
    FslValue, ScanEnd, emacs_token, encoding_specified, leading_line_count, line_fsl,
    line_slice,
    scan_front,
};
use super::line_index;
use super::parse_cache;

/// One (at most) offense, packed for the Ruby wrapper as
/// `(kind, start, fin, line, insert_line, is_emacs)`:
///
/// - `kind` 0 = missing (`always`), 1 = missing-true (`always_true`),
///   2 = unnecessary (`never`), 3 = disabled (`always_true`).
/// - kinds 0/1: `insert_line` is the 1-based line to insert AFTER, or 0 to
///   prepend at the file start; the other fields are 0.
/// - kind 2: `start`/`fin` are the comment's byte range.
/// - kind 3: `start`/`fin` = comment byte range, `line` = its 1-based line,
///   `is_emacs` = 1 when the comment is an Emacs comment (drives the
///   replacement string).
pub type FslResult = (i64, i64, i64, i64, i64, i64);

pub fn check_frozen_string_literal_comment(source: &[u8], style: u8) -> Vec<FslResult> {
    if source.is_empty() {
        return Vec::new();
    }

    let (scan, last_comment_start, comments) =
        parse_cache::with_parsed_and_comments(source, |_owner, _root, comments| {
            let (scan, last_comment_start) = scan_front(source, &comments);
            (scan, last_comment_start, comments)
        });

    // `processed_source.tokens.empty?`: no comment tokens and no code token.
    if comments.is_empty() && matches!(scan, ScanEnd::NoToken { .. }) {
        return Vec::new();
    }

    line_index::with_line_index(source, |li| {
        let leading = leading_line_count(source, li, &scan, last_comment_start);

        // Leading-line frozen-string-literal facts (`exists?`, `specified?`,
        // `enabled?`): each leading LINE run through `MagicComment.parse`.
        let mut exists = false;
        let mut first_specified: Option<FslValue> = None;
        for &start in li.line_starts().iter().take(leading) {
            let v = line_fsl(line_slice(source, start));
            if matches!(v, Some(FslValue::True) | Some(FslValue::False)) {
                exists = true;
            }
            if v.is_some() && first_specified.is_none() {
                first_specified = v;
            }
        }
        let specified = first_specified.is_some();
        let enabled = first_specified == Some(FslValue::True);

        match style {
            // never
            1 => {
                if !exists {
                    return Vec::new();
                }
                match found_comment(source, &comments) {
                    None => Vec::new(),
                    Some((s, e, _line, _emacs)) => {
                        vec![(2, s as i64, e as i64, 0, 0, 0)]
                    }
                }
            }
            // always_true
            2 => {
                if specified {
                    if enabled {
                        return Vec::new();
                    }
                    match found_comment(source, &comments) {
                        None => Vec::new(),
                        Some((s, e, line, emacs)) => {
                            vec![(3, s as i64, e as i64, line as i64, 0, emacs as i64)]
                        }
                    }
                } else {
                    let ins = insert_line(source, &comments, scan, li);
                    vec![(1, 0, 0, 0, ins as i64, 0)]
                }
            }
            // always (default; also any other style value)
            _ => {
                if exists {
                    Vec::new()
                } else {
                    let ins = insert_line(source, &comments, scan, li);
                    vec![(0, 0, 0, 0, ins as i64, 0)]
                }
            }
        }
    })
}

/// The first comment TOKEN carrying a frozen-string-literal setting, returned
/// as `(start, end, line, is_emacs)`. Mirrors stock's
/// `tokens.find { MagicComment.parse(token.text).frozen_string_literal_specified? }`
/// restricted to comment tokens: the earlier a token, the earlier its byte
/// offset, and a body string that looks like an Emacs comment can never come
/// before the leading comment that already fired the gate, so the comment
/// list yields the same first match as the full token stream for every case
/// where an offense is emitted.
fn found_comment(source: &[u8], comments: &[(usize, usize)]) -> Option<(usize, usize, usize, bool)> {
    for &(s, e) in comments {
        let text = &source[s..e];
        if line_fsl(text).is_some() {
            // Recompute the line lazily only for the match.
            let line = source[..s].iter().filter(|&&b| b == b'\n').count() + 1;
            let is_emacs = emacs_token(text).is_some();
            return Some((s, e, line, is_emacs));
        }
    }
    None
}

/// `last_special_comment` / `insert_comment`: the 1-based line to insert the
/// fix AFTER, or 0 to prepend at the file start.
fn insert_line(
    source: &[u8],
    comments: &[(usize, usize)],
    scan: ScanEnd,
    li: &line_index::LineIndex,
) -> usize {
    let code = match scan {
        ScanEnd::Token(pos) => Some(pos),
        ScanEnd::NoToken { .. } => None,
    };
    // tokens[0] is a comment iff the earliest comment precedes the first code
    // token.
    let tok0 = comments
        .first()
        .filter(|c| code.is_none_or(|cp| c.0 < cp));
    let Some(&(s0, e0)) = tok0 else {
        // tokens[0] is code (or there are no tokens). A plain string token's
        // text can still read as an encoding comment.
        return code.map_or(0, |cp| code_token_encoding_line(source, cp, li));
    };
    let text0 = &source[s0..e0];
    if text0.starts_with(b"#!") {
        let shebang_line = li.line_of(s0);
        // tokens[1] after the shebang.
        let tok1 = comments.get(1).filter(|c| code.is_none_or(|cp| c.0 < cp));
        if let Some(&(s1, e1)) = tok1 {
            let text1 = &source[s1..e1];
            if valid_encoding(text1) && encoding_specified(text1) {
                return li.line_of(s1);
            }
        } else if let Some(cp) = code {
            let line = code_token_encoding_line(source, cp, li);
            if line != 0 {
                return line;
            }
        }
        return shebang_line;
    }
    if valid_encoding(text0) && encoding_specified(text0) {
        return li.line_of(s0);
    }
    0
}

/// The line of a code token at `pos` whose parser token text passes the
/// encoding check, or 0. Only a plain `'...'` / `"..."` string lexes as one
/// `tSTRING` token whose text is the string content; anything else (an
/// identifier, a keyword, a `tSTRING_BEG`) can never parse as a magic
/// comment. Since 1.92 stock asks `MagicComment.parse(text).encoding_specified?`,
/// which the Emacs form `-*- coding: x -*-` passes without a `#`.
fn code_token_encoding_line(source: &[u8], pos: usize, li: &line_index::LineIndex) -> usize {
    let Some(&quote) = source.get(pos).filter(|&&q| q == b'\'' || q == b'"') else {
        return 0;
    };
    let mut i = pos + 1;
    while let Some(&b) = source.get(i) {
        match b {
            b'\n' => return 0,
            b'\\' => i += 2,
            b'#' if quote == b'"' && matches!(source.get(i + 1), Some(b'{' | b'@' | b'$')) => {
                return 0;
            }
            _ if b == quote => {
                let text = &source[pos + 1..i];
                if valid_encoding(text) && encoding_specified(text) {
                    return li.line_of(pos);
                }
                return 0;
            }
            _ => i += 1,
        }
    }
    0
}

/// `text.valid_encoding?` for a UTF-8 source buffer.
fn valid_encoding(text: &[u8]) -> bool {
    std::str::from_utf8(text).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // (kind, start, fin, line, insert_line, is_emacs)
    fn run(src: &str, style: u8) -> Vec<FslResult> {
        check_frozen_string_literal_comment(src.as_bytes(), style)
    }

    // ---- gate ----
    #[test]
    fn empty_and_whitespace_no_offense() {
        for style in 0..=2 {
            assert!(run("", style).is_empty());
            assert!(run(" ", style).is_empty());
            assert!(run("  \n\t\n", style).is_empty());
        }
    }

    // ---- always ----
    #[test]
    fn always_missing_prepend() {
        assert_eq!(run("puts 1\n", 0), vec![(0, 0, 0, 0, 0, 0)]);
    }

    #[test]
    fn always_present_true_or_false_ok() {
        assert!(run("# frozen_string_literal: true\nputs 1\n", 0).is_empty());
        assert!(run("# frozen_string_literal: false\nputs 1\n", 0).is_empty());
    }

    #[test]
    fn always_token_is_missing() {
        // `token` is specified but not a valid literal → still missing.
        assert_eq!(run("# frozen_string_literal: token\nputs 1\n", 0), vec![(0, 0, 0, 0, 0, 0)]);
    }

    #[test]
    fn always_insert_after_shebang() {
        assert_eq!(run("#!/usr/bin/env ruby\nputs 1\n", 0), vec![(0, 0, 0, 0, 1, 0)]);
    }

    #[test]
    fn always_insert_after_encoding() {
        assert_eq!(run("# encoding: utf-8\nputs 1\n", 0), vec![(0, 0, 0, 0, 1, 0)]);
    }

    #[test]
    fn always_insert_after_non_utf8_encoding() {
        // 1.92 (rubocop#15799): any encoding comment counts, not only UTF-8.
        // Expected from the stock CLI (`-A`).
        assert_eq!(run("# encoding: ascii-8bit\nputs 1\n", 0), vec![(0, 0, 0, 0, 1, 0)]);
        assert_eq!(run("# -*- coding: binary -*-\nputs 1\n", 0), vec![(0, 0, 0, 0, 1, 0)]);
        // A plain string token reads as an Emacs encoding comment too (stock
        // CLI inserts after it on the first pass).
        assert_eq!(run("'-*- coding: binary -*-'\nputs 1\n", 0), vec![(0, 0, 0, 0, 1, 0)]);
        assert_eq!(run("'-*- coding: binary; x: #{x} -*-'\nputs 1\n", 0), vec![(0, 0, 0, 0, 1, 0)]);
        assert_eq!(run("\"-*- coding: binary; x: #{x} -*-\"\nputs 1\n", 0), vec![(0, 0, 0, 0, 0, 0)]);
    }

    #[test]
    fn always_insert_after_encoding_under_shebang() {
        assert_eq!(
            run("#!/usr/bin/env ruby\n# encoding: utf-8\nputs 1\n", 0),
            vec![(0, 0, 0, 0, 2, 0)]
        );
    }

    #[test]
    fn always_shebang_not_line_one_is_prepend() {
        assert_eq!(run("x = 1\n#!/bin/ruby\n", 0), vec![(0, 0, 0, 0, 0, 0)]);
    }

    // ---- never ----
    #[test]
    fn never_removes_first_specified() {
        // The `foo` line (specified) is removed even though the gate matched
        // the `true` line (valid literal).
        let src = "# frozen_string_literal: foo\n# frozen_string_literal: true\nputs 1\n";
        assert_eq!(run(src, 1), vec![(2, 0, 28, 0, 0, 0)]);
    }

    #[test]
    fn never_token_only_no_offense() {
        // Not a valid literal → does not "exist" → no offense.
        assert!(run("# frozen_string_literal: foo\nputs 1\n", 1).is_empty());
    }

    #[test]
    fn never_offense_range() {
        assert_eq!(run("# frozen_string_literal: true\nputs 1\n", 1), vec![(2, 0, 29, 0, 0, 0)]);
    }

    #[test]
    fn never_begin_block_no_offense() {
        // Gate fires (leading line inside =begin) but no comment token carries
        // the setting → stock raises → no offense.
        assert!(run("=begin\n# frozen_string_literal: true\n=end\nputs 1\n", 1).is_empty());
    }

    // ---- always_true ----
    #[test]
    fn always_true_missing() {
        assert_eq!(run("puts 1\n", 2), vec![(1, 0, 0, 0, 0, 0)]);
    }

    #[test]
    fn always_true_true_ok() {
        assert!(run("# frozen_string_literal: true\nputs 1\n", 2).is_empty());
    }

    #[test]
    fn always_true_false_disabled() {
        // kind 3, comment range 0..30, line 1, simple.
        assert_eq!(run("# frozen_string_literal: false\nputs 1\n", 2), vec![(3, 0, 30, 1, 0, 0)]);
    }

    #[test]
    fn always_true_token_disabled() {
        assert_eq!(run("# frozen_string_literal: token\nputs 1\n", 2), vec![(3, 0, 30, 1, 0, 0)]);
    }

    #[test]
    fn always_true_emacs_false_disabled() {
        // is_emacs = 1 → replacement uses the emacs form.
        let src = "# -*- frozen_string_literal: false -*-\nputs 1\n";
        assert_eq!(run(src, 2), vec![(3, 0, 38, 1, 0, 1)]);
    }

    #[test]
    fn always_true_first_specified_wins_enabled() {
        // First specified is `true` → enabled → no offense even with a later
        // false.
        let src = "# frozen_string_literal: true\n# frozen_string_literal: false\nputs 1\n";
        assert!(run(src, 2).is_empty());
    }

    #[test]
    fn always_true_first_specified_false_disabled() {
        let src = "# frozen_string_literal: false\n# frozen_string_literal: true\nputs 1\n";
        // Disabled at the first (false) comment: range 0..30, line 1.
        assert_eq!(run(src, 2), vec![(3, 0, 30, 1, 0, 0)]);
    }

    // ---- value parsing ----
    #[test]
    fn value_case_and_dashes() {
        assert!(run("# frozen-string-literal: TRUE\nputs 1\n", 0).is_empty()); // exists
        assert_eq!(run("# FROZEN_STRING_LITERAL: FALSE\nputs 1\n", 2), vec![(3, 0, 30, 1, 0, 0)]);
    }

    #[test]
    fn simple_fsl_anchored_tail() {
        // Trailing junk after the value disqualifies the simple form.
        assert_eq!(run("# frozen_string_literal: true extra\nputs 1\n", 0), vec![(0, 0, 0, 0, 0, 0)]);
    }
}
