//! `Lint/SelfAssignment`: flags assignments whose right-hand side just re-reads
//! the left-hand side (e.g. `foo = foo`, `Foo = Foo`, `foo.bar = foo.bar`,
//! `foo['k'] = foo['k']`, `foo ||= foo`, `foo, bar = foo, bar`).
//!
//! Mirrors `vendor/rubocop/lib/rubocop/cop/lint/self_assignment.rb`. Detection
//! only — no autocorrect — matching stock.
//!
//! The `AllowRBSInlineAnnotation` config (default `false`) is *not* handled
//! here: this rule emits the raw offenses, plus a `rbs_anchor_offset` per
//! offense (the byte offset whose AST node stock checks against
//! `processed_source.ast_with_comments` for an `#:` annotation). The Ruby
//! wrapper does the comment lookup using stock's exact code path, but only
//! when the user actually enabled the config — keeping the default-config
//! path zero-cost on the Ruby side. See `lib/shirobai/cop/lint/self_assignment.rb`.

use ruby_prism::{
    CallAndWriteNode, CallNode, CallOrWriteNode, ClassVariableAndWriteNode,
    ClassVariableOrWriteNode, ClassVariableWriteNode, ConstantAndWriteNode,
    ConstantOrWriteNode, ConstantPathNode, ConstantPathWriteNode, ConstantWriteNode,
    GlobalVariableAndWriteNode, GlobalVariableOrWriteNode, GlobalVariableWriteNode,
    IndexAndWriteNode, IndexOrWriteNode, InstanceVariableAndWriteNode,
    InstanceVariableOrWriteNode, InstanceVariableWriteNode, LocalVariableAndWriteNode,
    LocalVariableOrWriteNode, LocalVariableWriteNode, MultiWriteNode, Node, Visit,
};

/// One offense candidate. `start_offset..end_offset` is the offense byte range
/// (the assignment's full source range). `rbs_anchor_offset` is the end byte
/// offset of the AST node stock would key into `processed_source.ast_with_comments`
/// to decide RBS-annotation exemption; the Ruby wrapper uses it to find that
/// node when the user has set `AllowRBSInlineAnnotation: true`.
pub struct SelfAssignmentOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    /// The byte offset whose AST node hosts the RBS annotation check (stock
    /// passes a specific subnode per assignment shape — see [`RbsAnchor`]).
    /// `0` means "no RBS check applies" (only used when both sides agree
    /// without any anchor; today every offense has an anchor).
    pub rbs_anchor_offset: usize,
}

/// Standalone entry point used by the per-cop fallback (the bundle is the
/// usual path).
pub fn check_self_assignment(source: &[u8]) -> Vec<SelfAssignmentOffense> {
    let mut visitor = build_rule(source);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

pub(crate) fn build_rule(source: &[u8]) -> SelfAssignmentVisitor<'_> {
    SelfAssignmentVisitor {
        source,
        offenses: Vec::new(),
    }
}

pub(crate) struct SelfAssignmentVisitor<'s> {
    source: &'s [u8],
    pub(crate) offenses: Vec<SelfAssignmentOffense>,
}

impl<'s> SelfAssignmentVisitor<'s> {
    fn push(&mut self, start: usize, end: usize, anchor_end: usize) {
        self.offenses.push(SelfAssignmentOffense {
            start_offset: start,
            end_offset: end,
            rbs_anchor_offset: anchor_end,
        });
    }

    fn check_lvasgn(&mut self, node: &LocalVariableWriteNode<'_>) {
        // `on_lvasgn`-equivalent: rhs is `LocalVariableReadNode` with same name.
        let value = node.value();
        let Some(rhs) = value.as_local_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        let rhs_loc = rhs.location();
        self.push(loc.start_offset(), loc.end_offset(), rhs_loc.end_offset());
    }

    fn check_ivasgn(&mut self, node: &InstanceVariableWriteNode<'_>) {
        let value = node.value();
        let Some(rhs) = value.as_instance_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        let rhs_loc = rhs.location();
        self.push(loc.start_offset(), loc.end_offset(), rhs_loc.end_offset());
    }

    fn check_cvasgn(&mut self, node: &ClassVariableWriteNode<'_>) {
        let value = node.value();
        let Some(rhs) = value.as_class_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        let rhs_loc = rhs.location();
        self.push(loc.start_offset(), loc.end_offset(), rhs_loc.end_offset());
    }

    fn check_gvasgn(&mut self, node: &GlobalVariableWriteNode<'_>) {
        let value = node.value();
        let Some(rhs) = value.as_global_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        let rhs_loc = rhs.location();
        self.push(loc.start_offset(), loc.end_offset(), rhs_loc.end_offset());
    }

