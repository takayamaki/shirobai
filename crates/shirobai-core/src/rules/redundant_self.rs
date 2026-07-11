//! `Style/RedundantSelf`.
//!
//! Faithful port of `RuboCop::Cop::Style::RedundantSelf`. The cop tracks
//! local-variable scopes so that a `self.foo` whose `foo` clashes with a local
//! variable / argument / block parameter is *not* flagged (the `self` is
//! required there), while genuinely redundant `self` receivers are reported and
//! their `self` + `.` removed.
//!
//! ## Scope model
//!
//! RuboCop keys `@local_variables_scopes` by node identity and resolves a send's
//! visible locals as "the send's own list, or any ancestor's list". The map is
//! fed by two mechanisms that interact through `add_scope`:
//!
//! * `on_def`/`on_defs`/`on_block` call `add_scope(node, vars)`, which assigns
//!   one *shared* array object to **every** descendant of the node. A `def`
//!   passes a fresh `[]` (resetting the scope); a block passes the array the
//!   block node itself already holds (so a block nested in a `def` reuses that
//!   `def`'s array, while a top-level block gets its own). Parameters and the
//!   assignment / condition / pattern handlers then push names onto whatever
//!   array the relevant node holds — which, inside an owner, is that shared
//!   array, making the name visible to every send in the owner.
//!
//! * Nodes outside any `def`/`block` (top-level statements) were never touched by
//!   `add_scope`, so each keeps an independent array; a name attached to such a
//!   node is visible only to that node and its descendants.
//!
//! We model the first case with a stack of owner arrays (`owner_arrays`: `def`
//! pushes a fresh one, a block pushes one only when there is no enclosing owner)
//! and the second with a location-keyed `subtree` map resolved against an
//! ancestor stack.

use std::collections::{HashMap, HashSet};

use ruby_prism::{CallNode, Node, Visit};

/// A redundant `self` receiver. `[self_start, self_end)` is the `self` token
/// range (the offense range), `[dot_start, dot_end)` is the `.` operator range.
pub struct RedundantSelfOffense {
    pub self_start: usize,
    pub self_end: usize,
    pub dot_start: usize,
    pub dot_end: usize,
}

/// Method names treated as keywords by `regular_method_call?` (`KEYWORDS`).
const KEYWORDS: &[&[u8]] = &[
    b"alias",
    b"and",
    b"begin",
    b"break",
    b"case",
    b"class",
    b"def",
    b"defined?",
    b"do",
    b"else",
    b"elsif",
    b"end",
    b"ensure",
    b"false",
    b"for",
    b"if",
    b"in",
    b"module",
    b"next",
    b"nil",
    b"not",
    b"or",
    b"redo",
    b"rescue",
    b"retry",
    b"return",
    b"self",
    b"super",
    b"then",
    b"true",
    b"undef",
    b"unless",
    b"until",
    b"when",
    b"while",
    b"yield",
    b"__FILE__",
    b"__LINE__",
    b"__ENCODING__",
];

/// Operator method names (`Node#operator_method?`).
const OPERATOR_METHODS: &[&[u8]] = &[
    b"|", b"^", b"&", b"<=>", b"==", b"===", b"=~", b">", b">=", b"<", b"<=", b"<<", b">>", b"+",
    b"-", b"*", b"/", b"%", b"**", b"~", b"+@", b"-@", b"[]", b"[]=", b"`", b"!", b"!=", b"!~",
];

type Loc = (usize, usize);

