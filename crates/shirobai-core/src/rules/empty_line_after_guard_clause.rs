//! `Layout/EmptyLineAfterGuardClause`.
//!
//! Stock's `on_if` flags every `if`/`unless` whose `if_branch` is a guard
//! clause (`raise`/`fail`/`return`/`break`/`next` as a single-line expression,
//! with `and`/`or` peeled to the rhs) UNLESS one of `correct_style?`'s gates
//! fires:
//!
//! - `node.parent` is `nil`, `rescue`, `ensure` (`next_line_rescue_or_ensure?`),
//! - `node.right_sibling.nil?` or the sibling's parent is an `if` with `else`
//!   (`next_sibling_parent_empty_or_else?`),
//! - the sibling is itself a guard-bearing `if`/`unless`
//!   (`next_sibling_empty_or_guard_clause?`).
//!
//! After that, `next_line_empty_or_allowed_directive_comment?` looks at the
//! line below the guard — blank, or a `# rubocop:enable` / `# :nocov:` /
//! `# simplecov:disable`/`enable` directive whose own next line is blank —
//! and suppresses the offense.  The Rust side handles every gate up to
//! (and not including) the directive check and emits a candidate;  the
//! Ruby wrapper does the directive regex match and the autocorrect.
//!
//! Offense location:
//! - heredoc-routed (`last_heredoc_argument` finds one on the modifier path):
//!   the heredoc closer (`loc.heredoc_end`),
//! - non-modifier (multi-line): `loc.end` (the `end` keyword),
//! - otherwise: the whole if node.

use ruby_prism::Node;
use std::rc::Rc;

use super::line_index::LineIndex;

/// One offense candidate; the Ruby wrapper finishes the
/// `next_line_empty_or_allowed_directive_comment?` check.
pub struct GuardClauseCandidate {
    /// Offense range stock passes to `add_offense`.
    pub offense_start: usize,
    pub offense_end: usize,
    /// First byte of the range stock's `range_by_whole_lines` spans (the
    /// guard's whole-line range, or the heredoc body's first line for the
    /// heredoc path).  The wrapper builds the range from this and `last_line`.
    pub ac_anchor_first_line_start: usize,
    /// 1-based last line of the same range — also the line whose
    /// blankness/directive status drives the `next_line_empty?` check
    /// (`processed_source[last_line]` in stock).
    pub ac_anchor_last_line: usize,
}

pub fn check_empty_line_after_guard_clause(source: &[u8]) -> Vec<GuardClauseCandidate> {
    let mut rule = build_rule(source);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.candidates
}

pub(crate) fn build_rule(source: &[u8]) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        frames: Vec::new(),
        candidates: Vec::new(),
    }
}

// We stash each frame's prism node (lifetime-erased) so the if visit can ask
// "what is my parent prism node?" and call prism accessors on it.  Prism's
// `Node` is two `*mut` pointers behind a `PhantomData<&'pr mut _>` (non-Copy
// because the marker is non-Copy); the underlying bytes ARE Copy.  We use
// `mem::transmute_copy` to byte-copy a `Node<'_>` into a `Node<'static>`
// stored in the frame.  This is safe because every frame is popped on
// `leave`, well within `dispatch::run` and therefore within the parse's
// lifetime held by `parse_cache`.
struct Frame {
    /// The prism node this frame represents, lifetime-erased.
    node: Node<'static>,
}

/// Byte-copy a `Node` into a `Node<'static>`.  Safe under the invariants
/// noted on `Frame::node`.
#[allow(clippy::missing_safety_doc)]
unsafe fn copy_to_static<'a>(node: &Node<'a>) -> Node<'static> {
    unsafe { std::mem::transmute_copy::<Node<'a>, Node<'static>>(node) }
}

/// Byte-copy a `Node` preserving its lifetime.
#[allow(clippy::missing_safety_doc)]
unsafe fn copy_node<'a>(node: &Node<'a>) -> Node<'a> {
    unsafe { std::mem::transmute_copy::<Node<'a>, Node<'a>>(node) }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    frames: Vec<Frame>,
    pub(crate) candidates: Vec<GuardClauseCandidate>,
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        let node_static = unsafe { copy_to_static(node) };
        self.frames.push(Frame { node: node_static });

        if node.as_if_node().is_some() || node.as_unless_node().is_some() {
            self.handle_if_or_unless(node);
        }
    }

    fn leave(&mut self) {
        self.frames.pop();
    }

    fn enter_rescue(&mut self, node: &Node<'_>) {
        let node_static = unsafe { copy_to_static(node) };
        self.frames.push(Frame { node: node_static });
    }

    fn leave_rescue(&mut self) {
        self.frames.pop();
    }
}

