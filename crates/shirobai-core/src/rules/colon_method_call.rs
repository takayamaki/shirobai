//! `Style/ColonMethodCall`: flags a method call dispatched with `::` instead of
//! `.` and rewrites the `::` to `.`.
//!
//! Mirrors `vendor/rubocop/lib/rubocop/cop/style/colon_method_call.rb`:
//!
//! - On a `send`/`csend` with `node.receiver && node.double_colon?` —
//!   in prism, the call has `Some` `receiver()` and `Some` `call_operator_loc()`
//!   whose source is the two bytes `::`. (csend always uses `&.` and never
//!   matches `::`, so `on_send` and `on_csend` coalesce.)
//! - Skip when `node.camel_case_method?` — `method_name.to_s =~ /\A[A-Z]/`.
//!   That's the `Tip::Top(arg)` constructor-style call which looks like a
//!   constant reference at the call site (`Top(...)` would be `Integer(x)` /
//!   `String(x)`).
//! - Skip when `java_type_node?(node)` — stock pattern
//!   `(send (const nil? :Java) _)`: the receiver is a top-level
//!   `ConstantReadNode` named `Java`. The pattern's `nil?` matches a nil
//!   receiver on the `const`, NOT a `cbase` receiver (`::Java::int` is FLAGGED
//!   by stock; verified by probe). So in prism the gate is
//!   `receiver.as_constant_read_node()` whose `name() == "Java"`. A
//!   `ConstantPathNode` (`::Java`) is a different prism node and structurally
//!   excluded.
//!
//! Offense range = `node.loc.dot` = the prism `call_operator_loc()` range
//! (`::`, two bytes). Autocorrect replaces those two bytes with `.`.

use ruby_prism::{Node, Visit};

#[derive(Debug, Clone)]
pub struct ColonMethodCallOffense {
    /// Start byte of the `::` token (offense highlight and autocorrect replace
    /// range begin).
    pub dot_start: usize,
    /// End byte of the `::` token. Always `dot_start + 2` for a well-formed
    /// call, but the wrapper takes both ends to avoid hardcoding the length on
    /// the Ruby side.
    pub dot_end: usize,
}

/// Standalone entry point used by the per-cop fallback. This cop is always
/// `bundle_eligible?` (config-less), so this path is exercised by tests only.
pub fn check_colon_method_call(source: &[u8]) -> Vec<ColonMethodCallOffense> {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
///
/// `Style/ColonMethodCall` carries no per-cop config and reads no source bytes
/// outside the AST locations, so the builder takes no arguments.
pub(crate) fn build_rule() -> ColonMethodCallVisitor {
    ColonMethodCallVisitor {
        offenses: Vec::new(),
    }
}

pub(crate) struct ColonMethodCallVisitor {
    pub(crate) offenses: Vec<ColonMethodCallOffense>,
}

impl ColonMethodCallVisitor {
    fn check_call(&mut self, call: &ruby_prism::CallNode<'_>) {
        // `node.receiver` — must be present (`foo` alone has no receiver and
        // can't use `::`).
        if call.receiver().is_none() {
            return;
        }

        // `node.double_colon?` — `loc.dot` source is `::`. prism's
        // `call_operator_loc()` is `Some` whenever a connecting token exists;
        // its source is `.`, `::`, or `&.`. csend (`&.`) and dot (`.`) calls
        // fall through here.
        let Some(op_loc) = call.call_operator_loc() else {
            return;
        };
        let op_start = op_loc.start_offset();
        let op_end = op_loc.end_offset();
        if op_end - op_start != 2 {
            return;
        }
        // The `call_operator_loc().slice()` is `::` exactly. A two-byte token
        // here can only be `::` or `&.` — but `&.` would be a CSendNode
        // analogue, prism encodes it in `call_operator_loc` too. We still gate
        // on the bytes to be safe.
        // SAFETY: byte-level comparison is correct for ASCII operator tokens.
        // The op_loc bytes are guaranteed in-bounds by prism.
        let bytes = op_loc.as_slice();
        if bytes != b"::" {
            return;
        }

        // `node.camel_case_method?` — `method_name.to_s =~ /\A[A-Z]/`. The
        // method name as bytes; the regex anchors at start so we only check the
        // first byte. ASCII-uppercase-start covers the `Integer(x)`-style
        // constructor names which look like constant references at the call
        // site.
        let name = call.name();
        let name = name.as_slice();
        if name.is_empty() {
            return;
        }
        if name[0].is_ascii_uppercase() {
            return;
        }

        // `java_type_node?(node)` — stock pattern `(send (const nil? :Java) _)`.
        // The receiver is a top-level `ConstantReadNode` whose name is `Java`.
        // `ConstantPathNode` (the `cbase` form `::Java`) is a different node
        // kind and is correctly NOT excluded — `::Java::int` is flagged by
        // stock.
        let receiver = call.receiver().expect("receiver presence checked above");
        if let Some(const_read) = receiver.as_constant_read_node()
            && const_read.name().as_slice() == b"Java"
        {
            return;
        }

        self.offenses.push(ColonMethodCallOffense {
            dot_start: op_start,
            dot_end: op_end,
        });
    }
}

impl<'pr> Visit<'pr> for ColonMethodCallVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        // Recurse so nested calls (a call's arguments, a block body, etc.)
        // are also checked.
        ruby_prism::visit_call_node(self, node);
    }
}

