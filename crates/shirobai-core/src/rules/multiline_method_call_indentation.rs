//! `Layout/MultilineMethodCallIndentation`.
//!
//! Checks the indentation of the method-name part of `.`-chained method calls
//! that span more than one line. The heavy half of the shared
//! `MultilineExpressionIndentation` mixin (the operation half lives in
//! `multiline_operation_indentation`).

/// One misindented method-call selector. `column_delta` is
/// `correct_column - actual_column`. `message` is the formatted offense message.
/// When the offending call carries a multiline block, `block_*` give the ranges
/// the Ruby side must realign in addition to the selector line (`0..0` = none).
pub struct MethodCallIndentOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub message: String,
    pub block_body_start: usize,
    pub block_body_end: usize,
    pub block_end_start: usize,
    pub block_end_end: usize,
}

pub fn check_multiline_method_call_indentation(
    source: &[u8],
    _style: u8,
    _indent_width: usize,
    _base_indent_width: usize,
) -> Vec<MethodCallIndentOffense> {
    super::parse_cache::with_parsed(source, |_source, _node| {
        // TODO: implement.
        Vec::new()
    })
}
