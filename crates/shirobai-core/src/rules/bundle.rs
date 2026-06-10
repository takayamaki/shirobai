//! Bundled single-walk runs: drive several cops over one parse + one AST walk.
//!
//! When multiple shirobai cops run on the same file, routing them through one
//! [`dispatch::run`](super::dispatch::run) collapses their N traversals into one
//! shared walk (the parse is already shared by `parse_cache`).

use super::multiline_method_call_indentation::{self as mc, MethodCallIndentOffense};
use super::multiline_operation_indentation::{self as op, OperationIndentOffense};

/// Run `Layout/MultilineOperationIndentation` and
/// `Layout/MultilineMethodCallIndentation` together in one walk.
#[allow(clippy::type_complexity)]
pub fn check_multiline_bundle(
    source: &[u8],
    op_style: u8,
    op_indent: usize,
    op_base: usize,
    mc_style: u8,
    mc_indent: usize,
    mc_base: usize,
) -> (Vec<OperationIndentOffense>, Vec<MethodCallIndentOffense>) {
    let mut op_rule = op::build_rule(source, op_style, op_indent, op_base);
    let mut mc_rule = mc::build_rule(source, mc_style, mc_indent, mc_base);
    super::dispatch::run(source, &mut [&mut op_rule, &mut mc_rule]);
    (op_rule.offenses, mc_rule.offenses)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_matches_standalone() {
        // A file that triggers both cops at once.
        let src = "if a +\n    b\n  something\nend\nFoo.a\n     .b\n      .c\n";
        let (op_off, mc_off) = check_multiline_bundle(src.as_bytes(), 0, 2, 2, 0, 2, 2);
        let op_alone = op::check_multiline_operation_indentation(src.as_bytes(), 0, 2, 2);
        let mc_alone = mc::check_multiline_method_call_indentation(src.as_bytes(), 0, 2, 2);
        assert_eq!(op_off.len(), op_alone.len());
        assert_eq!(mc_off.len(), mc_alone.len());
        assert!(!op_off.is_empty());
        assert!(!mc_off.is_empty());
        for (a, b) in op_off.iter().zip(&op_alone) {
            assert_eq!(
                (a.start_offset, a.column_delta),
                (b.start_offset, b.column_delta)
            );
        }
        for (a, b) in mc_off.iter().zip(&mc_alone) {
            assert_eq!(
                (a.start_offset, a.column_delta),
                (b.start_offset, b.column_delta)
            );
        }
    }
}
