//! `Lint/Void`.

use std::collections::HashSet;
use std::mem::discriminant;

use ruby_prism::{CallNode, Node, StatementsNode};

/// A void-context offense. The correction is up to one `replace` plus one
/// `remove` (a range is active when `end > start`), mirroring stock's
/// per-category corrector calls:
///
/// - operator with arguments: remove the call operator (if any) and replace
///   the space-extended selector with `"\n"`;
/// - operator without arguments (unary): replace the whole send with the
///   receiver source;
/// - variable / constant / literal / `self` / `defined?` / lambda-or-proc:
///   remove the expression extended left over spaces and newlines (suppressed
///   inside conditional branches and assignment-method defs);
/// - nonmutating method (`CheckForMethodsWithNoSideEffects`): replace the
///   selector with the bang/`each` suggestion.
pub struct VoidOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: String,
    pub replace_start: usize,
    pub replace_end: usize,
    pub replacement: String,
    pub remove_start: usize,
    pub remove_end: usize,
}

const BINARY_OPERATORS: &[&[u8]] = &[
    b"*", b"/", b"%", b"+", b"-", b"==", b"===", b"!=", b"<", b">", b"<=", b">=", b"<=>",
];
const UNARY_OPERATORS: &[&[u8]] = &[b"+@", b"-@", b"~", b"!"];

const NONMUTATING_METHODS_WITH_BANG_VERSION: &[&[u8]] = &[
    b"capitalize",
    b"chomp",
    b"chop",
    b"compact",
    b"delete_prefix",
    b"delete_suffix",
    b"downcase",
    b"encode",
    b"flatten",
    b"gsub",
    b"lstrip",
    b"merge",
    b"next",
    b"reject",
    b"reverse",
    b"rotate",
    b"rstrip",
    b"scrub",
    b"select",
    b"shuffle",
    b"slice",
    b"sort",
    b"sort_by",
    b"squeeze",
    b"strip",
    b"sub",
    b"succ",
    b"swapcase",
    b"tr",
    b"tr_s",
    b"transform_values",
    b"unicode_normalize",
    b"uniq",
    b"upcase",
];
const METHODS_REPLACEABLE_BY_EACH: &[&[u8]] = &[b"collect", b"map"];

pub fn check_void(source: &[u8], check_nonmutating: bool) -> Vec<VoidOffense> {
    let mut rule = build_rule(source, check_nonmutating);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule(source: &[u8], check_nonmutating: bool) -> Visitor<'_> {
    Visitor {
        source,
        check_nonmutating,
        frames: Vec::new(),
        pending_void: HashSet::new(),
        seen_ranges: HashSet::new(),
        offenses: Vec::new(),
    }
}

/// One ancestor frame per `enter` (popped on `leave`), tracking the two
/// ancestor lookups stock performs: `each_ancestor(:any_block).first` (is the
/// nearest block an `each`?) and `each_ancestor(:any_def).first` (is the
/// nearest def an assignment method?). Both searches pass through every other
/// node kind, including defs under blocks and vice versa.
enum Frame {
    Block { is_each: bool },
    Def { asgn: bool },
    Plain,
}

/// What stock does with a single-statement (non-`begin`) body in this
/// position.
enum SingleAction {
    /// `for` bodies, lambda bodies, multi-statement-only positions: nothing.
    Nothing,
    /// `on_ensure`: `check_expression` only (note: no void-operator check).
    CheckExpression,
    /// `on_block`: void-op + expression checks, but only for `tap`-like
    /// blocks (`in_void_context?` requires `each`/`tap`, then `each` returns).
    OnBlock { tap_like: bool },
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    check_nonmutating: bool,
    frames: Vec<Frame>,
    /// Parser `begin`-equivalents (parentheses / keyword `begin`) sitting as
    /// the single statement of a void body position (`initialize`/setter def
    /// body, `each`/`tap` block body, `ensure` branch, `for` body). Their
    /// parser parent is the def/block/ensure/for itself, so `in_void_context?`
    /// is true; everywhere else it is false (no other parent kind defines
    /// `void_context?`). Keyed by byte range, recorded by the parent's hook
    /// before the child's own hook fires.
    pending_void: HashSet<(usize, usize)>,
    /// `Base#add_offense` drops duplicate ranges (first wins); nested
    /// parentheses can re-derive the same operator offense.
    seen_ranges: HashSet<(usize, usize)>,
    pub(crate) offenses: Vec<VoidOffense>,
}

fn node_range(node: &Node<'_>) -> (usize, usize) {
    let loc = node.location();
    (loc.start_offset(), loc.end_offset())
}

fn is_paren_or_kwbegin(node: &Node<'_>) -> bool {
    node.as_parentheses_node().is_some() || node.as_begin_node().is_some()
}

/// `Node#literal?`: the parser `LITERALS` type list restated over Prism node
/// kinds. `__FILE__` / `__LINE__` translate to plain str/int literals.
fn is_literal(node: &Node<'_>) -> bool {
    node.as_true_node().is_some()
        || node.as_false_node().is_some()
        || node.as_nil_node().is_some()
        || node.as_integer_node().is_some()
        || node.as_float_node().is_some()
        || node.as_rational_node().is_some()
        || node.as_imaginary_node().is_some()
        || node.as_string_node().is_some()
        || node.as_interpolated_string_node().is_some()
        || node.as_x_string_node().is_some()
        || node.as_interpolated_x_string_node().is_some()
        || node.as_symbol_node().is_some()
        || node.as_interpolated_symbol_node().is_some()
        || node.as_array_node().is_some()
        || node.as_hash_node().is_some()
        || node.as_regular_expression_node().is_some()
        || node.as_interpolated_regular_expression_node().is_some()
        || node.as_range_node().is_some()
        || node.as_source_file_node().is_some()
        || node.as_source_line_node().is_some()
}

/// Whether `call` carries a literal block (a parser `block`/`numblock`/
/// `itblock` wrapper). A `BlockArgumentNode` (`&blk`) stays a plain send.
fn has_block_literal(call: &CallNode<'_>) -> bool {
    call.block().is_some_and(|b| b.as_block_node().is_some())
}

/// `node.arguments.none?` in parser terms: a `block_pass` argument is a child
/// of the parser send node, so it counts.
fn parser_args_empty(call: &CallNode<'_>) -> bool {
    call.arguments().is_none_or(|a| a.arguments().is_empty()) && call.block().is_none()
}

