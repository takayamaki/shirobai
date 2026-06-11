//! Shared single-walk dispatcher for the ancestor-stack cops.
//!
//! Each cop that maintains a parser-`each_ancestor`-style frame stack is a
//! [`Rule`]: it processes a node and pushes its frame on `enter`, and pops on
//! `leave`. [`run`] walks the AST once (through [`parse_cache`](super::parse_cache))
//! and drives any number of rules together, so N cops over one file share a
//! single traversal instead of re-walking N times.
//!
//! A cop's standalone `check_*` entry point runs a single rule through `run`;
//! the bundle entry points (e.g. `check_multiline_bundle`) run several at once.

use ruby_prism::{Node, Visit};

/// A cop driven by the shared walk. `enter` is called at every branch node
/// (before its frame would be pushed, so an `enter` body sees only ancestors on
/// its own stack); `leave` is called when that branch node closes.
///
/// `enter_leaf` is called at leaf nodes (nodes that cannot have children, e.g.
/// plain strings and symbols), which the branch hooks never see. `enter_rescue`
/// / `leave_rescue` are called around a `RescueNode`'s own clause (exceptions,
/// reference and statements — *not* the chained `subsequent` clause, which is a
/// sibling level): `RescueNode` is reached through `BeginNode`'s
/// concretely-typed `rescue_clause` field, which the generated dispatcher
/// visits directly, bypassing the branch hooks. All three default to no-ops so
/// existing rules are unaffected.
pub trait Rule {
    fn enter(&mut self, node: &Node<'_>);
    fn leave(&mut self);
    fn enter_leaf(&mut self, _node: &Node<'_>) {}
    fn enter_rescue(&mut self, _node: &Node<'_>) {}
    fn leave_rescue(&mut self) {}
}

/// Walk `source` once, driving every rule in `rules`.
pub fn run(source: &[u8], rules: &mut [&mut dyn Rule]) {
    super::parse_cache::with_parsed(source, |_source, node| {
        let mut walker = RuleWalker { rules };
        walker.visit(node);
    });
}

struct RuleWalker<'s, 'r> {
    rules: &'s mut [&'r mut dyn Rule],
}

impl<'pr, 's, 'r> Visit<'pr> for RuleWalker<'s, 'r> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        for rule in self.rules.iter_mut() {
            rule.enter(&node);
        }
    }

    fn visit_branch_node_leave(&mut self) {
        for rule in self.rules.iter_mut() {
            rule.leave();
        }
    }

    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        for rule in self.rules.iter_mut() {
            rule.enter_leaf(&node);
        }
    }

    // `RescueNode` is reached through `BeginNode`'s concretely-typed
    // `rescue_clause` field (and its own `subsequent` chain), so the generic
    // branch hooks above never fire for it. Mirror the generated default's
    // child order exactly — exceptions, reference, statements, subsequent —
    // and fire the dedicated rescue hooks around the clause's own children.
    // Chained clauses (`rescue A; rescue B`) are siblings at the same level in
    // parser, so `subsequent` is visited outside the hook pair.
    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        for rule in self.rules.iter_mut() {
            rule.enter_rescue(&node.as_node());
        }
        for exception in &node.exceptions() {
            self.visit(&exception);
        }
        if let Some(reference) = node.reference() {
            self.visit(&reference);
        }
        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
        for rule in self.rules.iter_mut() {
            rule.leave_rescue();
        }
        if let Some(subsequent) = node.subsequent() {
            self.visit_rescue_node(&subsequent);
        }
    }
}