    fn check_casgn(&mut self, node: &ConstantWriteNode<'_>) {
        // `on_casgn`: rhs must be `const_type?` AND `namespace == rhs.namespace
        // AND short_name == rhs.short_name`.
        //
        // `ConstantWriteNode` (e.g. `Foo = ...`) has an *implicit* nil namespace.
        // Stock's `node.namespace` returns `nil` for a bare `Foo = ...`.
        let value = node.value();
        let (rhs_ns, rhs_short) = match resolve_const_rhs(&value) {
            Some(p) => p,
            None => return,
        };
        if node.name().as_slice() != rhs_short {
            return;
        }
        if !namespaces_equal(None, rhs_ns.as_ref()) {
            return;
        }
        let loc = node.location();
        let rhs_loc = value.location();
        self.push(loc.start_offset(), loc.end_offset(), rhs_loc.end_offset());
    }

    fn check_const_path_asgn(&mut self, node: &ConstantPathWriteNode<'_>) {
        // `on_casgn` for the namespaced lhs form (`A::B = ...`, `::Foo = ...`).
        // Prism gives us the lhs as `ConstantPathNode`, decomposed into
        // (parent, name). The same `(namespace == rhs.namespace AND short_name
        // == rhs.short_name)` rule applies.
        let target = node.target();
        let Some(lhs_short) = target.name() else { return };
        let lhs_ns = constant_path_namespace(&target);
        let value = node.value();
        let (rhs_ns, rhs_short) = match resolve_const_rhs(&value) {
            Some(p) => p,
            None => return,
        };
        if lhs_short.as_slice() != rhs_short {
            return;
        }
        if !namespaces_equal(Some(&lhs_ns), rhs_ns.as_ref()) {
            return;
        }
        let loc = node.location();
        let rhs_loc = value.location();
        self.push(loc.start_offset(), loc.end_offset(), rhs_loc.end_offset());
    }

    fn check_masgn(&mut self, node: &MultiWriteNode<'_>) {
        // `on_masgn`: `multiple_self_assignment?`
        //  - rhs must be array_type?
        //  - lhs.children.size == rhs.children.size (no splat in either side)
        //  - each pair: lhs.type ∈ {lvasgn, ivasgn, cvasgn, gvasgn} (casgn map
        //    misses) AND rhs.type == ASSIGNMENT_TYPE_TO_RHS_TYPE[lhs.type] AND
        //    same name.
        let value = node.value();
        let Some(array) = value.as_array_node() else { return };
        let elements: Vec<_> = array.elements().iter().collect();
        let lefts: Vec<_> = node.lefts().iter().collect();
        if node.rest().is_some() {
            return; // splat on the lhs side
        }
        if lefts.len() != elements.len() {
            return;
        }
        if lefts.is_empty() {
            return;
        }
        let all_match = lefts.iter().zip(&elements).all(|(l, r)| {
            multi_pair_matches(l, r)
        });
        if !all_match {
            return;
        }
        let loc = node.location();
        // RBS anchor: stock checks `first_lhs = node.lhs.assignments.first`,
        // i.e. the first lhs assignment node (lvasgn/ivasgn/... in parser).
        let anchor = lefts[0].location().end_offset();
        self.push(loc.start_offset(), loc.end_offset(), anchor);
    }

    fn check_or_asgn(&mut self, node: &LocalVariableOrWriteNode<'_>) {
        // `on_or_asgn`: `rhs_matches_lhs?(node.rhs, node.lhs)`. For
        // `LocalVariableOrWriteNode` the lhs is a synthesized lvasgn-target,
        // and the rhs has to be an `lvar` with the same name.
        let value = node.value();
        let Some(rhs) = value.as_local_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        // RBS anchor: stock passes `node.lhs`, which is a synthesized
        // `lvasgn` whose source range is `node.name_loc` in prism.
        let anchor = node.name_loc().end_offset();
        self.push(loc.start_offset(), loc.end_offset(), anchor);
    }

    fn check_and_asgn(&mut self, node: &LocalVariableAndWriteNode<'_>) {
        let value = node.value();
        let Some(rhs) = value.as_local_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        let anchor = node.name_loc().end_offset();
        self.push(loc.start_offset(), loc.end_offset(), anchor);
    }

    fn check_ivar_or_asgn(&mut self, node: &InstanceVariableOrWriteNode<'_>) {
        let value = node.value();
        let Some(rhs) = value.as_instance_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        let anchor = node.name_loc().end_offset();
        self.push(loc.start_offset(), loc.end_offset(), anchor);
    }

    fn check_ivar_and_asgn(&mut self, node: &InstanceVariableAndWriteNode<'_>) {
        let value = node.value();
        let Some(rhs) = value.as_instance_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        let anchor = node.name_loc().end_offset();
        self.push(loc.start_offset(), loc.end_offset(), anchor);
    }

