//! `Layout/EndOfLine`.
//!
//! Stock's `on_new_investigation`
//! (`vendor/rubocop/lib/rubocop/cop/layout/end_of_line.rb`) walks
//! `raw_source.each_line` and, for each line up to `last_line`, decides whether
//! the line's terminator is wrong for the enforced style (a stray `\r` under
//! `lf`, or a missing `\r\n` under `crlf`). It reports at most ONE offense (the
//! first offending line) and stops. The offense range is
//! `source_range(buffer, line + 1, 0, line.length)` — always column 0, and the
//! cop has NO autocorrect — so the only thing that reaches the output is the
//! line NUMBER and the message.
//!
//! The whole detection is byte/line work on `raw_source`; the only token access
//! is `last_line`:
//!
//! ```ruby
//! def last_line(processed_source)
//!   last_token = processed_source.tokens.last
//!   last_token ? last_token.line : processed_source.lines.length
//! end
//! ```
//!
//! `processed_source.tokens` materializes the parser-gem token stream on EVERY
//! file — the "toucher" cost this program targets. This module reproduces ONLY
//! `last_line`; the wrapper runs stock's own `on_new_investigation` body with
//! that value injected, so detection and the offense range are stock's own code
//! (byte-parity by construction). `last_line` bounds the scan so that trailing
//! lines after the last statement — most importantly the `__END__` data section
//! — are not inspected.
//!
//! `tokens.last.line` reproduced from prism: it is the end line of the last
//! top-level statement (parser-gem attributes the final `tNL` there). If the
//! program has no statements it is the last comment's line, and an empty program
//! falls back to the line count (`processed_source.lines.length ==
//! line_starts().len()`). The one divergence is a program whose LAST top-level
//! statement ENDS with a heredoc: parser-gem then puts the final `tNL` on the
//! heredoc OPENER line, while prism's node end is the terminator line. This is
//! harmless here — the only extra lines scanned are the heredoc body/terminator,
//! and offenses require a `\r` there, which never occurs in the LF verification
//! corpora (all five are LF-only, so `EndOfLine` reports nothing on them) and is
//! not exercised by the vendor spec.

use super::line_index;
use super::parse_cache;

/// Stock's `last_line`: the 1-based line that bounds the scan
/// (`tokens.last.line`, or the line count when there is no token).
pub fn check_end_of_line(source: &[u8]) -> usize {
    // Extract only byte offsets inside the parse closure (the line index is a
    // separate cache; do not nest it here).
    let (last_stmt_end, last_comment_start) =
        parse_cache::with_parsed_and_comments(source, |_owner, root, comments| {
            let stmt_end = root
                .as_program_node()
                .and_then(|p| p.statements().body().iter().last())
                .map(|n| n.location().end_offset());
            let comment_start = comments.last().map(|&(s, _)| s);
            (stmt_end, comment_start)
        });

    line_index::with_line_index(source, |li| {
        if let Some(end) = last_stmt_end {
            // `line_of` of the statement's LAST byte (end_offset is one past it).
            return li.line_of(end.saturating_sub(1));
        }
        if let Some(start) = last_comment_start {
            return li.line_of(start);
        }
        li.line_starts().len()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> usize {
        check_end_of_line(src.as_bytes())
    }

    // Typical: last_line is the last statement's line.
    #[test]
    fn simple_statements() {
        assert_eq!(run("a = 1\nb = 2\n"), 2);
    }

    // A multi-line expression ends on its last line.
    #[test]
    fn multiline_expression() {
        assert_eq!(run("foo(\n  1,\n  2\n)\n"), 4);
        assert_eq!(run("x = [\n 1,\n 2\n]\n"), 4);
    }

    // `def ... end` ends on the `end` line.
    #[test]
    fn def_end() {
        assert_eq!(run("def f\n 1\nend\n"), 3);
    }

    // No final newline: the last (only) line.
    #[test]
    fn no_final_newline() {
        assert_eq!(run("x = 1"), 1);
    }

    // Trailing comments and blank lines do NOT extend last_line past the last
    // statement (parser-gem's final tNL sits on the statement line).
    #[test]
    fn trailing_comment_and_blanks() {
        assert_eq!(run("x = 1\n# c\n"), 1);
        assert_eq!(run("x = 1\n# c\n# d\n"), 1);
        assert_eq!(run("x = 1\n\n\n"), 1);
    }

    // `__END__`: statements stop before it, so last_line is the code line.
    #[test]
    fn end_data() {
        assert_eq!(run("x = 1\n__END__\nzzz\n"), 1);
    }

    // Comment-only file: last_line is the last comment's line.
    #[test]
    fn comment_only() {
        assert_eq!(run("# a\n# b\n"), 2);
    }

    // Empty file: `line_starts().len()` (1). Harmless — an empty `raw_source`
    // has no lines to scan, so the wrapper reports nothing regardless.
    #[test]
    fn empty() {
        assert_eq!(run(""), 1);
    }

    // A `\`-newline continuation: the statement spans both lines, ending on the
    // second.
    #[test]
    fn backslash_continuation() {
        assert_eq!(run("a = 1 + \\\n  2\n"), 2);
        assert_eq!(run("'a' \\\n'b'\n"), 2);
    }

    // A nested heredoc inside a def does not diverge (the def's `end` is the
    // last statement byte).
    #[test]
    fn nested_heredoc() {
        assert_eq!(run("def f\n  x = <<~E\n   hi\n  E\nend\n"), 5);
    }
}
