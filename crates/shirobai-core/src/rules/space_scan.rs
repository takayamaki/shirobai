//! Byte-level scans shared by the space-inside/before delimiter cops
//! (`Layout/SpaceInsideHashLiteralBraces`, `Layout/SpaceInsideArrayLiteralBrackets`,
//! `Layout/SpaceBeforeBlockBraces`).
//!
//! Stock implements these cops over `processed_source.tokens`. The scans here
//! reconstruct the token-level facts those cops read from byte inspection:
//!
//! - the start of the next token after a delimiter (whitespace and `\`-newline
//!   line continuations are not tokens, comments are);
//! - the end of the previous token before a delimiter on the same line;
//! - the `[ \t]` / `[ \t]`-then-`\n` runs used by `range_of_space_to_the_*`,
//!   `side_space_range` and `range_with_surrounding_space`.
//!
//! All character classes involved (`\s`, `[ \t]`, `\n`) are ASCII, so byte
//! scanning matches parser's character positions exactly.

/// Ruby `/\s/`: space, tab, newline, carriage return, form feed and vertical
/// tab. `u8::is_ascii_whitespace` omits the vertical tab (`\x0B`).
pub(crate) fn is_ruby_space(b: u8) -> bool {
    b.is_ascii_whitespace() || b == 0x0B
}

/// `[ \t]` run end going right from `pos` (RangeHelp `reposition` step `1`,
/// `SINGLE_SPACE_REGEXP`).
pub(crate) fn skip_space_right(source: &[u8], mut pos: usize) -> usize {
    while matches!(source.get(pos), Some(b' ' | b'\t')) {
        pos += 1;
    }
    pos
}

/// `[ \t]` run begin going left from `pos` (RangeHelp `reposition` step `-1`).
pub(crate) fn skip_space_left(source: &[u8], mut pos: usize) -> usize {
    while pos > 0 && matches!(source.get(pos - 1), Some(b' ' | b'\t')) {
        pos -= 1;
    }
    pos
}

/// `side_space_range(side: :left, include_newlines: true)`: one loop skipping
/// `[ \t]` or `\n` interleaved, going left.
pub(crate) fn skip_space_and_newlines_left(source: &[u8], mut pos: usize) -> usize {
    while pos > 0 && matches!(source.get(pos - 1), Some(b' ' | b'\t' | b'\n')) {
        pos -= 1;
    }
    pos
}

/// `side_space_range(side: :right, include_newlines: true)`: one loop skipping
/// `[ \t]` or `\n` interleaved, going right.
pub(crate) fn skip_space_and_newlines_right(source: &[u8], mut pos: usize) -> usize {
    while matches!(source.get(pos), Some(b' ' | b'\t' | b'\n')) {
        pos += 1;
    }
    pos
}

/// `range_with_surrounding_space(side: :left)` with default flags: skip
/// `[ \t]`, then `\n` (two sequential passes, not interleaved — a space
/// *before* a newline is left in place, matching stock's one-pass behavior).
pub(crate) fn surrounding_space_left(source: &[u8], pos: usize) -> usize {
    let mut pos = skip_space_left(source, pos);
    while pos > 0 && source.get(pos - 1) == Some(&b'\n') {
        pos -= 1;
    }
    pos
}

/// Start of the next token after `pos` and whether a line break was crossed.
///
/// Skips whitespace (`\s`, newline included) and `\`-newline continuations —
/// exactly the byte sequences that separate two adjacent parser tokens.
/// Comments are tokens: the scan stops at a `#`. `crossed` mirrors
/// `token1.line < token2.line` (a `\n` was passed, directly or inside a
/// continuation).
pub(crate) fn next_token_start(source: &[u8], mut pos: usize) -> (usize, bool) {
    let mut crossed = false;
    loop {
        match source.get(pos) {
            Some(&b) if is_ruby_space(b) => {
                if b == b'\n' {
                    crossed = true;
                }
                pos += 1;
            }
            Some(&b'\\') => match source.get(pos + 1) {
                Some(&b'\n') => {
                    crossed = true;
                    pos += 2;
                }
                Some(&b'\r') if source.get(pos + 2) == Some(&b'\n') => {
                    crossed = true;
                    pos += 3;
                }
                _ => return (pos, crossed),
            },
            _ => return (pos, crossed),
        }
    }
}

/// End of the previous token before `pos` when it sits on the same line.
///
/// Skips `[ \t\f\v\r]` going left; a `\n` means the previous token is on an
/// earlier line (`token1.line < token2.line` in stock's check) and yields
/// `None`. The gap between two same-line adjacent tokens contains only such
/// whitespace, so the first non-whitespace byte is the previous token's last
/// byte.
pub(crate) fn prev_token_end_same_line(source: &[u8], mut pos: usize) -> Option<usize> {
    while pos > 0 {
        let b = source[pos - 1];
        if b == b'\n' {
            return None;
        }
        if is_ruby_space(b) {
            pos -= 1;
        } else {
            return Some(pos);
        }
    }
    Some(pos)
}

/// End of the previous non-whitespace run before `pos`, crossing newlines
/// (the token walk that skips `tNL` tokens). `None` when only whitespace
/// precedes.
pub(crate) fn prev_non_space(source: &[u8], mut pos: usize) -> Option<usize> {
    while pos > 0 {
        if is_ruby_space(source[pos - 1]) {
            pos -= 1;
        } else {
            return Some(pos);
        }
    }
    None
}
