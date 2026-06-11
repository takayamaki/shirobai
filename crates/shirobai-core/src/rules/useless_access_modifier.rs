//! `Lint/UselessAccessModifier`.
//!
//! Stock runs THREE kinds of entry points whose offense sets overlap (the
//! duplicates collapse in `add_offense`'s per-range dedup):
//!
//! - `on_class`/`on_module`/`on_sclass`/`on_block` (eval-style and `included`
//!   blocks) call `check_node(body)`: a multi-statement body (a parser `begin`)
//!   gets the full `check_scope` tracking; a single-statement body is only
//!   flagged when it IS a bare access modifier (notably: a lone
//!   `private_class_method` or a lone `if` are never examined on this path).
//! - `check_scope`'s recursion (`check_child_nodes`) reaches nested scope
//!   nodes (`start_of_new_scope?`) and runs `check_scope` over ALL their
//!   children (constant path, superclass and body alike), with fresh state.
//! - `on_begin` handles the top-level `begin` only: every DIRECT child that is
//!   a `send` and an access modifier is flagged unconditionally (no tracking).
//!
//! This rule reproduces the union on one walk: every scope node opens a
//! [`Frame`] carrying up to two [`Tracker`]s — a `handler` component (the
//! `check_node` path: body statements only) and a `parent` component (the
//! `check_scope`-from-recursion path: the whole subtree), the latter only when
//! an enclosing tracking recursion actually reaches the node (`consuming`).
//! Offenses are deduplicated by range exactly like `add_offense`.
//!
//! `check_child_nodes` recursion semantics replicated on the walk:
//!
//! - a `send` access modifier (`bare_access_modifier?`, which requires
//!   `macro?` — see the `MacroEntry` stack — or any send named
//!   `private_class_method`) updates the visibility state; a
//!   `private_class_method` WITH arguments resets `cur_vis`/`unused` to nil
//!   (stock destructures the `nil` return of `check_send_node`);
//! - method definitions (`def`, `attr*` / `define_method` /
//!   `MethodCreatingMethods` sends, `define_method` blocks) clear `unused` and
//!   are NOT descended into (frame suspension);
//! - `defs` (`def self.x`) is skipped entirely (suspension, no state change);
//! - `included` blocks (`ActiveSupportExtensionsEnabled`) are skipped by the
//!   recursion but still get their own `on_block` handler frame;
//! - new scopes (class/module/sclass, `class_eval`-style blocks, class
//!   constructors, `ContextCreatingMethods` blocks) get fresh frames;
//! - everything else is transparent: the surrounding tracking continues into
//!   it (conditionally-defined methods, `begin`/`rescue` bodies, hash values…).

use std::collections::HashSet;

use ruby_prism::{CallNode, Node, ProgramNode};

/// One useless access modifier. `name` is interpolated into stock's message
/// (`Useless `%<current>s` access modifier.`); the autocorrect (whole-line
/// removal via `range_by_whole_lines`) is derived from the offense range by
/// the Ruby wrapper with the stock `RangeHelp` helper itself.
pub struct UselessAccessModifierOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub name: String,
}

const ATTR_METHODS: &[&[u8]] = &[b"attr", b"attr_reader", b"attr_writer", b"attr_accessor"];

pub fn check_useless_access_modifier(
    source: &[u8],
    context_creating: &[String],
    method_creating: &[String],
    active_support_extensions: bool,
) -> Vec<UselessAccessModifierOffense> {
    let mut rule = build_rule(context_creating, method_creating, active_support_extensions);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.into_offenses()
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule<'a>(
    context_creating: &'a [String],
    method_creating: &'a [String],
    active_support_extensions: bool,
) -> Visitor<'a> {
    Visitor {
        context_creating,
        method_creating,
        active_support_extensions,
        macro_stack: Vec::new(),
        frames: Vec::new(),
        seen: HashSet::new(),
        offenses: Vec::new(),
    }
}

/// The visibility states a bare access modifier can set. A scope starts at
/// `Some(Public)`; `private_class_method` with arguments resets to `None`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Vis {
    Public,
    Protected,
    Private,
    ModuleFunction,
}

impl Vis {
    fn from_name(name: &[u8]) -> Option<Vis> {
        match name {
            b"public" => Some(Vis::Public),
            b"protected" => Some(Vis::Protected),
            b"private" => Some(Vis::Private),
            b"module_function" => Some(Vis::ModuleFunction),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Vis::Public => "public",
            Vis::Protected => "protected",
            Vis::Private => "private",
            Vis::ModuleFunction => "module_function",
        }
    }
}

/// One `check_scope` state machine (`cur_vis` / `unused` of
/// `check_child_nodes`).
struct Tracker {
    cur_vis: Option<Vis>,
    unused: Option<(usize, usize, Vis)>,
}

impl Tracker {
    fn new() -> Tracker {
        Tracker {
            cur_vis: Some(Vis::Public),
            unused: None,
        }
    }

