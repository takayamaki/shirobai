//! The RSpec "empty-line family": one shared walk feeding five cops that all
//! wrap rubocop-rspec's `EmptyLineSeparation` mixin —
//! `RSpec/EmptyLineAfterExample`, `RSpec/EmptyLineAfterExampleGroup`,
//! `RSpec/EmptyLineAfterFinalLet`, `RSpec/EmptyLineAfterHook`,
//! `RSpec/EmptyLineAfterSubject`.
//!
//! # What the mixin does (probed against stock rubocop-rspec 3.10.2)
//!
//! Every one of the five cops resolves a "concept" node (an example / example
//! group / final let / hook / subject) and calls
//! `missing_separating_line_offense(node)`:
//!
//! 1. `last_child?(node)` — return (no offense) unless the node's parser parent
//!    is a `:begin` (a multi-statement sequence) AND the node is not that
//!    sequence's last child. So a concept is only a candidate when it has a
//!    following sibling inside a statement sequence.
//! 2. `missing_separating_line` walks the comment lines directly after the
//!    node's `final_end_location` (heredoc-aware end), tracks the last enabled
//!    `# rubocop:enable` directive, and suppresses the offense when the line
//!    after the last such comment is blank.
//! 3. The offense location is the trimmed content of `final_end_line` (or the
//!    enabled-directive line when present); autocorrect inserts one `"\n"`
//!    after it.
//!
//! Steps 2 and 3 are pure `ProcessedSource` line/comment work, so the Ruby
//! wrappers replay the mixin verbatim (byte-for-byte parity guaranteed). This
//! Rust rule owns step 1 plus the heredoc-aware `final_end_line` and the
//! per-cop concept classification, and emits, per cop, one
//! `(final_end_line, method_name)` for every candidate that clears `last_child?`
//! and the one-liner allowances.
//!
//! # parser `:begin` recovery from prism (probed)
//!
//! prism has no `:begin`. A concept's parser parent is `:begin` (offense
//! eligible when multi-statement) in exactly these shapes, recovered from the
//! immediate frame the walk is under:
//!
//! - a nested body `StatementsNode` (block / def / class / module / if / while
//!   / ensure / else body) with `>= 2` children — always `:begin`;
//! - the transparent top-level `ProgramNode` statements with `>= 2` children;
//! - a `RescueNode`'s own body (`rescue ...; a; b`) with `>= 2` children —
//!   parser wraps it in `:begin`;
//! - a `BeginNode`'s MAIN statements (visited via `visit_statements_node`
//!   directly, so the frame is the `BeginNode` itself) — `:begin` ONLY when the
//!   begin has a `rescue`/`ensure` clause (`begin a; b rescue ... end`); a plain
//!   `begin a; b end` keeps `a`,`b` as direct `:kwbegin` children, which is NOT
//!   `begin_type?` (probed: no offense).
//!
//! Every other parent shape (a single-statement body, an assignment value, an
//! argument, ...) is not `:begin`, so the concept is its parent's last/only
//! child and never an offense.

use std::rc::Rc;

use ruby_prism::{CallNode, Node, StatementsNode};

use super::dispatch::{Interest, Rule};
use super::line_index::{with_line_index, LineIndex};
use super::rspec_language::{roles, RSpecConfig};

/// One empty-line-family offense: the concept's 1-based `final_end` line and
/// the concept's method name (for the per-cop message). The Ruby wrapper
/// runs stock's comment/blank walk from `final_end_line` and decides the
/// exact offense location + suppression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyLineOffense {
    pub final_end_line: usize,
    pub method_name: String,
}

/// Everything the five empty-line cops produced for one file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RSpecEmptyLineResult {
    pub example: Vec<EmptyLineOffense>,
    pub example_group: Vec<EmptyLineOffense>,
    pub final_let: Vec<EmptyLineOffense>,
    pub hook: Vec<EmptyLineOffense>,
    pub subject: Vec<EmptyLineOffense>,
}

#[derive(Clone, Copy, PartialEq)]
enum Cop {
    Example,
    ExampleGroup,
    FinalLet,
    Hook,
    Subject,
}

/// A candidate found during the walk; `final_end_line` is finalized against
/// the collected heredocs in [`RSpecEmptyLineRule::finish`].
struct Pending {
    cop: Cop,
    node_start: usize,
    node_end: usize,
    node_end_line: usize,
    method_name: String,
}