impl<'a> Visitor<'a> {
    fn handle_if_or_unless(&mut self, node: &Node<'_>) {
        // `modifier_form?`: absence of `end_keyword_loc`.
        let (modifier, statements, _has_subsequent) = if let Some(i) = node.as_if_node() {
            (
                i.end_keyword_loc().is_none(),
                i.statements(),
                i.subsequent().is_some(),
            )
        } else if let Some(u) = node.as_unless_node() {
            (
                u.end_keyword_loc().is_none(),
                u.statements(),
                u.else_clause().is_some(),
            )
        } else {
            return;
        };

        // Step 1: `!node.if_branch&.guard_clause?` — skip when no guard.
        let Some(stmts) = statements else { return };
        let Some(if_branch) = stmts.body().iter().next() else {
            return;
        };
        if !is_guard_clause(&if_branch) {
            return;
        }

        // Resolve parser parent + right sibling from the prism context.
        let context = self.parser_context(node);

        // `multiple_statements_on_line?(node)`.
        if context.parent_is_begin
            && let Some(sib) = context.right_sibling.as_ref()
        {
            let n_line = self.line_index.line_of(node.location().start_offset());
            let s_line = self.line_index.line_of(sib.location().start_offset());
            if n_line == s_line {
                return;
            }
        }

        // `correct_style?`:
        if context.is_rescue_or_ensure_or_nil_parent {
            return;
        }
        if next_sibling_parent_empty_or_else(&context) {
            return;
        }
        if next_sibling_empty_or_guard_clause(&context.right_sibling) {
            return;
        }

        // We are an offense candidate; the directive-comment check happens
        // on the Ruby side.
        let node_loc = node.location();
        let node_first_line = self.line_index.line_of(node_loc.start_offset());
        let node_last_line = self
            .line_index
            .line_of(node_loc.end_offset().saturating_sub(1));

        let heredoc = if modifier {
            self.last_heredoc_argument(node, &if_branch)
        } else {
            None
        };

        let (offense_start, offense_end, ac_first, ac_last) = if let Some(h) = heredoc {
            let heredoc_first_line = self.line_index.line_of(h.body_start);
            let heredoc_last_line = self.line_index.line_of(h.closing_start);
            let first_line_start = self.line_start_byte(heredoc_first_line);
            (h.closing_start, h.closing_end, first_line_start, heredoc_last_line)
        } else if !modifier {
            let end_kw = match node.as_if_node().and_then(|n| n.end_keyword_loc()) {
                Some(loc) => (loc.start_offset(), loc.end_offset()),
                None => match node.as_unless_node().and_then(|n| n.end_keyword_loc()) {
                    Some(loc) => (loc.start_offset(), loc.end_offset()),
                    None => return,
                },
            };
            let first_line_start = self.line_start_byte(node_first_line);
            (end_kw.0, end_kw.1, first_line_start, node_last_line)
        } else {
            let first_line_start = self.line_start_byte(node_first_line);
            (
                node_loc.start_offset(),
                node_loc.end_offset(),
                first_line_start,
                node_last_line,
            )
        };

        self.candidates.push(GuardClauseCandidate {
            offense_start,
            offense_end,
            ac_anchor_first_line_start: ac_first,
            ac_anchor_last_line: ac_last,
        });
    }

    fn line_start_byte(&self, line1: usize) -> usize {
        self.line_index
            .line_starts()
            .get(line1.saturating_sub(1))
            .copied()
            .unwrap_or(self.source.len())
    }

    fn last_heredoc_argument(
        &self,
        node: &Node<'_>,
        if_branch: &Node<'_>,
    ) -> Option<HeredocInfo> {
        last_heredoc_for_if(node, if_branch)
    }

