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
pub trait Rule {
    fn enter(&mut self, node: &Node<'_>);
    fn leave(&mut self);
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
}