/// parser block-kind recovery (same recovery `rspec_dispatcher` uses).
#[derive(Clone, Copy, PartialEq)]
enum BlockKind {
    None,
    BlockArg,
    Plain,
    Numbered,
    It,
}

fn block_kind(call: &CallNode<'_>) -> BlockKind {
    match call.block() {
        None => BlockKind::None,
        Some(b) => match b.as_block_node() {
            Some(bn) => match bn.parameters() {
                Some(p) if p.as_numbered_parameters_node().is_some() => BlockKind::Numbered,
                Some(p) if p.as_it_parameters_node().is_some() => BlockKind::It,
                _ => BlockKind::Plain,
            },
            None => BlockKind::BlockArg,
        },
    }
}

/// `#rspec?` receiver: nil, `RSpec` or `::RSpec`.
fn rspec_const(recv: &Node<'_>) -> bool {
    if let Some(c) = recv.as_constant_read_node() {
        return c.name().as_slice() == b"RSpec";
    }
    if let Some(p) = recv.as_constant_path_node() {
        return p.parent().is_none() && p.name().is_some_and(|n| n.as_slice() == b"RSpec");
    }
    false
}

/// Byte-copy a `Node` into a `Node<'static>`. Safe as long as every stashed
/// copy is popped within the same `dispatch::run` call (the parse is held by
/// `parse_cache`) — see the same pattern in `empty_line_after_guard_clause`.
#[allow(clippy::missing_safety_doc)]
unsafe fn copy_to_static<'a>(node: &Node<'a>) -> Node<'static> {
    unsafe { std::mem::transmute_copy::<Node<'a>, Node<'static>>(node) }
}

struct Frame {
    node: Node<'static>,
    /// Whether this frame incremented `top_spec_depth` (a top-level spec group).
    top_spec: bool,
}

pub struct RSpecEmptyLineRule<'c> {
    cfg: &'c RSpecConfig,
    source: &'c [u8],
    li: Rc<LineIndex>,
    frames: Vec<Frame>,
    /// Positive while inside a top-level `spec_group?` (see `subject`'s
    /// `inside_example_group?`; identical to `rspec_dispatcher`'s gate).
    top_spec_depth: u32,
    /// `(heredoc node start_offset, closing terminator 1-based line)`.
    heredocs: Vec<(usize, usize)>,
    pending: Vec<Pending>,
}

pub fn build_rule<'c>(source: &'c [u8], cfg: &'c RSpecConfig) -> RSpecEmptyLineRule<'c> {
    let li = with_line_index(source, |li| li.clone());
    RSpecEmptyLineRule {
        cfg,
        source,
        li,
        frames: Vec::with_capacity(64),
        top_spec_depth: 0,
        heredocs: Vec::new(),
        pending: Vec::new(),
    }
}

