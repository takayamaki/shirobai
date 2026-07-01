//! `Style/RedundantSelfAssignment`: flags assignments where the rhs is a
//! destructive in-place method call on the same lvalue (e.g. `foo = foo.concat(ary)`
//! → `foo.concat(ary)`, `obj.foo = obj.foo.concat(ary)` → `obj.foo.concat(ary)`).
//!
//! Mirrors `vendor/rubocop/lib/rubocop/cop/style/redundant_self_assignment.rb`.
//! Two detection arms, both producing the same `=` operator offense range:
//!
//! * **variable assignment** (`lvasgn` / `ivasgn` / `cvasgn` / `gvasgn`):
//!   `var = var.<METHOD!>(...)` where `<METHOD!>` ∈ `METHODS_RETURNING_SELF` and
//!   the rhs receiver re-reads the same variable. Autocorrect replaces the
//!   whole node with the rhs source (i.e. drops `var = `).
//! * **setter assignment** (`obj.foo=` / `obj&.foo=`): pattern
//!   `(call obj :foo= (call (call obj :foo) <METHOD!> ...))`. Stock's matcher
//!   also matches a bare `(self).foo =` setter receiver, but stock's
//!   `on_send` returns early when `node.receiver` is `self`-typed, so the only
//!   live receiver shape is "non-`self` send/csend with same source as the rhs
//!   inner receiver". Autocorrect removes `[node_start, first_arg_start)` (the
//!   `obj.foo = ` prefix), leaving the rhs in place.
//!
//! The set of "destructive" methods is a closed list copied verbatim from
//! stock; the Ruby wrapper carries the offense end position and a method-name
//! pointer (offset into the source) so the message format string can quote
//! the exact method name without round-tripping it through magnus.

use ruby_prism::{
    CallNode, GlobalVariableWriteNode, InstanceVariableWriteNode, ClassVariableWriteNode,
    LocalVariableWriteNode, Node, Visit,
};

/// One offense. `op_start..op_end` is the offense highlight (the `=` operator
/// for variable assignment, the lhs's `=` token for setter assignment).
///
/// `method_name_start..method_name_end` points at the destructive method name
/// inside the source (e.g. `concat`), used by the Ruby wrapper to format the
/// `MSG` string without copying.
///
/// `kind` is `0` for variable-assignment-style autocorrect (replace the entire
/// `range_start..range_end` with `rhs_start..rhs_end` source bytes) and `1`
/// for setter-style autocorrect (delete `range_start..range_end` outright;
/// `rhs_*` fields are unused / zero for this kind).
pub struct RedundantSelfAssignmentOffense {
    pub op_start: usize,
    pub op_end: usize,
    pub method_name_start: usize,
    pub method_name_end: usize,
    pub kind: u8,
    pub range_start: usize,
    pub range_end: usize,
    pub rhs_start: usize,
    pub rhs_end: usize,
}

/// Standalone entry point used by the per-cop fallback (the bundle is the
/// usual path).
pub fn check_redundant_self_assignment(source: &[u8]) -> Vec<RedundantSelfAssignmentOffense> {
    let mut visitor = build_rule(source);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

pub(crate) fn build_rule(source: &[u8]) -> RedundantSelfAssignmentVisitor<'_> {
    RedundantSelfAssignmentVisitor {
        source,
        offenses: Vec::new(),
    }
}

pub(crate) struct RedundantSelfAssignmentVisitor<'s> {
    source: &'s [u8],
    pub(crate) offenses: Vec<RedundantSelfAssignmentOffense>,
}