pub fn check_redundant_self(source: &[u8], kernel_methods: &[String]) -> Vec<RedundantSelfOffense> {
    let mut visitor = build_rule(kernel_methods);
    super::dispatch::run(source, &mut [&mut visitor]);
    visitor.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule(kernel_methods: &[String]) -> Visitor<'_> {
    Visitor {
        kernel_methods,
        owner_arrays: Vec::new(),
        subtree: HashMap::new(),
        block_empty_params: HashMap::new(),
        ancestors: Vec::new(),
        mlhs_locs: HashSet::new(),
        pushed_owner: Vec::new(),
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    kernel_methods: &'a [String],
    /// Shared scope arrays of the enclosing `def`/`block` owners (RuboCop's
    /// `add_scope`: all descendants of an owner share one array). A `def` pushes
    /// a fresh array; a `block` reuses the enclosing owner's array (transparent),
    /// or — at top level, with no enclosing owner — pushes a fresh one. Names
    /// added inside an owner (parameters, assignment locals, condition shadows)
    /// go to the innermost owner array and are thus visible to every send in it.
    owner_arrays: Vec<Vec<String>>,
    /// Per-node locals for nodes that sit outside any `def`/`block` (top level),
    /// where RuboCop keeps an independent array per node: location → names
    /// visible to that node and its descendants.
    subtree: HashMap<Loc, Vec<String>>,
    /// Block node location → whether its parameters are
    /// `empty_and_without_delimiters?` (no params and no `| |`).
    block_empty_params: HashMap<Loc, bool>,
    /// Stack of ancestor node locations (innermost last), including the node
    /// currently being entered's ancestors only.
    ancestors: Vec<Loc>,
    /// Locations of multi-target / multi-write nodes (for `parent.mlhs_type?`).
    mlhs_locs: HashSet<Loc>,
    /// Per-entered-branch-node flag: whether it pushed an owner array (so
    /// `handle_leave` knows whether to pop one).
    pushed_owner: Vec<bool>,
    pub(crate) offenses: Vec<RedundantSelfOffense>,
}

impl Visitor<'_> {
    fn loc_of(node: &Node<'_>) -> Loc {
        let l = node.location();
        (l.start_offset(), l.end_offset())
    }

    /// `@local_variables_scopes[node] << name`. Inside a `def`/`block` owner the
    /// name joins the shared owner array (visible to every send in the owner);
    /// at top level it is attached to `key`'s own per-node array.
    fn add_local(&mut self, key: Loc, name: String) {
        if let Some(owner) = self.owner_arrays.last_mut() {
            owner.push(name);
        } else {
            self.subtree.entry(key).or_default().push(name);
        }
    }

    /// Add a parameter / owner-scoped local name. Always inside an owner.
    fn add_frame_var(&mut self, name: String) {
        if let Some(owner) = self.owner_arrays.last_mut() {
            owner.push(name);
        }
    }

    // --- `add_lhs_to_local_variables_scopes(rhs, lhs)` ---
    fn add_lhs_to_scopes(&mut self, rhs: &Node<'_>, lhs: String) {
        if let Some(call) = rhs.as_call_node()
            && let Some(args) = call.arguments()
            && args.arguments().iter().count() > 0
        {
            for arg in args.arguments().iter() {
                self.add_local(Self::loc_of(&arg), lhs.clone());
            }
            return;
        }
        self.add_local(Self::loc_of(rhs), lhs);
    }

    /// `add_masgn_lhs_variables(rhs, lhs)`.
    fn add_masgn_lhs_variables(&mut self, rhs: &Node<'_>, targets: &[Node<'_>]) {
        for target in targets {
            if let Some(name) = target_name(target) {
                self.add_lhs_to_scopes(rhs, name);
            }
        }
    }

    /// `allowed_send_node?(node)`.
    fn allowed_send_node(&self, send_loc: Loc, method: &str) -> bool {
        // Innermost owner (def/block) shared array.
        if self
            .owner_arrays
            .last()
            .is_some_and(|f| f.iter().any(|n| n == method))
        {
            return true;
        }
        // Per-node arrays: the send itself and each ancestor (top-level nodes).
        if self
            .subtree
            .get(&send_loc)
            .is_some_and(|v| v.iter().any(|n| n == method))
        {
            return true;
        }
        for anc in self.ancestors.iter().rev() {
            if self
                .subtree
                .get(anc)
                .is_some_and(|v| v.iter().any(|n| n == method))
            {
                return true;
            }
        }
        self.kernel_methods.iter().any(|m| m == method)
    }

    /// `regular_method_call?(node)`.
    fn regular_method_call(call: &CallNode<'_>) -> bool {
        let name = call.name().as_slice();
        if OPERATOR_METHODS.contains(&name) || KEYWORDS.contains(&name) {
            return false;
        }
        // `camel_case_method?`: starts with an uppercase letter.
        if name.first().is_some_and(|b| b.is_ascii_uppercase()) {
            return false;
        }
        // `setter_method?`: an attribute write (`self.foo = x`).
        if call.is_attribute_write() {
            return false;
        }
        // `implicit_call?`: `self.()` has no message selector.
        if call.message_loc().is_none() {
            return false;
        }
        true
    }

    /// `it_method_in_block?(node)`.
    fn it_method_in_block(&self, call: &CallNode<'_>) -> bool {
        if call.name().as_slice() != b"it" {
            return false;
        }
        let Some(empty) = self.nearest_block_empty_params() else {
            return false;
        };
        if !empty {
            return false;
        }
        let no_args = call
            .arguments()
            .is_none_or(|a| a.arguments().iter().count() == 0);
        // `!node.block_literal?`: the call has no block argument.
        let has_block = call.arguments().is_some_and(|a| {
            a.arguments()
                .iter()
                .any(|n| n.as_block_argument_node().is_some())
        }) || call.block().is_some();
        no_args && !has_block
    }

    /// Whether the nearest ancestor block has empty-and-without-delimiters params.
    fn nearest_block_empty_params(&self) -> Option<bool> {
        for anc in self.ancestors.iter().rev() {
            if let Some(&empty) = self.block_empty_params.get(anc) {
                return Some(empty);
            }
        }
        None
    }

    /// Process a `self.foo` send on enter, mirroring `on_send`.
    fn process_send(&mut self, call: &CallNode<'_>) {
        let Some(receiver) = call.receiver() else {
            return;
        };
        if receiver.as_self_node().is_none() {
            return;
        }
        if !Self::regular_method_call(call) {
            return;
        }
        // `return if node.parent&.mlhs_type?`.
        if self.parent_is_mlhs() {
            return;
        }
        let send_loc = Self::loc_of(&call.as_node());
        let method = String::from_utf8_lossy(call.name().as_slice()).into_owned();
        if self.allowed_send_node(send_loc, &method) {
            return;
        }
        if self.it_method_in_block(call) {
            return;
        }
        // Offense on the receiver; remove receiver + dot.
        let rloc = receiver.location();
        let Some(dot) = call.call_operator_loc() else {
            return;
        };
        self.offenses.push(RedundantSelfOffense {
            self_start: rloc.start_offset(),
            self_end: rloc.end_offset(),
            dot_start: dot.start_offset(),
            dot_end: dot.end_offset(),
        });
    }

    /// Whether the immediate parent (top of the ancestor stack) is a multi-target
    /// node. Mirrors `node.parent&.mlhs_type?`.
    fn parent_is_mlhs(&self) -> bool {
        self.ancestors
            .last()
            .is_some_and(|&p| self.mlhs_locs.contains(&p))
    }

    /// Collect every parameter name of a `ParametersNode` into the current frame.
    fn collect_param_names(&mut self, params: &ruby_prism::ParametersNode<'_>) {
        for p in params.requireds().iter() {
            self.push_param(&p);
        }
        for p in params.optionals().iter() {
            self.push_param(&p);
        }
        if let Some(rest) = params.rest() {
            self.push_param(&rest);
        }
        for p in params.posts().iter() {
            self.push_param(&p);
        }
        for p in params.keywords().iter() {
            self.push_param(&p);
        }
        if let Some(kwrest) = params.keyword_rest() {
            self.push_param(&kwrest);
        }
        if let Some(block) = params.block() {
            self.push_param(&block.as_node());
        }
    }

    /// Push a single parameter's name (if it has one) into the current frame.
    /// `mlhs` destructuring parameters recurse into their members.
    fn push_param(&mut self, node: &Node<'_>) {
        if let Some(p) = node.as_required_parameter_node() {
            self.add_frame_var(String::from_utf8_lossy(p.name().as_slice()).into_owned());
        } else if let Some(p) = node.as_optional_parameter_node() {
            self.add_frame_var(String::from_utf8_lossy(p.name().as_slice()).into_owned());
        } else if let Some(p) = node.as_rest_parameter_node() {
            if let Some(n) = p.name() {
                self.add_frame_var(String::from_utf8_lossy(n.as_slice()).into_owned());
            }
        } else if let Some(p) = node.as_required_keyword_parameter_node() {
            self.add_frame_var(String::from_utf8_lossy(p.name().as_slice()).into_owned());
        } else if let Some(p) = node.as_optional_keyword_parameter_node() {
            self.add_frame_var(String::from_utf8_lossy(p.name().as_slice()).into_owned());
        } else if let Some(p) = node.as_keyword_rest_parameter_node() {
            if let Some(n) = p.name() {
                self.add_frame_var(String::from_utf8_lossy(n.as_slice()).into_owned());
            }
        } else if let Some(p) = node.as_block_parameter_node() {
            if let Some(n) = p.name() {
                self.add_frame_var(String::from_utf8_lossy(n.as_slice()).into_owned());
            }
        } else if let Some(m) = node.as_multi_target_node() {
            // `def do_something((a, b))`: destructured members are locals.
            for member in m.lefts().iter() {
                self.push_param(&member);
            }
            if let Some(r) = m.rest() {
                self.push_param(&r);
            }
            for member in m.rights().iter() {
                self.push_param(&member);
            }
        }
    }

    /// `on_if`/`on_while`/`on_until`: the condition may use `self.x` where `x` is
    /// assigned (via `lvasgn`/`masgn`) anywhere inside the whole construct; such
    /// names shadow the condition. We scan every descendant of `whole` for
    /// assignments and attach their names to the `condition` subtree.
    fn shadow_condition_assignments(&mut self, whole: &Node<'_>, condition: &Node<'_>) {
        let mut names = Vec::new();
        collect_assignment_names(whole, &mut names);
        for name in names {
            self.add_lhs_to_scopes(condition, name);
        }
    }
}

