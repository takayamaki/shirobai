//! `Layout/AssignmentIndentation`.
//!
//! Checks the indentation of the first line of the right-hand side of a
//! multi-line assignment: the RHS should sit at the LHS's column plus
//! `IndentationWidth` (defaults to `Layout/IndentationWidth.Width`, falling
//! back to 2).
//!
//! Stock includes the `CheckAssignment` and `Alignment` mixins; the check is:
//!
//! ```ruby
//! def check_assignment(node, rhs)
//!   return unless rhs
//!   return unless node.loc.operator
//!   return if same_line?(node.loc.operator, rhs)
//!
//!   base = display_column(leftmost_multiple_assignment(node).source_range)
//!   check_alignment([rhs], base + configured_indentation_width)
//! end
//! ```
//!
//! `leftmost_multiple_assignment` climbs through same-line assignment parents
//! (e.g. `foo = bar = baz = 42` on one line), so the base column is the
//! outermost LHS's column. `check_alignment` only fires when the RHS
//! `begins_its_line?` (only whitespace precedes it on its own line) and its
//! display column differs from `base + IndentationWidth`.
//!
//! Here it is reconstructed over Prism in one ancestor-stack walk. Every
//! write / operator-write / multi-write / call node is considered (matching
//! `CheckAssignment`'s callback set on `lvasgn` / `ivasgn` / ... / `send`).
//! The autocorrect column delta is computed once; the Ruby wrapper hands it to
//! stock's `AlignmentCorrector#correct` against the matching `Parser::AST::Node`
//! (relocated by RHS start offset).

use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// One misindented RHS first-line. `column_delta` is
/// `expected_column - actual_column` (positive => the RHS line must move
/// right). The message is fixed at `MSG`.
pub struct AssignmentIndentationOffense {
    /// `[rhs_start, rhs_end)` is the RHS's full source range. The offense
    /// reports against this range; autocorrect shifts every line of it.
    pub rhs_start: usize,
    pub rhs_end: usize,
    /// `expected_column - actual_column` (display column).
    pub column_delta: i64,
}

#[derive(Clone, Copy)]
pub struct Config {
    /// `configured_indentation_width`: `Layout/AssignmentIndentation.IndentationWidth`
    /// when set, else `Layout/IndentationWidth.Width`, else 2.
    pub indentation_width: usize,
}

pub fn check_assignment_indentation(
    source: &[u8],
    config: Config,
) -> Vec<AssignmentIndentationOffense> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        indentation_width: config.indentation_width,
        offenses: Vec::new(),
        ancestors: Vec::new(),
    }
}

/// One open ancestor's facts used by `leftmost_multiple_assignment`:
/// whether the ancestor is itself an assignment (in stock's
/// `node.assignment?` sense), its expression start offset, and its first
/// line. The expression start gives the leftmost LHS column when same-line
/// assignment parents are climbed.
#[derive(Clone, Copy)]
struct Frame {
    /// `loc.expression.begin_pos` of the parent node.
    expr_start: usize,
    /// First (1-based) line of the parent node's expression.
    first_line: usize,
    /// `node.assignment?`: write/op_asgn/multi_write nodes. `send`/index_*
    /// nodes are NOT assignment? in parser-gem unless they are setter calls,
    /// which `leftmost_multiple_assignment` does not climb through. We are
    /// strict here too.
    is_assignment: bool,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    indentation_width: usize,
    pub(crate) offenses: Vec<AssignmentIndentationOffense>,
    /// Each open node's `Frame` (top = parent of the entering node).
    ancestors: Vec<Frame>,
}

impl<'a> Visitor<'a> {
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn display_column(&self, off: usize) -> usize {
        self.line_index.display_column(self.source, off)
    }

    /// `begins_its_line?(range)`: only whitespace precedes `off` on its line.
    fn begins_its_line(&self, off: usize) -> bool {
        let ls = self.line_index.line_start(off);
        self.source[ls..off]
            .iter()
            .all(|&b| matches!(b, b' ' | b'\t'))
    }

    /// `leftmost_multiple_assignment(node)`: climb same-line assignment
    /// parents and return the outermost expression start offset. Stock walks
    /// upward while `same_line?(node, node.parent) && node.parent.assignment?`.
    fn leftmost_assignment_start(&self, self_start: usize, self_line: usize) -> usize {
        let mut start = self_start;
        let mut line = self_line;
        // Walk ancestors top-down (deepest first). Each ancestor is the
        // immediate parent of the previous level.
        for frame in self.ancestors.iter().rev() {
            if !frame.is_assignment {
                break;
            }
            if frame.first_line != line {
                break;
            }
            start = frame.expr_start;
            line = frame.first_line;
        }
        start
    }
}