    fn check_cvar_or_asgn(&mut self, node: &ClassVariableOrWriteNode<'_>) {
        let value = node.value();
        let Some(rhs) = value.as_class_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        let anchor = node.name_loc().end_offset();
        self.push(loc.start_offset(), loc.end_offset(), anchor);
    }

    fn check_cvar_and_asgn(&mut self, node: &ClassVariableAndWriteNode<'_>) {
        let value = node.value();
        let Some(rhs) = value.as_class_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        let anchor = node.name_loc().end_offset();
        self.push(loc.start_offset(), loc.end_offset(), anchor);
    }

    fn check_gvar_or_asgn(&mut self, node: &GlobalVariableOrWriteNode<'_>) {
        let value = node.value();
        let Some(rhs) = value.as_global_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        let anchor = node.name_loc().end_offset();
        self.push(loc.start_offset(), loc.end_offset(), anchor);
    }

    fn check_gvar_and_asgn(&mut self, node: &GlobalVariableAndWriteNode<'_>) {
        let value = node.value();
        let Some(rhs) = value.as_global_variable_read_node() else { return };
        if rhs.name().as_slice() != node.name().as_slice() {
            return;
        }
        let loc = node.location();
        let anchor = node.name_loc().end_offset();
        self.push(loc.start_offset(), loc.end_offset(), anchor);
    }

    fn check_const_or_and_asgn(&mut self, name: &[u8], value: &Node<'_>, loc_start: usize, loc_end: usize, anchor: usize) {
        let (rhs_ns, rhs_short) = match resolve_const_rhs(value) {
            Some(p) => p,
            None => return,
        };
        if name != rhs_short {
            return;
        }
        if !namespaces_equal(None, rhs_ns.as_ref()) {
            return;
        }
        self.push(loc_start, loc_end, anchor);
    }

    fn check_const_or_asgn(&mut self, node: &ConstantOrWriteNode<'_>) {
        let value = node.value();
        let loc = node.location();
        let anchor = node.name_loc().end_offset();
        self.check_const_or_and_asgn(node.name().as_slice(), &value, loc.start_offset(), loc.end_offset(), anchor);
    }

    fn check_const_and_asgn(&mut self, node: &ConstantAndWriteNode<'_>) {
        let value = node.value();
        let loc = node.location();
        let anchor = node.name_loc().end_offset();
        self.check_const_or_and_asgn(node.name().as_slice(), &value, loc.start_offset(), loc.end_offset(), anchor);
    }