/// The static target name of a multi-assignment / for-index target node, if any.
fn target_name(node: &Node<'_>) -> Option<String> {
    if let Some(t) = node.as_local_variable_target_node() {
        return Some(String::from_utf8_lossy(t.name().as_slice()).into_owned());
    }
    None
}

/// The targets of a multi-write / multi-target node (lefts + rest + rights),
/// flattened. Nested multi-targets contribute their own leaves.
fn multi_targets<'pr>(node: &Node<'pr>) -> Vec<Node<'pr>> {
    let mut out = Vec::new();
    let mut push = |n: Node<'pr>| out.push(n);
    if let Some(m) = node.as_multi_write_node() {
        for n in m.lefts().iter() {
            push(n);
        }
        if let Some(r) = m.rest() {
            push(r);
        }
        for n in m.rights().iter() {
            push(n);
        }
    } else if let Some(m) = node.as_multi_target_node() {
        for n in m.lefts().iter() {
            push(n);
        }
        if let Some(r) = m.rest() {
            push(r);
        }
        for n in m.rights().iter() {
            push(n);
        }
    }
    out
}

impl super::dispatch::Rule for Visitor<'_> {
    /// Fired for every branch node before its children are visited. We run the
    /// matching RuboCop `on_*` handler here (with the ancestor stack still
    /// excluding this node, mirroring `node.parent` / `each_ancestor`), then
    /// push this node so its descendants see it as an ancestor. `leave` pops it
    /// again.
    fn enter(&mut self, node: &Node<'_>) {
        self.handle_enter(node);
        self.ancestors.push(Self::loc_of(node));
    }

    fn leave(&mut self) {
        self.ancestors.pop();
        self.handle_leave();
    }

    /// `on_resbody`: register the exception variable of `rescue => e` so a
    /// `self.e` in the body is not flagged. `RescueNode` is reached through
    /// `BeginNode`'s concretely-typed field and never goes through `enter`, so
    /// we also push its location onto the ancestor stack here (and pop it in
    /// `leave_rescue`) so the body's sends see it as an ancestor — mirroring
    /// stock's `node.each_ancestor` scope lookup.
    fn enter_rescue(&mut self, node: &Node<'_>) {
        if let Some(rescue) = node.as_rescue_node()
            && let Some(reference) = rescue.reference()
            && let Some(target) = reference.as_local_variable_target_node()
        {
            let name = String::from_utf8_lossy(target.name().as_slice()).into_owned();
            self.add_local(Self::loc_of(node), name);
        }
        self.ancestors.push(Self::loc_of(node));
    }

    fn leave_rescue(&mut self) {
        self.ancestors.pop();
    }
}