    /// Resolve the parser-AST parent shape of the if/unless node currently
    /// being entered.  Walks the prism ancestor stack (this `if`'s own frame
    /// is at the top — skip it), then uses prism accessors on each parent to
    /// find the containing statements and count.
    fn parser_context(&self, if_node: &Node<'_>) -> ParserContext {
        // The if's own frame is at the top; skip it.
        let len = self.frames.len();
        if len < 2 {
            // No parser parent (the if is the program root statement).
            return ParserContext {
                parser_kind: ParserKind::None,
                parent_is_begin: false,
                is_rescue_or_ensure_or_nil_parent: true,
                right_sibling: None,
            };
        }
        let mut i = len - 2; // start from the immediate parent
        loop {
            let parent = &self.frames[i].node;
            match resolve_via_parent(parent, if_node) {
                Resolution::ParserParent { kind, right_sibling } => {
                    let parent_is_begin = matches!(kind, ParserKind::Begin);
                    let is_rne = matches!(
                        kind,
                        ParserKind::None | ParserKind::Rescue | ParserKind::Ensure
                    );
                    return ParserContext {
                        parser_kind: kind,
                        parent_is_begin,
                        is_rescue_or_ensure_or_nil_parent: is_rne,
                        right_sibling,
                    };
                }
                Resolution::Continue => {
                    if i == 0 {
                        // No parser parent found.  Treat as nil.
                        return ParserContext {
                            parser_kind: ParserKind::None,
                            parent_is_begin: false,
                            is_rescue_or_ensure_or_nil_parent: true,
                            right_sibling: None,
                        };
                    }
                    i -= 1;
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ParserKind {
    /// No parser parent.
    None,
    /// `begin` (multi-stmt Statements or `kwbegin`).
    Begin,
    /// `rescue` (outer with else/resbody).
    Rescue,
    /// `ensure`.
    Ensure,
    /// `resbody`.
    Resbody,
    /// Then-branch of an `if`/`unless`.
    IfThenBranch,
    /// Else / elsif branch.
    IfElseBranch,
    /// `while`/`until`/`for` body.
    Loop,
    /// def / class / module / sclass / block / lambda body (no special gate).
    Other,
}

struct ParserContext {
    parser_kind: ParserKind,
    parent_is_begin: bool,
    is_rescue_or_ensure_or_nil_parent: bool,
    right_sibling: Option<Node<'static>>,
}

enum Resolution {
    /// We found the parser parent.  `right_sibling` is the parser right
    /// sibling of the if (`None` when nil).
    ParserParent {
        kind: ParserKind,
        right_sibling: Option<Node<'static>>,
    },
    /// This frame is transparent (e.g. ParenthesesNode); look one frame up.
    Continue,
}

/// Decide what the parser parent of `if_node` is, given `parent` (its
/// immediate non-transparent prism ancestor).  We inspect `parent` via prism
/// accessors to find which statements-block contains `if_node` and how many
/// children that block has.
fn resolve_via_parent(parent: &Node<'_>, if_node: &Node<'_>) -> Resolution {
    let if_start = if_node.location().start_offset();

    // ProgramNode: contains a single StatementsNode.
    if let Some(p) = parent.as_program_node() {
        let stmts = p.statements();
        return resolve_in_statements(&stmts, if_start, ParserKind::Other);
    }
    // DefNode / class/module/sclass: body is the whole-method body, which may
    // itself be a BeginNode (rescue/ensure-bearing) or a StatementsNode or
    // nothing.  The if is inside that body.
    if let Some(d) = parent.as_def_node() {
        return resolve_in_body(d.body(), if_start, ParserKind::Other);
    }
    if let Some(c) = parent.as_class_node() {
        return resolve_in_body(c.body(), if_start, ParserKind::Other);
    }
    if let Some(m) = parent.as_module_node() {
        return resolve_in_body(m.body(), if_start, ParserKind::Other);
    }
    if let Some(s) = parent.as_singleton_class_node() {
        return resolve_in_body(s.body(), if_start, ParserKind::Other);
    }
    if let Some(b) = parent.as_block_node() {
        return resolve_in_body(b.body(), if_start, ParserKind::Other);
    }
    if let Some(l) = parent.as_lambda_node() {
        return resolve_in_body(l.body(), if_start, ParserKind::Other);
    }
    // BeginNode: the if might be in the main statements, a rescue clause, the
    // else clause, or the ensure clause.  Try each.
    if let Some(b) = parent.as_begin_node() {
        let begin_keyword = b.begin_keyword_loc().is_some();
        let outer_kind = if begin_keyword {
            // kwbegin: parser node is `kwbegin`, begin_type? is true; nested
            // sole stmt's parent is kwbegin itself.
            ParserKind::Begin
        } else {
            // Implicit begin: parser node is `rescue` or `ensure`.  The
            // statements of the begin are the body of the rescue/ensure.
            if b.ensure_clause().is_some() {
                ParserKind::Ensure
            } else {
                ParserKind::Rescue
            }
        };
        if let Some(stmts) = b.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, outer_kind)
        {
            return r;
        }
        // Inside the else_clause (rescue's else): parser parent = rescue.
        if let Some(els) = b.else_clause()
            && let Some(stmts) = els.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Rescue)
        {
            return r;
        }
        // Inside the ensure_clause body: parser parent = ensure.
        if let Some(ens) = b.ensure_clause()
            && let Some(stmts) = ens.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Ensure)
        {
            return r;
        }
        // The if might be nested via the rescue_clause's body — but that
        // is reached through `enter_rescue` separately, so its parser parent
        // is `resbody` (we handle that under RescueNode below).
        if let Some(rescue) = b.rescue_clause()
            && let Some(stmts) = rescue.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Resbody)
        {
            return r;
        }
        // Walk the rescue's `subsequent` chain too.
        let mut cur = b.rescue_clause().and_then(|r| r.subsequent());
        while let Some(rn) = cur {
            if let Some(stmts) = rn.statements()
                && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Resbody)
            {
                return r;
            }
            cur = rn.subsequent();
        }
        // Not found through any direct child — fall through.
        return Resolution::Continue;
    }
    // IfNode/UnlessNode: the if may be in the then-branch statements, or in
    // the subsequent (`subsequent` may itself be an ElseNode wrapping
    // statements OR a chained IfNode for elsif).
    if let Some(outer) = parent.as_if_node() {
        if let Some(stmts) = outer.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::IfThenBranch)
        {
            return r;
        }
        if let Some(sub) = outer.subsequent() {
            // `subsequent` can be an ElseNode (else body) or a chained IfNode
            // (elsif).  An elsif's body is its own statements — that recursion
            // hits as the next frame.  Only ElseNode is interesting here.
            if let Some(els) = sub.as_else_node()
                && let Some(stmts) = els.statements()
                && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::IfElseBranch)
            {
                return r;
            }
        }
        return Resolution::Continue;
    }
    if let Some(outer) = parent.as_unless_node() {
        if let Some(stmts) = outer.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::IfThenBranch)
        {
            return r;
        }
        if let Some(els) = outer.else_clause()
            && let Some(stmts) = els.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::IfElseBranch)
        {
            return r;
        }
        return Resolution::Continue;
    }
    // EnsureNode: visited as a typed child of BeginNode (we don't have a
    // frame for it unless the dispatcher fires the branch hook for it).
    // Most cases here are reached through the BeginNode arm above.  Keep
    // for safety:
    if let Some(e) = parent.as_ensure_node() {
        if let Some(stmts) = e.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Ensure)
        {
            return r;
        }
        return Resolution::Continue;
    }
    if let Some(e) = parent.as_else_node() {
        if let Some(stmts) = e.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::IfElseBranch)
        {
            return r;
        }
        return Resolution::Continue;
    }
    // RescueNode (resbody): the dispatch's `enter_rescue` pushed this frame;
    // the if is inside this resbody's statements.
    if let Some(r) = parent.as_rescue_node() {
        if let Some(stmts) = r.statements()
            && let Some(res) = resolve_if_inside(&stmts, if_start, ParserKind::Resbody)
        {
            return res;
        }
        return Resolution::Continue;
    }
    // WhenNode / InNode: their body statements contain the if.
    if let Some(w) = parent.as_when_node() {
        if let Some(stmts) = w.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Other)
        {
            return r;
        }
        return Resolution::Continue;
    }
    if let Some(w) = parent.as_in_node() {
        if let Some(stmts) = w.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Other)
        {
            return r;
        }
        return Resolution::Continue;
    }
    // CaseNode's else clause: also has statements.
    if let Some(c) = parent.as_case_node() {
        if let Some(els) = c.else_clause()
            && let Some(stmts) = els.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Other)
        {
            return r;
        }
        return Resolution::Continue;
    }
    if let Some(c) = parent.as_case_match_node() {
        if let Some(els) = c.else_clause()
            && let Some(stmts) = els.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Other)
        {
            return r;
        }
        return Resolution::Continue;
    }
    // While/Until/For: body is a statements.
    if let Some(w) = parent.as_while_node() {
        if let Some(stmts) = w.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Loop)
        {
            return r;
        }
        return Resolution::Continue;
    }
    if let Some(w) = parent.as_until_node() {
        if let Some(stmts) = w.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Loop)
        {
            return r;
        }
        return Resolution::Continue;
    }
    if let Some(w) = parent.as_for_node() {
        if let Some(stmts) = w.statements()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Loop)
        {
            return r;
        }
        return Resolution::Continue;
    }
    // ParenthesesNode: transparent (parser sees its body as the value).
    if let Some(p) = parent.as_parentheses_node() {
        if let Some(body) = p.body()
            && let Some(stmts) = body.as_statements_node()
            && let Some(r) = resolve_if_inside(&stmts, if_start, ParserKind::Other)
        {
            return r;
        }
        return Resolution::Continue;
    }
    // StatementsNode as a parent (e.g. reached through enum-match visit of
    // StatementsNode): the if is one of its children.
    if let Some(s) = parent.as_statements_node() {
        return resolve_in_statements(&s, if_start, ParserKind::Other);
    }
    // Anything else: transparent.
    Resolution::Continue
}