impl<'s> RedundantSelfAssignmentVisitor<'s> {
    /// `var = var.<METHOD!>(...)`, shared across the four variable-write
    /// flavors (the same `node.loc.operator` + `corrector.replace(node, rhs.source)`
    /// shape stock uses for `on_lvasgn`). Takes one argument per slot stock's
    /// node accessors expose (location bounds + lhs/rhs names + the rhs node);
    /// regrouping them into a struct would shave one `clippy::too_many_arguments`
    /// without changing behaviour, so we keep the flat form for clarity.
    #[allow(clippy::too_many_arguments)]
    fn check_variable_assignment(
        &mut self,
        node_start: usize,
        node_end: usize,
        op_start: usize,
        op_end: usize,
        lhs_name: &[u8],
        rhs_node: Node<'_>,
        rhs_receiver_name: &[u8],
    ) {
        // Stock matches `rhs.type?(:any_block, :call)`. In prism a call with a
        // block (e.g. `foo.delete_if { … }`) is represented as a `CallNode`
        // whose `block` field is the `BlockNode`, *not* a standalone
        // `BlockNode` at the rhs slot — so a single `as_call_node()` check
        // covers both `:call` and `:any_block`.
        let Some(call) = rhs_node.as_call_node() else { return };
        if !methods_returning_self(call.name().as_slice()) {
            return;
        }
        // rhs must have a non-nil receiver (otherwise it can't match the lhs
        // variable read), and that receiver's *name* must equal the lhs name.
        if call.receiver().is_none() {
            return;
        }
        if rhs_receiver_name != lhs_name {
            return;
        }
        // Resolve the method-name source range: prism's `message_loc` on
        // CallNode points at the method-name token (excluding any `.` or `&.`
        // operator).
        let Some(name_loc) = call.message_loc() else { return };
        let (mn_start, mn_end) = (name_loc.start_offset(), name_loc.end_offset());
        let rhs_loc = rhs_node.location();
        self.offenses.push(RedundantSelfAssignmentOffense {
            op_start,
            op_end,
            method_name_start: mn_start,
            method_name_end: mn_end,
            kind: 0,
            range_start: node_start,
            range_end: node_end,
            rhs_start: rhs_loc.start_offset(),
            rhs_end: rhs_loc.end_offset(),
        });
    }

    fn check_lvasgn(&mut self, node: &LocalVariableWriteNode<'_>) {
        let value = node.value();
        // rhs receiver must be a `lvar` read of the same name. Get it before
        // dispatching to the shared checker.
        let call = match value.as_call_node() {
            Some(c) => c,
            None => return,
        };
        let Some(recv) = call.receiver() else { return };
        let Some(recv_lv) = recv.as_local_variable_read_node() else { return };
        let recv_name = recv_lv.name();
        let node_loc = node.location();
        let op_loc = node.operator_loc();
        self.check_variable_assignment(
            node_loc.start_offset(),
            node_loc.end_offset(),
            op_loc.start_offset(),
            op_loc.end_offset(),
            node.name().as_slice(),
            value,
            recv_name.as_slice(),
        );
    }

    fn check_ivasgn(&mut self, node: &InstanceVariableWriteNode<'_>) {
        let value = node.value();
        let call = match value.as_call_node() {
            Some(c) => c,
            None => return,
        };
        let Some(recv) = call.receiver() else { return };
        let Some(recv_iv) = recv.as_instance_variable_read_node() else { return };
        let node_loc = node.location();
        let op_loc = node.operator_loc();
        self.check_variable_assignment(
            node_loc.start_offset(),
            node_loc.end_offset(),
            op_loc.start_offset(),
            op_loc.end_offset(),
            node.name().as_slice(),
            value,
            recv_iv.name().as_slice(),
        );
    }

    fn check_cvasgn(&mut self, node: &ClassVariableWriteNode<'_>) {
        let value = node.value();
        let call = match value.as_call_node() {
            Some(c) => c,
            None => return,
        };
        let Some(recv) = call.receiver() else { return };
        let Some(recv_cv) = recv.as_class_variable_read_node() else { return };
        let node_loc = node.location();
        let op_loc = node.operator_loc();
        self.check_variable_assignment(
            node_loc.start_offset(),
            node_loc.end_offset(),
            op_loc.start_offset(),
            op_loc.end_offset(),
            node.name().as_slice(),
            value,
            recv_cv.name().as_slice(),
        );
    }

