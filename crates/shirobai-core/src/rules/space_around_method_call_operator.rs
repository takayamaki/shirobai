//! `Layout/SpaceAroundMethodCallOperator`.
//!
//! Flags spaces/tabs around method call operators: before and after `.`/`&.`
//! on a `CallNode`, and after `::` on a `ConstantPathNode`. A run that contains
//! a newline is not an offense (stock's `SPACES_REGEXP = /\A[ \t]+\z/` only
//! matches a non-empty run of spaces and tabs), so multi-line method chains are
//! left alone. Autocorrect removes the offending whitespace run.
//!
//! Stock pairs `on_send`/`on_csend` (receiver-end..dot-begin and
//! dot-end..selector-begin) and `on_const` (double_colon-end..name-begin, the
//! `after` side only — a space *before* `::` is not this cop's concern). The
//! `::` is checked on every read `ConstantPathNode`; assignment *targets*
//! (`A::B = 1`) are `ConstantPathTargetNode`s in prism and have no standalone
//! const node in parser either, so they produce no offense — handling only
//! `ConstantPathNode` reproduces that.
//!
//! The `on_send` side covers more than `CallNode`: in parser-gem an attribute
//! op-assign (`self.foo ||= 1`) is a `send` node with a dot, so stock's
//! `on_send` fires. Prism splits those into `Call{Or,Operator,And}WriteNode`
//! and `CallTargetNode` (the masgn target form), each carrying the same
//! receiver / call-operator / message locations. We process all five node
//! types identically so a dot inside an attribute assignment is flagged like
//! stock.

use ruby_prism::{
    CallAndWriteNode, CallNode, CallOperatorWriteNode, CallOrWriteNode, CallTargetNode,
    ConstantPathAndWriteNode, ConstantPathNode, ConstantPathOperatorWriteNode,
    ConstantPathOrWriteNode, ConstantPathWriteNode, Location, Node, Visit,
};

/// One offending whitespace run. `(start, end)` is both the offense highlight
/// and the autocorrect removal range (stock `corrector.remove(range)`).
pub struct SpaceAroundMethodCallOperatorOffense {
    pub start_offset: usize,
    pub end_offset: usize,
}

pub fn check_space_around_method_call_operator(
    source: &[u8],
) -> Vec<SpaceAroundMethodCallOperatorOffense> {
    let mut visitor = build_rule(source);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule(source: &[u8]) -> Visitor<'_> {
    Visitor {
        source,
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    pub(crate) offenses: Vec<SpaceAroundMethodCallOperatorOffense>,
}

impl Visitor<'_> {
    /// `on_send` / `on_csend` for a plain `CallNode`.
    fn process_send(&mut self, call: &CallNode<'_>) {
        // The selector is the message, or the opening `(` for the `Proc#call`
        // shorthand `foo.()` (no selector loc). The op-assign / masgn-target
        // call variants can't take the `.()` form, so only `CallNode` falls
        // back to `opening_loc`.
        let selector_start = call
            .message_loc()
            .map(|m| m.start_offset())
            .or_else(|| call.opening_loc().map(|o| o.start_offset()));
        self.process_call_operator(
            call.call_operator_loc(),
            call.receiver().map(|r| r.location().end_offset()),
            selector_start,
        );
    }

    /// Shared `on_send`/`on_csend` body: check the space before the `.`/`&.`
    /// operator (receiver end .. dot begin) and after it (dot end .. selector
    /// begin). `operator` is the call-operator location; non-dot operators
    /// (`::` on a method def receiver, index `[]`, plain operator calls without
    /// a dot) and missing locations are skipped.
    fn process_call_operator(
        &mut self,
        operator: Option<Location<'_>>,
        receiver_end: Option<usize>,
        selector_start: Option<usize>,
    ) {
        let Some(dot) = operator else {
            return;
        };
        let dot_text = &self.source[dot.start_offset()..dot.end_offset()];
        // `node.dot? || node.safe_navigation?`: only `.` and `&.`.
        if dot_text != b"." && dot_text != b"&." {
            return;
        }

        if let Some(receiver_end) = receiver_end {
            self.check_space(receiver_end, dot.start_offset());
        }
        if let Some(selector_start) = selector_start {
            self.check_space(dot.end_offset(), selector_start);
        }
    }

    /// `on_const`: only the `after` side of `::` (double_colon end .. name
    /// begin). A space *before* `::` is not flagged by stock.
    fn process_const_path(&mut self, path: &ConstantPathNode<'_>) {
        let delimiter = path.delimiter_loc();
        // `node.loc?(:double_colon)`: a `ConstantPathNode` always has a `::`.
        self.check_space(delimiter.end_offset(), path.name_loc().start_offset());
    }

    /// Recurse into a write node's `target` *without* treating the target's own
    /// `::` as an offense. In prism, `A::B = 1` stores the LHS as a
    /// `ConstantPathNode`, but parser-gem has no read `const` node there
    /// (`on_const` never fires), so stock ignores it. We still walk into the
    /// target's `parent` (the read scope `Foo::Bar` of `Foo::Bar::C = 1`) so its
    /// `::` is checked, mirroring stock.
    fn recurse_into_write_target(&mut self, target: &ConstantPathNode<'_>) {
        if let Some(parent) = target.parent() {
            self.visit(&parent);
        }
    }

    /// `check_space`: a non-empty run of only spaces/tabs between the two
    /// offsets. `range.source.match?(/\A[ \t]+\z/)` — any newline (or other
    /// byte) in the run means no offense.
    fn check_space(&mut self, begin_pos: usize, end_pos: usize) {
        if end_pos <= begin_pos {
            return;
        }
        let run = &self.source[begin_pos..end_pos];
        if run.iter().all(|&b| b == b' ' || b == b'\t') {
            self.offenses.push(SpaceAroundMethodCallOperatorOffense {
                start_offset: begin_pos,
                end_offset: end_pos,
            });
        }
    }
}