    fn check_call_or_and_asgn_common(&mut self, receiver: Option<Node<'_>>, read_name: &[u8], value: &Node<'_>, loc_start: usize, loc_end: usize) {
        let Some(rhs_call) = value.as_call_node() else { return };
        let rhs_args: Vec<Node<'_>> = rhs_call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        if !rhs_args.is_empty() {
            return;
        }
        if rhs_call.name().as_slice() != read_name {
            return;
        }
        let anchor = receiver
            .as_ref()
            .map(|r| r.location().end_offset())
            .unwrap_or(loc_start);
        if !ast_equal(receiver, rhs_call.receiver(), self.source) {
            return;
        }
        self.push(loc_start, loc_end, anchor);
    }

    fn check_call_or_asgn(&mut self, node: &CallOrWriteNode<'_>) {
        let loc = node.location();
        self.check_call_or_and_asgn_common(node.receiver(), node.read_name().as_slice(), &node.value(), loc.start_offset(), loc.end_offset());
    }

    fn check_call_and_asgn(&mut self, node: &CallAndWriteNode<'_>) {
        let loc = node.location();
        self.check_call_or_and_asgn_common(node.receiver(), node.read_name().as_slice(), &node.value(), loc.start_offset(), loc.end_offset());
    }

    fn check_index_or_and_asgn_common(&mut self, receiver: Option<Node<'_>>, arguments: Option<ruby_prism::ArgumentsNode<'_>>, value: &Node<'_>, loc_start: usize, loc_end: usize) {
        let Some(rhs_call) = value.as_call_node() else { return };
        if rhs_call.name().as_slice() != b"[]" {
            return;
        }
        let anchor = receiver
            .as_ref()
            .map(|r| r.location().end_offset())
            .unwrap_or(loc_start);
        if !ast_equal(receiver, rhs_call.receiver(), self.source) {
            return;
        }
        let lhs_args: Vec<Node<'_>> = arguments
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        if lhs_args.iter().any(|n| is_call_node(n)) {
            return;
        }
        let rhs_args: Vec<Node<'_>> = rhs_call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        if lhs_args.len() != rhs_args.len() {
            return;
        }
        if !lhs_args
            .iter()
            .zip(&rhs_args)
            .all(|(a, b)| ast_equal_node(a, b, self.source))
        {
            return;
        }
        self.push(loc_start, loc_end, anchor);
    }

    fn check_index_or_asgn(&mut self, node: &IndexOrWriteNode<'_>) {
        let loc = node.location();
        self.check_index_or_and_asgn_common(node.receiver(), node.arguments(), &node.value(), loc.start_offset(), loc.end_offset());
    }

    fn check_index_and_asgn(&mut self, node: &IndexAndWriteNode<'_>) {
        let loc = node.location();
        self.check_index_or_and_asgn_common(node.receiver(), node.arguments(), &node.value(), loc.start_offset(), loc.end_offset());
    }

    fn check_send(&mut self, call: &CallNode<'_>) {
        // `on_send` covers `[]=` (key assignment) and `assignment_method?`
        // (attribute setters whose name ends with `=`).
        let name_bytes = call.name();
        let name = name_bytes.as_slice();
        let arguments: Vec<Node<'_>> = call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        if name == b"[]=" {
            self.handle_key_assignment(call, &arguments);
        } else if is_assignment_method(name) && arguments.len() == 1 {
            self.handle_attribute_assignment(call, &arguments[0], name);
        }
    }

    fn handle_key_assignment(&mut self, call: &CallNode<'_>, args: &[Node<'_>]) {
        // `value_node = node.last_argument` (nil-safe when no args at all).
        // `node_arguments = node.arguments[0...-1]` (key portion).
        // Match if:
        //   value_node is a `[]` call,
        //   call.receiver == value_node.receiver,
        //   no node_arguments is a `call_type?` (i.e. a method call → unknown
        //     return value),
        //   node_arguments == value_node.arguments (by AST equality).
        if args.is_empty() {
            return; // `foo.[]=` with no args — stock bails (no value_node)
        }
        let (value_node, key_args) = args.split_last().unwrap();
        let Some(value_call) = value_node.as_call_node() else { return };
        if value_call.name().as_slice() != b"[]" {
            return;
        }
        // `arguments.none?(&:call_type?)`: stock excludes any key that is a
        // method call (e.g. `foo[bar] = foo[bar]` where `bar` is a method).
        // `call_type?` in rubocop-ast: `send_type? || csend_type?`.
        if key_args.iter().any(|n| is_call_node(n)) {
            return;
        }
        // receiver match — stock: `node.receiver == value_node.receiver`.
        if !ast_equal(call.receiver(), value_call.receiver(), self.source) {
            return;
        }
        // node_arguments == value_node.arguments (zip + ast equality).
        let value_args: Vec<Node<'_>> = value_call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        if key_args.len() != value_args.len() {
            return;
        }
        if !key_args
            .iter()
            .zip(&value_args)
            .all(|(a, b)| ast_equal_node(a, b, self.source))
        {
            return;
        }
        let loc = call.location();
        // RBS anchor: stock passes `node.receiver` — the call's receiver node.
        let anchor = call
            .receiver()
            .map(|r| r.location().end_offset())
            .unwrap_or(loc.start_offset());
        self.push(loc.start_offset(), loc.end_offset(), anchor);
    }

    fn handle_attribute_assignment(&mut self, call: &CallNode<'_>, first_arg: &Node<'_>, name: &[u8]) {
        // `first_argument.respond_to?(:arguments) && first_argument.arguments.empty?`
        // Stock requires `first_argument.call_type?` AND args.empty?,
        // so first arg must be a `send`/`csend` whose own arguments are empty.
        let Some(arg_call) = first_arg.as_call_node() else { return };
        let arg_call_args: Vec<Node<'_>> = arg_call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        if !arg_call_args.is_empty() {
            return;
        }
        if !ast_equal(call.receiver(), arg_call.receiver(), self.source) {
            return;
        }
        // method_name (`foo=` → `foo`) == first_argument.method_name (`foo`).
        let stripped = &name[..name.len() - 1];
        if stripped != arg_call.name().as_slice() {
            return;
        }
        let loc = call.location();
        let anchor = call
            .receiver()
            .map(|r| r.location().end_offset())
            .unwrap_or(loc.start_offset());
        self.push(loc.start_offset(), loc.end_offset(), anchor);
    }
}