    fn check_gvasgn(&mut self, node: &GlobalVariableWriteNode<'_>) {
        let value = node.value();
        let call = match value.as_call_node() {
            Some(c) => c,
            None => return,
        };
        let Some(recv) = call.receiver() else { return };
        let Some(recv_gv) = recv.as_global_variable_read_node() else { return };
        let node_loc = node.location();
        let op_loc = node.operator_loc();
        self.check_variable_assignment(
            node_loc.start_offset(),
            node_loc.end_offset(),
            op_loc.start_offset(),
            op_loc.end_offset(),
            node.name().as_slice(),
            value,
            recv_gv.name().as_slice(),
        );
    }

    /// Setter side: `obj.foo = obj.foo.<METHOD!>(...)`.
    ///
    /// Pattern (stock's `def_node_matcher`):
    /// ```text
    /// (call %1 _ (call (call %1 %2) #method_returning_self? ...))
    /// ```
    /// where `%1` = outer receiver, `%2` = setter name minus the trailing `=`.
    /// In prism a `send`/`csend` is one `CallNode` discriminated by
    /// `call_operator_loc` (`&.` vs `.`). Stock's `on_send` returns early when
    /// `assignment_method?` is false; for the assignment-method case it also
    /// requires `node.receiver` to be non-`self` (otherwise it would skip — but
    /// matching `(call self _ ...)` against an rhs `(call (call <non-self>...))`
    /// can never succeed, so the explicit self-skip is folded into the
    /// receiver-equality check below).
    fn check_setter(&mut self, call: &CallNode<'_>) {
        let name_bytes = call.name();
        let name = name_bytes.as_slice();
        if !is_assignment_method(name) {
            return;
        }
        // Exactly one argument (rhs of `obj.foo = rhs`).
        let arguments: Vec<Node<'_>> = call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        if arguments.len() != 1 {
            return;
        }
        let rhs = &arguments[0];
        // Outer receiver: skip if absent (no `obj.foo=` shape without a
        // receiver in source — prism shouldn't synthesize one here, but bail
        // defensively).
        let Some(outer_recv) = call.receiver() else { return };
        // Stock's pattern uses `%1` to bind the outer receiver, then matches
        // it again against the inner receiver's receiver. `self`-typed outer
        // receivers (`self.foo = …`) round-trip as a `SelfNode`, never as a
        // `CallNode`, so they can't structurally equal the rhs inner receiver
        // shape we need (a `CallNode`). We don't have to special-case `self`.
        let Some(inner) = rhs.as_call_node() else { return };
        let inner_method = inner.name();
        if !methods_returning_self(inner_method.as_slice()) {
            return;
        }
        // Setter pattern only matches `:call` (NOT `:any_block`): a
        // block-wrapped rhs (`obj.foo = obj.foo.concat(ary) { … }`) becomes a
        // parser-gem `(block (send …) … …)` and falls out of stock's matcher.
        // In prism the BlockNode is attached as `CallNode.block`, so detect it
        // there and bail to match stock's no-offense behaviour. (The lvasgn
        // arm DOES allow blocks — stock's `on_lvasgn` matches `:any_block,
        // :call` — so this skip is setter-side only.)
        if inner.block().is_some() {
            return;
        }
        // `(call (call %1 %2) #returning ...)` — `inner_recv` is the
        // `(call %1 %2)` middle CallNode (e.g. `other.foo` in
        // `other.foo = other.foo.concat(ary)`).
        let Some(inner_recv) = inner.receiver() else { return };
        let Some(middle) = inner_recv.as_call_node() else { return };
        // `%2` = setter name minus the trailing `=`; it must equal
        // `middle.name` (e.g. setter `:foo=` → `foo` == middle name `foo`).
        let setter_stripped = &name[..name.len() - 1];
        if middle.name().as_slice() != setter_stripped {
            return;
        }
        // `%1` bound twice → outer receiver and middle's receiver must be
        // structurally equal.
        let Some(middle_recv) = middle.receiver() else { return };
        if !calls_equal_for_pattern(&outer_recv, middle_recv, self.source) {
            return;
        }