impl super::dispatch::Rule for ColonMethodCallVisitor {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.check_call(&call);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(src: &str) -> Vec<(usize, usize)> {
        check_colon_method_call(src.as_bytes())
            .into_iter()
            .map(|o| (o.dot_start, o.dot_end))
            .collect()
    }

    #[test]
    fn flags_instance_double_colon_call() {
        // `test::method_name` — receiver=test (call w/o args), name=method_name
        // (lower), `::` at 4..6.
        let off = detect("test::method_name\n");
        assert_eq!(off, vec![(4, 6)]);
    }

    #[test]
    fn flags_const_double_colon_call() {
        // `Class::method_name` — receiver=ConstantReadNode(Class), `::` at 5..7.
        let off = detect("Class::method_name\n");
        assert_eq!(off, vec![(5, 7)]);
    }

    #[test]
    fn flags_const_double_colon_call_with_arg() {
        // `Class::method_name(arg, arg2)` — args present, still flagged.
        let off = detect("Class::method_name(arg, arg2)\n");
        assert_eq!(off, vec![(5, 7)]);
    }

    #[test]
    fn flags_double_const_path_then_method() {
        // `Foo::Bar::baz` — receiver=ConstantPathNode(Foo::Bar), `::` at 8..10.
        let off = detect("Foo::Bar::baz\n");
        assert_eq!(off, vec![(8, 10)]);
    }

    #[test]
    fn accepts_constant_access_only() {
        // `Tip::Top::SOME_CONST` — all ConstantPathNode; no CallNode.
        assert!(detect("Tip::Top::SOME_CONST\n").is_empty());
    }

    #[test]
    fn accepts_dot_call() {
        // `Tip::Top.some_method` — dot, not double colon.
        assert!(detect("Tip::Top.some_method\n").is_empty());
    }

    #[test]
    fn accepts_op_method_after_chain() {
        // `Tip::Top.some_method[3]` — outer is `[]` op call, no `::` at the
        // call boundary.
        assert!(detect("Tip::Top.some_method[3]\n").is_empty());
    }

    #[test]
    fn accepts_camel_case_method_constructor() {
        // `Tip::Top(some_arg)` — name=Top (uppercase), excluded.
        assert!(detect("Tip::Top(some_arg)\n").is_empty());
    }

    #[test]
    fn accepts_java_static_type() {
        // `Java::int` — receiver=ConstantReadNode(Java), excluded.
        assert!(detect("Java::int\n").is_empty());
    }

    #[test]
    fn accepts_java_package_namespace() {
        // `Java::com` — receiver=ConstantReadNode(Java), excluded.
        assert!(detect("Java::com\n").is_empty());
    }

    #[test]
    fn flags_after_java_package_chain() {
        // `Java::com.foo` — `Java::com` is excluded (java type node); `.foo`
        // uses `.` not `::`, so nothing flagged.
        assert!(detect("Java::com.foo\n").is_empty());
    }

    #[test]
    fn flags_when_cbase_java_present() {
        // `::Java::int` — receiver of `int` is `ConstantPathNode` (cbase form),
        // NOT a `ConstantReadNode` — so the java-type guard does NOT apply.
        // Stock flags this. `::` at 6..8.
        let off = detect("::Java::int\n");
        assert_eq!(off, vec![(6, 8)]);
    }

    #[test]
    fn flags_after_java_then_user_call() {
        // `Java::foo::bar` — inner `Java::foo` is java-typed (excluded), but
        // outer `bar` (receiver = `Java::foo` CallNode) is flagged. `::` at
        // 9..11.
        let off = detect("Java::foo::bar\n");
        assert_eq!(off, vec![(9, 11)]);
    }

    #[test]
    fn flags_ivar_receiver() {
        // `@x::y` — receiver=InstanceVariableReadNode, `::` at 2..4.
        let off = detect("@x::y\n");
        assert_eq!(off, vec![(2, 4)]);
    }

    #[test]
    fn flags_paren_expr_receiver() {
        // `(1+2)::to_s` — receiver=ParenthesesNode, `::` at 5..7.
        let off = detect("(1+2)::to_s\n");
        assert_eq!(off, vec![(5, 7)]);
    }

    #[test]
    fn flags_inside_def_body() {
        let off = detect("def m\n  test::method_name\nend\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn does_not_flag_safe_navigation() {
        // `foo&.bar` — `&.`, not `::`.
        assert!(detect("foo&.bar\n").is_empty());
    }
}