/// Look at `body` (a wrapper's body slot) and try to find `if_start` inside.
fn resolve_in_body(
    body: Option<Node<'_>>,
    if_start: usize,
    outer_kind: ParserKind,
) -> Resolution {
    let Some(body) = body else {
        return Resolution::Continue;
    };
    if let Some(stmts) = body.as_statements_node() {
        return resolve_in_statements(&stmts, if_start, outer_kind);
    }
    // BeginNode body: not directly statements; the BeginNode arm of
    // `resolve_via_parent` will handle it once we hit the BeginNode frame.
    // For a body that IS a BeginNode but we have the wrapper above only
    // (DefNode etc.), the BeginNode is a separate frame the walker pushed.
    // So we don't need to descend here.
    Resolution::Continue
}

fn resolve_in_statements(
    stmts: &ruby_prism::StatementsNode<'_>,
    if_start: usize,
    outer_kind: ParserKind,
) -> Resolution {
    let body: Vec<Node<'_>> = stmts.body().iter().collect();
    let mut found_idx: Option<usize> = None;
    for (i, child) in body.iter().enumerate() {
        if child.location().start_offset() == if_start {
            found_idx = Some(i);
            break;
        }
    }
    let Some(idx) = found_idx else {
        return Resolution::Continue;
    };
    // Parser parent: if count >= 2, Begin; else outer_kind.
    let (kind, right_sibling) = if body.len() >= 2 {
        let sibling = body.get(idx + 1).map(|n| {
            unsafe { copy_to_static(n) }
        });
        (ParserKind::Begin, sibling)
    } else {
        // Sole statement.  Parser right sibling is determined by outer_kind:
        // - IfThenBranch: the outer if's `subsequent` (we look that up
        //   separately by passing back outer_kind; the caller knows it has
        //   to consult the surrounding IfNode).  We return the sibling None
        //   here and the caller patches it.
        // - others: no right sibling.
        (outer_kind, None)
    };
    Resolution::ParserParent {
        kind,
        right_sibling,
    }
}

