//! `Layout/InitialIndentation`.
//!
//! Stock's `on_new_investigation`
//! (`vendor/rubocop/lib/rubocop/cop/layout/initial_indentation.rb`):
//!
//! 1. `first_token = processed_source.tokens.find { |t| !t.text.start_with?('#') }`
//!    — the first token that is not a `#` LINE comment. `=begin`/`=end` block
//!    comments start with `=`, so they are NOT skipped and become the first
//!    token themselves (always at column 0).
//! 2. `return unless first_token` (a `__END__`-only or empty file has none).
//! 3. `return if first_token.column.zero?`.
//! 4. `range_with_surrounding_space(first_token.pos, side: :left, newlines: false)`
//!    extends the token left over horizontal space; `return if` it did not move
//!    (no space to the left — the byte-order-mark case). Otherwise offense at
//!    `first_token.pos`, autocorrect removes the leading space.
//!
//! This module reproduces only the CHEAP part: does an offense exist? A byte
//! scan finds the first non-comment token's start and answers "is it preceded
//! by horizontal indentation on its line?". The full offense construction
//! (`first_token.pos`, the removal range) is left to the Ruby wrapper, which
//! runs stock's exact `first_token` / `space_before` logic — so the offense and
//! autocorrect bytes are stock's own, by construction, and byte-parity is
//! guaranteed.
//!
//! Why this split. Stock's `first_token` materializes the parser-gem token
//! stream (rubocop-ast's lazy `tokens`) on EVERY file — the "toucher" cost this
//! program targets. The vast majority of files start at column 0 (no offense);
//! the byte scan settles them without touching tokens. Only on the rare file
//! that actually has an indented first line does the wrapper fall through to
//! stock's token-based construction. Because the scan can only ever OVER-report
//! (it never skips a token stock would keep), a false "true" is harmless: the
//! wrapper's stock `space_before` guard filters it out (`column.zero?` /
//! no-space-to-the-left both still apply). It must never UNDER-report, which it
//! does not — it identifies the same first-token byte as stock.
//!
//! BOM / `__END__` notes:
//! - A leading UTF-8 BOM (`EF BB BF`) is skipped; the first line's content
//!   starts after it, so `﻿puts` (token right after the BOM) is column 0 (no
//!   offense) while `﻿  puts` is indented (offense).
//! - `__END__` at column 0 is a normal identifier byte to the scan, so it lands
//!   at column 0 (no offense) — same result as stock, whose lexer stops there
//!   with no token. Indented `__END__` diverges (prism treats it as an
//!   identifier; parser-gem stops), the documented TargetRubyVersion edge; the
//!   wrapper's stock re-check makes even that emit stock's own answer.

/// True iff the file's first non-comment token is preceded by horizontal
/// indentation on its own line (i.e. stock would register an offense — modulo
/// the wrapper's exact `space_before` re-check, which can only narrow this).
pub fn check_initial_indentation(source: &[u8]) -> bool {
    let len = source.len();
    // Skip a leading UTF-8 BOM: the first line's column-0 content is after it.
    let mut pos = if source.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
    // Start of the current line's content (after the BOM on line 1).
    let mut line_start = pos;
    loop {
        if pos >= len {
            return false; // no non-comment token
        }
        match source[pos] {
            b'\n' => {
                pos += 1;
                line_start = pos;
            }
            b' ' | b'\t' | b'\r' | 0x0C | 0x0B => {
                pos += 1;
            }
            b'#' => {
                // A `#` line comment is skipped (its bytes are not a token);
                // the next loop turn consumes the newline and resets line_start.
                while pos < len && source[pos] != b'\n' {
                    pos += 1;
                }
            }
            b'\\' if source.get(pos + 1) == Some(&b'\n') => {
                // `\`-newline continuation produces no token.
                pos += 2;
                line_start = pos;
            }
            b'\\' if source.get(pos + 1) == Some(&b'\r')
                && source.get(pos + 2) == Some(&b'\n') =>
            {
                pos += 3;
                line_start = pos;
            }
            _ => {
                // First non-comment token starts at `pos`; offense iff it is
                // preceded by indentation on its line.
                return pos > line_start;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> bool {
        check_initial_indentation(src.as_bytes())
    }

    // Typical offense: an indented first line of code.
    #[test]
    fn indented_def() {
        assert!(run("  def f\n  end\n"));
    }

    // Typical clean: column-0 start.
    #[test]
    fn unindented_def() {
        assert!(!run("def f\nend\n"));
    }

    // A `#` line comment is skipped; the indented code after it still offends.
    #[test]
    fn indented_code_after_comment() {
        assert!(run("# c\n  x = 1\n"));
        assert!(run("   # comment\n   x = 1\n"));
    }

    // An unindented comment + unindented code is clean.
    #[test]
    fn unindented_comment_and_code() {
        assert!(!run("# comment\nx = 1\n"));
    }

    // A `=begin`/`=end` block comment is the first token itself (column 0), so
    // indented code AFTER it is never reached: no offense.
    #[test]
    fn block_comment_then_indented_code() {
        assert!(!run("=begin\nhi\n=end\n  code\n"));
    }

    // Empty / comment-only / `__END__`-only files have no token: no offense.
    #[test]
    fn no_token_files() {
        assert!(!run(""));
        assert!(!run("# only a comment\n"));
        assert!(!run("__END__\ndata\n"));
    }

    // Tab indentation counts.
    #[test]
    fn tab_indent() {
        assert!(run("\tputs 1\n"));
    }

    // BOM: a token right after the BOM is column 0 (clean); indentation after
    // the BOM offends.
    #[test]
    fn bom_cases() {
        assert!(!run("\u{feff}puts 1\n"));
        assert!(run("\u{feff}  puts 1\n"));
        assert!(run("\u{feff}# comment\n  puts 1\n"));
    }

    // Blank lines before the first token do not by themselves offend (the token
    // sits at column 0 on its own line).
    #[test]
    fn blank_lines_then_col0() {
        assert!(!run("\n\ndef f\nend\n"));
    }

    // A `\`-newline continuation before an indented token: the token's own line
    // indentation still offends.
    #[test]
    fn backslash_continuation_then_indent() {
        assert!(run("\\\n  puts 1\n"));
    }
}