/// `#global_const?(:Proc)`: `Proc` or `::Proc`.
fn is_global_proc_const(node: &Node<'_>) -> bool {
    if let Some(c) = node.as_constant_read_node() {
        return c.name().as_slice() == b"Proc";
    }
    if let Some(p) = node.as_constant_path_node() {
        return p.parent().is_none() && p.name().is_some_and(|n| n.as_slice() == b"Proc");
    }
    false
}

impl<'a> Visitor<'a> {
    fn lossy(&self, start: usize, end: usize) -> String {
        String::from_utf8_lossy(&self.source[start..end]).into_owned()
    }

    fn node_source(&self, node: &Node<'_>) -> String {
        let (s, e) = node_range(node);
        self.lossy(s, e)
    }

    fn nearest_block_is_each(&self) -> bool {
        self.frames
            .iter()
            .rev()
            .find_map(|f| match f {
                Frame::Block { is_each } => Some(*is_each),
                _ => None,
            })
            .unwrap_or(false)
    }

    fn nearest_def_is_asgn(&self) -> bool {
        self.frames
            .iter()
            .rev()
            .find_map(|f| match f {
                Frame::Def { asgn } => Some(*asgn),
                _ => None,
            })
            .unwrap_or(false)
    }

    #[allow(clippy::too_many_arguments)]
    fn push_offense(
        &mut self,
        start: usize,
        end: usize,
        message: String,
        replace: Option<(usize, usize, String)>,
        remove: Option<(usize, usize)>,
    ) {
        if !self.seen_ranges.insert((start, end)) {
            return;
        }
        let (replace_start, replace_end, replacement) = replace.unwrap_or((0, 0, String::new()));
        let (remove_start, remove_end) = remove.unwrap_or((0, 0));
        self.offenses.push(VoidOffense {
            start_offset: start,
            end_offset: end,
            message,
            replace_start,
            replace_end,
            replacement,
            remove_start,
            remove_end,
        });
    }

    /// `range_with_surrounding_space(side: :left)`: extend the start left over
    /// spaces/tabs, then over newlines (two phases, exactly like `final_pos`).
    fn extend_left_over_space_and_newlines(&self, mut pos: usize) -> usize {
        while pos > 0 && matches!(self.source[pos - 1], b' ' | b'\t') {
            pos -= 1;
        }
        while pos > 0 && self.source[pos - 1] == b'\n' {
            pos -= 1;
        }
        pos
    }

    /// `range_with_surrounding_space(side: :both, newlines: false)`.
    fn extend_both_over_space(&self, mut start: usize, mut end: usize) -> (usize, usize) {
        while start > 0 && matches!(self.source[start - 1], b' ' | b'\t') {
            start -= 1;
        }
        while end < self.source.len() && matches!(self.source[end], b' ' | b'\t') {
            end += 1;
        }
        (start, end)
    }

    /// Node equality proxy for parser's structural `Node#==`: same node kind
    /// and same source text. (Structurally-equal-but-differently-spelled
    /// nodes — `foo()` vs `foo` — compare unequal here; that only narrows the
    /// degenerate `begin X ensure X end` void quirk below.)
    fn node_eq(&self, a: &Node<'_>, b: &Node<'_>) -> bool {
        let (a_s, a_e) = node_range(a);
        let (b_s, b_e) = node_range(b);
        discriminant(a) == discriminant(b) && self.source[a_s..a_e] == self.source[b_s..b_e]
    }

    // --- sequence processing (stock `check_begin`) -------------------------

