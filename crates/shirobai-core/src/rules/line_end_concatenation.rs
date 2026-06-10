//! `Style/LineEndConcatenation`.

/// A multiline string concatenation (`"a" +` / `"a" <<` at line end followed by
/// a string literal on the next line) that should use `\` continuation.
pub struct LineEndConcatOffense {
    /// Offense range: the operator token (`+` or `<<`).
    pub start_offset: usize,
    pub end_offset: usize,
    /// Operator text (`+` or `<<`), used by Ruby to format the message.
    pub operator: String,
    /// Autocorrect replacement range start (operator plus trailing whitespace,
    /// extended by one when followed by a backslash).
    pub replace_start: usize,
    pub replace_end: usize,
}

pub fn check_line_end_concatenation(_source: &[u8]) -> Vec<LineEndConcatOffense> {
    Vec::new()
}