/// Inside-statements helper that returns `Option<Resolution>` so the
/// BeginNode/IfNode arms can try several statement slots.
fn resolve_if_inside(
    stmts: &ruby_prism::StatementsNode<'_>,
    if_start: usize,
    outer_kind: ParserKind,
) -> Option<Resolution> {
    let body: Vec<Node<'_>> = stmts.body().iter().collect();
    let mut found_idx: Option<usize> = None;
    for (i, child) in body.iter().enumerate() {
        if child.location().start_offset() == if_start {
            found_idx = Some(i);
            break;
        }
    }
    let idx = found_idx?;
    let (kind, right_sibling) = if body.len() >= 2 {
        let sibling = body.get(idx + 1).map(|n| {
            unsafe { copy_to_static(n) }
        });
        (ParserKind::Begin, sibling)
    } else {
        (outer_kind, None)
    };
    Some(Resolution::ParserParent { kind, right_sibling })
}

/// `next_sibling_parent_empty_or_else?(node)`:
/// - `right_sibling.nil?` → true
/// - else: `right_sibling.parent.if_type? && right_sibling.parent.else?` → true
///
/// In our model: the sibling's parser parent equals the if's parser parent.
/// `parent.if_type? && parent.else?` is true ONLY when parser parent is
/// `IfThenBranch` AND the outer if has an `else_branch` slot — which is the
/// case here because the sibling itself IS that else_branch (no other source
/// of a right sibling exists when parser parent is IfThenBranch).  So:
/// - right_sibling None → true
/// - right_sibling Some AND parser_kind == IfThenBranch → true
/// - otherwise → false
fn next_sibling_parent_empty_or_else(ctx: &ParserContext) -> bool {
    if ctx.right_sibling.is_none() {
        return true;
    }
    ctx.parser_kind == ParserKind::IfThenBranch
}

fn next_sibling_empty_or_guard_clause(right_sib: &Option<Node<'static>>) -> bool {
    match right_sib {
        None => true,
        Some(sib) => sibling_is_guard_if(sib),
    }
}

// --- guard-clause matching ---

fn is_guard_clause(node: &Node<'_>) -> bool {
    let cur = peel_operator_keyword(node);
    matches_guard_shape(&cur) && is_single_line(&cur)
}

fn is_single_line(node: &Node<'_>) -> bool {
    let loc = node.location();
    let s = loc.as_slice();
    !s.contains(&b'\n')
}