    /// Process a parser `begin`-equivalent's expression list. `void` is
    /// `in_void_context?` of the begin node.
    fn seq(&mut self, exprs: &[Node<'_>], void: bool) {
        let inside_each = self.nearest_block_is_each();
        let upto = if void && !inside_each {
            exprs.len()
        } else {
            exprs.len().saturating_sub(1)
        };
        for expr in &exprs[..upto] {
            if !inside_each {
                self.check_void_op(expr);
            }
            self.check_expression(expr, false);
        }
    }

    fn seq_stmts(&mut self, stmts: &StatementsNode<'_>, void: bool) {
        let exprs: Vec<Node> = stmts.body().iter().collect();
        self.seq(&exprs, void);
    }

    /// Process a body position (def body, block body, `for` body, `ensure`
    /// branch): a multi-statement list is a parser `begin` whose parent is the
    /// position's owner; a single parenthesised/keyword-`begin` statement IS
    /// the parser child, so its own hook must inherit the position's void
    /// context via `pending_void`; any other single statement gets the
    /// position-specific callback behaviour.
    fn body_position(&mut self, stmts: &StatementsNode<'_>, void: bool, single: SingleAction) {
        let exprs: Vec<Node> = stmts.body().iter().collect();
        match exprs.len() {
            0 => {}
            1 => {
                let only = &exprs[0];
                if is_paren_or_kwbegin(only) {
                    if void {
                        self.pending_void.insert(node_range(only));
                    }
                } else {
                    match single {
                        SingleAction::Nothing => {}
                        SingleAction::CheckExpression => self.check_expression(only, false),
                        SingleAction::OnBlock { tap_like } => {
                            if tap_like {
                                self.check_void_op(only);
                                self.check_expression(only, false);
                            }
                        }
                    }
                }
            }
            _ => self.seq(&exprs, void),
        }
    }

    // --- per-expression checks ---------------------------------------------

    /// `check_void_op`: unwrap parentheses (taking the FIRST statement, like
    /// `node.children.first while begin_type?`), then flag operator sends.
    fn check_void_op(&mut self, expr: &Node<'_>) {
        if let Some(paren) = expr.as_parentheses_node() {
            let first = paren
                .body()
                .and_then(|body| match body.as_statements_node() {
                    Some(stmts) => stmts.body().iter().next(),
                    None => Some(body),
                });
            if let Some(inner) = first {
                self.check_void_op(&inner);
            }
            return;
        }
        let Some(call) = expr.as_call_node() else {
            return;
        };
        // A literal block makes this a parser `block` node, not `call_type?`.
        if has_block_literal(&call) {
            return;
        }
        let name = call.name().as_slice();
        let binary = BINARY_OPERATORS.contains(&name);
        if !binary && !UNARY_OPERATORS.contains(&name) {
            return;
        }
        let args_empty = parser_args_empty(&call);
        if binary && call.call_operator_loc().is_some() && args_empty {
            return;
        }
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (sel_s, sel_e) = (selector.start_offset(), selector.end_offset());
        let message = format!(
            "Operator `{}` used in void context.",
            String::from_utf8_lossy(name)
        );
        let (replace, remove) = if args_empty {
            // Unary operator (a dotted binary send without arguments returned
            // above): replace the whole send with its receiver.
            let Some(receiver) = call.receiver() else {
                return;
            };
            let (n_s, n_e) = node_range(expr);
            (Some((n_s, n_e, self.node_source(&receiver))), None)
        } else {
            let remove = call
                .call_operator_loc()
                .map(|l| (l.start_offset(), l.end_offset()));
            let (r_s, r_e) = self.extend_both_over_space(sel_s, sel_e);
            (Some((r_s, r_e, "\n".to_string())), remove)
        };
        self.push_offense(sel_s, sel_e, message, replace, remove);
    }

    /// `check_expression`: conditionals descend into their branch bodies
    /// (corrections suppressed there); everything else gets the
    /// void-expression node checks.
    fn check_expression(&mut self, expr: &Node<'_>, branch: bool) {
        if let Some(if_node) = expr.as_if_node() {
            if let Some(body) = if_node.statements().and_then(|s| single_stmt(&s)) {
                self.check_void_expression_nodes(&body, true);
            }
            return;
        }
        if let Some(unless_node) = expr.as_unless_node() {
            if let Some(body) = unless_node.statements().and_then(|s| single_stmt(&s)) {
                self.check_void_expression_nodes(&body, true);
            }
            return;
        }
        if let Some(case_node) = expr.as_case_node() {
            for cond in case_node.conditions().iter() {
                if let Some(when_node) = cond.as_when_node()
                    && let Some(body) = when_node.statements().and_then(|s| single_stmt(&s))
                {
                    self.check_expression(&body, true);
                }
            }
            if let Some(body) = case_node
                .else_clause()
                .and_then(|e| e.statements())
                .and_then(|s| single_stmt(&s))
            {
                self.check_expression(&body, true);
            }
            return;
        }
        if let Some(case_match) = expr.as_case_match_node() {
            for cond in case_match.conditions().iter() {
                if let Some(in_node) = cond.as_in_node()
                    && let Some(body) = in_node.statements().and_then(|s| single_stmt(&s))
                {
                    self.check_expression(&body, true);
                }
            }
            if let Some(body) = case_match
                .else_clause()
                .and_then(|e| e.statements())
                .and_then(|s| single_stmt(&s))
            {
                self.check_expression(&body, true);
            }
            return;
        }
        self.check_void_expression_nodes(expr, branch);
    }

    fn check_void_expression_nodes(&mut self, expr: &Node<'_>, branch: bool) {
        self.check_literal(expr, branch);
        self.check_var(expr, branch);
        self.check_self(expr, branch);
        self.check_void_expression(expr, branch);
        if self.check_nonmutating {
            self.check_nonmutating(expr);
        }
    }

    /// `autocorrect_void_expression`: deletion suppressed inside conditional
    /// branch bodies (`if`/`case`/`when`/`case_match`/`in_pattern` parents)
    /// and anywhere under an assignment-method def.
    fn removal_correction(&self, expr: &Node<'_>, branch: bool) -> Option<(usize, usize)> {
        if branch || self.nearest_def_is_asgn() {
            return None;
        }
        let (start, end) = node_range(expr);
        Some((self.extend_left_over_space_and_newlines(start), end))
    }

    fn check_literal(&mut self, expr: &Node<'_>, branch: bool) {
        if expr.as_x_string_node().is_some()
            || expr.as_interpolated_x_string_node().is_some()
            || expr.as_range_node().is_some()
            || expr.as_nil_node().is_some()
            || !self.entirely_literal(expr)
        {
            return;
        }
        let (start, end) = node_range(expr);
        let message = format!("Literal `{}` used in void context.", self.lossy(start, end));
        let remove = self.removal_correction(expr, branch);
        self.push_offense(start, end, message, None, remove);
    }

    fn entirely_literal(&self, node: &Node<'_>) -> bool {
        if let Some(array) = node.as_array_node() {
            return array.elements().iter().all(|e| self.entirely_literal(&e));
        }
        if let Some(hash) = node.as_hash_node() {
            // `each_key` / `each_value` iterate pairs only; a kwsplat
            // contributes neither keys nor values.
            return hash.elements().iter().all(|e| match e.as_assoc_node() {
                Some(assoc) => {
                    self.entirely_literal(&assoc.key()) && self.entirely_literal(&assoc.value())
                }
                None => true,
            });
        }
        if let Some(call) = node.as_call_node() {
            if has_block_literal(&call) {
                return false;
            }
            return call.name().as_slice() == b"freeze"
                && call.receiver().is_some_and(|r| self.entirely_literal(&r));
        }
        is_literal(node)
    }

    fn check_var(&mut self, expr: &Node<'_>, branch: bool) {
        // `it` references stay method sends in the parser engine (`it`-blocks
        // are plain blocks), so `ItLocalVariableReadNode` is not a variable.
        let is_variable = expr.as_local_variable_read_node().is_some()
            || expr.as_instance_variable_read_node().is_some()
            || expr.as_class_variable_read_node().is_some()
            || expr.as_global_variable_read_node().is_some();
        let is_const =
            expr.as_constant_read_node().is_some() || expr.as_constant_path_node().is_some();
        // `__ENCODING__` is a const node in parser; `special_keyword?` routes
        // it to the variable message.
        let is_special = expr.as_source_encoding_node().is_some();
        if !is_variable && !is_const && !is_special {
            return;
        }
        let (start, end) = node_range(expr);
        let template = if is_const { "Constant" } else { "Variable" };
        let message = format!(
            "{template} `{}` used in void context.",
            self.lossy(start, end)
        );
        let remove = self.removal_correction(expr, branch);
        self.push_offense(start, end, message, None, remove);
    }

    fn check_self(&mut self, expr: &Node<'_>, branch: bool) {
        if expr.as_self_node().is_none() {
            return;
        }
        let (start, end) = node_range(expr);
        let remove = self.removal_correction(expr, branch);
        self.push_offense(
            start,
            end,
            "`self` used in void context.".to_string(),
            None,
            remove,
        );
    }

    fn check_void_expression(&mut self, expr: &Node<'_>, branch: bool) {
        if expr.as_defined_node().is_none() && !self.lambda_or_proc(expr) {
            return;
        }
        let (start, end) = node_range(expr);
        let message = format!("`{}` used in void context.", self.lossy(start, end));
        let remove = self.removal_correction(expr, branch);
        self.push_offense(start, end, message, None, remove);
    }

    /// `lambda_or_proc?`: a stabby lambda, any block whose send is named
    /// `lambda` (receiver and arguments ignored, like `BlockNode#lambda?`), a
    /// plain block on a bare zero-argument `proc` or global `Proc.new` send,
    /// or a bare `Proc.new` send without a block.
    fn lambda_or_proc(&self, expr: &Node<'_>) -> bool {
        if expr.as_lambda_node().is_some() {
            return true;
        }
        let Some(call) = expr.as_call_node() else {
            return false;
        };
        let name = call.name().as_slice();
        if let Some(block) = call.block() {
            if let Some(block_node) = block.as_block_node() {
                if name == b"lambda" {
                    return true;
                }
                // The `proc?` patterns require a parser `block` node: a
                // numbered-parameter block is a `numblock` and never matches
                // (`it` blocks stay plain blocks in the parser engine).
                let plain_block = block_node
                    .parameters()
                    .is_none_or(|p| p.as_numbered_parameters_node().is_none());
                if !plain_block || call.is_safe_navigation() {
                    return false;
                }
                let no_args = call.arguments().is_none_or(|a| a.arguments().is_empty());
                if !no_args {
                    return false;
                }
                if name == b"proc" && call.receiver().is_none() {
                    return true;
                }
                return name == b"new" && call.receiver().is_some_and(|r| is_global_proc_const(&r));
            }
            return false; // block-pass argument breaks every pattern
        }
        // `(send #global_const?(:Proc) :new)` — a bare `Proc.new`.
        name == b"new"
            && !call.is_safe_navigation()
            && call.arguments().is_none_or(|a| a.arguments().is_empty())
            && call.receiver().is_some_and(|r| is_global_proc_const(&r))
    }

    fn check_nonmutating(&mut self, expr: &Node<'_>) {
        let Some(call) = expr.as_call_node() else {
            return;
        };
        // `node.type?(:call, :any_block)` (rubocop#15419): `:call` covers both
        // `:send` and `:csend`, so a safe-navigation call (`x&.sort`) with no
        // literal block is now checked too. Every prism `CallNode` is a parser
        // `:call` (no block) or `:any_block` (block literal), so there is no
        // node kind to reject here.
        let name = call.name().as_slice();
        let replaceable_by_each = METHODS_REPLACEABLE_BY_EACH.contains(&name);
        if !replaceable_by_each && !NONMUTATING_METHODS_WITH_BANG_VERSION.contains(&name) {
            return;
        }
        let method = String::from_utf8_lossy(name).into_owned();
        let suggestion = if replaceable_by_each {
            "each".to_string()
        } else {
            format!("{method}!")
        };
        let Some(selector) = call.message_loc() else {
            return;
        };
        let (start, end) = node_range(expr);
        let message =
            format!("Method `#{method}` used in void context. Did you mean `#{suggestion}`?");
        let replace = Some((selector.start_offset(), selector.end_offset(), suggestion));
        self.push_offense(start, end, message, replace, None);
    }
}

fn single_stmt<'pr>(stmts: &StatementsNode<'pr>) -> Option<Node<'pr>> {
    let body = stmts.body();
    if body.len() == 1 {
        body.iter().next()
    } else {
        None
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(program) = node.as_program_node() {
            self.frames.push(Frame::Plain);
            self.seq_stmts(&program.statements(), false);
            return;
        }
        if let Some(def) = node.as_def_node() {
            let name = def.name().as_slice();
            let comparison = matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=" | b">" | b"<");
            let asgn = !comparison && name.ends_with(b"=");
            // `DefNode#void_context?`: `initialize` counts for instance defs
            // only (`def self.initialize` is an ordinary `defs`).
            let void = (def.receiver().is_none() && name == b"initialize") || asgn;
            self.frames.push(Frame::Def { asgn });
            if let Some(body) = def.body()
                && let Some(stmts) = body.as_statements_node()
            {
                self.body_position(&stmts, void, SingleAction::Nothing);
            }
            // A `BeginNode` body (def with rescue/ensure) is handled by its
            // own hook: its statement positions are never in a void context
            // (the protected body's parser parent is the rescue/ensure node).
            return;
        }
        if let Some(call) = node.as_call_node() {
            let block_node = call.block().and_then(|b| b.as_block_node());
            let Some(block) = block_node else {
                self.frames.push(Frame::Plain);
                return;
            };
            let name = call.name().as_slice();
            let is_each = name == b"each";
            // The frame goes on first: the block, its arguments and even the
            // receiver are all descendants of the parser block node.
            self.frames.push(Frame::Block { is_each });
            // `BlockNode::VOID_CONTEXT_METHODS`.
            let void = is_each || name == b"tap";
            if let Some(body) = block.body()
                && let Some(stmts) = body.as_statements_node()
            {
                self.body_position(
                    &stmts,
                    void,
                    SingleAction::OnBlock {
                        tap_like: void && !is_each,
                    },
                );
            }
            return;
        }
        if let Some(lambda) = node.as_lambda_node() {
            // A stabby lambda is a parser block named `lambda`: it is an
            // `any_block` ancestor but never a void context.
            self.frames.push(Frame::Block { is_each: false });
            if let Some(body) = lambda.body()
                && let Some(stmts) = body.as_statements_node()
            {
                self.body_position(&stmts, false, SingleAction::Nothing);
            }
            return;
        }
        self.frames.push(Frame::Plain);
        if let Some(paren) = node.as_parentheses_node() {
            let (s, e) = node_range(node);
            let void = self.pending_void.remove(&(s, e));
            if let Some(body) = paren.body() {
                match body.as_statements_node() {
                    Some(stmts) => self.seq_stmts(&stmts, void),
                    None => self.seq(&[body], void),
                }
            }
            return;
        }
        if let Some(begin) = node.as_begin_node() {
            self.enter_begin(&begin, node);
            return;
        }
        if let Some(if_node) = node.as_if_node() {
            if let Some(stmts) = if_node.statements() {
                self.multi_seq(&stmts);
            }
            if let Some(else_stmts) = if_node
                .subsequent()
                .and_then(|s| s.as_else_node())
                .and_then(|e| e.statements())
            {
                self.multi_seq(&else_stmts);
            }
            return;
        }
        if let Some(unless_node) = node.as_unless_node() {
            if let Some(stmts) = unless_node.statements() {
                self.multi_seq(&stmts);
            }
            if let Some(else_stmts) = unless_node.else_clause().and_then(|e| e.statements()) {
                self.multi_seq(&else_stmts);
            }
            return;
        }
        if let Some(when_node) = node.as_when_node() {
            if let Some(stmts) = when_node.statements() {
                self.multi_seq(&stmts);
            }
            return;
        }
        if let Some(in_node) = node.as_in_node() {
            if let Some(stmts) = in_node.statements() {
                self.multi_seq(&stmts);
            }
            return;
        }
        if let Some(case_node) = node.as_case_node() {
            // `when` bodies are processed by the `WhenNode` hooks; the else
            // clause is reached through a typed field and gets no hook.
            if let Some(stmts) = case_node.else_clause().and_then(|e| e.statements()) {
                self.multi_seq(&stmts);
            }
            return;
        }
        if let Some(case_match) = node.as_case_match_node() {
            if let Some(stmts) = case_match.else_clause().and_then(|e| e.statements()) {
                self.multi_seq(&stmts);
            }
            return;
        }
        if let Some(while_node) = node.as_while_node() {
            if let Some(stmts) = while_node.statements() {
                self.multi_seq(&stmts);
            }
            return;
        }
        if let Some(until_node) = node.as_until_node() {
            if let Some(stmts) = until_node.statements() {
                self.multi_seq(&stmts);
            }
            return;
        }
        if let Some(for_node) = node.as_for_node() {
            // `ForNode#void_context?` is unconditionally true.
            if let Some(stmts) = for_node.statements() {
                self.body_position(&stmts, true, SingleAction::Nothing);
            }
            return;
        }
        if let Some(class_node) = node.as_class_node() {
            if let Some(stmts) = class_node.body().and_then(|b| b.as_statements_node()) {
                self.multi_seq(&stmts);
            }
            return;
        }
        if let Some(module_node) = node.as_module_node() {
            if let Some(stmts) = module_node.body().and_then(|b| b.as_statements_node()) {
                self.multi_seq(&stmts);
            }
            return;
        }
        if let Some(sclass) = node.as_singleton_class_node() {
            if let Some(stmts) = sclass.body().and_then(|b| b.as_statements_node()) {
                self.multi_seq(&stmts);
            }
            return;
        }
        if let Some(pre) = node.as_pre_execution_node() {
            if let Some(stmts) = pre.statements() {
                self.multi_seq(&stmts);
            }
            return;
        }
        if let Some(post) = node.as_post_execution_node() {
            if let Some(stmts) = post.statements() {
                self.multi_seq(&stmts);
            }
            return;
        }
        if let Some(embedded) = node.as_embedded_statements_node() {
            // A string interpolation is always begin-wrapped in parser.
            if let Some(stmts) = embedded.statements() {
                self.multi_seq(&stmts);
            }
        }
    }

    fn leave(&mut self) {
        self.frames.pop();
    }

    fn enter_rescue(&mut self, node: &Node<'_>) {
        self.frames.push(Frame::Plain);
        // A rescue handler body: the parser `resbody`'s last child, but
        // `resbody` defines no `void_context?`, so never void.
        if let Some(rescue) = node.as_rescue_node()
            && let Some(stmts) = rescue.statements()
        {
            self.multi_seq(&stmts);
        }
    }

    fn leave_rescue(&mut self) {
        self.frames.pop();
    }
}