/// `multiple_self_assignment?` per-pair check (single AST step).
///
/// Stock requires `ASSIGNMENT_TYPE_TO_RHS_TYPE[lhs.type]`: only the four
/// entries `lvasgn → lvar`, `ivasgn → ivar`, `cvasgn → cvar`,
/// `gvasgn → gvar` exist. Constants (`Foo`) and any other target type
/// fail the map lookup → false. Then the name (children.first) must match.
fn multi_pair_matches(left: &Node<'_>, right: &Node<'_>) -> bool {
    let pair = if let (Some(t), Some(r)) = (
        left.as_local_variable_target_node(),
        right.as_local_variable_read_node(),
    ) {
        Some((t.name().as_slice(), r.name().as_slice()))
    } else if let (Some(t), Some(r)) = (
        left.as_instance_variable_target_node(),
        right.as_instance_variable_read_node(),
    ) {
        Some((t.name().as_slice(), r.name().as_slice()))
    } else if let (Some(t), Some(r)) = (
        left.as_class_variable_target_node(),
        right.as_class_variable_read_node(),
    ) {
        Some((t.name().as_slice(), r.name().as_slice()))
    } else if let (Some(t), Some(r)) = (
        left.as_global_variable_target_node(),
        right.as_global_variable_read_node(),
    ) {
        Some((t.name().as_slice(), r.name().as_slice()))
    } else {
        None
    };

    // ConstantTargetNode / other targets: stock's map misses → false.
    pair.is_some_and(|(l, r)| l == r)
}

fn is_assignment_method(name: &[u8]) -> bool {
    // `assignment_method?` in rubocop-ast: !comparison_method? && ends_with('=').
    // The only comparison op ending in `=` is `!=`, plus `==`, `===`, `<=`,
    // `>=`. Stock also implicitly excludes `[]=` (handled by the `:[]=` branch
    // before assignment_method? would fire for it).
    if !name.ends_with(b"=") {
        return false;
    }
    !matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=" | b"[]=")
}

fn is_call_node(node: &Node<'_>) -> bool {
    // `call_type?` in rubocop-ast is `send_type? || csend_type?`. Both flavors
    // are a single `CallNode` in prism, distinguished only by `call_operator_loc`
    // (`&.` for csend).
    node.as_call_node().is_some()
}

// `Foo`           -> namespace nil
// `::Foo`         -> namespace = cbase
// `A::Foo`        -> namespace = (const nil :A)
// `(expr)::Foo`   -> namespace = arbitrary AST node
//
// We represent the namespace as one of:
//   None        -> nil (bare top-level identifier on the lhs)
//   Cbase       -> `::Foo` form
//   Path(node)  -> any other parent node (compared by AST equality)
enum ConstNamespace<'pr> {
    Cbase,
    Path(Node<'pr>),
}

fn constant_path_namespace<'pr>(path: &ConstantPathNode<'pr>) -> ConstNamespace<'pr> {
    match path.parent() {
        None => ConstNamespace::Cbase,
        Some(p) => ConstNamespace::Path(p),
    }
}

/// Decode an rhs of a constant assignment: returns `Some((namespace, short_name))`
/// if rhs is `ConstantReadNode` or `ConstantPathNode`, else `None`
/// (stock bails when `rhs.const_type?` is false).
fn resolve_const_rhs<'pr>(value: &Node<'pr>) -> Option<(Option<ConstNamespace<'pr>>, &'pr [u8])> {
    if let Some(rhs) = value.as_constant_read_node() {
        Some((None, rhs.name().as_slice()))
    } else if let Some(rhs) = value.as_constant_path_node() {
        let short = rhs.name()?;
        let ns = constant_path_namespace(&rhs);
        Some((Some(ns), short.as_slice()))
    } else {
        None
    }
}

fn namespaces_equal(a: Option<&ConstNamespace<'_>>, b: Option<&ConstNamespace<'_>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(ConstNamespace::Cbase), Some(ConstNamespace::Cbase)) => true,
        (Some(ConstNamespace::Path(x)), Some(ConstNamespace::Path(y))) => {
            // Compare by source-range identity: if two const-path namespaces
            // are byte-equal, parser-gem `==` (deep structural equality) would
            // also be true. We have the source on hand from the visitor.
            x.location().as_slice() == y.location().as_slice()
        }
        _ => false,
    }
}

/// AST equality between two optional nodes — modeled as parser-gem `==`, which
/// compares type + children deeply. For our purposes (small expressions: lvar,
/// ivar, cvar, gvar, const, self, send chains, ints, syms, strings, ...), the
/// source-range slice equality is a faithful proxy: prism's source ranges are
/// the exact byte span of the corresponding AST node, and two identical-looking
/// expressions render to the same bytes. The structural comparison only differs
/// when the same bytes parse to different AST shapes, which we don't see in
/// the contexts stock cares about here (receivers, key arguments, attribute
/// receivers). See doc on `ast_equal_node`.
fn ast_equal(a: Option<Node<'_>>, b: Option<Node<'_>>, source: &[u8]) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => ast_equal_node(&x, &y, source),
        _ => false,
    }
}