/// `CheckAssignment#extract_rhs`: for assignment nodes the assigned value,
/// for call nodes (setter-call / op_asgn-call) the last argument. Returns
/// `None` when there is no RHS (e.g. a bare `lvasgn` with no value would be
/// a Prism `LocalVariableReadNode`, not a write node).
fn assignment_rhs_and_operator<'pr>(node: &Node<'pr>) -> Option<(Node<'pr>, Location<'pr>)> {
    // Operator location is stock's `node.loc.operator` (`=`, `+=`, `||=`,
    // `&&=`, ...). The cop early-returns when there is no operator, so callers
    // must have one.
    if let Some(n) = node.as_local_variable_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_instance_variable_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_class_variable_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_global_variable_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_constant_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_constant_path_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_multi_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    // op_asgn / or_asgn / and_asgn families: each has `value()` (the RHS)
    // and `operator_loc()` (`+=`, `-=`, `||=`, `&&=`, ...).
    if let Some(n) = node.as_local_variable_operator_write_node() {
        return Some((n.value(), n.binary_operator_loc()));
    }
    if let Some(n) = node.as_local_variable_or_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_local_variable_and_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_instance_variable_operator_write_node() {
        return Some((n.value(), n.binary_operator_loc()));
    }
    if let Some(n) = node.as_instance_variable_or_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_instance_variable_and_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_class_variable_operator_write_node() {
        return Some((n.value(), n.binary_operator_loc()));
    }
    if let Some(n) = node.as_class_variable_or_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_class_variable_and_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_global_variable_operator_write_node() {
        return Some((n.value(), n.binary_operator_loc()));
    }
    if let Some(n) = node.as_global_variable_or_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_global_variable_and_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_constant_operator_write_node() {
        return Some((n.value(), n.binary_operator_loc()));
    }
    if let Some(n) = node.as_constant_or_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_constant_and_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_constant_path_operator_write_node() {
        return Some((n.value(), n.binary_operator_loc()));
    }
    if let Some(n) = node.as_constant_path_or_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_constant_path_and_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_index_operator_write_node() {
        return Some((n.value(), n.binary_operator_loc()));
    }
    if let Some(n) = node.as_index_or_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_index_and_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_call_operator_write_node() {
        return Some((n.value(), n.binary_operator_loc()));
    }
    if let Some(n) = node.as_call_or_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    if let Some(n) = node.as_call_and_write_node() {
        return Some((n.value(), n.operator_loc()));
    }
    None
}

/// Whether this node would respond `assignment? == true` in parser-gem
/// (`lvasgn`, `ivasgn`, ..., `op_asgn`, `or_asgn`, `and_asgn`, `masgn`,
/// `casgn`). `send`/`csend` are NOT assignment? unless they are setter calls
/// (`x.foo = y`, `x[i] = y`), which `leftmost_multiple_assignment` does not
/// climb through.
fn is_assignment_node(node: &Node<'_>) -> bool {
    assignment_rhs_and_operator(node).is_some()
}

impl<'a> Visitor<'a> {
    fn check(&mut self, node: &Node<'_>) {
        // `CheckAssignment` callbacks: every assignment node AND `on_send`
        // (calls whose `last_argument` becomes the implicit RHS, e.g. setter
        // calls without an operator location go nowhere because the early
        // `return unless node.loc.operator` short-circuits below).
        let (rhs, operator_loc) = if let Some(call) = node.as_call_node() {
            // `on_send`: only when the call has a last argument.
            let Some(rhs) = call_last_argument(&call) else {
                return;
            };
            // `node.loc.operator` for a parser-gem `send` is only set when the
            // call is an attribute setter (`x.foo = y`) or an index setter
            // (`x[1] = y`). Prism exposes both as `CallNode#equal_loc`. Without
            // it, stock returns early.
            let Some(op) = call.equal_loc() else {
                return;
            };
            (rhs, op)
        } else if let Some(pair) = assignment_rhs_and_operator(node) {
            pair
        } else {
            return;
        };

        // `return if same_line?(node.loc.operator, rhs)`.
        let rhs_start = rhs.location().start_offset();
        let rhs_line = self.line_of(rhs_start);
        let op_line = self.line_of(operator_loc.start_offset());
        if op_line == rhs_line {
            return;
        }

        // `check_alignment` only acts when `begins_its_line?(rhs)`.
        if !self.begins_its_line(rhs_start) {
            return;
        }

        // Base column = display_column(leftmost_multiple_assignment.source_range).
        let node_start = node.location().start_offset();
        let node_line = self.line_of(node_start);
        let leftmost_start = self.leftmost_assignment_start(node_start, node_line);
        let base = self.display_column(leftmost_start) as i64;
        let actual = self.display_column(rhs_start) as i64;
        let expected = base + self.indentation_width as i64;
        let delta = expected - actual;
        if delta == 0 {
            return;
        }

        let rhs_end = rhs.location().end_offset();
        self.offenses.push(AssignmentIndentationOffense {
            rhs_start,
            rhs_end,
            column_delta: delta,
        });
    }
}

