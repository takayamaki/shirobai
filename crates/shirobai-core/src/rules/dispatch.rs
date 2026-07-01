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
//!
//! # Interest masks
//!
//! With ~60 rules on the shared walk, calling every hook of every rule at
//! every node dominates the walk cost: most `enter` bodies are a kind match
//! that falls through. A rule can narrow what it receives by overriding
//! [`Rule::interest`]:
//!
//! * The `ENTER_*` bits pick which branch-node classes reach `enter`. A rule
//!   may only narrow these when its `enter` body ignores every other class
//!   (i.e. skipping the call is exactly equivalent to the body's fall-through
//!   arm).
//! * `LEAVE` / `LEAF` / `RESCUE` gate the `leave` / `enter_leaf` /
//!   `enter_rescue`+`leave_rescue` hooks. Dropping a bit is exactly
//!   equivalent when the rule's hook is empty (the trait defaults, or an
//!   empty `leave` body).
//!
//! The default is [`Interest::ALL`] — a rule that does not override
//! `interest` gets every hook at every node, exactly like the pre-mask
//! dispatcher. When in doubt, leave the default.

use ruby_prism::{Node, Visit};

/// What a [`Rule`] wants to receive from the shared walk. Bits are grouped
/// into hook gates (`LEAVE` / `LEAF` / `RESCUE`) and branch-node classes for
/// `enter` (`ENTER_*`, one bit per [`class_of`] bucket).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Interest(pub u32);

impl Interest {
    /// `leave` is called for every branch node close.
    pub const LEAVE: u32 = 1 << 0;
    /// `enter_leaf` is called for every leaf node.
    pub const LEAF: u32 = 1 << 1;
    /// `enter_rescue` / `leave_rescue` are called around rescue clauses.
    pub const RESCUE: u32 = 1 << 2;

    /// `CallNode`
    pub const ENTER_CALL: u32 = 1 << 3;
    /// `DefNode`
    pub const ENTER_DEF: u32 = 1 << 4;
    /// `BlockNode`
    pub const ENTER_BLOCK: u32 = 1 << 5;
    /// `LambdaNode`
    pub const ENTER_LAMBDA: u32 = 1 << 6;
    /// `ClassNode` / `ModuleNode` / `SingletonClassNode`
    pub const ENTER_CLASS_MOD: u32 = 1 << 7;
    /// Every `*WriteNode` / `*TargetNode` / `MultiWriteNode` assignment form
    pub const ENTER_WRITE: u32 = 1 << 8;
    /// `ConstantPathNode`
    pub const ENTER_CONST_PATH: u32 = 1 << 9;
    /// `Interpolated{String,XString,RegularExpression,Symbol}Node`
    pub const ENTER_ISTRING: u32 = 1 << 10;
    /// `ArrayNode` and branch-side literal nodes (`String` / `XString` /
    /// `RegularExpression` / `Symbol`)
    pub const ENTER_LITERAL: u32 = 1 << 11;
    /// `SuperNode` / `ForwardingSuperNode`
    pub const ENTER_SUPER: u32 = 1 << 12;
    /// Any branch node not covered by an `ENTER_*` class above
    pub const ENTER_OTHER: u32 = 1 << 13;

    pub const ENTER_ALL: u32 = Self::ENTER_CALL
        | Self::ENTER_DEF
        | Self::ENTER_BLOCK
        | Self::ENTER_LAMBDA
        | Self::ENTER_CLASS_MOD
        | Self::ENTER_WRITE
        | Self::ENTER_CONST_PATH
        | Self::ENTER_ISTRING
        | Self::ENTER_LITERAL
        | Self::ENTER_SUPER
        | Self::ENTER_OTHER;

    /// Every hook at every node — the exact pre-mask dispatcher behaviour.
    pub const ALL: Interest =
        Interest(Self::LEAVE | Self::LEAF | Self::RESCUE | Self::ENTER_ALL);
}

/// The number of `ENTER_*` classes ([`class_index`] range).
const N_CLASSES: usize = 11;