        // Offense: stock highlights `node.loc.operator` (the `=` of the setter).
        // In prism the `=` is *not* in `message_loc` — the message is just
        // `foo` (probed: `other.foo = …` has message_loc=[6,9], `=` at byte 10).
        // Scan between `message_loc.end` and the rhs start to find the `=`
        // byte; that's the operator highlight.
        let Some(msg_loc) = call.message_loc() else { return };
        let scan_start = msg_loc.end_offset();
        let scan_end = rhs.location().start_offset();
        let mut eq_pos: Option<usize> = None;
        let mut i = scan_start;
        while i < scan_end && i < self.source.len() {
            if self.source[i] == b'=' {
                eq_pos = Some(i);
                break;
            }
            i += 1;
        }
        let Some(eq_p) = eq_pos else { return };
        let (op_start, op_end) = (eq_p, eq_p + 1);

        let node_loc = call.location();
        let first_arg_loc = rhs.location();
        let Some(inner_method_loc) = inner.message_loc() else { return };
        let (mn_start, mn_end) = (
            inner_method_loc.start_offset(),
            inner_method_loc.end_offset(),
        );
        self.offenses.push(RedundantSelfAssignmentOffense {
            op_start,
            op_end,
            method_name_start: mn_start,
            method_name_end: mn_end,
            kind: 1,
            range_start: node_loc.start_offset(),
            range_end: first_arg_loc.start_offset(),
            rhs_start: 0,
            rhs_end: 0,
        });
    }
}

/// stock's `METHODS_RETURNING_SELF` set.
fn methods_returning_self(name: &[u8]) -> bool {
    matches!(
        name,
        b"append"
            | b"clear"
            | b"collect!"
            | b"compare_by_identity"
            | b"concat"
            | b"delete_if"
            | b"fill"
            | b"initialize_copy"
            | b"insert"
            | b"keep_if"
            | b"map!"
            | b"merge!"
            | b"prepend"
            | b"push"
            | b"rehash"
            | b"replace"
            | b"reverse!"
            | b"rotate!"
            | b"shuffle!"
            | b"sort!"
            | b"sort_by!"
            | b"transform_keys!"
            | b"transform_values!"
            | b"unshift"
            | b"update"
    )
}

fn is_assignment_method(name: &[u8]) -> bool {
    // `assignment_method?` from rubocop-ast: !comparison_method? && ends_with(=).
    if !name.ends_with(b"=") {
        return false;
    }
    !matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=" | b"[]=")
}

/// Compare two optional nodes for the purposes of the setter-pattern `%1`
/// binding. Stock matches with parser-gem deep `==`; for receivers that appear
/// here (`send`/`csend`/`const`/`lvar`/...) source-byte equality + same
/// discriminant is a faithful proxy (two identical-looking expressions render
/// to the same bytes and parse to the same prism node shape).
fn calls_equal_for_pattern(a: &Node<'_>, b: Node<'_>, source: &[u8]) -> bool {
    if std::mem::discriminant(a) != std::mem::discriminant(&b) {
        return false;
    }
    let la = a.location();
    let lb = b.location();
    let sa = &source[la.start_offset()..la.end_offset()];
    let sb = &source[lb.start_offset()..lb.end_offset()];
    sa == sb
}

impl<'pr, 's> Visit<'pr> for RedundantSelfAssignmentVisitor<'s> {
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
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        self.check_setter(node);
        ruby_prism::visit_call_node(self, node);
    }
}