fn ast_equal_node(a: &Node<'_>, b: &Node<'_>, source: &[u8]) -> bool {
    // Same prism node type AND identical source-range bytes ⇒ parser `==`
    // holds for the constructions we feed in (receivers / key arguments /
    // attribute receivers of `self_assignment`). We check the variant tag via
    // `discriminant` and the source-range slice via `source[s..e]`. The variant
    // check prevents false positives when two semantically-different bytes
    // happen to coincide (none in practice for our domain, but cheap to keep).
    if std::mem::discriminant(a) != std::mem::discriminant(b) {
        return false;
    }
    let la = a.location();
    let lb = b.location();
    let sa = &source[la.start_offset()..la.end_offset()];
    let sb = &source[lb.start_offset()..lb.end_offset()];
    sa == sb
}

impl<'pr, 's> Visit<'pr> for SelfAssignmentVisitor<'s> {
    fn visit_local_variable_write_node(&mut self, node: &LocalVariableWriteNode<'pr>) {
        self.check_lvasgn(node);
        ruby_prism::visit_local_variable_write_node(self, node);
    }
    fn visit_instance_variable_write_node(&mut self, node: &InstanceVariableWriteNode<'pr>) {
        self.check_ivasgn(node);
        ruby_prism::visit_instance_variable_write_node(self, node);
    }
    fn visit_class_variable_write_node(&mut self, node: &ClassVariableWriteNode<'pr>) {
        self.check_cvasgn(node);
        ruby_prism::visit_class_variable_write_node(self, node);
    }
    fn visit_global_variable_write_node(&mut self, node: &GlobalVariableWriteNode<'pr>) {
        self.check_gvasgn(node);
        ruby_prism::visit_global_variable_write_node(self, node);
    }
    fn visit_constant_write_node(&mut self, node: &ConstantWriteNode<'pr>) {
        self.check_casgn(node);
        ruby_prism::visit_constant_write_node(self, node);
    }
    fn visit_constant_path_write_node(&mut self, node: &ConstantPathWriteNode<'pr>) {
        self.check_const_path_asgn(node);
        ruby_prism::visit_constant_path_write_node(self, node);
    }
    fn visit_multi_write_node(&mut self, node: &MultiWriteNode<'pr>) {
        self.check_masgn(node);
        ruby_prism::visit_multi_write_node(self, node);
    }
    fn visit_local_variable_or_write_node(&mut self, node: &LocalVariableOrWriteNode<'pr>) {
        self.check_or_asgn(node);
        ruby_prism::visit_local_variable_or_write_node(self, node);
    }
    fn visit_local_variable_and_write_node(&mut self, node: &LocalVariableAndWriteNode<'pr>) {
        self.check_and_asgn(node);
        ruby_prism::visit_local_variable_and_write_node(self, node);
    }
    fn visit_instance_variable_or_write_node(&mut self, node: &InstanceVariableOrWriteNode<'pr>) {
        self.check_ivar_or_asgn(node);
        ruby_prism::visit_instance_variable_or_write_node(self, node);
    }
    fn visit_instance_variable_and_write_node(&mut self, node: &InstanceVariableAndWriteNode<'pr>) {
        self.check_ivar_and_asgn(node);
        ruby_prism::visit_instance_variable_and_write_node(self, node);
    }
    fn visit_class_variable_or_write_node(&mut self, node: &ClassVariableOrWriteNode<'pr>) {
        self.check_cvar_or_asgn(node);
        ruby_prism::visit_class_variable_or_write_node(self, node);
    }
    fn visit_class_variable_and_write_node(&mut self, node: &ClassVariableAndWriteNode<'pr>) {
        self.check_cvar_and_asgn(node);
        ruby_prism::visit_class_variable_and_write_node(self, node);
    }
    fn visit_global_variable_or_write_node(&mut self, node: &GlobalVariableOrWriteNode<'pr>) {
        self.check_gvar_or_asgn(node);
        ruby_prism::visit_global_variable_or_write_node(self, node);
    }
    fn visit_global_variable_and_write_node(&mut self, node: &GlobalVariableAndWriteNode<'pr>) {
        self.check_gvar_and_asgn(node);
        ruby_prism::visit_global_variable_and_write_node(self, node);
    }
    fn visit_constant_or_write_node(&mut self, node: &ConstantOrWriteNode<'pr>) {
        self.check_const_or_asgn(node);
        ruby_prism::visit_constant_or_write_node(self, node);
    }
    fn visit_constant_and_write_node(&mut self, node: &ConstantAndWriteNode<'pr>) {
        self.check_const_and_asgn(node);
        ruby_prism::visit_constant_and_write_node(self, node);
    }
    fn visit_call_or_write_node(&mut self, node: &CallOrWriteNode<'pr>) {
        self.check_call_or_asgn(node);
        ruby_prism::visit_call_or_write_node(self, node);
    }
    fn visit_call_and_write_node(&mut self, node: &CallAndWriteNode<'pr>) {
        self.check_call_and_asgn(node);
        ruby_prism::visit_call_and_write_node(self, node);
    }
    fn visit_index_or_write_node(&mut self, node: &IndexOrWriteNode<'pr>) {
        self.check_index_or_asgn(node);
        ruby_prism::visit_index_or_write_node(self, node);
    }
    fn visit_index_and_write_node(&mut self, node: &IndexAndWriteNode<'pr>) {
        self.check_index_and_asgn(node);
        ruby_prism::visit_index_and_write_node(self, node);
    }
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        self.check_send(node);
        ruby_prism::visit_call_node(self, node);
    }
}

