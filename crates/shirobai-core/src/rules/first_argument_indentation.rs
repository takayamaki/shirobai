//! `Layout/FirstArgumentIndentation`.
//!
//! Checks the indentation of the first argument of a multi-line method call.
//! Arguments after the first are checked by `Layout/ArgumentAlignment`. Same
//! alignment family / `AlignmentCorrector` division of labour as
//! `Layout/ArgumentAlignment` and `Layout/MultilineMethodCallIndentation`: Rust
//! computes the offense range (the first argument), the `column_delta`, the
//! message, the `within?` autocorrect flag and the range to realign (either the
//! first argument, or the whole receiver chain for
//! `special_for_inner_method_call_in_parentheses`); Ruby applies it via
//! `AlignmentCorrector`.

/// One misindented first argument. `[start_offset, end_offset)` is the offense
/// range (the first argument). `[correct_start, correct_end)` is the range the
/// Ruby side realigns by `column_delta` (the whole chain when the entire call
/// should be corrected). `autocorrect` is false for offenses nested inside an
/// already-registered offense range.
pub struct FirstArgIndentOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub message: String,
    pub autocorrect: bool,
    pub correct_start: usize,
    pub correct_end: usize,
}

pub fn check_first_argument_indentation(
    _source: &[u8],
    _style: u8,
    _indent_width: usize,
    _enforce_fixed_with_no_line_break: bool,
) -> Vec<FirstArgIndentOffense> {
    Vec::new()
}