impl<'pr> Visit<'pr> for Visitor<'_> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        self.process_send(node);
        ruby_prism::visit_call_node(self, node);
    }

    // Attribute op-assigns and masgn targets carry a dot too. Prism gives them
    // their own node types (bypassing `visit_call_node`); process each the same
    // way stock's `on_send` does — there is no `.()` shorthand here, so the
    // selector is always `message_loc`.
    fn visit_call_or_write_node(&mut self, node: &CallOrWriteNode<'pr>) {
        self.process_call_operator(
            node.call_operator_loc(),
            node.receiver().map(|r| r.location().end_offset()),
            node.message_loc().map(|m| m.start_offset()),
        );
        ruby_prism::visit_call_or_write_node(self, node);
    }

    fn visit_call_operator_write_node(&mut self, node: &CallOperatorWriteNode<'pr>) {
        self.process_call_operator(
            node.call_operator_loc(),
            node.receiver().map(|r| r.location().end_offset()),
            node.message_loc().map(|m| m.start_offset()),
        );
        ruby_prism::visit_call_operator_write_node(self, node);
    }

    fn visit_call_and_write_node(&mut self, node: &CallAndWriteNode<'pr>) {
        self.process_call_operator(
            node.call_operator_loc(),
            node.receiver().map(|r| r.location().end_offset()),
            node.message_loc().map(|m| m.start_offset()),
        );
        ruby_prism::visit_call_and_write_node(self, node);
    }

    fn visit_call_target_node(&mut self, node: &CallTargetNode<'pr>) {
        self.process_call_operator(
            Some(node.call_operator_loc()),
            Some(node.receiver().location().end_offset()),
            Some(node.message_loc().start_offset()),
        );
        ruby_prism::visit_call_target_node(self, node);
    }

    fn visit_constant_path_node(&mut self, node: &ConstantPathNode<'pr>) {
        self.process_const_path(node);
        ruby_prism::visit_constant_path_node(self, node);
    }

    // The four constant-path *write* nodes store the LHS as a `ConstantPathNode`
    // and the generated default routes it through `visit_constant_path_node`,
    // which would flag the target's `::`. Stock never sees a read `const` node
    // there, so override each to skip the target and recurse into its scope +
    // value instead.
    fn visit_constant_path_write_node(&mut self, node: &ConstantPathWriteNode<'pr>) {
        self.recurse_into_write_target(&node.target());
        self.visit(&node.value());
    }

    fn visit_constant_path_operator_write_node(
        &mut self,
        node: &ConstantPathOperatorWriteNode<'pr>,
    ) {
        self.recurse_into_write_target(&node.target());
        self.visit(&node.value());
    }

    fn visit_constant_path_or_write_node(&mut self, node: &ConstantPathOrWriteNode<'pr>) {
        self.recurse_into_write_target(&node.target());
        self.visit(&node.value());
    }

    fn visit_constant_path_and_write_node(&mut self, node: &ConstantPathAndWriteNode<'pr>) {
        self.recurse_into_write_target(&node.target());
        self.visit(&node.value());
    }
}

