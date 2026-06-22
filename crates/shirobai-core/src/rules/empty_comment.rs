//! `Layout/EmptyComment`.
//!
//! Flags comments that have nothing but `#` characters after stripping. Two
//! config flags shape the check:
//!
//! - `AllowBorderComment` (default true) — when true, `#######` (more than one
//!   `#`) is allowed; when false, even a long run of `#`s is empty.
//! - `AllowMarginComment` (default true) — when true, `#` lines adjacent to a
//!   real comment line are treated as `# foo\n#\n` margin and NOT flagged.
//!   When false, every empty `#` is flagged on its own.
//!
//! Stock's `empty_comment_pattern`:
//! - allow border: `/\A(#\n)+\z/` (one `#` per line, all-empty chunk)
//! - disallow border: `/\A(#+\n)+\z/` (any run of `#` per line)
//!
//! Stock joins `comment.text.strip + "\n"` for each comment in the chunk and
//! pattern-matches the join. We do the same byte-for-byte.
//!
//! Chunking (`AllowMarginComment: true`): consecutive comments belong to one
//! chunk when `prev.line.succ == cur.line && prev.column == cur.column`. Block
//! comments (`=begin/=end`, which span multiple lines) naturally split chunks
//! by the line gap.
//!
//! Autocorrect:
//! - When the comment is preceded by a token on the same line (any non-space
//!   byte earlier on the line), use `range_with_surrounding_space(newlines:
//!   false)` — the comment plus its leading horizontal whitespace, but the
//!   trailing `\n` is kept.
//! - Otherwise, `range_by_whole_lines(include_final_newline: true)` — the
//!   whole line(s) the comment occupies, including the final `\n`.
//!
//! Prism's comment `location` includes a trailing `\r` for CRLF endings while
//! parser-gem's `comment.source_range` excludes it. The Rust side snaps the
//! comment end back by one byte in that case so the offense range matches stock.

use super::line_index;
use super::parse_cache;

/// One offense candidate the Ruby wrapper turns into `add_offense` +
/// `corrector.remove`.
pub struct EmptyCommentOffense {
    /// Comment source range stock passes to `add_offense` (`comment.source_range`).
    pub offense_start: usize,
    pub offense_end: usize,
    /// Range stock passes to `corrector.remove`. Either
    /// `range_with_surrounding_space(newlines: false)` (same-line previous
    /// token) or `range_by_whole_lines(include_final_newline: true)`.
    pub ac_start: usize,
    pub ac_end: usize,
}

#[derive(Clone, Copy)]
pub struct Config {
    pub allow_border_comment: bool,
    pub allow_margin_comment: bool,
}

pub fn check_empty_comment(source: &[u8], cfg: Config) -> Vec<EmptyCommentOffense> {
    // Collect (start, end, line, column, normalized_text) for every comment.
    // Prism's comment ranges may include a trailing `\r` (CRLF); we snap the
    // end back so the reported offense range matches parser-gem.
    let comments_raw = parse_cache::comment_ranges(source);
    if comments_raw.is_empty() {
        return Vec::new();
    }
    let line_index = line_index::with_line_index(source, |li| li.clone());
    let comments: Vec<CommentInfo> = comments_raw
        .into_iter()
        .map(|(s, e_raw)| {
            let e = snap_trailing_cr(source, s, e_raw);
            let line = line_index.line_of(s);
            let line_start = line_index.line_start(s);
            let column = s - line_start;
            let normalized = normalize_text(&source[s..e]);
            CommentInfo {
                start: s,
                end: e,
                line,
                column,
                normalized,
            }
        })
        .collect();

    let mut out = Vec::new();
    if cfg.allow_margin_comment {
        // Chunk by `prev.line + 1 == cur.line && prev.column == cur.column`,
        // pattern-match the joined normalized text per chunk, and emit every
        // comment in matching chunks as an offense.
        let mut i = 0;
        while i < comments.len() {
            let mut j = i + 1;
            while j < comments.len()
                && comments[j - 1].line + 1 == comments[j].line
                && comments[j - 1].column == comments[j].column
            {
                j += 1;
            }
            let mut joined: Vec<u8> = Vec::new();
            for c in &comments[i..j] {
                joined.extend_from_slice(&c.normalized);
            }
            if is_empty_comment(&joined, cfg.allow_border_comment) {
                for c in &comments[i..j] {
                    out.push(make_offense(source, c, &line_index));
                }
            }
            i = j;
        }
    } else {
        for c in &comments {
            if is_empty_comment(&c.normalized, cfg.allow_border_comment) {
                out.push(make_offense(source, c, &line_index));
            }
        }
    }
    out
}