/// Bucket index (0..[`N_CLASSES`]) for a branch node, mirroring the
/// `ENTER_*` bit assignment (`bit == 1 << (3 + index)`).
fn class_index(node: &Node<'_>) -> usize {
    match node {
        Node::CallNode { .. } => 0,
        Node::DefNode { .. } => 1,
        Node::BlockNode { .. } => 2,
        Node::LambdaNode { .. } => 3,
        Node::ClassNode { .. } | Node::ModuleNode { .. } | Node::SingletonClassNode { .. } => 4,
        Node::LocalVariableWriteNode { .. }
        | Node::LocalVariableOrWriteNode { .. }
        | Node::LocalVariableAndWriteNode { .. }
        | Node::LocalVariableOperatorWriteNode { .. }
        | Node::LocalVariableTargetNode { .. }
        | Node::InstanceVariableWriteNode { .. }
        | Node::InstanceVariableOrWriteNode { .. }
        | Node::InstanceVariableAndWriteNode { .. }
        | Node::InstanceVariableOperatorWriteNode { .. }
        | Node::InstanceVariableTargetNode { .. }
        | Node::ClassVariableWriteNode { .. }
        | Node::ClassVariableOrWriteNode { .. }
        | Node::ClassVariableAndWriteNode { .. }
        | Node::ClassVariableOperatorWriteNode { .. }
        | Node::ClassVariableTargetNode { .. }
        | Node::GlobalVariableWriteNode { .. }
        | Node::GlobalVariableOrWriteNode { .. }
        | Node::GlobalVariableAndWriteNode { .. }
        | Node::GlobalVariableOperatorWriteNode { .. }
        | Node::GlobalVariableTargetNode { .. }
        | Node::ConstantWriteNode { .. }
        | Node::ConstantOrWriteNode { .. }
        | Node::ConstantAndWriteNode { .. }
        | Node::ConstantOperatorWriteNode { .. }
        | Node::ConstantTargetNode { .. }
        | Node::ConstantPathWriteNode { .. }
        | Node::ConstantPathOrWriteNode { .. }
        | Node::ConstantPathAndWriteNode { .. }
        | Node::ConstantPathOperatorWriteNode { .. }
        | Node::ConstantPathTargetNode { .. }
        | Node::CallOrWriteNode { .. }
        | Node::CallAndWriteNode { .. }
        | Node::CallOperatorWriteNode { .. }
        | Node::CallTargetNode { .. }
        | Node::IndexOrWriteNode { .. }
        | Node::IndexAndWriteNode { .. }
        | Node::IndexOperatorWriteNode { .. }
        | Node::IndexTargetNode { .. }
        | Node::MultiWriteNode { .. }
        | Node::MultiTargetNode { .. }
        | Node::MatchWriteNode { .. } => 5,
        Node::ConstantPathNode { .. } => 6,
        Node::InterpolatedStringNode { .. }
        | Node::InterpolatedXStringNode { .. }
        | Node::InterpolatedRegularExpressionNode { .. }
        | Node::InterpolatedSymbolNode { .. } => 7,
        Node::ArrayNode { .. }
        | Node::StringNode { .. }
        | Node::XStringNode { .. }
        | Node::RegularExpressionNode { .. }
        | Node::SymbolNode { .. } => 8,
        Node::SuperNode { .. } | Node::ForwardingSuperNode { .. } => 9,
        _ => 10,
    }
}

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
///
/// `interest` narrows which of these calls the rule receives (see the module
/// doc); the default is everything.
pub trait Rule {
    fn enter(&mut self, node: &Node<'_>);
    fn leave(&mut self);
    fn enter_leaf(&mut self, _node: &Node<'_>) {}
    fn enter_rescue(&mut self, _node: &Node<'_>) {}
    fn leave_rescue(&mut self) {}
    fn interest(&self) -> Interest {
        Interest::ALL
    }
}

/// Walk `source` once, driving every rule in `rules`.
pub fn run(source: &[u8], rules: &mut [&mut dyn Rule]) {
    // Route each hook to the rules whose interest covers it. Bucket contents
    // preserve `rules` order, so co-driven rules fire in the same relative
    // order as the pre-mask dispatcher.
    let mut enter_buckets: [Vec<u16>; N_CLASSES] = Default::default();
    let mut leave_bucket: Vec<u16> = Vec::new();
    let mut leaf_bucket: Vec<u16> = Vec::new();
    let mut rescue_bucket: Vec<u16> = Vec::new();
    for (i, rule) in rules.iter().enumerate() {
        let Interest(mask) = rule.interest();
        let i = u16::try_from(i).expect("more rules than u16");
        for (class, bucket) in enter_buckets.iter_mut().enumerate() {
            if mask & (1 << (3 + class)) != 0 {
                bucket.push(i);
            }
        }
        if mask & Interest::LEAVE != 0 {
            leave_bucket.push(i);
        }
        if mask & Interest::LEAF != 0 {
            leaf_bucket.push(i);
        }
        if mask & Interest::RESCUE != 0 {
            rescue_bucket.push(i);
        }
    }
    super::parse_cache::with_parsed(source, |_source, node| {
        let mut walker = RuleWalker {
            rules,
            enter_buckets,
            leave_bucket,
            leaf_bucket,
            rescue_bucket,
        };
        walker.visit(node);
    });
}

struct RuleWalker<'s, 'r> {
    rules: &'s mut [&'r mut dyn Rule],
    enter_buckets: [Vec<u16>; N_CLASSES],
    leave_bucket: Vec<u16>,
    leaf_bucket: Vec<u16>,
    rescue_bucket: Vec<u16>,
}

impl<'pr, 's, 'r> Visit<'pr> for RuleWalker<'s, 'r> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        for &i in &self.enter_buckets[class_index(&node)] {
            self.rules[i as usize].enter(&node);
        }
    }

    fn visit_branch_node_leave(&mut self) {
        for &i in &self.leave_bucket {
            self.rules[i as usize].leave();
        }
    }

    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        for &i in &self.leaf_bucket {
            self.rules[i as usize].enter_leaf(&node);
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
        for &i in &self.rescue_bucket {
            self.rules[i as usize].enter_rescue(&node.as_node());
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
        for &i in &self.rescue_bucket {
            self.rules[i as usize].leave_rescue();
        }
        if let Some(subsequent) = node.subsequent() {
            self.visit_rescue_node(&subsequent);
        }
    }
}