impl<'s> super::dispatch::Rule for RedundantSelfAssignmentVisitor<'s> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_WRITE
                    | Interest::ENTER_CALL,
        )
    }
    
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(n) = node.as_local_variable_write_node() {
            self.check_lvasgn(&n);
        } else if let Some(n) = node.as_instance_variable_write_node() {
            self.check_ivasgn(&n);
        } else if let Some(n) = node.as_class_variable_write_node() {
            self.check_cvasgn(&n);
        } else if let Some(n) = node.as_global_variable_write_node() {
            self.check_gvasgn(&n);
        } else if let Some(call) = node.as_call_node() {
            self.check_setter(&call);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(src: &str) -> Vec<(usize, usize, u8)> {
        check_redundant_self_assignment(src.as_bytes())
            .into_iter()
            .map(|o| (o.op_start, o.op_end, o.kind))
            .collect()
    }

    #[test]
    fn flags_local_variable_self_assignment() {
        // `foo = foo.concat(ary)` — `=` at offset 4.
        assert_eq!(detect("foo = foo.concat(ary)\n"), vec![(4, 5, 0)]);
    }

    #[test]
    fn flags_instance_variable() {
        // `@foo = @foo.concat(ary)` — `=` at offset 5.
        assert_eq!(detect("@foo = @foo.concat(ary)\n"), vec![(5, 6, 0)]);
    }

    #[test]
    fn flags_class_variable() {
        // `@@foo = @@foo.concat(ary)` — `=` at offset 6.
        assert_eq!(detect("@@foo = @@foo.concat(ary)\n"), vec![(6, 7, 0)]);
    }

    #[test]
    fn flags_global_variable() {
        // `$foo = $foo.concat(ary)` — `=` at offset 5.
        assert_eq!(detect("$foo = $foo.concat(ary)\n"), vec![(5, 6, 0)]);
    }

    #[test]
    fn flags_safe_navigation_rhs() {
        // `foo = foo&.concat(ary)` — still matches.
        assert_eq!(detect("foo = foo&.concat(ary)\n"), vec![(4, 5, 0)]);
    }

    #[test]
    fn flags_block_on_rhs() {
        // `foo = foo.delete_if { true }` — rhs is a call-with-block but prism
        // still presents it as a CallNode (the block is attached). delete_if
        // is in METHODS_RETURNING_SELF.
        assert_eq!(detect("foo = foo.delete_if { true }\n"), vec![(4, 5, 0)]);
    }

    #[test]
    fn ignores_method_not_in_set() {
        // `upcase` is not destructive → no offense.
        assert!(detect("foo = foo.upcase\n").is_empty());
    }

    #[test]
    fn ignores_different_receiver() {
        assert!(detect("foo = bar.concat(ary)\n").is_empty());
    }

    #[test]
    fn ignores_no_receiver() {
        assert!(detect("foo = concat(ary)\n").is_empty());
    }

    #[test]
    fn ignores_self_setter_with_bare_rhs_receiver() {
        // `self.foo = foo.concat(ary)` — receiver mismatch (`self` ≠ `nil`).
        assert!(detect("self.foo = foo.concat(ary)\n").is_empty());
    }

    #[test]
    fn flags_non_self_setter() {
        // `other.foo = other.foo.concat(ary)` — receiver `other` matches.
        assert_eq!(detect("other.foo = other.foo.concat(ary)\n"), vec![(10, 11, 1)]);
    }

    #[test]
    fn flags_non_self_setter_safe_navigation_rhs() {
        assert_eq!(detect("other.foo = other.foo&.concat(ary)\n"), vec![(10, 11, 1)]);
    }

    #[test]
    fn flags_non_self_setter_safe_navigation_chain() {
        // `other&.foo = other&.foo&.concat(ary)` — outer receiver `other&.…`
        // (csend) must match inner inner receiver.
        assert_eq!(detect("other&.foo = other&.foo&.concat(ary)\n"), vec![(11, 12, 1)]);
    }

    #[test]
    fn ignores_setter_value_from_other_object() {
        assert!(detect("self.foo = bar.concat(ary)\n").is_empty());
    }

    #[test]
    fn ignores_casgn() {
        // CASGN isn't in stock's `on_*` aliases — no offense expected.
        assert!(detect("FOO = FOO.concat(ary)\n").is_empty());
    }
}