    /// `check_new_visibility`: returns the node to flag, if any. A repeated
    /// modifier flags itself; a changed modifier flags the previous unused
    /// one. (`unused`'s message vis always equals `cur_vis` at flag time —
    /// every path that changes `cur_vis` away also rewrites/clears `unused`.)
    fn on_modifier(&mut self, range: (usize, usize), vis: Vis) -> Option<(usize, usize, Vis)> {
        let flagged = if self.cur_vis == Some(vis) {
            Some((range.0, range.1, vis))
        } else {
            let previous = self.unused.take();
            self.unused = Some((range.0, range.1, vis));
            previous
        };
        self.cur_vis = Some(vis);
        flagged
    }
}

/// `macro?` context for the children of one walked branch node, restating
/// rubocop-ast's `in_macro_scope?` wrapper chain (`kwbegin`/`begin`/
/// `any_block`/non-condition `if` positions pass the flag through; class-like
/// nodes and `class_constructor?` calls reset it to true; everything else
/// breaks it).
enum MacroEntry {
    Uniform(bool),
    /// `if`/`unless`: the condition child is excluded from macro scope.
    Cond {
        cond_start: usize,
        cond_end: usize,
        inherited: bool,
    },
    /// A call: receiver/argument children of the parser send vs. the
    /// `BlockNode` child (the parser `block` wrapper position).
    Call {
        args: bool,
        block: bool,
    },
}

/// One open scope. `handler` is the `check_node` component (armed when the
/// body statements node enters, body-only region); `parent` is the
/// `check_scope`-from-recursion component (whole subtree). `suspend_at` marks
/// a subtree the stock recursion does not descend into.
struct Frame {
    open_depth: usize,
    suspend_at: Option<usize>,
    parent: Option<Tracker>,
    handler: Option<Tracker>,
    /// `(start, end, depth)` of the multi-statement body statements node that
    /// arms the handler component.
    handler_body: Option<(usize, usize, usize)>,
    handler_active_at: Option<usize>,
}

pub(crate) struct Visitor<'a> {
    context_creating: &'a [String],
    method_creating: &'a [String],
    active_support_extensions: bool,
    macro_stack: Vec<MacroEntry>,
    frames: Vec<Frame>,
    /// `add_offense` drops duplicate ranges (first wins); the handler and
    /// parent components, and stock's own redundant passes, overlap.
    seen: HashSet<(usize, usize)>,
    offenses: Vec<UselessAccessModifierOffense>,
}

fn node_range(node: &Node<'_>) -> (usize, usize) {
    let loc = node.location();
    (loc.start_offset(), loc.end_offset())
}

/// `#global_const?`: `Name` or `::Name`.
fn is_global_const(node: &Node<'_>, name: &[u8]) -> bool {
    if let Some(c) = node.as_constant_read_node() {
        return c.name().as_slice() == name;
    }
    if let Some(p) = node.as_constant_path_node() {
        return p.parent().is_none() && p.name().is_some_and(|n| n.as_slice() == name);
    }
    false
}

/// `const` in node-pattern terms: any constant node, whatever its parent
/// expression.
fn is_const(node: &Node<'_>) -> bool {
    node.as_constant_read_node().is_some() || node.as_constant_path_node().is_some()
}

/// `Node#class_constructor?`, send form: `{Class,Module,Struct}.new` or
/// `Data.define` on the global constant, any arguments. (The `any_block` form
/// is this same send carrying a literal block.)
fn is_constructor_send(call: &CallNode<'_>) -> bool {
    if call.is_safe_navigation() {
        return false;
    }
    let Some(recv) = call.receiver() else {
        return false;
    };
    match call.name().as_slice() {
        b"new" => {
            is_global_const(&recv, b"Class")
                || is_global_const(&recv, b"Module")
                || is_global_const(&recv, b"Struct")
        }
        b"define" => is_global_const(&recv, b"Data"),
        _ => false,
    }
}

/// `node.arguments?` in parser terms: positional/keyword arguments or a
/// block-pass (`&blk` is a child of the parser send). A literal block is a
/// separate parser node and never reaches this.
fn parser_has_args(call: &CallNode<'_>) -> bool {
    call.arguments().is_some_and(|a| !a.arguments().is_empty()) || call.block().is_some()
}

impl<'a> Visitor<'a> {
    pub(crate) fn into_offenses(mut self) -> Vec<UselessAccessModifierOffense> {
        self.offenses
            .sort_by_key(|o| (o.start_offset, o.end_offset));
        self.offenses
    }

    fn push_offense(&mut self, range: (usize, usize), name: &str) {
        if !self.seen.insert(range) {
            return;
        }
        self.offenses.push(UselessAccessModifierOffense {
            start_offset: range.0,
            end_offset: range.1,
            name: name.to_string(),
        });
    }