impl RSpecEmptyLineRule<'_> {
    fn line_of(&self, off: usize) -> usize {
        self.li.line_of(off)
    }

    fn node_last_line(&self, node_end: usize) -> usize {
        self.line_of(node_end.saturating_sub(1))
    }

    /// `single_line?`: the node's own start and end fall on one line.
    fn single_line(&self, start: usize, end: usize) -> bool {
        self.line_of(start) == self.node_last_line(end)
    }

    /// Record a heredoc string node (`<<...`), if this node is one.
    fn maybe_heredoc(&mut self, node: &Node<'_>) {
        let (opening, closing) = if let Some(s) = node.as_string_node() {
            (s.opening_loc(), s.closing_loc())
        } else if let Some(s) = node.as_x_string_node() {
            (Some(s.opening_loc()), Some(s.closing_loc()))
        } else if let Some(s) = node.as_interpolated_string_node() {
            (s.opening_loc(), s.closing_loc())
        } else if let Some(s) = node.as_interpolated_x_string_node() {
            (Some(s.opening_loc()), Some(s.closing_loc()))
        } else {
            return;
        };
        let (Some(open), Some(close)) = (opening, closing) else {
            return;
        };
        if self.source.get(open.start_offset()) == Some(&b'<') {
            let start = node.location().start_offset();
            self.heredocs.push((start, self.line_of(close.start_offset())));
        }
    }

    /// Resolve the concept's parser-`:begin` context (keyed by `node_start`)
    /// from the current frames. `Some((eligible, sibling))` where `eligible`
    /// is true when the node's parser parent is `:begin` and the node is not
    /// its last child; the sibling (the parser right sibling) is returned for
    /// the one-liner checks. `None` when the node is not found or has no
    /// `:begin` parent.
    fn resolve(&self, node_start: usize) -> Option<(bool, Option<Node<'static>>)> {
        // The current concept node is NOT pushed onto `frames` until after
        // handling, so its immediate parent is the current top frame.
        let plen = self.frames.len();
        if plen < 1 {
            return None;
        }
        let parent = &self.frames[plen - 1].node;
        // The parent's statement sequence (children read from the
        // lifetime-erased frame, inspected only within this call). Every
        // multi-statement branch below is parser `:begin`; `classify_children`
        // returns `Some` only when the concept is found among the children, so
        // trying each shape in turn is unambiguous. Explicit `BeginNode` is
        // special-cased (main body needs a rescue/ensure clause).
        // Single-shape containers (their statements are the parser `:begin`
        // sequence when multi-statement).
        if let Some(s) = parent.as_program_node().map(|p| p.statements()) {
            return search_statements(&s, node_start);
        }
        if let Some(s) = parent.as_statements_node() {
            return search_statements(&s, node_start);
        }
        if let Some(s) = parent.as_rescue_node().and_then(|r| r.statements()) {
            return search_statements(&s, node_start);
        }
        // Branch-body containers: the then-branch of if/unless, an else body,
        // and loop / case-clause bodies. Each exposes one `statements()`.
        for stmts in [
            parent.as_if_node().and_then(|n| n.statements()),
            parent.as_unless_node().and_then(|n| n.statements()),
            parent.as_else_node().and_then(|n| n.statements()),
            parent.as_while_node().and_then(|n| n.statements()),
            parent.as_until_node().and_then(|n| n.statements()),
            parent.as_for_node().and_then(|n| n.statements()),
            parent.as_when_node().and_then(|n| n.statements()),
            parent.as_in_node().and_then(|n| n.statements()),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(r) = search_statements(&stmts, node_start) {
                return Some(r);
            }
        }
        if let Some(b) = parent.as_begin_node() {
            // The begin's main statements, ensure body and else body are all
            // visited FRAMELESS (via `visit_statements_node` directly), so the
            // top frame for a concept in any of them is the `BeginNode`.
            // - main body: `:begin` only when a rescue/ensure clause is present
            //   (a bare `begin a; b end` keeps `a`,`b` as `:kwbegin` children).
            // - ensure / else body: `:begin` when multi-statement.
            let has_handler = b.rescue_clause().is_some() || b.ensure_clause().is_some();
            if let Some(stmts) = b.statements() {
                let len = stmts.body().iter().count();
                if let Some(r) =
                    classify_children(stmts.body().iter(), node_start, has_handler && len >= 2)
                {
                    return Some(r);
                }
            }
            for stmts in [
                b.ensure_clause().and_then(|e| e.statements()),
                b.else_clause().and_then(|e| e.statements()),
            ]
            .into_iter()
            .flatten()
            {
                if let Some(r) = search_statements(&stmts, node_start) {
                    return Some(r);
                }
            }
        }
        None
    }

    fn handle_call(&mut self, node: &Node<'_>, call: &CallNode<'_>) {
        let name = call.name().as_slice();
        let role_mask = self.cfg.roles_of(name);
        let kind = block_kind(call);
        let recv = call.receiver();
        let recv_none = recv.is_none();
        let rspec_recv = recv_none || recv.as_ref().is_some_and(rspec_const);
        let loc = node.location();
        let (start, end) = (loc.start_offset(), loc.end_offset());

        // --- final_let: detected at the GROUP; find the last let in its body.
        // example_group_or_include? = plain block, rspec receiver, EG|SG|Includes.
        if kind == BlockKind::Plain
            && rspec_recv
            && role_mask & (roles::EG_ALL | roles::SG_ALL | roles::INC_ALL) != 0
        {
            self.handle_final_let(call);
        }

        if role_mask == 0 {
            return;
        }

        // --- example (plain block, nil receiver, Examples.all).
        if kind == BlockKind::Plain
            && recv_none
            && role_mask & roles::EX_ALL != 0
            && let Some((true, sibling)) = self.resolve(start)
            && !self.example_one_liner_allowed(start, end, sibling.as_ref())
        {
            self.push_pending(Cop::Example, name, start, end);
        }

        // --- example_group (spec_group?: plain block, rspec receiver, EG|SG).
        if kind == BlockKind::Plain
            && rspec_recv
            && role_mask & (roles::EG_ALL | roles::SG_ALL) != 0
            && matches!(self.resolve(start), Some((true, _)))
        {
            self.push_pending(Cop::ExampleGroup, name, start, end);
        }

        // --- hook (any block kind, nil receiver, Hooks.all).
        if matches!(kind, BlockKind::Plain | BlockKind::Numbered | BlockKind::It)
            && recv_none
            && role_mask & roles::HOOKS != 0
            && let Some((true, sibling)) = self.resolve(start)
            && !self.hook_one_liner_allowed(start, end, sibling.as_ref())
        {
            self.push_pending(Cop::Hook, name, start, end);
        }

        // --- subject (plain block, nil receiver, Subjects.all, inside group).
        if kind == BlockKind::Plain
            && recv_none
            && role_mask & roles::SUBJECTS != 0
            && self.top_spec_depth > 0
            && matches!(self.resolve(start), Some((true, _)))
        {
            self.push_pending(Cop::Subject, name, start, end);
        }
    }

    /// `EmptyLineAfterFinalLet` at a qualifying group `call`: the last `let?`
    /// among the group body's direct children, when it is not the body's last
    /// child (and the body has `>= 2` children).
    fn handle_final_let(&mut self, group: &CallNode<'_>) {
        let Some(block) = group.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        let Some(stmts) = block.body().and_then(|b| b.as_statements_node()) else {
            return;
        };
        let children: Vec<Node<'_>> = stmts.body().iter().collect();
        let len = children.len();
        if len < 2 {
            return;
        }
        // Find the LAST `let?` child (block form or `let(:x, &blk)` send form).
        let mut last_let: Option<(usize, &Node<'_>)> = None;
        for (i, child) in children.iter().enumerate() {
            if self.is_let(child) {
                last_let = Some((i, child));
            }
        }
        let Some((i, let_node)) = last_let else {
            return;
        };
        if i == len - 1 {
            return; // last child of the body: no offense.
        }
        let Some(call) = let_node.as_call_node() else {
            return;
        };
        let loc = let_node.location();
        self.push_pending(
            Cop::FinalLet,
            call.name().as_slice(),
            loc.start_offset(),
            loc.end_offset(),
        );
    }

    /// `let?`: `(block (send nil? #Helpers.all ...) ...)` OR
    /// `(send nil? #Helpers.all _ block_pass)`.
    fn is_let(&self, node: &Node<'_>) -> bool {
        let Some(call) = node.as_call_node() else {
            return false;
        };
        if call.receiver().is_some() {
            return false;
        }
        if self.cfg.roles_of(call.name().as_slice()) & roles::HELPERS == 0 {
            return false;
        }
        match block_kind(&call) {
            BlockKind::Plain | BlockKind::Numbered | BlockKind::It => true,
            // send form `(send nil? #Helpers.all _ block_pass)`: the block-pass
            // is prism's `call.block()` (a `BlockArgumentNode`), and there must
            // be exactly one positional argument (`_`).
            BlockKind::BlockArg => {
                call.arguments().map(|a| a.arguments().iter().count()) == Some(1)
            }
            BlockKind::None => false,
        }
    }

    fn example_one_liner_allowed(
        &self,
        start: usize,
        end: usize,
        sibling: Option<&Node<'_>>,
    ) -> bool {
        if !self.cfg.example_allow_consecutive {
            return false;
        }
        if !self.single_line(start, end) {
            return false;
        }
        // next sibling is a single-line example (plain block, nil recv,
        // Examples.all).
        let Some(sib) = sibling else { return false };
        let Some(call) = sib.as_call_node() else {
            return false;
        };
        if call.receiver().is_some() || block_kind(&call) != BlockKind::Plain {
            return false;
        }
        if self.cfg.roles_of(call.name().as_slice()) & roles::EX_ALL == 0 {
            return false;
        }
        let sl = sib.location();
        self.single_line(sl.start_offset(), sl.end_offset())
    }

    fn hook_one_liner_allowed(
        &self,
        start: usize,
        end: usize,
        sibling: Option<&Node<'_>>,
    ) -> bool {
        if !self.cfg.hook_allow_consecutive {
            return false;
        }
        if !self.single_line(start, end) {
            return false;
        }
        // next sibling is a single-line hook (any block kind, nil recv,
        // Hooks.all).
        let Some(sib) = sibling else { return false };
        let Some(call) = sib.as_call_node() else {
            return false;
        };
        if call.receiver().is_some() {
            return false;
        }
        if !matches!(
            block_kind(&call),
            BlockKind::Plain | BlockKind::Numbered | BlockKind::It
        ) {
            return false;
        }
        if self.cfg.roles_of(call.name().as_slice()) & roles::HOOKS == 0 {
            return false;
        }
        let sl = sib.location();
        self.single_line(sl.start_offset(), sl.end_offset())
    }

    fn push_pending(&mut self, cop: Cop, name: &[u8], start: usize, end: usize) {
        self.pending.push(Pending {
            cop,
            node_start: start,
            node_end: end,
            node_end_line: self.node_last_line(end),
            method_name: String::from_utf8_lossy(name).into_owned(),
        });
    }

    /// Post-walk: finalize `final_end_line` per candidate against the collected
    /// heredocs (a heredoc terminator below the node's own end line wins) and
    /// route each offense to its cop's slot.
    pub fn finish(self) -> RSpecEmptyLineResult {
        let mut result = RSpecEmptyLineResult::default();
        for p in &self.pending {
            let mut final_line = p.node_end_line;
            for &(hd_start, hd_line) in &self.heredocs {
                if hd_start >= p.node_start && hd_start < p.node_end && hd_line > final_line {
                    final_line = hd_line;
                }
            }
            let off = EmptyLineOffense {
                final_end_line: final_line,
                method_name: p.method_name.clone(),
            };
            match p.cop {
                Cop::Example => result.example.push(off),
                Cop::ExampleGroup => result.example_group.push(off),
                Cop::FinalLet => result.final_let.push(off),
                Cop::Hook => result.hook.push(off),
                Cop::Subject => result.subject.push(off),
            }
        }
        result
    }

    /// `top_spec`: this node is a top-level `spec_group?` (a spec group whose
    /// parser parent is the program root). Mirrors `rspec_dispatcher`.
    fn is_top_spec(&self, call: &CallNode<'_>) -> bool {
        // Parent frame is the ProgramNode ⇒ this is a top-level statement
        // (the current node is not yet pushed, so the top frame is the parent).
        let plen = self.frames.len();
        if plen < 1 || self.frames[plen - 1].node.as_program_node().is_none() {
            return false;
        }
        let kind = block_kind(call);
        if !matches!(kind, BlockKind::Plain | BlockKind::Numbered | BlockKind::It) {
            return false;
        }
        let recv = call.receiver();
        let rspec_recv = recv.is_none() || recv.as_ref().is_some_and(rspec_const);
        rspec_recv
            && self.cfg.roles_of(call.name().as_slice()) & (roles::EG_ALL | roles::SG_ALL) != 0
    }
}