impl<'s> super::dispatch::Rule for SelfAssignmentVisitor<'s> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(n) = node.as_local_variable_write_node() {
            self.check_lvasgn(&n);
        } else if let Some(n) = node.as_instance_variable_write_node() {
            self.check_ivasgn(&n);
        } else if let Some(n) = node.as_class_variable_write_node() {
            self.check_cvasgn(&n);
        } else if let Some(n) = node.as_global_variable_write_node() {
            self.check_gvasgn(&n);
        } else if let Some(n) = node.as_constant_write_node() {
            self.check_casgn(&n);
        } else if let Some(n) = node.as_constant_path_write_node() {
            self.check_const_path_asgn(&n);
        } else if let Some(n) = node.as_multi_write_node() {
            self.check_masgn(&n);
        } else if let Some(n) = node.as_local_variable_or_write_node() {
            self.check_or_asgn(&n);
        } else if let Some(n) = node.as_local_variable_and_write_node() {
            self.check_and_asgn(&n);
        } else if let Some(n) = node.as_instance_variable_or_write_node() {
            self.check_ivar_or_asgn(&n);
        } else if let Some(n) = node.as_instance_variable_and_write_node() {
            self.check_ivar_and_asgn(&n);
        } else if let Some(n) = node.as_class_variable_or_write_node() {
            self.check_cvar_or_asgn(&n);
        } else if let Some(n) = node.as_class_variable_and_write_node() {
            self.check_cvar_and_asgn(&n);
        } else if let Some(n) = node.as_global_variable_or_write_node() {
            self.check_gvar_or_asgn(&n);
        } else if let Some(n) = node.as_global_variable_and_write_node() {
            self.check_gvar_and_asgn(&n);
        } else if let Some(n) = node.as_constant_or_write_node() {
            self.check_const_or_asgn(&n);
        } else if let Some(n) = node.as_constant_and_write_node() {
            self.check_const_and_asgn(&n);
        } else if let Some(n) = node.as_call_or_write_node() {
            self.check_call_or_asgn(&n);
        } else if let Some(n) = node.as_call_and_write_node() {
            self.check_call_and_asgn(&n);
        } else if let Some(n) = node.as_index_or_write_node() {
            self.check_index_or_asgn(&n);
        } else if let Some(n) = node.as_index_and_write_node() {
            self.check_index_and_asgn(&n);
        } else if let Some(call) = node.as_call_node() {
            self.check_send(&call);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(src: &str) -> Vec<(usize, usize)> {
        check_self_assignment(src.as_bytes())
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset))
            .collect()
    }

    #[test]
    fn flags_local_variable_self_assignment() {
        assert_eq!(detect("foo = foo\n"), vec![(0, 9)]);
    }

    #[test]
    fn flags_instance_variable() {
        assert_eq!(detect("@foo = @foo\n"), vec![(0, 11)]);
    }

    #[test]
    fn flags_class_variable() {
        assert_eq!(detect("@@foo = @@foo\n"), vec![(0, 13)]);
    }

    #[test]
    fn flags_global_variable() {
        assert_eq!(detect("$foo = $foo\n"), vec![(0, 11)]);
    }

    #[test]
    fn flags_bare_constant() {
        assert_eq!(detect("Foo = Foo\n"), vec![(0, 9)]);
    }

    #[test]
    fn ignores_constant_from_another_scope() {
        assert!(detect("Foo = ::Foo\n").is_empty());
    }

    #[test]
    fn flags_masgn_pair() {
        assert_eq!(detect("foo, bar = foo, bar\n"), vec![(0, 19)]);
    }

    #[test]
    fn flags_masgn_through_array() {
        assert_eq!(detect("foo, bar = [foo, bar]\n"), vec![(0, 21)]);
    }

    #[test]
    fn ignores_masgn_with_splat() {
        assert!(detect("foo, bar = *something\n").is_empty());
    }

    #[test]
    fn ignores_masgn_with_method_call() {
        assert!(detect("foo, bar = something\n").is_empty());
    }

    #[test]
    fn ignores_masgn_const_pair() {
        // ASSIGNMENT_TYPE_TO_RHS_TYPE has no entry for casgn → no offense.
        assert!(detect("Foo, Bar = Foo, Bar\n").is_empty());
    }

    #[test]
    fn flags_or_asgn() {
        assert_eq!(detect("foo ||= foo\n"), vec![(0, 11)]);
    }

    #[test]
    fn flags_and_asgn() {
        assert_eq!(detect("foo &&= foo\n"), vec![(0, 11)]);
    }

    #[test]
    fn flags_attribute_assignment() {
        assert_eq!(detect("foo.bar = foo.bar\n"), vec![(0, 17)]);
    }

    #[test]
    fn flags_attribute_assignment_csend() {
        assert_eq!(detect("foo&.bar = foo&.bar\n"), vec![(0, 19)]);
    }

    #[test]
    fn ignores_attribute_different_attr() {
        assert!(detect("foo.bar = foo.baz\n").is_empty());
    }

    #[test]
    fn ignores_attribute_different_receiver() {
        assert!(detect("bar.foo = baz.foo\n").is_empty());
    }

    #[test]
    fn ignores_attribute_with_extra() {
        assert!(detect("foo.bar = foo.bar + 1\n").is_empty());
    }

    #[test]
    fn flags_index_assignment_string() {
        assert_eq!(detect("foo[\"bar\"] = foo[\"bar\"]\n"), vec![(0, 23)]);
    }

    #[test]
    fn ignores_index_assignment_diff_string() {
        assert!(detect("foo[\"bar\"] = foo[\"baz\"]\n").is_empty());
    }

    #[test]
    fn flags_index_assignment_int() {
        assert_eq!(detect("foo[1] = foo[1]\n"), vec![(0, 15)]);
    }

    #[test]
    fn flags_index_assignment_float() {
        assert_eq!(detect("foo[1.2] = foo[1.2]\n"), vec![(0, 19)]);
    }

    #[test]
    fn flags_index_assignment_const() {
        assert_eq!(detect("foo[Foo] = foo[Foo]\n"), vec![(0, 19)]);
    }

    #[test]
    fn flags_index_assignment_sym() {
        assert_eq!(detect("foo[:bar] = foo[:bar]\n"), vec![(0, 21)]);
    }

    #[test]
    fn flags_index_assignment_local_var() {
        let src = "var = 1\nfoo[var] = foo[var]\n";
        let off = detect(src);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].0, 8);
    }

    #[test]
    fn ignores_index_assignment_method_call_key() {
        // `foo[bar] = foo[bar]` where `bar` is a method call — stock excludes.
        assert!(detect("foo[bar] = foo[bar]\n").is_empty());
    }

    #[test]
    fn flags_index_assignment_safe_navigation() {
        let off = detect("foo&.[]=(\"bar\", foo[\"bar\"])\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_index_assignment_multi_args() {
        assert_eq!(detect("matrix[1, 2] = matrix[1, 2]\n"), vec![(0, 27)]);
    }

    #[test]
    fn flags_index_assignment_zero_args() {
        assert_eq!(detect("singleton[] = singleton[]\n"), vec![(0, 25)]);
    }

    #[test]
    fn ignores_index_assignment_no_args_send() {
        assert!(detect("foo.[]=\n").is_empty());
    }

    #[test]
    fn flags_constant_path_self_assignment() {
        // `A::B = A::B` — namespace and short_name match.
        assert_eq!(detect("A::B = A::B\n"), vec![(0, 11)]);
    }

    #[test]
    fn rbs_anchor_for_lvasgn_is_rhs_end() {
        let off = check_self_assignment(b"foo = foo\n");
        assert_eq!(off[0].rbs_anchor_offset, 9); // end of rhs `foo`
    }

    #[test]
    fn rbs_anchor_for_masgn_is_first_lhs_end() {
        // `foo, bar = foo, bar` — first lhs `foo` ends at offset 3.
        let off = check_self_assignment(b"foo, bar = foo, bar\n");
        assert_eq!(off[0].rbs_anchor_offset, 3);
    }

    #[test]
    fn rbs_anchor_for_or_asgn_is_lhs_end() {
        // `foo ||= foo` — lhs name `foo` ends at offset 3.
        let off = check_self_assignment(b"foo ||= foo\n");
        assert_eq!(off[0].rbs_anchor_offset, 3);
    }

    #[test]
    fn rbs_anchor_for_attribute_is_receiver_end() {
        // `foo.bar = foo.bar` — receiver `foo` ends at offset 3.
        let off = check_self_assignment(b"foo.bar = foo.bar\n");
        assert_eq!(off[0].rbs_anchor_offset, 3);
    }
}

