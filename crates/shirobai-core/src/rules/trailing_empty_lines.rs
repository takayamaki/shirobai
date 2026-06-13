//! `Layout/TrailingEmptyLines`.
//!
//! Checks the very end of the source for the right number of trailing blank
//! lines and a final newline. This cop touches no AST at all: stock's
//! `on_new_investigation` looks only at the source buffer, so the whole check is
//! a byte scan of the trailing whitespace run.
//!
//! Stock's algorithm (`Layout/TrailingEmptyLines#on_new_investigation`):
//!
//! ```text
//! return if buffer.source.empty?
//! return if ends_in_end?(processed_source)                 # /\s*__END__/ substring
//! return if end_with_percent_blank_string?(processed_source) # ends with "%\n\n"
//! whitespace_at_end = source[/\s*\Z/]                       # longest trailing \s run
//! blank_lines = whitespace_at_end.count("\n") - 1
//! wanted_blank_lines = style == :final_newline ? 0 : 1
//! return unless blank_lines != wanted_blank_lines
//! # offense
//! begin_pos = source.length - whitespace_at_end.length
//! autocorrect_range = [begin_pos, source.length)
//! begin_pos += 1 unless whitespace_at_end.empty?
//! report_range = [begin_pos, source.length)
//! corrector.replace(autocorrect_range, final_newline ? "\n" : "\n\n")
//! ```
//!
//! Two stock quirks, both verified against the real CLI:
//!
//! - `ends_in_end?` first does `buffer.source.match?(/\s*__END__/)`, whose `\s*`
//!   is optional, so it matches whenever the *byte substring* `__END__` appears
//!   **anywhere** in the source — even inside a string literal, a comment, or a
//!   larger identifier (`MY__END__X`). That short-circuits `return true` before
//!   the token-based fallback ever runs, so the token check is dead code and we
//!   reproduce only the substring test.
//! - `end_with_percent_blank_string?` bails when the source literally ends with
//!   `"%\n\n"` (the parser would otherwise see a dangling `%` literal).
//!
//! `\s` here is Ruby's default (ASCII) class `[ \t\r\n\f\v]`, so CRLF endings
//! (`\r\n\r\n`) collapse into one trailing run and `count("\n")` still drives
//! `blank_lines`. Offsets are byte offsets; the Ruby wrapper maps them through
//! `SourceOffsets` (the trailing run is ASCII, but a preceding multibyte body
//! shifts byte vs. char positions, so the conversion is mandatory).

/// `Layout/TrailingEmptyLines` configuration.
#[derive(Clone, Copy)]
pub struct Config {
    /// 0 = `final_newline` (want 0 trailing blank lines), 1 = `final_blank_line`
    /// (want 1 trailing blank line).
    pub style: u8,
}

pub const STYLE_FINAL_NEWLINE: u8 = 0;
pub const STYLE_FINAL_BLANK_LINE: u8 = 1;

/// The single offense a file can carry (there is at most one).
pub struct TrailingEmptyLinesOffense {
    /// Reported caret range `[report_start, report_end)` (byte offsets).
    pub report_start: usize,
    pub report_end: usize,
    /// Autocorrect replacement range `[ac_start, ac_end)` (byte offsets).
    pub ac_start: usize,
    pub ac_end: usize,
    /// Replacement text stock writes into the autocorrect range.
    pub replacement: &'static str,
    /// `whitespace_at_end.count("\n") - 1`. Drives stock's message (`-1` =
    /// "Final newline missing.", `0` = "Trailing blank line missing.", else the
    /// "N trailing blank lines …" form). The Ruby wrapper builds the message.
    pub blank_lines: i64,
}

/// Is `b` in Ruby's default `\s` class (`[ \t\r\n\f\v]`)?
fn is_ascii_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0x0b)
}