struct CommentInfo {
    start: usize,
    end: usize,
    line: usize,
    column: usize,
    normalized: Vec<u8>,
}

/// Build the offense + autocorrect range. The autocorrect arm depends on
/// whether the comment has a non-whitespace token earlier on the same line.
fn make_offense(
    source: &[u8],
    c: &CommentInfo,
    line_index: &line_index::LineIndex,
) -> EmptyCommentOffense {
    let line_start = line_index.line_start(c.start);
    let same_line_prev = has_non_whitespace_before(source, c.start, line_start);
    let (ac_start, ac_end) = if same_line_prev {
        range_with_surrounding_horizontal_space(source, c.start, c.end)
    } else {
        range_by_whole_lines_with_newline(source, line_start, c.end)
    };
    EmptyCommentOffense {
        offense_start: c.start,
        offense_end: c.end,
        ac_start,
        ac_end,
    }
}

/// `comment.text.strip + "\n"`. `strip` removes ASCII whitespace from both
/// ends — most relevantly, the trailing `\r` (if any) and any trailing
/// space/tab. Leading whitespace is trimmed too, but a comment always begins
/// with `#` or `=begin`, so trimming has no effect on the prefix.
fn normalize_text(slice: &[u8]) -> Vec<u8> {
    let trimmed = trim_ascii_whitespace(slice);
    let mut out = Vec::with_capacity(trimmed.len() + 1);
    out.extend_from_slice(trimmed);
    out.push(b'\n');
    out
}

fn trim_ascii_whitespace(s: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = s.len();
    while start < end && is_ascii_whitespace_strip(s[start]) {
        start += 1;
    }
    while end > start && is_ascii_whitespace_strip(s[end - 1]) {
        end -= 1;
    }
    &s[start..end]
}

/// Bytes Ruby's `String#strip` strips: space, tab, `\n`, `\r`, `\v`, `\f`,
/// `\0`.
fn is_ascii_whitespace_strip(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c | 0x00)
}

/// True iff the joined text matches stock's `empty_comment_pattern`. We test
/// each `<run-of-#>\n` segment in turn instead of compiling a regex.
fn is_empty_comment(text: &[u8], allow_border: bool) -> bool {
    if text.is_empty() {
        return false;
    }
    let mut i = 0;
    while i < text.len() {
        // Each segment must be one or more `#`, then exactly one `\n`.
        let seg_start = i;
        while i < text.len() && text[i] == b'#' {
            i += 1;
        }
        let hash_count = i - seg_start;
        if hash_count == 0 {
            return false;
        }
        if allow_border && hash_count != 1 {
            return false;
        }
        if i >= text.len() || text[i] != b'\n' {
            return false;
        }
        i += 1;
    }
    true
}

/// Walk backward from `comment_start - 1` to `line_start`. If any byte is
/// non-whitespace (anything other than ASCII space / tab), there is at least
/// one token on the same line before the comment. `\r` counts as whitespace
/// (CRLF normalization), but it is impossible to find `\r` mid-line; only
/// trailing `\r` immediately before `\n`. `\n` itself never appears in a
/// `[line_start, comment_start)` slice.
fn has_non_whitespace_before(source: &[u8], comment_start: usize, line_start: usize) -> bool {
    source[line_start..comment_start]
        .iter()
        .any(|&b| !matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c))
}