impl<'a> Visitor<'a> {
    /// A statement list that is a parser `begin` only when it has two or more
    /// statements, in a position whose parent defines no `void_context?`.
    fn multi_seq(&mut self, stmts: &StatementsNode<'_>) {
        if stmts.body().len() >= 2 {
            self.seq_stmts(stmts, false);
        }
    }

    fn enter_begin(&mut self, begin: &ruby_prism::BeginNode<'_>, node: &Node<'_>) {
        let (s, e) = node_range(node);
        let pending = self.pending_void.remove(&(s, e));
        let rescue = begin.rescue_clause();
        let ensure = begin.ensure_clause();
        if rescue.is_some() {
            // Parser shape: `(kwbegin (rescue (begin …) resbody* else?))` (or
            // the implicit def/block form without the kwbegin). The protected
            // body's parent is the rescue node and it is its FIRST child, so
            // it is never in a void context.
            if let Some(stmts) = begin.statements() {
                self.multi_seq(&stmts);
            }
        } else if let Some(ens) = &ensure {
            // `(ensure BODY BRANCH)`: `in_void_context?` compares
            // `parent.children.last == node` with parser's STRUCTURAL
            // equality, so a protected body that mirrors the ensure branch is
            // "the last child" and `EnsureNode#void_context?` is true. Real
            // pattern: `begin cleanup ensure cleanup end`.
            if let Some(stmts) = begin.statements() {
                let exprs: Vec<Node> = stmts.body().iter().collect();
                match exprs.len() {
                    0 => {}
                    1 => {
                        if is_paren_or_kwbegin(&exprs[0])
                            && ens
                                .statements()
                                .and_then(|b| single_stmt(&b))
                                .is_some_and(|b| self.node_eq(&exprs[0], &b))
                        {
                            self.pending_void.insert(node_range(&exprs[0]));
                        }
                    }
                    _ => {
                        let void = self.stmts_eq(&exprs, ens.statements());
                        self.seq(&exprs, void);
                    }
                }
            }
        } else if let Some(stmts) = begin.statements() {
            // A pure keyword `begin`: the expression list is the kwbegin's
            // children; void context comes from the recorded body position.
            self.seq_stmts(&stmts, pending);
        }
        if let Some(ens) = &ensure {
            // The ensure branch is the ensure node's last child and
            // `EnsureNode#void_context?` is always true. A single non-begin
            // statement goes through `on_ensure`'s `check_expression` (with
            // no void-operator check).
            if let Some(stmts) = ens.statements() {
                self.body_position(&stmts, true, SingleAction::CheckExpression);
            }
        }
        if let Some(else_stmts) = begin.else_clause().and_then(|e| e.statements()) {
            // The rescue else clause: last child of the rescue node, which
            // defines no `void_context?`.
            self.multi_seq(&else_stmts);
        }
    }