    /// Resolve `node`'s own macro flag against its parent's stack entry.
    fn macro_at(&self, node: &Node<'_>) -> bool {
        match self.macro_stack.last() {
            None => true, // the root node (its scope is the root itself)
            Some(MacroEntry::Uniform(b)) => *b,
            Some(MacroEntry::Cond {
                cond_start,
                cond_end,
                inherited,
            }) => {
                let start = node.location().start_offset();
                if start >= *cond_start && start < *cond_end {
                    false
                } else {
                    *inherited
                }
            }
            Some(MacroEntry::Call { args, block }) => {
                if node.as_block_node().is_some() {
                    *block
                } else {
                    *args
                }
            }
        }
    }

    /// Whether the stock recursion is consuming events here: some tracking
    /// component on the top frame is active and not suspended.
    fn consuming(&self) -> bool {
        match self.frames.last() {
            None => false,
            Some(f) => {
                f.suspend_at.is_none() && (f.parent.is_some() || f.handler_active_at.is_some())
            }
        }
    }

    fn suspend_top(&mut self, depth: usize) {
        if let Some(f) = self.frames.last_mut() {
            f.suspend_at = Some(depth);
        }
    }

    /// Apply a bare access modifier to every active tracker on the top frame.
    fn apply_modifier(&mut self, range: (usize, usize), vis: Vis) {
        let Some(f) = self.frames.last_mut() else {
            return;
        };
        let mut flagged = Vec::new();
        if let Some(t) = f.parent.as_mut()
            && let Some(o) = t.on_modifier(range, vis)
        {
            flagged.push(o);
        }
        if f.handler_active_at.is_some()
            && let Some(t) = f.handler.as_mut()
            && let Some(o) = t.on_modifier(range, vis)
        {
            flagged.push(o);
        }
        for (s, e, v) in flagged {
            self.push_offense((s, e), v.name());
        }
    }

    /// `check_send_node`'s `private_class_method` arm: without arguments it is
    /// an immediate offense; with arguments stock destructures the `nil`
    /// return into `cur_vis, unused = nil`.
    fn apply_private_class_method(&mut self, range: (usize, usize), has_args: bool) {
        let Some(f) = self.frames.last_mut() else {
            return;
        };
        let mut flag = false;
        let handler_active = f.handler_active_at.is_some();
        for tracker in [
            f.parent.as_mut(),
            if handler_active {
                f.handler.as_mut()
            } else {
                None
            },
        ]
        .into_iter()
        .flatten()
        {
            if has_args {
                tracker.cur_vis = None;
                tracker.unused = None;
            } else {
                flag = true;
            }
        }
        if flag {
            self.push_offense(range, "private_class_method");
        }
    }

    /// A method definition: clear `unused` on every active tracker.
    fn apply_method_def(&mut self) {
        let Some(f) = self.frames.last_mut() else {
            return;
        };
        if let Some(t) = f.parent.as_mut() {
            t.unused = None;
        }
        if f.handler_active_at.is_some()
            && let Some(t) = f.handler.as_mut()
        {
            t.unused = None;
        }
    }

    fn list_match(&self, list: &[String], name: &[u8]) -> bool {
        // Stock skips `"included"` in both configured lists to avoid clashing
        // with the ActiveSupport handling.
        list.iter().any(|m| m != "included" && m.as_bytes() == name)
    }

    /// Open the frame for a scope node. `body` is the prism body field;
    /// `with_parent` reflects whether the stock recursion reaches this node
    /// (`start_of_new_scope?` → `check_scope`); `with_handler` whether a
    /// commissioner handler fires for it (everything except constructor
    /// sends); `body_macro` is the macro flag of the body statements.
    fn open_frame(
        &mut self,
        open_depth: usize,
        body: Option<Node<'_>>,
        body_depth: usize,
        with_parent: bool,
        with_handler: bool,
        body_macro: bool,
    ) {
        let mut handler = None;
        let mut handler_body = None;
        if with_handler
            && let Some(body_node) = body
            && let Some(stmts) = body_node.as_statements_node()
        {
            // `check_node`: a multi-statement body is a parser `begin`
            // (`check_scope`); a single parenthesised statement is ALSO a
            // parser `begin`; any other single statement is only flagged when
            // it is a bare access modifier. A `BeginNode` body (a block with
            // rescue/ensure) is a parser `rescue`/`ensure` node: no-op.
            let list: Vec<Node> = stmts.body().iter().collect();
            if list.len() >= 2 || (list.len() == 1 && list[0].as_parentheses_node().is_some()) {
                handler = Some(Tracker::new());
                let range = node_range(&body_node);
                handler_body = Some((range.0, range.1, body_depth));
            } else if list.len() == 1 {
                self.check_single_statement(&list[0], body_macro);
            }
        }
        self.frames.push(Frame {
            open_depth,
            suspend_at: None,
            parent: with_parent.then(Tracker::new),
            handler,
            handler_body,
            handler_active_at: None,
        });
    }