/// Reproduce stock's `on_new_investigation`. Returns the lone offense, or `None`
/// when the file is clean / exempt.
pub fn check_trailing_empty_lines(source: &[u8], cfg: &Config) -> Option<TrailingEmptyLinesOffense> {
    if source.is_empty() {
        return None;
    }

    // `ends_in_end?`: `/\s*__END__/` is an unanchored substring match.
    if contains_subslice(source, b"__END__") {
        return None;
    }

    // `end_with_percent_blank_string?`.
    if source.ends_with(b"%\n\n") {
        return None;
    }

    // `whitespace_at_end = source[/\s*\Z/]`: the longest trailing run of `\s`.
    let mut ws_start = source.len();
    while ws_start > 0 && is_ascii_ws(source[ws_start - 1]) {
        ws_start -= 1;
    }
    let whitespace_at_end = &source[ws_start..];
    let ws_empty = whitespace_at_end.is_empty();

    let newline_count = whitespace_at_end.iter().filter(|&&b| b == b'\n').count() as i64;
    let blank_lines = newline_count - 1;
    let wanted_blank_lines: i64 = if cfg.style == STYLE_FINAL_NEWLINE { 0 } else { 1 };

    if blank_lines == wanted_blank_lines {
        return None;
    }

    let begin_pos = ws_start; // source.length - whitespace_at_end.length
    let ac_start = begin_pos;
    let ac_end = source.len();
    let report_start = if ws_empty { begin_pos } else { begin_pos + 1 };
    let replacement = if cfg.style == STYLE_FINAL_NEWLINE {
        "\n"
    } else {
        "\n\n"
    };

    Some(TrailingEmptyLinesOffense {
        report_start,
        report_end: source.len(),
        ac_start,
        ac_end,
        replacement,
        blank_lines,
    })
}