fn matches_guard_shape(node: &Node<'_>) -> bool {
    if node.as_return_node().is_some()
        || node.as_break_node().is_some()
        || node.as_next_node().is_some()
    {
        return true;
    }
    if let Some(call) = node.as_call_node() {
        if call.receiver().is_some() {
            return false;
        }
        return matches!(call.name().as_slice(), b"raise" | b"fail");
    }
    false
}

/// Peel through `and` / `or` keyword nodes to their right-hand side. Only the
/// keyword forms (not `&&` / `||`) count.
fn peel_operator_keyword<'a>(node: &Node<'a>) -> Node<'a> {
    let mut cur = unsafe { copy_node(node) };
    loop {
        if let Some(a) = cur.as_and_node() {
            let op = a.operator_loc();
            if op.end_offset() - op.start_offset() == 3 {
                cur = a.right();
                continue;
            }
        }
        if let Some(o) = cur.as_or_node() {
            let op = o.operator_loc();
            if op.end_offset() - op.start_offset() == 2 {
                cur = o.right();
                continue;
            }
        }
        return cur;
    }
}

fn sibling_is_guard_if(sib: &Node<'_>) -> bool {
    let branch = if let Some(i) = sib.as_if_node() {
        i.statements().and_then(|s| s.body().iter().next())
    } else if let Some(u) = sib.as_unless_node() {
        u.statements().and_then(|s| s.body().iter().next())
    } else {
        return false;
    };
    let Some(branch) = branch else { return false };
    is_guard_clause(&branch) || matches_guard_shape(&peel_operator_keyword(&branch))
}

// --- heredoc descent ---

struct HeredocInfo {
    body_start: usize,
    closing_start: usize,
    closing_end: usize,
}

fn last_heredoc_for_if(node: &Node<'_>, if_branch: &Node<'_>) -> Option<HeredocInfo> {
    let start = pick_start_for_heredoc_search(node, if_branch);
    find_heredoc_descendant(&start)
}

fn pick_start_for_heredoc_search<'a>(node: &Node<'a>, if_branch: &Node<'a>) -> Node<'a> {
    // `last_heredoc_argument_node`:
    if let Some(and) = if_branch.as_and_node() {
        return and.left();
    }
    let cond = if let Some(i) = node.as_if_node() {
        Some(i.predicate())
    } else {
        node.as_unless_node().map(|u| u.predicate())
    };
    if let Some(c) = cond.as_ref()
        && has_heredoc_descendant(c)
    {
        return unsafe { copy_node(c) };
    }
    // `children.last`: for a `send` it is the last argument.
    if let Some(c) = if_branch.as_call_node() {
        if let Some(args) = c.arguments() {
            let mut last = None;
            for a in args.arguments().iter() {
                last = Some(a);
            }
            if let Some(a) = last {
                return a;
            }
        }
        if let Some(recv) = c.receiver() {
            return recv;
        }
    }
    unsafe { copy_node(if_branch) }
}

fn has_heredoc_descendant(node: &Node<'_>) -> bool {
    if is_heredoc(node) {
        return true;
    }
    descend_children(node, &mut |c| has_heredoc_descendant(c))
}

fn descend_children(node: &Node<'_>, visit: &mut dyn FnMut(&Node<'_>) -> bool) -> bool {
    if let Some(c) = node.as_call_node() {
        if let Some(r) = c.receiver()
            && visit(&r)
        {
            return true;
        }
        if let Some(args) = c.arguments() {
            for a in args.arguments().iter() {
                if visit(&a) {
                    return true;
                }
            }
        }
        return false;
    }
    if let Some(a) = node.as_and_node() {
        return visit(&a.left()) || visit(&a.right());
    }
    if let Some(o) = node.as_or_node() {
        return visit(&o.left()) || visit(&o.right());
    }
    if let Some(p) = node.as_parentheses_node() {
        if let Some(b) = p.body() {
            return visit(&b);
        }
        return false;
    }
    if let Some(s) = node.as_statements_node() {
        for child in s.body().iter() {
            if visit(&child) {
                return true;
            }
        }
        return false;
    }
    if let Some(ret) = node.as_return_node() {
        if let Some(args) = ret.arguments() {
            for a in args.arguments().iter() {
                if visit(&a) {
                    return true;
                }
            }
        }
        return false;
    }
    if let Some(br) = node.as_break_node() {
        if let Some(args) = br.arguments() {
            for a in args.arguments().iter() {
                if visit(&a) {
                    return true;
                }
            }
        }
        return false;
    }
    if let Some(nx) = node.as_next_node() {
        if let Some(args) = nx.arguments() {
            for a in args.arguments().iter() {
                if visit(&a) {
                    return true;
                }
            }
        }
        return false;
    }
    false
}