    /// `check_node` over a single-statement body: flagged only when it is a
    /// bare access modifier (shape + macro context).
    fn check_single_statement(&mut self, stmt: &Node<'_>, body_macro: bool) {
        let Some(call) = stmt.as_call_node() else {
            return;
        };
        if call.is_safe_navigation()
            || call.receiver().is_some()
            || parser_has_args(&call)
            || !body_macro
        {
            return;
        }
        if let Some(vis) = Vis::from_name(call.name().as_slice()) {
            self.push_offense(node_range(stmt), vis.name());
        }
    }

    /// `on_begin`: only the top-level parser `begin` (≥2 program statements,
    /// or one parenthesised statement group). Every direct `send` child that
    /// is an access modifier is flagged unconditionally.
    fn top_level(&mut self, program: &ProgramNode<'_>) {
        let stmts = program.statements();
        let list: Vec<Node> = stmts.body().iter().collect();
        let items: Vec<Node> = if list.len() >= 2 {
            list
        } else if list.len() == 1
            && let Some(paren) = list[0].as_parentheses_node()
        {
            match paren.body() {
                Some(body) => match body.as_statements_node() {
                    Some(inner) => inner.body().iter().collect(),
                    None => vec![body],
                },
                None => Vec::new(),
            }
        } else {
            return;
        };
        for stmt in &items {
            let Some(call) = stmt.as_call_node() else {
                continue;
            };
            if call.is_safe_navigation() {
                continue;
            }
            // A literal block makes this a parser `block` node, not a send.
            if call.block().is_some_and(|b| b.as_block_node().is_some()) {
                continue;
            }
            let name = call.name().as_slice();
            let has_args = parser_has_args(&call);
            let range = node_range(stmt);
            if call.receiver().is_none()
                && !has_args
                && let Some(vis) = Vis::from_name(name)
            {
                self.push_offense(range, vis.name());
            } else if name == b"private_class_method" && !has_args {
                self.push_offense(range, "private_class_method");
            }
        }
    }