/// Does `haystack` contain `needle` as a contiguous byte subslice?
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Option<(usize, usize, usize, usize, String, i64)> {
        check_trailing_empty_lines(source.as_bytes(), &Config { style }).map(|o| {
            (
                o.report_start,
                o.report_end,
                o.ac_start,
                o.ac_end,
                o.replacement.to_string(),
                o.blank_lines,
            )
        })
    }

    // Typical (final_newline): a file ending in exactly one newline is clean.
    #[test]
    fn final_newline_single_newline_ok() {
        assert_eq!(run("x = 0\n", STYLE_FINAL_NEWLINE), None);
    }

    // Typical: an empty file is always accepted.
    #[test]
    fn empty_file_ok() {
        assert_eq!(run("", STYLE_FINAL_NEWLINE), None);
        assert_eq!(run("", STYLE_FINAL_BLANK_LINE), None);
    }

    // Typical (final_newline): a missing final newline reports a zero-width
    // range at end-of-source and inserts "\n".
    #[test]
    fn final_newline_missing() {
        // "x = 0" len 5, no trailing ws -> report [5,5), ac [5,5), blank -1.
        assert_eq!(
            run("x = 0", STYLE_FINAL_NEWLINE),
            Some((5, 5, 5, 5, "\n".to_string(), -1))
        );
    }

    // Typical (final_newline): a single trailing blank line is removed; the
    // report range starts one byte after the first newline.
    #[test]
    fn final_newline_one_trailing_blank() {
        // "x = 0\n\n": ws run = "\n\n" at [5,7), count("\n")=2 -> blank 1.
        // report begin 5+1=6, ac [5,7) replace "\n".
        assert_eq!(
            run("x = 0\n\n", STYLE_FINAL_NEWLINE),
            Some((6, 7, 5, 7, "\n".to_string(), 1))
        );
    }

    // Multiple trailing blank lines: still one offense, ac spans the whole run.
    #[test]
    fn final_newline_multi_trailing_blank() {
        // "x = 0\n\n\n\n": ws [5,9) count 4 -> blank 3.
        assert_eq!(
            run("x = 0\n\n\n\n", STYLE_FINAL_NEWLINE),
            Some((6, 9, 5, 9, "\n".to_string(), 3))
        );
    }

    // Trailing whitespace that includes spaces is part of the run.
    #[test]
    fn final_newline_spaces_in_blanks() {
        // "x = 0\n   \n\n\n": ws starts at index 5, len 7.
        let src = "x = 0\n   \n\n\n";
        assert_eq!(
            run(src, STYLE_FINAL_NEWLINE),
            Some((6, src.len(), 5, src.len(), "\n".to_string(), 3))
        );
    }

    // A file of only newlines: blank_lines = count - 1.
    #[test]
    fn only_newlines() {
        // "\n\n\n": ws [0,3) count 3 -> blank 2, report begin 0+1=1.
        assert_eq!(
            run("\n\n\n", STYLE_FINAL_NEWLINE),
            Some((1, 3, 0, 3, "\n".to_string(), 2))
        );
    }

    // Typical (final_blank_line): a file ending in one blank line is clean.
    #[test]
    fn final_blank_line_one_blank_ok() {
        assert_eq!(run("x = 0\n\n", STYLE_FINAL_BLANK_LINE), None);
    }

    // final_blank_line: only a final newline -> "Trailing blank line missing.".
    #[test]
    fn final_blank_line_needs_blank() {
        // "x = 0\n": ws "\n" count 1 -> blank 0, wanted 1. report begin 5+1=6.
        assert_eq!(
            run("x = 0\n", STYLE_FINAL_BLANK_LINE),
            Some((6, 6, 5, 6, "\n\n".to_string(), 0))
        );
    }

    // final_blank_line: missing final newline -> blank -1, insert "\n\n".
    #[test]
    fn final_blank_line_missing_newline() {
        assert_eq!(
            run("x = 0", STYLE_FINAL_BLANK_LINE),
            Some((5, 5, 5, 5, "\n\n".to_string(), -1))
        );
    }

    // final_blank_line: too many blanks collapse to one.
    #[test]
    fn final_blank_line_multi_collapses() {
        // "x = 0\n\n\n": ws [5,8) count 3 -> blank 2, wanted 1, replace "\n\n".
        assert_eq!(
            run("x = 0\n\n\n", STYLE_FINAL_BLANK_LINE),
            Some((6, 8, 5, 8, "\n\n".to_string(), 2))
        );
    }

    // CRLF: `\r` counts as `\s`, so "\r\n\r\n" is one run, count("\n")=2.
    #[test]
    fn crlf_trailing_blank_final_newline() {
        // "x = 0\r\n\r\n": ws [5,9) count 2 -> blank 1, wanted 0, replace "\n".
        assert_eq!(
            run("x = 0\r\n\r\n", STYLE_FINAL_NEWLINE),
            Some((6, 9, 5, 9, "\n".to_string(), 1))
        );
    }

    // CRLF: one blank line under final_blank_line is clean.
    #[test]
    fn crlf_trailing_blank_final_blank_line_ok() {
        assert_eq!(run("x = 0\r\n\r\n", STYLE_FINAL_BLANK_LINE), None);
    }

    // `__END__` anywhere (even mid-file with data after) exempts the file.
    #[test]
    fn end_marker_exempts() {
        assert_eq!(run("x = 0\n__END__\n\n\n\n", STYLE_FINAL_NEWLINE), None);
    }

    // `__END__` as a byte substring inside a string literal still exempts
    // (stock's regex is an unanchored substring match).
    #[test]
    fn end_marker_substring_exempts() {
        assert_eq!(run("x = \"__END__\"\n\n\n", STYLE_FINAL_NEWLINE), None);
        assert_eq!(run("MY__END__X = 1\n\n\n", STYLE_FINAL_NEWLINE), None);
    }

    // The `%\n\n` percent-form ending is exempt.
    #[test]
    fn percent_blank_string_exempts() {
        assert_eq!(run("%\n\n", STYLE_FINAL_NEWLINE), None);
        assert_eq!(run("x = %\n\n", STYLE_FINAL_NEWLINE), None);
        assert_eq!(run("%\n\n", STYLE_FINAL_BLANK_LINE), None);
    }

    // But `%\n\n\n` does NOT end with `%\n\n`, so it offends.
    #[test]
    fn percent_extra_blank_offends() {
        // "%\n\n\n": ws [1,4) count 3 -> blank 2, report begin 1+1=2.
        assert_eq!(
            run("%\n\n\n", STYLE_FINAL_NEWLINE),
            Some((2, 4, 1, 4, "\n".to_string(), 2))
        );
    }
}
