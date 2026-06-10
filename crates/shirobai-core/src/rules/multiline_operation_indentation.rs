//! `Layout/MultilineOperationIndentation`.
//!
//! Checks the indentation of the right-hand operand of binary operations
//! (`+`, `<<`, `&&`, `||`, ...) that span more than one line. Ports the shared
//! `MultilineExpressionIndentation` mixin logic to Rust for the operation cop.

/// One misindented operand. `column_delta` is `correct_column - actual_column`
/// (positive => operand must move right). `message` is the fully formatted
/// RuboCop offense message.
pub struct OperationIndentOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub message: String,
}

/// Enforced indentation style.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Aligned,
    Indented,
}

impl Style {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Style::Indented,
            _ => Style::Aligned,
        }
    }
}

pub fn check_multiline_operation_indentation(
    source: &[u8],
    _style: u8,
    _indent_width: usize,
    _base_indent_width: usize,
) -> Vec<OperationIndentOffense> {
    super::parse_cache::with_parsed(source, |_source, _node| {
        // TODO: implement.
        Vec::new()
    })
}
