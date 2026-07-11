//! `Lint/UnreachableCode`: flags expressions in a multi-statement sequence that
//! sit *after* an expression whose control flow always exits the surrounding
//! frame (`return`/`next`/`break`/`retry`/`redo` keywords, or
//! `raise`/`fail`/`throw`/`exit`/`exit!`/`abort` Kernel-style sends).
//!
//! Mirrors `vendor/rubocop/lib/rubocop/cop/lint/unreachable_code.rb`. Detection
//! only — no autocorrect — matching stock.
//!
//! ## Mapping parser `:begin` / `:kwbegin` to prism
//!
//! Stock's `on_begin` / `on_kwbegin` hook runs on every multi-statement
//! sequence. In parser-gem terms:
//!
//! - `:begin` is the synthesized "multi-statement parent" node — a body of two
//!   or more expressions in any parent position (file top-level, def body,
//!   block body, `if`/`else` branch, parenthesised group, ...). Single-statement
//!   positions do not introduce a `:begin` wrapper.
//! - `:kwbegin` is an explicit `begin ... end` keyword block (with optional
//!   `rescue` / `else` / `ensure` clauses chained onto it).
//!
//! In prism:
//!
//! - The parser `:begin` shape corresponds to a `StatementsNode` whose
//!   `body()` has 2+ elements. Single-statement `StatementsNode`s have
//!   no parser counterpart and we skip them.
//! - The parser `:kwbegin` shape corresponds to a `BeginNode`. Its
//!   `statements()` field is the "top section" (everything before any
//!   `rescue`/`else`/`ensure` clause), which is what stock's
//!   `expressions = *node` returns for `:kwbegin`.
//!
//! We process both. To avoid double-firing on the inner `StatementsNode` of a
//! `BeginNode` (which the shared walk will also visit), we remember each
//! `BeginNode.statements()` byte range we already processed and skip the
//! matching `StatementsNode`.
//!
//! ## `flow_expression?` recursion
//!
//! Stock's `flow_expression?` is recursive: a `:begin`/`:kwbegin` is flow if
//! ANY of its expressions is flow; an `:if` is flow if both arms are flow; a
//! `:case`/`:case_match` is flow if its else branch and every when/in branch
//! are flow; a `:def`/`:defs` is never flow (but is registered for redefinition
//! tracking before returning false). We reproduce this exactly.
//!
//! ## `instance_eval` and `@redefined` tracking
//!
//! Stock keeps two pieces of state across the walk:
//!
//! - `@instance_eval_count`: incremented on entering an `instance_eval` block
//!   and decremented on leaving it. Inside `instance_eval` a bare-receiver
//!   call to a redefinable flow method is *not* reported (the receiver could
//!   be anything; stock avoids false positives).
//! - `@redefined`: every redefinable flow-method name (`raise`/`fail`/...)
//!   that appears as a `def`/`defs` reached by `flow_expression?` recursion.
//!   A subsequent bare-receiver call to one of those names is treated as a
//!   regular method call (no flow), unless it goes through `Kernel.` (explicit
//!   receiver always reports).
//!
//! We mirror both. `instance_eval` is detected at the `CallNode` whose name is
//! `instance_eval` and whose `block()` is present: we mark the block's byte
//! range and bump the counter on the matching `BlockNode` enter. The
//! decrement happens on the corresponding `leave` via a frame stack (the
//! shared-walk `Rule::leave` takes no node, so we record per-frame what to
//! undo).

use std::collections::HashSet;

use ruby_prism::{
    BeginNode, BlockNode, CallNode, CaseMatchNode, CaseNode, ElseNode, Node, StatementsNode, Visit,
};

/// One offense candidate. `[start_offset, end_offset)` is the byte range of the
/// unreachable expression — exactly stock's `add_offense(expression2)` range.
#[derive(Debug, Clone)]
pub struct UnreachableCodeOffense {
    pub start_offset: usize,
    pub end_offset: usize,
}