/// The last positional argument of a `CallNode` (stock's `send.last_argument`).
fn call_last_argument<'pr>(call: &ruby_prism::CallNode<'pr>) -> Option<Node<'pr>> {
    let args = call.arguments()?;
    args.arguments().iter().last()
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        self.check(node);
        let loc = node.location();
        self.ancestors.push(Frame {
            expr_start: loc.start_offset(),
            first_line: self.line_of(loc.start_offset()),
            is_assignment: is_assignment_node(node),
        });
    }

    fn leave(&mut self) {
        self.ancestors.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, indent: usize) -> Vec<AssignmentIndentationOffense> {
        check_assignment_indentation(
            source.as_bytes(),
            Config {
                indentation_width: indent,
            },
        )
    }

    #[test]
    fn registers_offense_for_misaligned_rhs() {
        // `a =\nif b ; end` — base=0, expected=2, actual=0, delta=2.
        let r = run("a =\nif b ; end\n", 2);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].column_delta, 2);
    }

    #[test]
    fn allows_aligned_rhs() {
        let r = run("a =\n  if b ; end\n", 2);
        assert!(r.is_empty());
    }

    #[test]
    fn allows_inline_assignment() {
        // operator and rhs on same line — skipped.
        let r = run("a = if b\n      foo\n    end\n", 2);
        assert!(r.is_empty());
    }

    #[test]
    fn ignores_comparison_operator() {
        // `===` is a send (not an assignment) with no operator_loc.
        let r = run("a ===\nif b ; end\n", 2);
        assert!(r.is_empty());
    }

    #[test]
    fn multi_lhs_misaligned() {
        // `a,\nb =\nif b ; end` — base=0 (masgn), delta=2.
        let r = run("a,\nb =\nif b ; end\n", 2);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].column_delta, 2);
    }

    #[test]
    fn chain_single_line_lhs() {
        // `foo = bar =\nbaz = ''` — child `bar = baz = ''`: parent same line,
        // so leftmost climbs to `foo`. base=0, delta=2.
        let r = run("foo = bar =\nbaz = ''\n", 2);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].column_delta, 2);
    }

    #[test]
    fn chain_multi_line_lhs() {
        // `foo =\n  bar =\n  baz = 42` — innermost grandchild `bar = ...` is
        // on a different line from `foo`, so leftmost is `bar`. base=2,
        // expected=4, actual=2, delta=2.
        let r = run("foo =\n  bar =\n  baz = 42\n", 2);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].column_delta, 2);
    }

    #[test]
    fn allows_aligned_chain_multi_line() {
        let r = run("foo =\n  bar =\n    baz = 42\n", 2);
        assert!(r.is_empty());
    }

    #[test]
    fn full_width_lhs_skips() {
        // `f 'Ｒｕｂｙ', a =\n                b` — the rhs is on a different
        // line, but the assignment `a = b` is a same-line operator/rhs pair
        // for `a` itself. So the inner assignment (when reached) has
        // operator and rhs on same line, and is skipped.
        // The send `f` is a regular call without operator_loc → skipped.
        let r = run("f 'Ｒｕｂｙ', a =\n                b\n", 2);
        assert!(r.is_empty());
    }

    #[test]
    fn custom_indentation_width() {
        // With width=7, properly indented at col 7 → no offense.
        let r = run("a =\n       if b ; end\n", 7);
        assert!(r.is_empty());
    }

    #[test]
    fn custom_indentation_width_misaligned() {
        // With width=7, indented at col 2 → delta=5.
        let r = run("a =\n  if b ; end\n", 7);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].column_delta, 5);
    }
}