    fn enter_call(&mut self, call: &CallNode<'_>, node: &Node<'_>, depth: usize) {
        let is_ctor = is_constructor_send(call);
        let inherited = self.macro_at(node);
        let macro_entry = MacroEntry::Call {
            args: is_ctor,
            block: is_ctor || inherited,
        };

        if let Some(block) = call.block().and_then(|b| b.as_block_node()) {
            // Parser `block`/`numblock`/`itblock`.
            let name = call.name().as_slice();
            let csend = call.is_safe_navigation();
            let is_included = self.active_support_extensions && name == b"included";
            let send_args_empty = call.arguments().is_none_or(|a| a.arguments().is_empty());
            let is_dynamic_def = !csend && call.receiver().is_none() && name == b"define_method";
            let is_eval = !csend
                && ((send_args_empty && (name == b"class_eval" || name == b"instance_eval"))
                    || is_ctor
                    || (call.receiver().is_none_or(|r| is_const(&r))
                        && self.list_match(self.context_creating, name)));
            let consumed = self.consuming();
            let body_macro = is_ctor || inherited;
            if is_included {
                // The recursion skips it (`next`), only `on_block` fires:
                // handler component only, and the frame shields the
                // surrounding trackers from its subtree.
                self.open_frame(depth, block.body(), depth + 2, false, true, body_macro);
            } else if is_dynamic_def {
                // `dynamic_method_definition?` outranks `eval_call?` in the
                // recursion; `on_block` does not fire for it.
                if consumed {
                    self.apply_method_def();
                    self.suspend_top(depth);
                }
            } else if is_eval {
                self.open_frame(depth, block.body(), depth + 2, consumed, true, body_macro);
            }
            // Any other block is transparent: the surrounding tracking
            // continues into its subtree.
            self.macro_stack.push(macro_entry);
            return;
        }

        if !call.is_safe_navigation() && self.consuming() {
            // Parser send in `check_child_nodes`, in stock's branch order.
            let name = call.name().as_slice();
            let has_args = parser_has_args(call);
            let range = node_range(node);
            let bare_vis = if call.receiver().is_none() && !has_args && inherited {
                Vis::from_name(name)
            } else {
                None
            };
            if let Some(vis) = bare_vis {
                self.apply_modifier(range, vis);
            } else if name == b"private_class_method" {
                // `access_modifier?`'s second arm matches by name alone
                // (receiver included); the recursion never descends into it.
                self.apply_private_class_method(range, has_args);
                self.suspend_top(depth);
            } else if call.receiver().is_none()
                && (ATTR_METHODS.contains(&name)
                    || name == b"define_method"
                    || self.list_match(self.method_creating, name))
            {
                self.apply_method_def();
                self.suspend_top(depth);
            } else if is_ctor {
                // `start_of_new_scope?` on a plain send: `check_scope` over
                // its children (no commissioner handler exists for sends).
                self.frames.push(Frame {
                    open_depth: depth,
                    suspend_at: None,
                    parent: Some(Tracker::new()),
                    handler: None,
                    handler_body: None,
                    handler_active_at: None,
                });
            }
        }
        self.macro_stack.push(macro_entry);
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        let depth = self.macro_stack.len();

        if let Some(program) = node.as_program_node() {
            self.top_level(&program);
            self.macro_stack.push(MacroEntry::Uniform(true));
            return;
        }
        if let Some(class_node) = node.as_class_node() {
            let consumed = self.consuming();
            self.open_frame(depth, class_node.body(), depth + 1, consumed, true, true);
            self.macro_stack.push(MacroEntry::Uniform(true));
            return;
        }
        if let Some(module_node) = node.as_module_node() {
            let consumed = self.consuming();
            self.open_frame(depth, module_node.body(), depth + 1, consumed, true, true);
            self.macro_stack.push(MacroEntry::Uniform(true));
            return;
        }
        if let Some(sclass) = node.as_singleton_class_node() {
            let consumed = self.consuming();
            self.open_frame(depth, sclass.body(), depth + 1, consumed, true, true);
            self.macro_stack.push(MacroEntry::Uniform(true));
            return;
        }
        if let Some(def) = node.as_def_node() {
            if self.consuming() {
                // `def` is a method definition; `defs` is skipped outright
                // (`!child.defs_type?`). Neither is descended into.
                if def.receiver().is_none() {
                    self.apply_method_def();
                }
                self.suspend_top(depth);
            }
            self.macro_stack.push(MacroEntry::Uniform(false));
            return;
        }
        if let Some(call) = node.as_call_node() {
            self.enter_call(&call, node, depth);
            return;
        }
        if node.as_statements_node().is_some() {
            // A multi-statement scope body arms the handler component.
            let range = node_range(node);
            let inherited = self.macro_at(node);
            if let Some(f) = self.frames.last_mut()
                && f.handler_body == Some((range.0, range.1, depth))
                && f.handler_active_at.is_none()
            {
                f.handler_active_at = Some(depth);
            }
            self.macro_stack.push(MacroEntry::Uniform(inherited));
            return;
        }
        if let Some(if_node) = node.as_if_node() {
            let inherited = self.macro_at(node);
            let pred = if_node.predicate().location();
            self.macro_stack.push(MacroEntry::Cond {
                cond_start: pred.start_offset(),
                cond_end: pred.end_offset(),
                inherited,
            });
            return;
        }
        if let Some(unless_node) = node.as_unless_node() {
            let inherited = self.macro_at(node);
            let pred = unless_node.predicate().location();
            self.macro_stack.push(MacroEntry::Cond {
                cond_start: pred.start_offset(),
                cond_end: pred.end_offset(),
                inherited,
            });
            return;
        }
        if node.as_parentheses_node().is_some()
            || node.as_lambda_node().is_some()
            || node.as_else_node().is_some()
        {
            // Parser `begin` (parentheses) and stabby lambdas (`any_block`)
            // are macro-scope wrappers. An `ElseNode` only fires the generic
            // hooks as an `IfNode`'s `subsequent` (case/unless else clauses
            // are typed-visited): parser hangs those statements directly
            // under the `if` — a wrapper position — so it must pass the flag
            // through; resolving against the parent entry also yields the
            // right value for any other position (e.g. `Uniform(false)`
            // parents stay false).
            let inherited = self.macro_at(node);
            self.macro_stack.push(MacroEntry::Uniform(inherited));
            return;
        }
        if let Some(begin) = node.as_begin_node() {
            // A pure keyword `begin` is a `kwbegin` wrapper; with a rescue or
            // ensure clause the parser children hang under `rescue`/`ensure`
            // nodes, which break the macro chain.
            let value = if begin.rescue_clause().is_some() || begin.ensure_clause().is_some() {
                false
            } else {
                self.macro_at(node)
            };
            self.macro_stack.push(MacroEntry::Uniform(value));
            return;
        }
        if node.as_block_node().is_some() {
            // The parser `block` wrapper position: resolves against the
            // owning call's entry (`MacroEntry::Call`).
            let value = self.macro_at(node);
            self.macro_stack.push(MacroEntry::Uniform(value));
            return;
        }
        self.macro_stack.push(MacroEntry::Uniform(false));
    }

    fn leave(&mut self) {
        self.macro_stack.pop();
        let depth = self.macro_stack.len();
        let mut close = false;
        if let Some(f) = self.frames.last_mut() {
            if f.suspend_at == Some(depth) {
                f.suspend_at = None;
            }
            if f.handler_active_at == Some(depth) {
                f.handler_active_at = None;
            }
            if f.open_depth == depth {
                close = true;
            }
        }
        if close {
            let frame = self.frames.pop().expect("checked above");
            for tracker in [frame.parent, frame.handler].into_iter().flatten() {
                if let Some((s, e, vis)) = tracker.unused {
                    self.push_offense((s, e), vis.name());
                }
            }
        }
    }

    fn enter_rescue(&mut self, _node: &Node<'_>) {
        // A parser `rescue`/`resbody` position: transparent for the tracking
        // recursion, but it breaks the macro-scope chain.
        self.macro_stack.push(MacroEntry::Uniform(false));
    }