/// Standalone entry point used by the per-cop fallback. This cop is always
/// `bundle_eligible?` (no per-investigation state), so this path is exercised
/// by tests only.
pub fn check_unreachable_code(source: &[u8]) -> Vec<UnreachableCodeOffense> {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
///
/// `Lint/UnreachableCode` is config-less and source-independent (every offset
/// comes from the AST locations), so the builder takes no arguments.
pub(crate) fn build_rule() -> UnreachableCodeVisitor {
    UnreachableCodeVisitor {
        instance_eval_depth: 0,
        pending_instance_eval_blocks: HashSet::new(),
        processed_statements: HashSet::new(),
        redefined: HashSet::new(),
        frames: Vec::new(),
        offenses: Vec::new(),
    }
}

/// What was pushed on `enter` so `leave` knows what to undo. The shared-walk
/// `Rule::leave` takes no node argument so we cannot derive the kind from
/// there; the frame remembers exactly what bookkeeping the matching `enter`
/// performed.
#[derive(Debug)]
enum Frame {
    /// The matching `enter` bumped `instance_eval_depth`.
    InstanceEvalBlock,
    /// The matching `enter` did nothing requiring cleanup. We still push so
    /// the stack stays in lockstep with branch enter/leave (every `enter`
    /// pairs with one `leave`).
    Other,
}

pub(crate) struct UnreachableCodeVisitor {
    instance_eval_depth: usize,
    /// Byte ranges of `BlockNode` children that belong to an `instance_eval`
    /// call. Populated when we visit the parent `CallNode` (whose name is
    /// `instance_eval` and whose `block()` is a `BlockNode`) and consumed when
    /// we enter the matching block node. In prism every `do ... end` /
    /// `{ ... }` literal — plain, numblock, or `it`-block — is a `BlockNode`,
    /// distinguished by the inner `parameters` field; stock treats all three
    /// shapes as `any_block_type?` so we just look at `BlockNode`. `LambdaNode`
    /// is excluded by construction: `obj.instance_eval -> { ... }` parses as
    /// `(send obj :instance_eval (lambda ...))` (the lambda is the *argument*,
    /// not a block on the send), so stock's `any_block_type?` is false there
    /// too.
    pending_instance_eval_blocks: HashSet<(usize, usize)>,
    /// Byte ranges of `StatementsNode` children that a `BeginNode` already
    /// processed in `enter`. The shared walk visits the inner `StatementsNode`
    /// after the `BeginNode`; we use this set to skip it and avoid double-
    /// firing.
    processed_statements: HashSet<(usize, usize)>,
    /// Redefinable flow-method names registered by a `def`/`defs` reached
    /// through `flow_expression?` recursion (e.g. `def raise; end` sitting
    /// inside the same multi-statement sequence before a bare `raise` call).
    /// Once registered, a bare-receiver call to that name is treated as a
    /// regular call (no flow) until end of file. Stock keeps `@redefined`
    /// across the whole investigation — we do the same.
    redefined: HashSet<Vec<u8>>,
    frames: Vec<Frame>,
    pub(crate) offenses: Vec<UnreachableCodeOffense>,
}

impl UnreachableCodeVisitor {
    fn push_offense(&mut self, node: &Node<'_>) {
        let loc = node.location();
        self.offenses.push(UnreachableCodeOffense {
            start_offset: loc.start_offset(),
            end_offset: loc.end_offset(),
        });
    }

    /// Process a parser `:begin`-equivalent expression list. rubocop#15418:
    /// once a flow-of-control statement is reached, EVERY following statement in
    /// the block is unreachable (not just the one immediately after it). The
    /// flow statement itself must not be the last expression. `flow_expression?`
    /// (which registers `def` redefinitions) is only consulted while searching
    /// for the first flow statement, exactly like stock.
    fn process_sequence(&mut self, exprs: &[Node<'_>]) {
        if exprs.len() < 2 {
            return;
        }
        let n = exprs.len();
        let mut flow_reached = false;
        for (index, expr) in exprs.iter().enumerate() {
            if flow_reached {
                self.push_offense(expr);
            } else if index < n - 1 && self.flow_expression(expr) {
                flow_reached = true;
            }
        }
    }

    fn process_statements_node(&mut self, stmts: &StatementsNode<'_>) {
        let exprs: Vec<Node<'_>> = stmts.body().iter().collect();
        self.process_sequence(&exprs);
    }

    fn process_begin_node(&mut self, begin: &BeginNode<'_>) {
        // Stock's `on_kwbegin` body: `expressions = *node`. parser-gem
        // `:kwbegin` children are exactly the "top section" before
        // rescue/else/ensure, which is what `BeginNode.statements()` gives us.
        // Rescue/else/ensure clauses are processed separately by the shared
        // walk (and stock doesn't include them in `expressions` either).
        let Some(stmts) = begin.statements() else { return };
        let r = stmts_range(&stmts);
        self.processed_statements.insert(r);
        self.process_statements_node(&stmts);
    }

    /// Stock's `flow_expression?(node)`. Returns true if `node` always exits
    /// the surrounding frame. `def`/`defs` is the only kind with a side
    /// effect (`register_redefinition`).
    fn flow_expression(&mut self, node: &Node<'_>) -> bool {
        // `flow_command?` (keyword or redefinable Kernel-ish send).
        if let Some(decision) = self.classify_flow_command(node) {
            return decision;
        }

        // `:begin` / `:kwbegin` -> recurse on every expression.
        if let Some(begin) = node.as_begin_node() {
            // In parser-gem, `begin...rescue...end` is a `:kwbegin` whose
            // children (via `*node`) are `[(rescue ...)]`. `flow_expression?`
            // on a `:rescue` node falls to `else => false`, so the whole
            // `begin/rescue/end` is NOT considered flow — rescue/ensure can
            // alter control flow (e.g. `retry` loops back, a rescue handler
            // may not exit). We match that: if the `BeginNode` has rescue or
            // ensure clauses, return false unconditionally.
            if begin.rescue_clause().is_some() || begin.ensure_clause().is_some() {
                return false;
            }
            // Plain `begin...end` (no rescue/ensure): the top statements
            // section participates in `expressions = *node`.
            if let Some(stmts) = begin.statements() {
                return stmts.body().iter().any(|e| self.flow_expression(&e));
            }
            return false;
        }
        // parser :begin (synthesized multi-statement parent) — in prism the
        // shape that appears as a child is `ParenthesesNode` whose body is a
        // multi-statement `StatementsNode`. A `StatementsNode` is otherwise
        // owned by structural parents (def body, block body, if branch, ...)
        // and is not a `flow_expression?` argument because parser hands a
        // bare child (the single statement) or wraps multiple into `:begin`
        // at the *parent* hook (already processed by `process_sequence`).
        if let Some(parens) = node.as_parentheses_node() {
            if let Some(body) = parens.body() {
                if let Some(stmts) = body.as_statements_node() {
                    return stmts.body().iter().any(|e| self.flow_expression(&e));
                }
                // Single non-StatementsNode body inside parens: recurse.
                return self.flow_expression(&body);
            }
            return false;
        }

        // `:if` / `:unless` -> check_if.
        if let Some(if_node) = node.as_if_node() {
            return self.check_if_like(
                if_node.statements().map(|s| s.as_node()),
                if_node.subsequent(),
            );
        }
        if let Some(unless_node) = node.as_unless_node() {
            return self.check_if_like(
                unless_node.statements().map(|s| s.as_node()),
                unless_node.else_clause().map(|e| e.as_node()),
            );
        }

        // `:case` / `:case_match` -> check_case.
        if let Some(case_node) = node.as_case_node() {
            return self.check_case(&case_node);
        }
        if let Some(case_match) = node.as_case_match_node() {
            return self.check_case_match(&case_match);
        }

        // `:def` / `:defs` -> register and return false.
        if let Some(def) = node.as_def_node() {
            self.register_redefinition(def.name().as_slice());
            return false;
        }

        false
    }

    /// `Some(true)` if `node` is a flow command we report on, `Some(false)`
    /// if `node` is a flow command we silence (redefined / inside
    /// `instance_eval`), `None` if `node` is not a flow command at all.
    fn classify_flow_command(&self, node: &Node<'_>) -> Option<bool> {
        // Keyword flow commands always report (stock's
        // `report_on_flow_command?` short-circuits on `unless node.send_type?`).
        if node.as_return_node().is_some()
            || node.as_next_node().is_some()
            || node.as_break_node().is_some()
            || node.as_retry_node().is_some()
            || node.as_redo_node().is_some()
        {
            return Some(true);
        }
        // Redefinable Kernel-shaped sends.
        if let Some(call) = node.as_call_node() {
            if !is_redefinable_flow_method_name(call.name().as_slice()) {
                return None;
            }
            if !self.matches_kernel_receiver(&call) {
                return None;
            }
            // Inside `instance_eval` a bare-receiver call could be anything;
            // stock silences it.
            if call.receiver().is_none() {
                if self.instance_eval_depth > 0 {
                    return Some(false);
                }
                if self.redefined.contains(call.name().as_slice()) {
                    return Some(false);
                }
            }
            // Otherwise (explicit Kernel receiver or unredefined bare call)
            // it's a flow command we report on.
            return Some(true);
        }
        None
    }

    /// `{nil? (const {nil? cbase} :Kernel)}` — bare receiver or `Kernel`/`::Kernel`.
    fn matches_kernel_receiver(&self, call: &CallNode<'_>) -> bool {
        match call.receiver() {
            None => true,
            Some(recv) => {
                if let Some(c) = recv.as_constant_read_node() {
                    return c.name().as_slice() == b"Kernel";
                }
                if let Some(p) = recv.as_constant_path_node() {
                    return p.parent().is_none()
                        && p.name().is_some_and(|n| n.as_slice() == b"Kernel");
                }
                false
            }
        }
    }

    /// `check_if(node)` — if_branch and else_branch both flow.
    fn check_if_like<'pr>(
        &mut self,
        if_branch: Option<Node<'pr>>,
        else_subsequent: Option<Node<'pr>>,
    ) -> bool {
        let Some(if_branch) = if_branch else { return false };
        let Some(else_branch) = else_subsequent else { return false };
        if !self.flow_branch_node(&if_branch) {
            return false;
        }
        // `else_branch` in parser-gem is the else body itself (a bare node or a
        // synthesized `:begin`). On prism: an `ElseNode` (its `statements` is
        // the body), or an inner `IfNode` for the elsif chain.
        if let Some(else_node) = else_branch.as_else_node() {
            self.flow_else_node(&else_node)
        } else {
            // elsif IfNode (or any other subsequent shape) — recurse as a
            // single expression.
            self.flow_expression(&else_branch)
        }
    }

    /// `flow_expression?` for an `if`/`unless` body's `StatementsNode`. parser
    /// gives stock a bare node (single statement) or a synthesized `:begin`
    /// (multi-statement); the latter recurses with `any?`. prism gives us a
    /// `StatementsNode` either way: 0 statements = empty body (false), 1 =
    /// bare expression, 2+ = parser `:begin`.
    fn flow_statements(&mut self, stmts: &StatementsNode<'_>) -> bool {
        let exprs: Vec<Node<'_>> = stmts.body().iter().collect();
        match exprs.len() {
            0 => false,
            1 => self.flow_expression(&exprs[0]),
            _ => exprs.iter().any(|e| self.flow_expression(e)),
        }
    }

    fn flow_branch_node(&mut self, branch: &Node<'_>) -> bool {
        if let Some(stmts) = branch.as_statements_node() {
            return self.flow_statements(&stmts);
        }
        self.flow_expression(branch)
    }

    fn flow_else_node(&mut self, else_node: &ElseNode<'_>) -> bool {
        match else_node.statements() {
            None => false,
            Some(stmts) => self.flow_statements(&stmts),
        }
    }

    fn check_case(&mut self, case_node: &CaseNode<'_>) -> bool {
        // `else_branch = node.else_branch; return false unless else_branch;
        //  return false unless flow_expression?(else_branch)`
        let Some(else_clause) = case_node.else_clause() else { return false };
        if !self.flow_else_node(&else_clause) {
            return false;
        }
        // `node.when_branches.all? { |b| b.body && flow_expression?(b.body) }`
        // A when branch without a body fails the `b.body &&` guard, so the
        // whole case is not flow.
        for when_node in case_node.conditions().iter() {
            let Some(when_n) = when_node.as_when_node() else { return false };
            let Some(stmts) = when_n.statements() else { return false };
            if !self.flow_statements(&stmts) {
                return false;
            }
        }
        true
    }

    fn check_case_match(&mut self, case_match: &CaseMatchNode<'_>) -> bool {
        let Some(else_clause) = case_match.else_clause() else { return false };
        if !self.flow_else_node(&else_clause) {
            return false;
        }
        for in_node in case_match.conditions().iter() {
            let Some(in_n) = in_node.as_in_node() else { return false };
            let Some(stmts) = in_n.statements() else { return false };
            if !self.flow_statements(&stmts) {
                return false;
            }
        }
        true
    }

    fn register_redefinition(&mut self, method_name: &[u8]) {
        if is_redefinable_flow_method_name(method_name) {
            self.redefined.insert(method_name.to_vec());
        }
    }

    // --- instance_eval bookkeeping ---

    /// Called when we see a `CallNode` — if its name is `instance_eval` and it
    /// has a block child, record the block's byte range. The matching block
    /// node enter will bump the counter.
    fn mark_instance_eval_block(&mut self, call: &CallNode<'_>) {
        if call.name().as_slice() != b"instance_eval" {
            return;
        }
        let Some(block) = call.block() else { return };
        // `instance_eval_block?(node)` in stock: `node.any_block_type? &&
        // node.method?(:instance_eval)`. parser-gem `any_block_type?` covers
        // `:block`/`:numblock`/`:itblock`. In prism every block literal is a
        // `BlockNode`; numblock vs `it`-block vs plain are distinguished by
        // the inner `parameters` field. `BlockArgumentNode` (`&blk`) is not
        // a block literal — stock's `any_block_type?` returns false for it
        // — so we filter for `BlockNode` only (the `CallNode.block` field is
        // typed as `Option<Node>` precisely to allow either).
        if block.as_block_node().is_none() {
            return;
        }
        let r = node_range(&block);
        self.pending_instance_eval_blocks.insert(r);
    }

    fn is_pending_instance_eval(&mut self, node: &Node<'_>) -> bool {
        let r = node_range(node);
        self.pending_instance_eval_blocks.remove(&r)
    }
}

fn node_range(node: &Node<'_>) -> (usize, usize) {
    let loc = node.location();
    (loc.start_offset(), loc.end_offset())
}

fn stmts_range(stmts: &StatementsNode<'_>) -> (usize, usize) {
    let loc = stmts.location();
    (loc.start_offset(), loc.end_offset())
}

fn is_redefinable_flow_method_name(name: &[u8]) -> bool {
    matches!(
        name,
        b"raise" | b"fail" | b"throw" | b"exit" | b"exit!" | b"abort"
    )
}

// ---------------- Visit (standalone) ----------------

impl<'pr> Visit<'pr> for UnreachableCodeVisitor {
    fn visit_statements_node(&mut self, node: &StatementsNode<'pr>) {
        let r = stmts_range(node);
        if !self.processed_statements.contains(&r) {
            self.process_statements_node(node);
        }
        ruby_prism::visit_statements_node(self, node);
    }

    fn visit_begin_node(&mut self, node: &BeginNode<'pr>) {
        self.process_begin_node(node);
        ruby_prism::visit_begin_node(self, node);
    }

    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        self.mark_instance_eval_block(node);
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_block_node(&mut self, node: &BlockNode<'pr>) {
        let bumped = self.is_pending_instance_eval(&node.as_node());
        if bumped {
            self.instance_eval_depth += 1;
        }
        ruby_prism::visit_block_node(self, node);
        if bumped {
            self.instance_eval_depth -= 1;
        }
    }
}

// ---------------- Rule (shared walk) ----------------

impl super::dispatch::Rule for UnreachableCodeVisitor {
    fn enter(&mut self, node: &Node<'_>) {
        // Record `instance_eval` parent calls before we see their block child
        // (the shared walk enters parents before children).
        if let Some(call) = node.as_call_node() {
            self.mark_instance_eval_block(&call);
        }

        // Bump `instance_eval_depth` if this node is the block child of a
        // previously-recorded `instance_eval` call.
        let mut frame = Frame::Other;
        if node.as_block_node().is_some() && self.is_pending_instance_eval(node) {
            self.instance_eval_depth += 1;
            frame = Frame::InstanceEvalBlock;
        }
        self.frames.push(frame);

        // Process parser `:kwbegin` sequence at the `BeginNode` and mark its
        // child `StatementsNode` so the shared walk doesn't double-process.
        // For the top-level `ProgramNode` the generic walk skips its enter
        // (the generated `visit_program_node` jumps straight to
        // `visit_statements_node` without going through `visit()`, so
        // `visit_branch_node_enter` is never called on the top
        // `StatementsNode`). We handle it explicitly here: the ProgramNode's
        // own enter does fire (it IS visited via `visit()`), and from there
        // we process its statements and mark them to suppress any later
        // duplicate via `StatementsNode` enter (in practice the latter
        // doesn't fire for the top statements, but marking is cheap and
        // self-documenting).
        if let Some(program) = node.as_program_node() {
            let stmts = program.statements();
            let r = stmts_range(&stmts);
            self.processed_statements.insert(r);
            self.process_statements_node(&stmts);
        } else if let Some(begin) = node.as_begin_node() {
            self.process_begin_node(&begin);
        } else if let Some(stmts) = node.as_statements_node() {
            let r = stmts_range(&stmts);
            if !self.processed_statements.contains(&r) {
                self.process_statements_node(&stmts);
            }
        }
    }

    fn leave(&mut self) {
        if let Some(Frame::InstanceEvalBlock) = self.frames.pop() {
            self.instance_eval_depth -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(src: &str) -> Vec<(usize, usize)> {
        check_unreachable_code(src.as_bytes())
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset))
            .collect()
    }

    #[test]
    fn flags_return_followed_by_statement() {
        // `return\nbar` — `bar` is unreachable.
        let off = detect("def f\n  return\n  bar\nend\n");
        assert_eq!(off.len(), 1);
        let (s, e) = off[0];
        assert_eq!(&"def f\n  return\n  bar\nend\n"[s..e], "bar");
    }

    #[test]
    fn flags_raise_followed_by_statement() {
        let off = detect("def f\n  raise\n  bar\nend\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_every_statement_after_flow() {
        // rubocop#15418: `a`, `b`, and `c` are all unreachable after `return`.
        let src = "def f\n  return\n  a\n  b\n  c\nend\n";
        let off = detect(src);
        assert_eq!(off.len(), 3);
        assert_eq!(&src[off[0].0..off[0].1], "a");
        assert_eq!(&src[off[1].0..off[1].1], "b");
        assert_eq!(&src[off[2].0..off[2].1], "c");
    }

    #[test]
    fn flags_inside_kwbegin() {
        let off = detect("def f\n  begin\n    raise\n    bar\n  end\nend\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_when_both_if_branches_flow() {
        let src = "def f\n  if cond\n    return\n  else\n    return\n  end\n  bar\nend\n";
        let off = detect(src);
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn accepts_when_if_branch_only_flow() {
        let src = "def f\n  if cond\n    return\n  end\n  bar\nend\n";
        assert!(detect(src).is_empty());
    }

    #[test]
    fn flags_when_all_elsif_branches_flow() {
        let src = "def f\n  if a\n    return\n  elsif b\n    return\n  else\n    return\n  end\n  bar\nend\n";
        let off = detect(src);
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn accepts_modifier_form() {
        let src = "def f\n  return if cond\n  bar\nend\n";
        assert!(detect(src).is_empty());
    }

    #[test]
    fn flags_case_all_branches() {
        let src = "def f\n  case cond\n  when 1\n    return\n  when 2\n    return\n  else\n    return\n  end\n  bar\nend\n";
        let off = detect(src);
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn accepts_case_without_else() {
        let src = "def f\n  case cond\n  when 1\n    return\n  when 2\n    return\n  end\n  bar\nend\n";
        assert!(detect(src).is_empty());
    }

    #[test]
    fn flags_case_match_all_branches() {
        let src = "def f\n  case cond\n  in 1\n    return\n  in 2\n    return\n  else\n    return\n  end\n  bar\nend\n";
        let off = detect(src);
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn redefined_raise_silences_offense() {
        // `def raise; end` registers redefinition. Subsequent bare `raise`
        // call is no longer a flow command -> no offense on `bar`.
        let src = "def f\n  def raise; end\n  raise\n  bar\nend\n";
        assert!(detect(src).is_empty());
    }

    #[test]
    fn kernel_raise_reports_even_when_redefined() {
        // Explicit `Kernel.raise` always reports.
        let src = "def raise; end\n\nKernel.raise\nfoo\n";
        let off = detect(src);
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn instance_eval_silences_bare_redefinable() {
        // Inside `instance_eval` a bare `raise` does NOT report.
        let src = "d.instance_eval do\n  raise\n  bar\nend\n";
        assert!(detect(src).is_empty());
    }

    #[test]
    fn instance_eval_kernel_still_reports() {
        let src = "d.instance_eval do\n  Kernel.raise\n  bar\nend\n";
        let off = detect(src);
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_next_in_block() {
        let src = "list.each do |x|\n  next\n  bar\nend\n";
        let off = detect(src);
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_via_dispatch_run_same_as_standalone() {
        // Same source as the vendor `throw after instance_eval` case. Drive it
        // through the shared-walk dispatch (matching how the bundle exercises
        // the Rule path) and assert identical results to the standalone visit.
        let src = "class Dummy\n  def throw; end\nend\n\nd = Dummy.new\nd.instance_eval do\n  throw\n  bar\nend\n\nthrow\nbar\n";
        let mut rule = build_rule();
        crate::rules::dispatch::run(src.as_bytes(), &mut [&mut rule]);
        let off: Vec<_> = rule.offenses.iter().map(|o| (o.start_offset, o.end_offset)).collect();
        assert_eq!(off.len(), 1, "bundle path expected one offense, got {:?}", off);
    }


    #[test]
    fn flags_outer_throw_after_class_with_redef_def() {
        // Mirrors the vendor spec "registers an offense for `throw` after `instance_eval`"
        // — `def throw` is INSIDE a class body so it does NOT register at top
        // level, and the outer bare `throw` must remain a flow command.
        let src = "class Dummy\n  def throw; end\nend\n\nd = Dummy.new\nd.instance_eval do\n  throw\n  bar\nend\n\nthrow\nbar\n";
        let off = detect(src);
        assert_eq!(off.len(), 1, "expected one offense, got {:?}", off);
    }

    #[test]
    fn flags_break_in_block() {
        let src = "list.each do |x|\n  break\n  bar\nend\n";
        let off = detect(src);
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn accepts_begin_rescue_with_retry() {
        // `begin; return x; rescue; retry; end; return true` — the rescue
        // clause may loop back via `retry`, so the `begin/rescue/end` does
        // NOT unconditionally exit. `return true` is reachable. Stock treats
        // the `:kwbegin`'s child `:rescue` as non-flow (`else => false` in
        // `flow_expression?`).
        let src = "def f\n  begin\n    return regexp.match(string)\n  rescue ArgumentError => e\n    retry\n  end\n  return true\nend\n";
        assert!(detect(src).is_empty());
    }

    #[test]
    fn accepts_begin_ensure() {
        // `begin; return x; ensure; cleanup; end; bar` — ensure runs after
        // the body regardless; the whole `begin/ensure/end` is not flow.
        let src = "def f\n  begin\n    return x\n  ensure\n    cleanup\n  end\n  bar\nend\n";
        assert!(detect(src).is_empty());
    }

    #[test]
    fn flags_plain_begin_with_return() {
        // `begin; return x; end; bar` — plain begin without rescue/ensure
        // is flow if any statement is flow.
        let src = "def f\n  begin\n    return x\n  end\n  bar\nend\n";
        let off = detect(src);
        assert_eq!(off.len(), 1);
    }
}