    fn stmts_eq(&self, exprs: &[Node<'_>], other: Option<StatementsNode<'_>>) -> bool {
        let Some(other) = other else { return false };
        let other: Vec<Node> = other.body().iter().collect();
        exprs.len() == other.len() && exprs.iter().zip(&other).all(|(a, b)| self.node_eq(a, b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<VoidOffense> {
        check_void(source.as_bytes(), false)
    }

    fn run_nonmutating(source: &str) -> Vec<VoidOffense> {
        check_void(source.as_bytes(), true)
    }

    fn apply(source: &str, offenses: &[VoidOffense]) -> String {
        let mut out = source.as_bytes().to_vec();
        let mut edits: Vec<(usize, usize, Vec<u8>)> = Vec::new();
        for o in offenses {
            if o.replace_end > o.replace_start {
                edits.push((
                    o.replace_start,
                    o.replace_end,
                    o.replacement.clone().into_bytes(),
                ));
            }
            if o.remove_end > o.remove_start {
                edits.push((o.remove_start, o.remove_end, Vec::new()));
            }
        }
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        for (start, end, text) in edits {
            out.splice(start..end, text);
        }
        String::from_utf8(out).unwrap()
    }

    fn corrected(source: &str) -> String {
        apply(source, &run(source))
    }

    #[test]
    fn binary_op_sequence() {
        let src = "a + b\na + b\na + b\n";
        let got = run(src);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].message, "Operator `+` used in void context.");
        assert_eq!(&src[got[0].start_offset..got[0].end_offset], "+");
        assert_eq!(corrected(src), "a\nb\na\nb\na + b\n");
    }

    #[test]
    fn op_by_itself_and_last_line() {
        assert!(run("a + b\n").is_empty());
        assert!(run("something\na + b\n").is_empty());
        // A dotted binary send without arguments is an ordinary call.
        assert!(run("a.+\nsomething\n").is_empty());
        assert!(run("a&.+\nsomething\n").is_empty());
    }

    #[test]
    fn parenthesized_ops() {
        let src = "(a * b)\n((a * b))\n(((a * b)))\n";
        let got = run(src);
        assert_eq!(got.len(), 2);
        assert_eq!(corrected(src), "(a\nb)\n((a\nb))\n(((a * b)))\n");
    }

    #[test]
    fn dotted_binary_with_argument() {
        let src = "a.==(b)\nnil\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(corrected(src), "a\n(b)\nnil\n");
        assert_eq!(corrected("a&.==(b)\nnil\n"), "a\n(b)\nnil\n");
    }

    #[test]
    fn unary_ops() {
        assert_eq!(run("+b\n+b\n+b\n").len(), 2);
        assert_eq!(
            run("+b\nfoo\n")[0].message,
            "Operator `+@` used in void context."
        );
        assert_eq!(
            run("~b\nfoo\n")[0].message,
            "Operator `~` used in void context."
        );
        assert_eq!(corrected("+b\n+b\n+b\n"), "b\nb\n+b\n");
        assert_eq!(corrected("b.!\nb.!\n"), "b\nb.!\n");
        assert_eq!(corrected("b&.!\nb&.!\n"), "b\nb&.!\n");
    }

    #[test]
    fn not_keyword() {
        let got = run("not a\ntop\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].message, "Operator `!` used in void context.");
        assert_eq!((got[0].start_offset, got[0].end_offset), (0, 3));
    }

    #[test]
    fn variables_and_constants() {
        let src = "var = 5\nvar\ntop\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].message, "Variable `var` used in void context.");
        assert_eq!(corrected(src), "var = 5\ntop\n");
        assert_eq!(
            run("@var = 5\n@var\ntop\n")[0].message,
            "Variable `@var` used in void context."
        );
        assert_eq!(
            run("CONST = 5\nCONST\ntop\n")[0].message,
            "Constant `CONST` used in void context."
        );
        assert_eq!(
            run("A::B\ntop\n")[0].message,
            "Constant `A::B` used in void context."
        );
        assert_eq!(
            run("def foo\n  __ENCODING__\n  42\nend\n")[0].message,
            "Variable `__ENCODING__` used in void context."
        );
    }

    #[test]
    fn guard_and_branch_offenses_without_correction() {
        let src = "var = 5\nvar unless condition\ntop\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].message, "Variable `var` used in void context.");
        assert_eq!(corrected(src), src);
        // Conditional / ternary / case branches: offense, no correction.
        assert_eq!(
            corrected("x = 5\ncondition ? x : nil\ntop\n"),
            "x = 5\ncondition ? x : nil\ntop\n"
        );
        let got = run("case foo\nwhen 1 then 2\nend\nputs 3\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].message, "Literal `2` used in void context.");
        assert_eq!(got[0].remove_end, 0);
        let got = run("case foo\nin 1 then 2\nelse 4\nend\nputs 5\n");
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn literals() {
        let src = "def something\n  [1, 2, [3]]\n  baz\nend\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].message,
            "Literal `[1, 2, [3]]` used in void context."
        );
        assert_eq!(corrected(src), "def something\n  baz\nend\n");
        // Non-literal elements disable the array/hash branch.
        assert!(run("def f\n  [foo, bar]\n  baz\nend\n").is_empty());
        assert!(run("def f\n  {k1: foo}\n  baz\nend\n").is_empty());
        assert!(run("def f\n  {foo => 1}\n  baz\nend\n").is_empty());
        assert_eq!(run("def f\n  {k1: 1, k2: {a: 2}}\n  baz\nend\n").len(), 1);
        // freeze chains.
        assert_eq!(run("def f\n  'foo'.freeze\n  baz\nend\n").len(), 1);
        assert_eq!(run("def f\n  'foo'&.freeze\n  baz\nend\n").len(), 1);
        assert!(run("def f\n  foo.freeze\n  baz\nend\n").is_empty());
        assert_eq!(run("def f\n  [1, ['a'.freeze]]\n  baz\nend\n").len(), 1);
        // nil / ranges / backticks are excluded.
        assert!(run("nil\ntop\n").is_empty());
        assert!(run("1..2\ntop\n").is_empty());
        assert!(run("`touch x`\nnil\n").is_empty());
        assert!(run("%x(touch x)\nnil\n").is_empty());
        // dstr and kwsplat-only hashes count as literals (stock quirk).
        assert_eq!(run("\"#{foo}\"\ntop\n").len(), 1);
        assert_eq!(run("{**foo}\ntop\n").len(), 1);
        assert_eq!(
            run("__FILE__\ntop\n")[0].message,
            "Literal `__FILE__` used in void context."
        );
    }

    #[test]
    fn self_and_defined() {
        assert_eq!(corrected("self; top\n"), "; top\n");
        let got = run("defined?(x)\ntop\n");
        assert_eq!(got[0].message, "`defined?(x)` used in void context.");
        assert_eq!(corrected("defined?(x)\ntop\n"), "\ntop\n");
    }

    #[test]
    fn lambda_and_proc() {
        let src = "def foo\n  -> { bar }\n  top\nend\n";
        assert_eq!(run(src)[0].message, "`-> { bar }` used in void context.");
        assert_eq!(corrected(src), "def foo\n  top\nend\n");
        assert_eq!(run("def f\n  lambda { bar }\n  top\nend\n").len(), 1);
        assert_eq!(run("def f\n  proc { bar }\n  top\nend\n").len(), 1);
        assert_eq!(run("def f\n  Proc.new { bar }\n  top\nend\n").len(), 1);
        assert_eq!(run("def f\n  Proc.new\n  top\nend\n").len(), 1);
        // `.call` chains and receivers break the patterns…
        assert!(run("def f\n  -> { bar }.call\n  top\nend\n").is_empty());
        assert!(run("def f\n  proc { bar }.call\n  top\nend\n").is_empty());
        assert!(run("def f\n  foo.proc { bar }\n  top\nend\n").is_empty());
        // …but `lambda?` only checks the method name.
        assert_eq!(run("foo.lambda { bar }\ntop\n").len(), 1);
        // numblock proc is a parser numblock and never matches `proc?`.
        assert!(run("def f\n  proc { _1 }\n  top\nend\n").is_empty());
        // `it` blocks stay plain blocks in the parser engine.
        assert_eq!(run("def f\n  proc { it }\n  top\nend\n").len(), 1);
    }

    #[test]
    fn def_bodies() {
        // Ordinary def: last expression is the return value.
        let got = run("def something\n  42\n  42\nend\n");
        assert_eq!(got.len(), 1);
        // initialize / setters: every expression is void.
        assert_eq!(run("def initialize\n  42\n  42\nend\n").len(), 2);
        assert_eq!(
            corrected("def initialize\n  42\n  42\nend\n"),
            "def initialize\nend\n"
        );
        let setter = run("def foo=(rhs)\n  42\n  42\nend\n");
        assert_eq!(setter.len(), 2);
        assert!(setter.iter().all(|o| o.remove_end == 0)); // no corrections
        assert_eq!(run("def self.foo=(rhs)\n  42\n  42\nend\n").len(), 2);
        // `def self.initialize` is a defs: not void.
        assert_eq!(run("def self.initialize\n  42\n  42\nend\n").len(), 1);
        // comparison defs are not assignment methods.
        assert_eq!(run("def ==(other)\n  42\n  42\nend\n").len(), 1);
        // single-statement paren/kwbegin bodies inherit the def's context.
        assert_eq!(run("def initialize\n  (42; 42)\nend\n").len(), 2);
        assert_eq!(
            run("def initialize\n  begin\n    42\n    42\n  end\nend\n").len(),
            2
        );
        assert_eq!(run("def initialize\n  ((42; 42))\nend\n").len(), 1);
    }

    #[test]
    fn blocks() {
        // each: pop the last expression and skip operator checks entirely.
        assert_eq!(run("array.each do |x|\n  42\n  42\nend\n").len(), 1);
        assert!(run("array.each do |x|\n  CONST\nend\n").is_empty());
        assert!(run("array.each do |x|\n  42\nend\n").is_empty());
        assert!(run("e.each do |x|\n  puts x\n  x == 42\nend\n").is_empty());
        assert!(run("e.each do\n  puts _1\n  _1 == 42\nend\n").is_empty());
        // …including sequences in the receiver/arguments of the each call.
        assert!(run("(a == b; c).each { |x| }\ntop\n").is_empty());
        // …and through nested defs.
        assert!(run("arr.each do |x|\n  def foo\n    a == b\n    bar\n  end\nend\n").is_empty());
        // tap: a void context for every expression.
        assert_eq!(run("foo.tap do |x|\n  42\n  42\nend\n").len(), 2);
        assert_eq!(
            corrected("foo.tap do |x|\n  42\n  42\nend\n"),
            "foo.tap do |x|\nend\n"
        );
        assert_eq!(run("foo.tap do\n  _1\n  42\nend\n").len(), 2);
        // tap with a single (non-begin) statement: on_block path.
        assert_eq!(run("foo.tap { a == b }\ntop\n").len(), 1);
        assert!(run("foo.tap { it }\ntop\n").is_empty()); // `it` is a send
        assert_eq!(
            run("foo.tap do\n  begin\n    42\n    42\n  end\nend\n").len(),
            2
        );
        // lambdas and ordinary blocks: only non-final expressions.
        assert_eq!(run("x = lambda {\n  42\n  foo\n}\ntop\n").len(), 1);
        assert!(run("array.each { |_item| }\n").is_empty());
    }

    #[test]
    fn for_loops() {
        assert_eq!(run("for _item in array do\n  42\n  42\nend\n").len(), 2);
        assert_eq!(
            corrected("for _item in array do\n  42\n  42\nend\n"),
            "for _item in array do\nend\n"
        );
        assert_eq!(run("for x in arr do (42; 43) end\ntop\n").len(), 2);
    }

    #[test]
    fn ensure_and_rescue() {
        let src = "def foo\nensure\n  bar\n  42\n  42\nend\n";
        assert_eq!(run(src).len(), 2);
        assert_eq!(corrected(src), "def foo\nensure\n  bar\nend\n");
        // single-statement ensure branch: check_expression only — literals
        // are flagged but operators are not.
        assert_eq!(run("def foo\n  bar\nensure\n  [1, 2, [3]]\nend\n").len(), 1);
        assert!(run("def foo\nensure\n  a == b\nend\n").is_empty());
        // rescue handler bodies and the else clause pop their last statement.
        assert_eq!(
            run("begin\n  foo\nrescue A\n  42\n  bar\nrescue B\n  43\n  baz\nelse\n  44\n  qux\nend\ntop\n")
                .len(),
            3
        );
        // protected body: parent is the rescue node, not a void context.
        assert_eq!(
            run("def initialize\n  42\n  42\nrescue\n  bar\nend\n").len(),
            1
        );
        // the structural-equality quirk: a protected body mirroring the
        // ensure branch is "the last child" and becomes void.
        assert_eq!(run("def m\n  42\n  42\nensure\n  42\n  42\nend\n").len(), 4);
        assert_eq!(run("def m\n  42\n  42\nensure\n  bar\nend\n").len(), 1);
        // ensure inside an each block: pop + no op checks.
        assert!(
            run("arr.each do |x|\n  begin\n    foo\n  ensure\n    a == b\n    42\n  end\nend\n")
                .is_empty()
        );
    }

    #[test]
    fn explicit_begin_blocks() {
        let src = "begin\n 1\n 2\nend\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(corrected(src), "begin\n 2\nend\n");
        assert!(run("((); 1)\n").is_empty());
        assert_eq!(run("begin\n  42\n  foo\nend while cond\ntop\n").len(), 1);
    }

    #[test]
    fn conditional_bodies() {
        // multi-statement branches create plain begins: pop the last.
        assert_eq!(run("if cond\n  42\n  foo\nend\ntop\n").len(), 1);
        assert_eq!(run("while c\n  42\n  foo\nend\ntop\n").len(), 1);
        assert_eq!(run("case foo\nwhen 1\n  42\n  bar\nend\ntop\n").len(), 1);
        assert_eq!(
            run("case foo\nwhen 1\n  ok\nelse\n  42\n  bar\nend\ntop\n").len(),
            1
        );
        // assigned conditionals are not void.
        assert!(run("x = if condition\n      42\n    end\nnil\n").is_empty());
        assert!(run("x = (42 if condition)\nnil\n").is_empty());
        assert!(run("x = condition ? 42 : nil\nnil\n").is_empty());
        // if without body / case without when body.
        assert!(run("if some_condition\nend\n\nputs :ok\n").is_empty());
        assert!(run("case foo\nwhen 1\nend\nputs :ok\n").is_empty());
        assert!(run("case foo\nin 1\nend\nputs :ok\n").is_empty());
        // `nil` in a branch is not a literal offense.
        assert!(run("case foo\nwhen 1\n  nil\nend\nputs 3\n").is_empty());
        // case on the last line is not void.
        assert!(run("case foo\nwhen 1 then 2\nend\n").is_empty());
    }

    #[test]
    fn class_and_interpolation_bodies() {
        assert_eq!(run("class Foo\n  42\n  bar\nend\n").len(), 1);
        assert_eq!(run("module M\n  42\n  bar\nend\n").len(), 1);
        assert_eq!(run("class << self\n  42\n  bar\nend\n").len(), 1);
        assert_eq!(run("BEGIN { 42; foo }\ntop\n").len(), 1);
        // multi-statement interpolations are parser begins.
        let got = run("\"#{a == b; c}\"\ntop\n");
        assert_eq!(got.len(), 2);
        assert!(got.iter().any(|o| o.message.contains("Operator `==`")));
        assert_eq!(run("\"#{a == b}\"\ntop\n").len(), 1); // dstr literal only
    }

    #[test]
    fn nonmutating_methods() {
        assert!(run("x.sort\ntop(x)\n").is_empty());
        let got = run_nonmutating("x.sort\ntop(x)\n");
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].message,
            "Method `#sort` used in void context. Did you mean `#sort!`?"
        );
        assert_eq!(apply("x.sort\ntop(x)\n", &got), "x.sort!\ntop(x)\n");
        let got = run_nonmutating("x.sort.flatten\ntop(x)\n");
        assert_eq!(
            got[0].message,
            "Method `#flatten` used in void context. Did you mean `#flatten!`?"
        );
        let src = "[1,2,3].collect do |n|\n  n.to_s\nend\n\"done\"\n";
        let got = run_nonmutating(src);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].message,
            "Method `#collect` used in void context. Did you mean `#each`?"
        );
        assert_eq!(
            apply(src, &got),
            "[1,2,3].each do |n|\n  n.to_s\nend\n\"done\"\n"
        );
        // csend without a block literal is now checked (rubocop#15419).
        let csend = "x&.sort\ntop\n";
        let got = run_nonmutating(csend);
        assert_eq!(got.len(), 1);
        assert_eq!(&csend[got[0].start_offset..got[0].end_offset], "x&.sort");
        assert_eq!(apply(csend, &got), "x&.sort!\ntop\n");
        // branch bodies still correct (no removal suppression for replaces).
        let got = run_nonmutating("cond ? x.sort : nil\ntop\n");
        assert_eq!(got.len(), 1);
        assert!(got[0].replace_end > got[0].replace_start);
        assert!(run_nonmutating("foo = bar\nbaz\n").is_empty());
        assert!(run_nonmutating("def merge\nend\n\n42\n").is_empty());
    }

    #[test]
    fn misc_non_offenses() {
        assert!(run("lambda.(a)\ntop\n").is_empty());
        assert!(run("def foo\n  1..100.times.each { puts 1 }\n  do_something\nend\n").is_empty());
        assert!(run("keys.each { |k| p k }\n").is_empty());
    }

    #[test]
    fn rvalue_and_arg_sequences() {
        assert_eq!(run("x = (42; foo)\ntop\n").len(), 1);
        assert_eq!(run("a, b = (42; foo), 1\ntop\n").len(), 1);
        assert_eq!(run("def f = (42; foo)\ntop\n").len(), 1);
        // nested paren op: one offense, deduplicated by range.
        assert_eq!(run("((a + b); c)\ntop\n").len(), 1);
    }
}