impl Visitor<'_> {
    /// Dispatch the RuboCop `on_*` handler for `node` and arrange any owner push.
    fn handle_enter(&mut self, node: &Node<'_>) {
        // A `def`/`defs` opens a fresh owner array (resetting visible locals). A
        // block opens a fresh owner only at top level; nested in a `def` it reuses
        // the enclosing array (transparent). Record whether we pushed so
        // `handle_leave` can pop.
        let is_def = node.as_def_node().is_some();
        let is_block = node.as_block_node().is_some();
        // A `def` always opens a fresh owner; a block only when there is no
        // enclosing owner to reuse (top level).
        let pushed = is_def || (is_block && self.owner_arrays.is_empty());
        if pushed {
            self.owner_arrays.push(Vec::new());
        }
        self.pushed_owner.push(pushed);

        if let Some(call) = node.as_call_node() {
            self.process_send(&call);
        } else if let Some(def) = node.as_def_node() {
            if let Some(params) = def.parameters() {
                self.collect_param_names(&params);
            }
        } else if let Some(block) = node.as_block_node() {
            self.enter_block(&block);
        } else if let Some(w) = node.as_local_variable_write_node() {
            let lhs = String::from_utf8_lossy(w.name().as_slice()).into_owned();
            self.add_lhs_to_scopes(&w.value(), lhs);
        } else if let Some(w) = node.as_local_variable_or_write_node() {
            let lhs = String::from_utf8_lossy(w.name().as_slice()).into_owned();
            self.add_lhs_to_scopes(&w.value(), lhs);
        } else if let Some(w) = node.as_local_variable_and_write_node() {
            let lhs = String::from_utf8_lossy(w.name().as_slice()).into_owned();
            self.add_lhs_to_scopes(&w.value(), lhs);
        } else if let Some(w) = node.as_multi_write_node() {
            let targets = multi_targets(node);
            self.add_masgn_lhs_variables(&w.value(), &targets);
            // Do NOT insert MultiWriteNode into mlhs_locs: in parser-gem
            // `masgn.mlhs_type?` is false. Only `mlhs` (prism's
            // `MultiTargetNode`) returns true for `mlhs_type?`. Adding
            // MultiWriteNode here would make `parent_is_mlhs` skip
            // legitimate offenses on the RHS of multi-assignments.
        } else if node.as_multi_target_node().is_some() {
            self.mlhs_locs.insert(Self::loc_of(node));
        } else if let Some(i) = node.as_if_node() {
            self.shadow_condition_assignments(node, &i.predicate());
        } else if let Some(u) = node.as_unless_node() {
            self.shadow_condition_assignments(node, &u.predicate());
        } else if let Some(w) = node.as_while_node() {
            self.shadow_condition_assignments(node, &w.predicate());
        } else if let Some(u) = node.as_until_node() {
            self.shadow_condition_assignments(node, &u.predicate());
        } else if let Some(in_node) = node.as_in_node() {
            let key = Self::loc_of(node);
            let mut names = Vec::new();
            collect_match_vars(&in_node.pattern(), &mut names);
            for name in names {
                self.add_local(key, name);
            }
        }
    }

    fn handle_leave(&mut self) {
        if self.pushed_owner.pop() == Some(true) {
            self.owner_arrays.pop();
        }
    }

    /// `on_block`/`on_numblock`/`on_itblock`: record block-param emptiness and
    /// add the block's parameters to the enclosing def frame.
    fn enter_block(&mut self, block: &ruby_prism::BlockNode<'_>) {
        let loc = Self::loc_of(&block.as_node());
        // `empty_and_without_delimiters?`: a plain `do … end` / `{ … }` with no
        // parameter list. `numblock`/`itblock` have implicit parameters, so they
        // are not "without delimiters" for the `it` rule.
        let empty = block.parameters().is_none();
        self.block_empty_params.insert(loc, empty);
        if let Some(params) = block.parameters() {
            if let Some(bp) = params.as_block_parameters_node() {
                if let Some(inner) = bp.parameters() {
                    self.collect_param_names(&inner);
                }
            } else if let Some(inner) = params.as_parameters_node() {
                self.collect_param_names(&inner);
            }
        }
    }
}