/// `range_with_surrounding_space(node.source_range, newlines: false)`:
/// expand the range by consuming runs of `\t` / ` ` on both sides. Bound by
/// the source ends.
fn range_with_surrounding_horizontal_space(
    source: &[u8],
    mut start: usize,
    mut end: usize,
) -> (usize, usize) {
    while start > 0 && matches!(source[start - 1], b' ' | b'\t') {
        start -= 1;
    }
    while end < source.len() && matches!(source[end], b' ' | b'\t') {
        end += 1;
    }
    (start, end)
}

/// `range_by_whole_lines(node.source_range, include_final_newline: true)`:
/// from the comment's line start to one byte past the closing `\n` (or EOF
/// when the file has no trailing newline). For a single-line `#` comment the
/// whole `# ...\n` line is dropped.
fn range_by_whole_lines_with_newline(
    source: &[u8],
    line_start: usize,
    comment_end: usize,
) -> (usize, usize) {
    // Find the first `\n` at or after `comment_end - 1`.
    let mut i = comment_end;
    while i < source.len() && source[i] != b'\n' {
        i += 1;
    }
    let end = if i < source.len() { i + 1 } else { source.len() };
    (line_start, end)
}

/// Prism's comment location includes a trailing `\r` (CRLF endings) while
/// parser-gem's `comment.source_range` excludes it. Snap the end back by one
/// byte when the slice ends in `\r`.
fn snap_trailing_cr(source: &[u8], start: usize, end: usize) -> usize {
    if end > start && source.get(end - 1) == Some(&b'\r') {
        end - 1
    } else {
        end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(border: bool, margin: bool) -> Config {
        Config {
            allow_border_comment: border,
            allow_margin_comment: margin,
        }
    }

    fn run(src: &str, c: Config) -> Vec<EmptyCommentOffense> {
        check_empty_comment(src.as_bytes(), c)
    }

    // Typical: single lonely `#` is empty and gets dropped.
    #[test]
    fn lonely_hash_default() {
        let got = run("#\n", cfg(true, true));
        assert_eq!(got.len(), 1);
        let o = &got[0];
        assert_eq!((o.offense_start, o.offense_end), (0, 1));
        // Whole-line removal including the newline.
        assert_eq!((o.ac_start, o.ac_end), (0, 2));
    }

    // Border allowed (default): `##########` is a divider, no offense.
    #[test]
    fn border_default_allows() {
        let got = run("##########\n", cfg(true, true));
        assert!(got.is_empty());
    }

    // Border disallowed: even a long run of `#` is empty.
    #[test]
    fn border_disallow_flags_long_run() {
        let got = run("##########\n", cfg(false, true));
        assert_eq!(got.len(), 1);
        let o = &got[0];
        assert_eq!((o.offense_start, o.offense_end), (0, 10));
    }

    // Margin allowed (default): `#` lines wrapping a text comment are ignored.
    #[test]
    fn margin_chunk_with_text() {
        let src = "#\n# foo\n#\n";
        let got = run(src, cfg(true, true));
        assert!(got.is_empty());
    }

    // Margin disallowed: each `#` is its own offense; the text line is skipped.
    #[test]
    fn margin_disallow_flags_each() {
        let src = "#\n# foo\n#\nx = 1\n";
        let got = run(src, cfg(true, false));
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].offense_start, got[0].offense_end), (0, 1));
        assert_eq!((got[1].offense_start, got[1].offense_end), (8, 9));
    }

    // Inline comment after code uses range_with_surrounding_space (no newlines).
    #[test]
    fn inline_after_code_strips_leading_space() {
        let src = "def foo #\n  bar\nend\n";
        let got = run(src, cfg(true, true));
        assert_eq!(got.len(), 1);
        let o = &got[0];
        assert_eq!((o.offense_start, o.offense_end), (8, 9));
        // Removes the leading space + `#`, leaves `\n` alone.
        assert_eq!((o.ac_start, o.ac_end), (7, 9));
    }

    // Inline without leading space: just remove the `#`.
    #[test]
    fn inline_no_leading_space() {
        let src = "def foo#\n  x\nend\n";
        let got = run(src, cfg(true, true));
        assert_eq!(got.len(), 1);
        let o = &got[0];
        assert_eq!((o.offense_start, o.offense_end), (7, 8));
        assert_eq!((o.ac_start, o.ac_end), (7, 8));
    }

    // CRLF: comment text is `#\r`, offense range still ends at `#` (parser-gem
    // shape). Autocorrect drops the whole `#\r\n`.
    #[test]
    fn crlf_line_ending() {
        let src = "#\r\n";
        let got = run(src, cfg(true, true));
        assert_eq!(got.len(), 1);
        let o = &got[0];
        assert_eq!((o.offense_start, o.offense_end), (0, 1));
        assert_eq!((o.ac_start, o.ac_end), (0, 3));
    }

    // Comment with trailing whitespace: strip removes it, pattern still matches.
    #[test]
    fn trailing_whitespace_still_empty() {
        let got = run("#  \n", cfg(true, true));
        assert_eq!(got.len(), 1);
        let o = &got[0];
        assert_eq!((o.offense_start, o.offense_end), (0, 3));
    }

    // Border in the middle of a chunk: with `AllowBorderComment: false` the
    // whole chunk pattern matches and every comment is flagged.
    #[test]
    fn border_in_chunk_disallow() {
        let src = "#\n#####\n#\n";
        let got = run(src, cfg(false, true));
        assert_eq!(got.len(), 3);
    }

    // Same input under `AllowBorderComment: true` does NOT match the all-`#\n`
    // pattern, so no offense.
    #[test]
    fn border_in_chunk_allow() {
        let src = "#\n#####\n#\n";
        let got = run(src, cfg(true, true));
        assert!(got.is_empty());
    }

    // No trailing newline at EOF: still flagged, ac end clamps at source len.
    #[test]
    fn no_trailing_newline() {
        let src = "x = 1\n#";
        let got = run(src, cfg(true, true));
        assert_eq!(got.len(), 1);
        let o = &got[0];
        assert_eq!((o.offense_start, o.offense_end), (6, 7));
        // Walk back from offset 6 to line_start 6: no prev token on line → whole
        // line removal, end clamped to EOF.
        assert_eq!((o.ac_start, o.ac_end), (6, 7));
    }

    // Indented chunks at different columns split the chunk, but each indented
    // chunk pattern-matches as all-`#\n` and gets flagged.
    #[test]
    fn indented_chunks_split() {
        let src = "  #\n  #\nx = 1\n   #\n   #\n";
        let got = run(src, cfg(true, true));
        // First chunk: lines 1-2 at col 2 — both flagged.
        // Skip text line.
        // Second chunk: lines 4-5 at col 3 — both flagged.
        assert_eq!(got.len(), 4);
    }

    // Block comment splits the chunk by line gap (block `=begin` starts at line
    // 2, body extends to line 3, so the next `#` at line 4 is not chunked with
    // the block comment). The first `#`+block chunk joins as `#\n=begin\n=end\n`
    // which doesn't match either pattern. The lone `#` at line 4 forms its own
    // chunk and is flagged. Matches stock: one offense at offset 14.
    #[test]
    fn block_comment_splits_chunk() {
        let src = "#\n=begin\n=end\n#\n";
        let got = run(src, cfg(true, true));
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].offense_start, got[0].offense_end), (14, 15));
    }

    // Aligned inline comments next to code (same column): two chunks (different
    // lines but same column), each is its own offense in default mode.
    #[test]
    fn aligned_inline_columns_same() {
        let src = "def foo     #\n  bar       #\nend\n";
        let got = run(src, cfg(true, true));
        assert_eq!(got.len(), 2);
        // Both inline → surrounding-space removal.
        assert_eq!((got[0].ac_start, got[0].ac_end), (7, 13));
        assert_eq!((got[1].ac_start, got[1].ac_end), (19, 27));
    }

    // `##` under default (allow border) is a 2-char border → no offense.
    #[test]
    fn double_hash_border_allowed() {
        let got = run("##\n", cfg(true, true));
        assert!(got.is_empty());
    }

    // Same input with border disallowed → flagged.
    #[test]
    fn double_hash_border_disallowed() {
        let got = run("##\n", cfg(false, true));
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].offense_start, got[0].offense_end), (0, 2));
    }
}