fn is_heredoc(node: &Node<'_>) -> bool {
    if let Some(s) = node.as_string_node() {
        if let Some(open) = s.opening_loc() {
            return open.as_slice().starts_with(b"<<");
        }
        return false;
    }
    if let Some(s) = node.as_interpolated_string_node() {
        if let Some(open) = s.opening_loc() {
            return open.as_slice().starts_with(b"<<");
        }
        return false;
    }
    if let Some(s) = node.as_x_string_node() {
        return s.opening_loc().as_slice().starts_with(b"<<");
    }
    if let Some(s) = node.as_interpolated_x_string_node() {
        return s.opening_loc().as_slice().starts_with(b"<<");
    }
    false
}

fn find_heredoc_descendant(node: &Node<'_>) -> Option<HeredocInfo> {
    // `n = n.children.first while n.begin_type?` then heredoc check then
    // arguments/receiver recurse.
    let mut cur = unsafe { copy_node(node) };
    loop {
        if let Some(b) = cur.as_begin_node()
            && let Some(stmts) = b.statements()
            && let Some(first) = stmts.body().iter().next()
        {
            cur = first;
            continue;
        }
        if let Some(p) = cur.as_parentheses_node()
            && let Some(body) = p.body()
        {
            if let Some(stmts) = body.as_statements_node()
                && let Some(first) = stmts.body().iter().next()
            {
                cur = first;
                continue;
            }
            cur = body;
            continue;
        }
        break;
    }
    if let Some(h) = heredoc_info(&cur) {
        return Some(h);
    }
    if let Some(c) = cur.as_call_node() {
        if let Some(args) = c.arguments() {
            for a in args.arguments().iter() {
                if let Some(h) = find_heredoc_descendant(&a) {
                    return Some(h);
                }
            }
        }
        if let Some(recv) = c.receiver() {
            return find_heredoc_descendant(&recv);
        }
    }
    None
}

fn heredoc_info(node: &Node<'_>) -> Option<HeredocInfo> {
    if let Some(s) = node.as_string_node() {
        let open = s.opening_loc()?;
        if !open.as_slice().starts_with(b"<<") {
            return None;
        }
        let content = s.content_loc();
        let close = s.closing_loc()?;
        let close_end = strip_trailing_newline(close.as_slice(), close.end_offset());
        return Some(HeredocInfo {
            body_start: content.start_offset(),
            closing_start: close.start_offset(),
            closing_end: close_end,
        });
    }
    if let Some(s) = node.as_interpolated_string_node() {
        let open = s.opening_loc()?;
        if !open.as_slice().starts_with(b"<<") {
            return None;
        }
        let close = s.closing_loc()?;
        let body_start = s
            .parts()
            .iter()
            .next()
            .map(|p| p.location().start_offset())
            .unwrap_or_else(|| close.start_offset());
        let close_end = strip_trailing_newline(close.as_slice(), close.end_offset());
        return Some(HeredocInfo {
            body_start,
            closing_start: close.start_offset(),
            closing_end: close_end,
        });
    }
    if let Some(s) = node.as_x_string_node() {
        let open = s.opening_loc();
        if !open.as_slice().starts_with(b"<<") {
            return None;
        }
        let content = s.content_loc();
        let close = s.closing_loc();
        let close_end = strip_trailing_newline(close.as_slice(), close.end_offset());
        return Some(HeredocInfo {
            body_start: content.start_offset(),
            closing_start: close.start_offset(),
            closing_end: close_end,
        });
    }
    if let Some(s) = node.as_interpolated_x_string_node() {
        let open = s.opening_loc();
        if !open.as_slice().starts_with(b"<<") {
            return None;
        }
        let close = s.closing_loc();
        let body_start = s
            .parts()
            .iter()
            .next()
            .map(|p| p.location().start_offset())
            .unwrap_or_else(|| close.start_offset());
        let close_end = strip_trailing_newline(close.as_slice(), close.end_offset());
        return Some(HeredocInfo {
            body_start,
            closing_start: close.start_offset(),
            closing_end: close_end,
        });
    }
    None
}