/// Search a statement sequence for `node_start`; the sequence is parser
/// `:begin` (offense eligible) when it has `>= 2` children.
fn search_statements<'p>(
    s: &StatementsNode<'p>,
    node_start: usize,
) -> Option<(bool, Option<Node<'p>>)> {
    let len = s.body().iter().count();
    classify_children(s.body().iter(), node_start, len >= 2)
}

/// Find `node_start` among `children` (by start offset); return
/// `(eligible && not-last, right_sibling)`.
fn classify_children<'p>(
    children: impl Iterator<Item = Node<'p>>,
    node_start: usize,
    eligible: bool,
) -> Option<(bool, Option<Node<'p>>)> {
    let list: Vec<Node<'p>> = children.collect();
    let idx = list
        .iter()
        .position(|c| c.location().start_offset() == node_start)?;
    let not_last = idx + 1 < list.len();
    let sibling = if not_last {
        // Clone the sibling node (Copy-by-bytes) so it outlives `list`.
        let sib = &list[idx + 1];
        Some(unsafe { copy_node(sib) })
    } else {
        None
    };
    Some((eligible && not_last, sibling))
}

/// Byte-copy a `Node` preserving its lifetime (prism `Node` is not `Clone`).
#[allow(clippy::missing_safety_doc)]
unsafe fn copy_node<'a>(node: &Node<'a>) -> Node<'a> {
    unsafe { std::mem::transmute_copy::<Node<'a>, Node<'a>>(node) }
}