    fn leave_rescue(&mut self) {
        self.macro_stack.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<UselessAccessModifierOffense> {
        check_useless_access_modifier(source.as_bytes(), &[], &[], false)
    }

    fn run_with(
        source: &str,
        context_creating: &[&str],
        method_creating: &[&str],
        asee: bool,
    ) -> Vec<UselessAccessModifierOffense> {
        let ctx: Vec<String> = context_creating.iter().map(|s| s.to_string()).collect();
        let mcm: Vec<String> = method_creating.iter().map(|s| s.to_string()).collect();
        check_useless_access_modifier(source.as_bytes(), &ctx, &mcm, asee)
    }

    fn names(offenses: &[UselessAccessModifierOffense]) -> Vec<&str> {
        offenses.iter().map(|o| o.name.as_str()).collect()
    }

    fn sources<'s>(source: &'s str, offenses: &[UselessAccessModifierOffense]) -> Vec<&'s str> {
        offenses
            .iter()
            .map(|o| &source[o.start_offset..o.end_offset])
            .collect()
    }

    #[test]
    fn modifier_before_defs_only() {
        let src = "class C\n  def a\n  end\n  private\n  def self.b\n  end\nend\n";
        let got = run(src);
        assert_eq!(names(&got), vec!["private"]);
        assert_eq!(sources(src, &got), vec!["private"]);
    }

    #[test]
    fn top_level_modifiers() {
        assert_eq!(
            names(&run("def a\nend\nprivate\ndef b\nend\n")),
            vec!["private"]
        );
        assert_eq!(
            names(&run("module_function\ndef m; end\n")),
            vec!["module_function"]
        );
        // A single top-level statement is not a `begin`: no `on_begin`.
        assert!(run("private\n").is_empty());
        // …but a parenthesised group IS the root `begin`.
        assert_eq!(names(&run("(private)\n")), vec!["private"]);
        assert!(run("((private))\n").is_empty());
        // Top-level `private_class_method`, receiver or not.
        assert_eq!(
            names(&run("x = 1\nFoo.private_class_method\n")),
            vec!["private_class_method"]
        );
        assert!(run("x = 1\nprivate_class_method :foo\n").is_empty());
    }

    #[test]
    fn trailing_and_no_method_modifiers() {
        assert_eq!(names(&run("class C\n  private\nend\n")), vec!["private"]);
        assert_eq!(
            names(&run("class C\n  def a\n  end\n  protected\nend\n")),
            vec!["protected"]
        );
        // Leading `public` repeats the implicit default.
        assert_eq!(
            names(&run("class C\n  public\n  def a\n  end\nend\n")),
            vec!["public"]
        );
        assert!(run("class C\n  private\n  def a\n  end\nend\n").is_empty());
    }

    #[test]
    fn consecutive_and_unused_pairs() {
        // Repeat offense on the second, plus the first is never used.
        let src = "class C\n  private\n  private\nend\n";
        let got = run(src);
        assert_eq!(got.len(), 2);
        // With following defs only the repeat is flagged.
        assert_eq!(
            run("class C\n  private\n  private\n  def a\n  end\nend\n").len(),
            1
        );
        // public…private without intervening defs flags the public.
        let src = "class C\n  private\n  def a\n  end\n  public\n  private\n  def b\n  end\nend\n";
        let got = run(src);
        assert_eq!(names(&got), vec!["public"]);
    }

    #[test]
    fn method_creating_constructs() {
        assert!(run("class C\n  private\n  attr_accessor :x\nend\n").is_empty());
        assert!(run("class C\n  x = 1\n  private\n  attr\nend\n").is_empty());
        assert!(run("class C\n  private\n  define_method(:m) do\n  end\nend\n").is_empty());
        assert!(run("class C\n  private\n  define_method(:m, lambda { })\nend\n").is_empty());
        // Conditionally / iteratively defined methods count.
        assert!(run("class C\n  private\n  if x\n    def m\n    end\n  end\nend\n").is_empty());
        assert!(
            run("class C\n  private\n  [1].each do |i|\n    define_method(\"m#{i}\") do\n    end\n  end\nend\n")
                .is_empty()
        );
        // A def as a method argument counts too (recursion into plain sends).
        assert!(run("class C\n  private\n  helper_method def m\n  end\nend\n").is_empty());
        // `private :sym` is no bare modifier.
        assert!(run("class C\n  def m\n  end\n  private :m\nend\n").is_empty());
        // Inline `private def` is an argument send, not a bare modifier.
        assert!(run("class C\n  private def m\n  end\nend\n").is_empty());
    }

    #[test]
    fn private_class_method_quirks() {
        assert_eq!(
            names(&run(
                "class C\n  private_class_method\n\n  def self.m\n  end\nend\n"
            )),
            vec!["private_class_method"]
        );
        assert!(run("class C\n  private_class_method def self.m\n  end\nend\n").is_empty());
        // A lone `private_class_method` body is NOT flagged (check_node only
        // handles bare modifiers)…
        assert!(run("class C\n  private_class_method\nend\n").is_empty());
        // …but the parent recursion path does flag it.
        assert_eq!(
            names(&run(
                "class B\n  x = 1\n  class C\n    private_class_method\n  end\nend\n"
            )),
            vec!["private_class_method"]
        );
        // With arguments it resets cur_vis/unused to nil (stock destructures
        // the nil return).
        assert!(
            run("class C\n  def a; end\n  private\n  private_class_method :x\n  private\n  def b; end\nend\n")
                .is_empty()
        );
        // It matches by name even with a receiver or in odd positions.
        assert_eq!(
            names(&run("class C\n  x = 1\n  Foo.private_class_method\nend\n")),
            vec!["private_class_method"]
        );
        assert_eq!(
            names(&run(
                "class C\n  x = 1\n  z = { a: private_class_method }\nend\n"
            )),
            vec!["private_class_method"]
        );
    }

    #[test]
    fn scope_creating_blocks() {
        assert!(run("A.class_eval do\n  private\n  define_method(:m) do\n  end\nend\n").is_empty());
        assert_eq!(
            names(&run("A.class_eval do\n  private\nend\n")),
            vec!["private"]
        );
        assert_eq!(
            names(&run("A.instance_eval do\n  private\nend\n")),
            vec!["private"]
        );
        // A modifier outside the eval block is not used by defs inside it.
        assert_eq!(
            names(&run(
                "class A\n  private\n  A.class_eval do\n    def m\n    end\n  end\nend\n"
            )),
            vec!["private"]
        );
        // class_eval WITH arguments is no eval call: the block is transparent.
        assert_eq!(
            run("class A\n  x = 1\n  A.class_eval(\"x\") do\n    private\n  end\nend\n").len(),
            1
        );
        // Constructor blocks create scopes.
        assert!(run("Class.new do\n  private\n  def m\n  end\nend\n").is_empty());
        assert_eq!(
            names(&run("Class.new do\n  private\nend\n")),
            vec!["private"]
        );
        assert_eq!(
            names(&run("::Struct.new do\n  private\nend\n")),
            vec!["private"]
        );
        assert_eq!(
            names(&run("Data.define do\n  private\nend\n")),
            vec!["private"]
        );
        assert_eq!(
            names(&run("Data.define do\n  private\n  do_something(_1)\nend\n")),
            vec!["private"]
        );
        // Constructor SENDS create scopes when reached by the recursion: a
        // bare modifier in their arguments is tracked there.
        assert_eq!(
            names(&run(
                "class X\n  y = 1\n  class Y < Struct.new(:a, private)\n    def m; end\n  end\nend\n"
            )),
            vec!["private"]
        );
    }

    #[test]
    fn singleton_class_scopes() {
        assert!(
            run("class A\n  class << self\n    private\n    def m\n    end\n  end\nend\n")
                .is_empty()
        );
        assert_eq!(
            names(&run("class A\n  class << self\n    private\n  end\nend\n")),
            vec!["private"]
        );
        // Handler fires even inside a def (where no recursion reaches).
        assert_eq!(
            names(&run("def foo\n  class << self\n    private\n  end\nend\n")),
            vec!["private"]
        );
        assert_eq!(
            names(&run("class << A\n  def m\n  end\n  private\nend\n")),
            vec!["private"]
        );
    }

    #[test]
    fn nested_scopes() {
        assert!(
            run("module A\n  private\n  def m1\n  end\n  module B\n    def m2\n    end\n    private\n    def m3\n    end\n  end\nend\n")
                .is_empty()
        );
        // Inner and outer flagged independently.
        assert_eq!(
            run("module A\n  private\n  module B\n    private\n  end\nend\n").len(),
            2
        );
        // A def in a nested module does not use the outer modifier.
        assert_eq!(
            names(&run(
                "module A\n  private\n  module B\n    def m\n    end\n  end\nend\n"
            )),
            vec!["private"]
        );
    }

    #[test]
    fn begin_blocks_are_transparent() {
        assert!(run("class A\n  private\n  begin\n    def m\n    end\n  end\nend\n").is_empty());
        let src = "class A\n  x = 1\n  begin\n    def m1\n    end\n    private\n    private\n    def m2\n    end\n  end\nend\n";
        assert_eq!(run(src).len(), 1);
        // A lone kwbegin body is never examined by check_node (probed:
        // stock's `check_node` only handles `begin`, and kwbegin is not it).
        assert!(run("class A\n  begin\n    private\n  end\nend\n").is_empty());
        assert!(
            run("class A\n  begin\n    def m1\n    end\n    private\n    private\n  end\nend\n")
                .is_empty()
        );
        // A lone parenthesised body IS a parser begin.
        assert_eq!(names(&run("class A\n  (private)\nend\n")), vec!["private"]);
    }

    #[test]
    fn macro_scope_edges() {
        // rescue clauses break the macro chain: no modifier, still tracked
        // through transparently.
        assert!(
            run("class X\n  y = 1\n  begin\n    private\n  rescue\n    nil\n  end\nend\n")
                .is_empty()
        );
        // while is not a macro wrapper…
        assert!(run("class X\n  y = 1\n  while z\n    private\n  end\nend\n").is_empty());
        // …but defs inside it still count as definitions.
        assert!(run("class X\n  private\n  while z\n    def m; end\n  end\nend\n").is_empty());
        // if conditions are excluded from macro scope.
        assert!(run("class X\n  y = 1\n  if private\n    z\n  end\nend\n").is_empty());
        // A lone `if` body is never examined by check_node…
        assert!(run("class A\n  if x\n    private\n  end\nend\n").is_empty());
        // …but the parent recursion reaches it (and the if passes macro on).
        assert_eq!(
            names(&run(
                "class B\n  x = 1\n  class A\n    y = 1\n    if x\n      private\n    end\n  end\nend\n"
            )),
            vec!["private"]
        );
        // Eval blocks inside a def lose the macro chain entirely.
        assert!(run("def f\n  A.class_eval do\n    private\n  end\nend\n").is_empty());
        assert!(run("def f\n  A.class_eval do\n    x = 1\n    private\n  end\nend\n").is_empty());
        // Method-named hash keys/values are not modifiers.
        assert!(
            run("class A\n  def m\n  end\n\n  do_something do\n    { private: private }\n  end\nend\n")
                .is_empty()
        );
    }

    #[test]
    fn conditional_branches_share_tracking_state() {
        // The recursion runs through then- and else-branches sequentially:
        // a repeated modifier across branches is flagged (probed; the else
        // statements hang directly under the parser `if`, a macro wrapper).
        let src = "class C\n  x = 1\n  if cond\n    private\n    def a\n    end\n  else\n    private\n    def b\n    end\n  end\nend\n";
        let got = run(src);
        assert_eq!(sources(src, &got), vec!["private"]);
        assert_eq!(got[0].start_offset, 67); // the else-branch one
        // Same through an elsif chain (a nested if in the else position).
        let src = "class C\n  x = 1\n  if a\n    private\n    def m\n    end\n  elsif b\n    private\n    def n\n    end\n  end\nend\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].start_offset, 67);
        // case/else children hang under `case`, which is no macro wrapper.
        assert!(
            run("class C\n  x = 1\n  case y\n  when 1\n    private\n  else\n    private\n  end\nend\n")
                .is_empty()
        );
        // Ternary branches are macro positions: both modifiers are useless
        // (the first is flushed by the second, the second at scope close).
        let src = "class C\n  x = 1\n  cond ? private : protected\nend\n";
        assert_eq!(sources(src, &run(src)), vec!["private", "protected"]);
    }

    #[test]
    fn context_creating_methods_config() {
        let src = "class C\n  concerning :X do\n    def m\n    end\n    private\n    def n\n    end\n  end\n  private\n  def o\n  end\nend\n";
        assert_eq!(run(src).len(), 1); // without config: transparent block
        assert!(run_with(src, &["concerning"], &[], false).is_empty());
        // "included" entries are skipped.
        let inc =
            "class C\n  included do\n    def m\n    end\n  end\n  private\n  def n\n  end\nend\n";
        assert!(run_with(inc, &["included"], &[], false).is_empty());
    }

    #[test]
    fn method_creating_methods_config() {
        let src = "class C\n  private\n\n  delegate :foo, to: :bar\nend\n";
        assert_eq!(names(&run(src)), vec!["private"]);
        assert!(run_with(src, &[], &["delegate"], false).is_empty());
        let trailing = "class C\n  delegate :foo, to: :bar\n\n  private\nend\n";
        assert_eq!(
            names(&run_with(trailing, &[], &["delegate"], false)),
            vec!["private"]
        );
    }

    #[test]
    fn active_support_included_blocks() {
        let src = "class C\n  included do\n    private\n    def foo; end\n  end\n  private\n  def bar; end\nend\n";
        // ASEE on: the included block is its own context.
        assert!(run_with(src, &[], &[], true).is_empty());
        // ASEE off: the block is transparent, the second private repeats.
        assert_eq!(run(src).len(), 1);
        let repeated = "class C\n  included do\n    private\n    private\n    def foo; end\n  end\n  private\n  private\n  def bar; end\nend\n";
        assert_eq!(run_with(repeated, &[], &[], true).len(), 2);
        assert_eq!(run(repeated).len(), 3);
    }

    #[test]
    fn offenses_sorted_and_deduplicated() {
        // The repeat offense (later position) is recorded before the unused
        // one (earlier position); the result must come out sorted.
        let src = "class C\n  private\n  private\nend\n";
        let got = run(src);
        assert_eq!(got.len(), 2);
        assert!(got[0].start_offset < got[1].start_offset);
    }
}