/// Prism's heredoc `closing_loc` ends just past the trailing newline; stock
/// reports the `loc.heredoc_end` line WITHOUT the newline (it sits inside
/// `range_by_whole_lines`).  Snap the end back to exclude a trailing `\n`.
fn strip_trailing_newline(slice: &[u8], end: usize) -> usize {
    if slice.last() == Some(&b'\n') {
        end - 1
    } else {
        end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<GuardClauseCandidate> {
        check_empty_line_after_guard_clause(src.as_bytes())
    }

    // Typical: a guard clause followed directly by another statement.
    #[test]
    fn typical_guard_then_statement() {
        let src = "def foo\n  return if x\n  bar\nend\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        // Offense is the whole modifier-if (`return if x`).
        let c = &got[0];
        assert_eq!(c.offense_start, 10);
        assert_eq!(c.offense_end, 21);
        // Anchor is `range_by_whole_lines(node.source_range)` = `  return if x`.
        assert_eq!(c.ac_anchor_first_line_start, 8);
        assert_eq!(c.ac_anchor_last_line, 2);
    }

    // No follow-up: no offense.
    #[test]
    fn guard_followed_by_nothing() {
        let src = "def foo\n  return if x\nend\n";
        assert!(run(src).is_empty());
    }

    // Multi-line if with guard branch + non-guard follow-up.
    #[test]
    fn multiline_if_guard_then_other() {
        let src = "def foo\n  if cond\n    return\n  end\n  bar\nend\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        // Offense is the `end` keyword.
        assert_eq!(got[0].offense_end - got[0].offense_start, 3);
    }

    // Guard sole stmt in if then-branch: no offense.
    #[test]
    fn sole_in_if_then_branch() {
        let src = "def foo\n  if cond\n    return if x\n  end\nend\n";
        assert!(run(src).is_empty());
    }

    // Top-level guard with sibling.
    #[test]
    fn top_level_guard_with_sibling() {
        let src = "return if x\nfoo\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
    }

    // Top-level alone: no offense.
    #[test]
    fn top_level_alone() {
        let src = "return if x\n";
        assert!(run(src).is_empty());
    }

    // Right sibling is itself a guard-if.
    #[test]
    fn multiple_guards_in_a_row() {
        let src = "def foo\n  return if a\n  return if b\n  foobar\nend\n";
        let got = run(src);
        // Only the LAST guard is flagged.
        assert_eq!(got.len(), 1);
    }

    // Heredoc routes the offense to the closer line.
    #[test]
    fn heredoc_argument() {
        let src = "def foo\n  raise <<~MSG unless guard\n    body\n  MSG\n  bar\nend\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        // Offense range is the heredoc closer `  MSG` (2 spaces + MSG = 5 bytes).
        let c = &got[0];
        let off = &src[c.offense_start..c.offense_end];
        assert_eq!(off, "  MSG");
    }

    // `and return` after a heredoc-bearing call: also flagged via heredoc.
    #[test]
    fn heredoc_with_and_return() {
        let src = "def foo\n  puts(<<~MSG) and return if bar\n    body\n  MSG\n  baz\nend\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
    }

    // Sole stmt in resbody: no offense.
    #[test]
    fn sole_in_resbody() {
        let src = "def foo\n  bar\nrescue\n  return if x\nend\n";
        assert!(run(src).is_empty());
    }

    // Multi-stmt resbody: offense.
    #[test]
    fn multi_stmt_resbody() {
        let src = "def foo\n  bar\nrescue\n  return if x\n  baz\nend\n";
        assert_eq!(run(src).len(), 1);
    }

    // Sole stmt in ensure body: no offense.
    #[test]
    fn sole_in_ensure() {
        let src = "def foo\n  baz\nensure\n  return if x\nend\n";
        assert!(run(src).is_empty());
    }

    // Modifier `next` in a block.
    #[test]
    fn next_in_block() {
        let src = "items.each do |i|\n  next if i.bad?\n  bar\nend\n";
        assert_eq!(run(src).len(), 1);
    }

    // Multi-statement-on-line via semicolon: no offense.
    #[test]
    fn multi_statements_on_line() {
        let src = "def foo(item)\n  return unless item.positive?; item * 2\nend\n";
        assert!(run(src).is_empty());
    }

    // Semicolon ended but newline: STILL an offense.
    #[test]
    fn semi_then_newline() {
        let src = "def foo(item)\n  return unless item.positive?;\n  item * 2\nend\n";
        assert_eq!(run(src).len(), 1);
    }

    // Inside if's else branch with sibling.
    #[test]
    fn guard_in_else_with_sibling() {
        let src = "def foo\n  if cond\n    bar\n  else\n    return if x\n    baz\n  end\nend\n";
        assert_eq!(run(src).len(), 1);
    }

    // Inside if's else branch sole: no offense.
    #[test]
    fn guard_in_else_alone() {
        let src = "def foo\n  if cond\n    bar\n  else\n    return if x\n  end\nend\n";
        assert!(run(src).is_empty());
    }

    // Single-line if then return end: no offense (multiline if has guard
    // branch but the next call chains on it, complicating the parser shape).
    #[test]
    fn single_line_if_with_chain() {
        let src = "if cond then return end.then { 42 }\n";
        assert!(run(src).is_empty());
    }
}