impl Rule for RSpecEmptyLineRule<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        self.maybe_heredoc(node);
        let mut top_spec = false;
        if let Some(call) = node.as_call_node() {
            if self.is_top_spec(&call) {
                top_spec = true;
            }
            self.handle_call(node, &call);
        }
        if top_spec {
            self.top_spec_depth += 1;
        }
        let node_static = unsafe { copy_to_static(node) };
        self.frames.push(Frame {
            node: node_static,
            top_spec,
        });
    }

    fn leave(&mut self) {
        if let Some(frame) = self.frames.pop()
            && frame.top_spec
        {
            self.top_spec_depth -= 1;
        }
    }

    fn enter_leaf(&mut self, node: &Node<'_>) {
        self.maybe_heredoc(node);
    }

    fn enter_rescue(&mut self, node: &Node<'_>) {
        let node_static = unsafe { copy_to_static(node) };
        self.frames.push(Frame {
            node: node_static,
            top_spec: false,
        });
    }

    fn leave_rescue(&mut self) {
        self.frames.pop();
    }

    fn interest(&self) -> Interest {
        Interest::ALL
    }
}

/// Standalone entry point (the wrappers' fallback path): run the rule alone.
pub fn check_rspec_empty_line(source: &[u8], cfg: &RSpecConfig) -> RSpecEmptyLineResult {
    let mut rule = build_rule(source, cfg);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::rspec_language;

    fn cfg() -> RSpecConfig {
        RSpecConfig::from_role_lists(&rspec_language::tests::default_role_lists()).unwrap()
    }

    /// `(final_end_line, method_name)` per offense of each cop.
    fn run(src: &str) -> RSpecEmptyLineResult {
        check_rspec_empty_line(src.as_bytes(), &cfg())
    }

    fn pairs(v: &[EmptyLineOffense]) -> Vec<(usize, &str)> {
        v.iter()
            .map(|o| (o.final_end_line, o.method_name.as_str()))
            .collect()
    }

    #[test]
    fn example_basic_multiline() {
        let src = "RSpec.describe Foo do\n  it 'a' do\n    x\n  end\n  it 'b' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(4, "it")]);
    }

    #[test]
    fn example_consecutive_one_liners_allowed_by_default() {
        let src = "RSpec.describe Foo do\n  it { one }\n  it { two }\nend\n";
        assert!(run(src).example.is_empty());
    }

    #[test]
    fn example_one_liner_flagged_when_next_is_not_an_example() {
        // single-line example followed by a multi-line example: not the
        // allowed consecutive-one-liner shape.
        let src = "RSpec.describe Foo do\n  it { one }\n  it 'b' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(2, "it")]);
    }

    #[test]
    fn example_disabled_allow_consecutive_flags_one_liners() {
        let mut c = cfg();
        c.example_allow_consecutive = false;
        let src = "RSpec.describe Foo do\n  it { one }\n  it { two }\nend\n";
        let res = check_rspec_empty_line(src.as_bytes(), &c);
        assert_eq!(pairs(&res.example), vec![(2, "it")]);
    }

    #[test]
    fn example_kwbegin_without_handler_is_not_begin() {
        let src = "RSpec.describe Foo do\n  begin\n    it 'a' do\n      x\n    end\n    it 'b' do\n      y\n    end\n  end\nend\n";
        assert!(run(src).example.is_empty());
    }

    #[test]
    fn example_kwbegin_with_rescue_is_begin() {
        let src = "RSpec.describe Foo do\n  begin\n    it 'a' do\n      x\n    end\n    it 'b' do\n      y\n    end\n  rescue\n    z\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(5, "it")]);
    }

    #[test]
    fn example_in_rescue_body_multi_is_begin() {
        let src = "RSpec.describe Foo do\n  work\nrescue\n  it 'a' do\n    x\n  end\n  it 'b' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(6, "it")]);
    }

    #[test]
    fn example_in_ensure_body_multi_is_begin() {
        let src = "RSpec.describe Foo do\n  work\nensure\n  it 'a' do\n    x\n  end\n  it 'b' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(6, "it")]);
    }

    #[test]
    fn example_three_multiline() {
        let src = "RSpec.describe Foo do\n  it 'a' do\n    x\n  end\n  it 'b' do\n    y\n  end\n  it 'c' do\n    z\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(4, "it"), (7, "it")]);
    }

    #[test]
    fn example_single_is_last_child() {
        let src = "RSpec.describe Foo do\n  it 'a' do\n    x\n  end\nend\n";
        assert!(run(src).example.is_empty());
    }

    #[test]
    fn hook_consecutive_one_liner_chain() {
        // `before` chains to `after` (both one-liner hooks) => allowed;
        // `after` precedes `it` => flagged.
        let src = "RSpec.describe Foo do\n  before { a }\n  after { b }\n  it { c }\nend\n";
        assert_eq!(pairs(&run(src).hook), vec![(3, "after")]);
    }

    #[test]
    fn hook_numblock_fires() {
        let src = "RSpec.describe Foo do\n  before { _1 }\n  it 'x' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).hook), vec![(2, "before")]);
    }

    #[test]
    fn final_let_multi() {
        let src = "RSpec.describe Foo do\n  let(:a) { 1 }\n  let(:b) { 2 }\n  it 'x' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).final_let), vec![(3, "let")]);
    }

    #[test]
    fn final_let_send_form() {
        let src = "describe 'x' do\n  let(:a) { 1 }\n  let(:b, &blk)\n  it 'y' do\n    z\n  end\nend\n";
        assert_eq!(pairs(&run(src).final_let), vec![(3, "let")]);
    }

    #[test]
    fn final_let_it_then_let_is_last_child() {
        let src = "RSpec.describe Foo do\n  it 'x' do\n    y\n  end\n  let(:a) { 1 }\nend\n";
        assert!(run(src).final_let.is_empty());
    }

    #[test]
    fn final_let_single_is_last_child() {
        let src = "RSpec.describe Foo do\n  let(:a) { 1 }\nend\n";
        assert!(run(src).final_let.is_empty());
    }

    #[test]
    fn subject_heredoc_final_end_follows_terminator() {
        let src = "RSpec.describe Foo do\n  subject(:obj) { described_class.new(<<~ARGS) }\n    a\n    b\n  ARGS\n  let(:foo) { bar }\nend\n";
        assert_eq!(pairs(&run(src).subject), vec![(5, "subject")]);
    }

    #[test]
    fn subject_top_level_is_not_inside_group() {
        let src = "subject(:obj) { described_class }\nlet(:foo) { bar }\n";
        assert!(run(src).subject.is_empty());
    }

    #[test]
    fn subject_inside_class_wrapped_group_is_excluded() {
        let src = "class Wrap\n  RSpec.describe Foo do\n    subject(:obj) { described_class }\n    let(:foo) { bar }\n  end\nend\n";
        assert!(run(src).subject.is_empty());
    }

    #[test]
    fn example_group_fires_on_shared_group() {
        let src = "RSpec.describe Foo do\n  shared_examples 'x' do\n    it { a }\n  end\n  describe '#bar' do\n    it { b }\n  end\nend\n";
        assert_eq!(pairs(&run(src).example_group), vec![(4, "shared_examples")]);
    }

    #[test]
    fn example_group_two_top_level_groups() {
        let src = "RSpec.describe Foo do\n  it { a }\nend\nRSpec.describe Bar do\n  it { b }\nend\n";
        assert_eq!(pairs(&run(src).example_group), vec![(3, "describe")]);
    }

    #[test]
    fn numblock_example_is_not_an_example() {
        // `it('a') { _1 }` is a numblock, so on_block never fires => no offense.
        let src = "RSpec.describe Foo do\n  it('a') { _1 }\n  it('b') { _1 }\nend\n";
        assert!(run(src).example.is_empty());
    }
}