/// Collect the names of every `lvasgn`/`masgn` target found in `node`'s subtree
/// (the node itself included). Used by the `if`/`while`/`until` shadow rule.
fn collect_assignment_names(node: &Node<'_>, out: &mut Vec<String>) {
    let mut c = AssignmentNameCollector { out };
    c.visit(node);
}

struct AssignmentNameCollector<'o> {
    out: &'o mut Vec<String>,
}

impl<'pr> Visit<'pr> for AssignmentNameCollector<'_> {
    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        self.out
            .push(String::from_utf8_lossy(node.name().as_slice()).into_owned());
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_local_variable_target_node(
        &mut self,
        node: &ruby_prism::LocalVariableTargetNode<'pr>,
    ) {
        // Targets of a `masgn` (e.g. `a, b = ...`).
        self.out
            .push(String::from_utf8_lossy(node.name().as_slice()).into_owned());
        ruby_prism::visit_local_variable_target_node(self, node);
    }
}

/// Collect pattern-match binding names (`LocalVariableTargetNode`) inside a
/// pattern. Pins (`^foo`) bind nothing and are naturally excluded.
fn collect_match_vars(node: &Node<'_>, out: &mut Vec<String>) {
    let mut c = MatchVarCollector { out };
    c.visit(node);
}

struct MatchVarCollector<'o> {
    out: &'o mut Vec<String>,
}

