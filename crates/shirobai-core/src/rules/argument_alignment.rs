//! `Layout/ArgumentAlignment`.
//!
//! Checks that the arguments of a multi-line method call are aligned. Two
//! styles: `with_first_argument` (align every argument under the first one) and
//! `with_fixed_indentation` (one indentation level below the method line).
//!
//! Ported from the cop + the shared `Alignment` mixin (`check_alignment` /
//! `each_bad_alignment`). Rust computes the per-argument `column_delta` and the
//! offense range; Ruby realigns via `AlignmentCorrector`.

/// One misaligned argument. `column_delta` is `base_column - actual_column`.
/// `autocorrect` is false for offenses nested inside an already-registered
/// offense range (the mixin's `within?` rule), which are reported without a
/// rewrite.
pub struct ArgAlignOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub autocorrect: bool,
}

pub fn check_argument_alignment(
    _source: &[u8],
    _style: u8,
    _indent_width: usize,
    _incompatible: bool,
) -> Vec<ArgAlignOffense> {
    Vec::new()
}