/// Shared-walk driver. `CallNode`, the `Call{Or,Operator,And}WriteNode` /
/// `CallTargetNode` attribute-assign variants, and `ConstantPathNode` are all
/// reached through the generic branch hook (a read `ConstantPathNode` is a
/// `parent` field, walked generically; assignment-target const paths are a
/// different node type and never match `as_constant_path_node`).
impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.process_send(&call);
        } else if let Some(path) = node.as_constant_path_node() {
            self.process_const_path(&path);
        } else if let Some(n) = node.as_call_or_write_node() {
            self.process_call_operator(
                n.call_operator_loc(),
                n.receiver().map(|r| r.location().end_offset()),
                n.message_loc().map(|m| m.start_offset()),
            );
        } else if let Some(n) = node.as_call_operator_write_node() {
            self.process_call_operator(
                n.call_operator_loc(),
                n.receiver().map(|r| r.location().end_offset()),
                n.message_loc().map(|m| m.start_offset()),
            );
        } else if let Some(n) = node.as_call_and_write_node() {
            self.process_call_operator(
                n.call_operator_loc(),
                n.receiver().map(|r| r.location().end_offset()),
                n.message_loc().map(|m| m.start_offset()),
            );
        } else if let Some(n) = node.as_call_target_node() {
            self.process_call_operator(
                Some(n.call_operator_loc()),
                Some(n.receiver().location().end_offset()),
                Some(n.message_loc().start_offset()),
            );
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<(usize, usize)> {
        check_space_around_method_call_operator(source.as_bytes())
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset))
            .collect()
    }

    #[test]
    fn no_space_around_dot_is_clean() {
        assert!(run("foo.bar\n").is_empty());
    }

    #[test]
    fn flags_space_before_dot() {
        assert_eq!(run("foo .bar\n"), vec![(3, 4)]);
    }

    #[test]
    fn flags_space_after_dot() {
        assert_eq!(run("foo. bar\n"), vec![(4, 5)]);
    }

    #[test]
    fn flags_space_both_sides_of_dot() {
        assert_eq!(run("foo . bar\n"), vec![(3, 4), (5, 6)]);
    }

    #[test]
    fn flags_tab_before_dot() {
        assert_eq!(run("foo\t.bar\n"), vec![(3, 4)]);
    }

    #[test]
    fn flags_safe_navigation_spaces() {
        assert_eq!(run("foo &. bar\n"), vec![(3, 4), (6, 7)]);
    }

    #[test]
    fn ignores_newline_in_multiline_chain() {
        // Space before the dot spans a newline (no offense); space after the
        // dot is a single line (offense).
        assert_eq!(run("foo\n  . bar\n"), vec![(7, 8)]);
    }

    #[test]
    fn clean_multiline_chain_is_clean() {
        assert!(run("foo\n  .bar\n").is_empty());
    }

    #[test]
    fn flags_proc_call_shorthand_after_dot() {
        // `l. ()` — no selector loc; the `(` is the selector position.
        assert_eq!(run("l. ()\n"), vec![(2, 3)]);
    }

    #[test]
    fn flags_after_double_colon() {
        assert_eq!(run("RuboCop:: Cop\n"), vec![(9, 10)]);
    }

    #[test]
    fn ignores_space_before_double_colon() {
        assert!(run("RuboCop ::Cop\n").is_empty());
    }

    #[test]
    fn flags_after_leading_double_colon() {
        assert_eq!(run(":: RuboCop\n"), vec![(2, 3)]);
    }

    #[test]
    fn flags_each_segment_of_const_chain() {
        // Outer path first, then the inner scope (matches stock's emit order).
        assert_eq!(run("RuboCop:: Cop:: Base\n"), vec![(15, 16), (9, 10)]);
    }

    #[test]
    fn ignores_const_assignment_target() {
        // `A:: B = 1` — the target is a ConstantPathTargetNode, no offense.
        assert!(run("A:: B = 1\n").is_empty());
    }

    #[test]
    fn const_assignment_scope_read_is_flagged() {
        // Only the `Foo:: Bar` read scope is flagged; the `:: Baz` target is not.
        assert_eq!(run("Foo:: Bar:: Baz = 1\n"), vec![(5, 6)]);
    }

    #[test]
    fn module_name_const_path_is_flagged() {
        assert_eq!(run("module Foo:: Bar\nend\n"), vec![(12, 13)]);
    }

    #[test]
    fn flags_dot_in_attribute_or_assign() {
        // `self . foo ||= 1` — CallOrWriteNode, both sides flagged.
        assert_eq!(run("self . foo ||= 1\n"), vec![(4, 5), (6, 7)]);
    }

    #[test]
    fn flags_dot_in_attribute_op_assign() {
        // `self . foo += 1` — CallOperatorWriteNode.
        assert_eq!(run("self . foo += 1\n"), vec![(4, 5), (6, 7)]);
    }

    #[test]
    fn flags_dot_in_attribute_and_assign() {
        // `self . foo &&= 1` — CallAndWriteNode.
        assert_eq!(run("self . foo &&= 1\n"), vec![(4, 5), (6, 7)]);
    }

    #[test]
    fn flags_dot_in_masgn_target() {
        // `self . foo, x = 1, 2` — CallTargetNode in a MultiWriteNode.
        assert_eq!(run("self . foo, x = 1, 2\n"), vec![(4, 5), (6, 7)]);
    }

    #[test]
    fn flags_safe_nav_in_attribute_or_assign() {
        // `self &. foo ||= 1` — `&.` operator, both sides flagged.
        assert_eq!(run("self &. foo ||= 1\n"), vec![(4, 5), (7, 8)]);
    }
}