impl<'pr> Visit<'pr> for MatchVarCollector<'_> {
    fn visit_local_variable_target_node(
        &mut self,
        node: &ruby_prism::LocalVariableTargetNode<'pr>,
    ) {
        self.out
            .push(String::from_utf8_lossy(node.name().as_slice()).into_owned());
        ruby_prism::visit_local_variable_target_node(self, node);
    }

    fn visit_pinned_variable_node(&mut self, _node: &ruby_prism::PinnedVariableNode<'pr>) {
        // A pin references an existing variable; it binds nothing. Do not recurse
        // into it (its inner read is not a binding).
    }

    fn visit_pinned_expression_node(&mut self, _node: &ruby_prism::PinnedExpressionNode<'pr>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offenses(source: &str) -> Vec<(usize, usize)> {
        let kernel = vec!["open".to_string(), "puts".to_string()];
        check_redundant_self(source.as_bytes(), &kernel)
            .into_iter()
            .map(|o| (o.self_start, o.self_end))
            .collect()
    }

    #[test]
    fn flags_plain_redundant_self() {
        let got = offenses("a = self.b");
        assert_eq!(got.len(), 1);
        assert_eq!(&"a = self.b"[got[0].0..got[0].1], "self");
    }

    #[test]
    fn accepts_local_variable_clash() {
        // `a` is the assignment target attached to the rhs `self.a`.
        assert!(offenses("a = self.a").is_empty());
    }

    #[test]
    fn accepts_argument_clash_in_def() {
        assert!(offenses("def foo(bar)\n  self.bar\nend").is_empty());
    }

    #[test]
    fn shared_def_scope_via_binop_rhs() {
        // Inside a def, `groups` from the lvasgn is visible to `self.groups`
        // through the shared owner array even though it is attached to the rhs
        // operator's argument.
        let src = "def f\n  groups = self.groups - x\n  groups\nend";
        assert!(offenses(src).is_empty());
    }

    #[test]
    fn top_level_binop_rhs_is_flagged() {
        // At top level there is no shared owner, so the name only reaches the
        // argument, leaving `self.groups` redundant.
        assert_eq!(offenses("groups = self.groups - x").len(), 1);
    }

    #[test]
    fn accepts_keyword_method() {
        assert!(offenses("a = self.class").is_empty());
        assert!(offenses("self.if").is_empty());
    }

    #[test]
    fn accepts_setter_and_operator() {
        assert!(offenses("self.a = b").is_empty());
        assert!(offenses("self << a").is_empty());
        assert!(offenses("self[a]").is_empty());
    }

    #[test]
    fn accepts_kernel_method() {
        assert!(offenses("self.open").is_empty());
    }

    #[test]
    fn flags_call_after_or_assign() {
        let got = offenses("self.x ||= 42\nself.x");
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn accepts_block_argument_clash() {
        let src = "%w[a].each do |state|\n  self.state == state\nend";
        assert!(offenses(src).is_empty());
    }

    #[test]
    fn flags_self_in_multi_assign_rhs() {
        // `key, data = self.last_emit_via_buffer` — in parser-gem the parent
        // of the send is `masgn`, and `masgn.mlhs_type?` is false. Only
        // `mlhs` (prism's `MultiTargetNode`) returns true for `mlhs_type?`.
        // So `self.last_emit_via_buffer` on the RHS is flagged.
        let src = "def f\n  key, data = self.last_emit_via_buffer\nend";
        let got = offenses(src);
        assert_eq!(got.len(), 1);
    }
}
