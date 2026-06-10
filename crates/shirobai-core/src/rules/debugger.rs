use ruby_prism::{Node, Visit, parse};

pub struct DebuggerOffense {
    pub start_offset: usize,
    pub end_offset: usize,
}

/// Detect debugger entry points (`binding.pry`, `byebug`, ...) and return the
/// byte range RuboCop would mark as an offense.
///
/// `methods` is the already-flattened `DebuggerMethods` list (chained names
/// such as `binding.pry` or `Foo::Bar::Baz.debug`). `requires` is the
/// flattened `DebuggerRequires` list (e.g. `debug/start`).
pub fn check_debugger(
    source: &[u8],
    methods: &[String],
    requires: &[String],
) -> Vec<DebuggerOffense> {
    let result = parse(source);
    let node = result.node();
    let mut visitor = DebuggerVisitor {
        source,
        methods,
        requires,
        stack: Vec::new(),
        offenses: Vec::new(),
    };
    visitor.visit(&node);
    visitor.offenses
}

/// Coarse classification of an ancestor node, mirroring the predicates RuboCop's
/// `assumed_usage_context?` relies on (`call_type?`, `literal?`, `pair_type?`,
/// `any_block`, `kwbegin`, `lambda_or_proc?`).
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Call,
    Block,
    Lambda,
    Begin,
    Args,
    Pair,
    Literal,
    Other,
}

fn kind_of(node: &Node<'_>) -> Kind {
    match node {
        Node::CallNode { .. } => Kind::Call,
        Node::BlockNode { .. } => Kind::Block,
        Node::LambdaNode { .. } => Kind::Lambda,
        Node::BeginNode { .. } => Kind::Begin,
        Node::ArgumentsNode { .. } => Kind::Args,
        Node::AssocNode { .. } => Kind::Pair,
        Node::ArrayNode { .. }
        | Node::HashNode { .. }
        | Node::InterpolatedStringNode { .. }
        | Node::InterpolatedXStringNode { .. }
        | Node::InterpolatedSymbolNode { .. }
        | Node::InterpolatedRegularExpressionNode { .. }
        | Node::RangeNode { .. } => Kind::Literal,
        _ => Kind::Other,
    }
}

struct DebuggerVisitor<'a> {
    source: &'a [u8],
    methods: &'a [String],
    requires: &'a [String],
    /// Ancestor kinds of the node currently being visited (self not included).
    stack: Vec<Kind>,
    offenses: Vec<DebuggerOffense>,
}

impl DebuggerVisitor<'_> {
    /// Reconstruct the dotted call name the way RuboCop's `chained_method_name`
    /// does: a `CallNode` receiver contributes its method name, anything else
    /// (constant paths) contributes its source text with a leading `::` removed.
    fn chained_method_name(&self, call: &ruby_prism::CallNode<'_>) -> String {
        let mut name = String::from_utf8_lossy(call.name().as_slice()).into_owned();
        let mut receiver = call.receiver();
        while let Some(recv) = receiver {
            if let Some(inner) = recv.as_call_node() {
                let part = String::from_utf8_lossy(inner.name().as_slice());
                name = format!("{part}.{name}");
                receiver = inner.receiver();
            } else {
                let loc = recv.location();
                let src =
                    String::from_utf8_lossy(&self.source[loc.start_offset()..loc.end_offset()]);
                let part = src.trim_start_matches("::");
                name = format!("{part}.{name}");
                receiver = None;
            }
        }
        name
    }

    /// End offset of the offense range. A trailing literal block (`do..end` /
    /// `{ }`) is excluded, matching RuboCop's `send` node which does not span
    /// its block.
    fn offense_end(&self, call: &ruby_prism::CallNode<'_>) -> usize {
        if let Some(block) = call.block()
            && let Some(block_node) = block.as_block_node()
        {
            let start = call.location().start_offset();
            let mut end = block_node.location().start_offset();
            while end > start && self.source[end - 1].is_ascii_whitespace() {
                end -= 1;
            }
            return end;
        }
        call.location().end_offset()
    }

    fn has_arguments(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        call.arguments()
            .is_some_and(|args| !args.arguments().is_empty())
    }

    fn debugger_method(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        let name = self.chained_method_name(call);
        self.methods.iter().any(|m| m == &name)
    }

    /// Port of RuboCop's `debugger_require?`: a `require` call with a single
    /// string-literal argument whose value is in `DebuggerRequires`.
    fn debugger_require(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        if call.name().as_slice() != b"require" {
            return false;
        }
        let Some(args) = call.arguments() else {
            return false;
        };
        let args = args.arguments();
        if args.len() != 1 {
            return false;
        }
        let Some(arg) = args.iter().next() else {
            return false;
        };
        let Some(string) = arg.as_string_node() else {
            return false;
        };
        let value = string.unescaped();
        self.requires.iter().any(|r| r.as_bytes() == value)
    }

    /// Nearest ancestor kind, skipping Prism's `ArgumentsNode` wrapper so that a
    /// call argument reports the call itself as its parent (as in the
    /// parser-gem AST RuboCop sees).
    fn effective_parent(&self) -> Kind {
        for kind in self.stack.iter().rev() {
            if *kind == Kind::Args {
                continue;
            }
            return *kind;
        }
        Kind::Other
    }

    /// Port of RuboCop's `assumed_argument?`: the call sits directly inside a
    /// call, a literal, or a pair, so it is almost certainly an argument.
    fn assumed_argument(&self) -> bool {
        matches!(
            self.effective_parent(),
            Kind::Call | Kind::Literal | Kind::Pair
        )
    }

    /// Port of RuboCop's `assumed_usage_context?`. A no-argument debugger-named
    /// call nested in another call is assumed to be used as an argument unless a
    /// block/lambda/`begin` ancestor proves it is a real statement.
    fn assumed_usage_context(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        let any_call_ancestor = self.stack.contains(&Kind::Call);
        if self.has_arguments(call) || !any_call_ancestor {
            return false;
        }
        if self.assumed_argument() {
            return true;
        }
        !self
            .stack
            .iter()
            .any(|k| matches!(k, Kind::Block | Kind::Lambda | Kind::Begin))
    }

    fn check_call(&mut self, call: &ruby_prism::CallNode<'_>) {
        if self.assumed_usage_context(call) {
            return;
        }
        // RuboCop's `Lint/Debugger` only defines `on_send`, not `on_csend`, so
        // safe-navigation calls are never reported.
        if call.is_safe_navigation() {
            return;
        }
        if self.debugger_method(call) || self.debugger_require(call) {
            let loc = call.location();
            self.offenses.push(DebuggerOffense {
                start_offset: loc.start_offset(),
                end_offset: self.offense_end(call),
            });
        }
    }
}

impl<'pr> Visit<'pr> for DebuggerVisitor<'_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        if let Some(call) = node.as_call_node() {
            self.check_call(&call);
        }
        self.stack.push(kind_of(&node));
    }

    fn visit_branch_node_leave(&mut self) {
        self.stack.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(source: &str, methods: &[String]) -> Vec<(usize, usize)> {
        ranges_req(source, methods, &[])
    }

    fn ranges_req(source: &str, methods: &[String], requires: &[String]) -> Vec<(usize, usize)> {
        check_debugger(source.as_bytes(), methods, requires)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset))
            .collect()
    }

    fn m(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn default_methods() -> Vec<String> {
        m(&[
            "binding.irb",
            "Kernel.binding.irb",
            "byebug",
            "Kernel.byebug",
            "binding.pry",
            "Pry.rescue",
            "pry",
            "debugger",
            "Kernel.debugger",
            "jard",
            "binding.console",
        ])
    }

    // Typical: a plain method call matches.
    #[test]
    fn plain_method() {
        assert_eq!(ranges("byebug", &default_methods()), vec![(0, 6)]);
    }

    // Typical: no debugger call at all.
    #[test]
    fn no_match() {
        assert!(ranges("puts 'hello'", &default_methods()).is_empty());
    }

    // Typical: a call with arguments still spans the whole call.
    #[test]
    fn with_arguments() {
        assert_eq!(ranges("byebug foo", &default_methods()), vec![(0, 10)]);
    }

    // Typical: a chained receiver (`binding.pry`).
    #[test]
    fn chained_receiver() {
        assert_eq!(ranges("binding.pry", &default_methods()), vec![(0, 11)]);
    }

    // Typical: a constant receiver chain (`Kernel.binding.irb`).
    #[test]
    fn const_receiver_chain() {
        assert_eq!(
            ranges("Kernel.binding.irb", &default_methods()),
            vec![(0, 18)]
        );
    }

    // A constant path chain (`Foo::Bar::Baz.debug`).
    #[test]
    fn const_path_chain_config() {
        assert_eq!(
            ranges("Foo::Bar::Baz.debug", &m(&["Foo::Bar::Baz.debug"])),
            vec![(0, 19)]
        );
    }

    // A cbase constant (`::Kernel.debugger`) drops the leading `::` when matching.
    #[test]
    fn cbase_constant() {
        assert_eq!(
            ranges("::Kernel.debugger", &default_methods()),
            vec![(0, 17)]
        );
    }

    // A trailing literal block is excluded from the offense range.
    #[test]
    fn block_excluded_from_range() {
        assert_eq!(
            ranges("Pry.rescue { puts 1 }", &default_methods()),
            vec![(0, 10)]
        );
    }

    // Comments are ignored by the parser.
    #[test]
    fn comment_ignored() {
        assert!(ranges("# byebug", &default_methods()).is_empty());
    }

    // --- assumed_usage_context ---

    // A no-argument debugger call assigned to something is treated as an argument.
    #[test]
    fn assignment_is_skipped() {
        assert!(ranges("x.y = custom_debugger", &m(&["custom_debugger"])).is_empty());
    }

    // `p` as a receiver of another call is not a debug call.
    #[test]
    fn receiver_usage_skipped() {
        assert!(ranges("p.do_something", &m(&["p"])).is_empty());
    }

    // `p` passed as a positional argument is skipped.
    #[test]
    fn positional_argument_skipped() {
        assert!(ranges("do_something(p)", &m(&["p"])).is_empty());
    }

    // `p` inside an array argument is skipped (parent is a literal).
    #[test]
    fn array_argument_skipped() {
        assert!(ranges("do_something([k, p])", &m(&["p"])).is_empty());
    }

    // `p` as a keyword argument value is skipped (parent is a pair).
    #[test]
    fn keyword_argument_skipped() {
        assert!(ranges("do_something(k: p)", &m(&["p"])).is_empty());
    }

    // A debugger call inside a block is a real usage and is reported.
    #[test]
    fn inside_block_reported() {
        assert_eq!(
            ranges("do_something { custom_debugger }", &m(&["custom_debugger"])).len(),
            1
        );
    }

    // A debugger call inside an explicit `begin` argument is reported.
    #[test]
    fn inside_begin_reported() {
        let src = "do_something(\n  begin\n    custom_debugger\n  end\n)";
        assert_eq!(ranges(src, &m(&["custom_debugger"])).len(), 1);
    }

    // A debugger call with arguments is always reported, even as an argument.
    #[test]
    fn argument_with_args_reported() {
        assert_eq!(ranges("p 'foo'", &m(&["p"])), vec![(0, 7)]);
    }

    // A top-level debugger call has no call ancestor and is reported.
    #[test]
    fn top_level_reported() {
        assert_eq!(ranges("custom_debugger", &m(&["custom_debugger"])).len(), 1);
    }

    // Safe-navigation calls are not reported (no `on_csend`).
    #[test]
    fn safe_navigation_not_reported() {
        assert!(ranges("binding&.pry", &default_methods()).is_empty());
    }

    // --- DebuggerRequires ---

    // A `require` of a configured debugger entry point is reported.
    #[test]
    fn require_reported() {
        assert_eq!(
            ranges_req("require 'my_debugger'", &[], &m(&["my_debugger"])),
            vec![(0, 21)]
        );
    }

    // `require` without arguments is not a debugger require.
    #[test]
    fn require_without_arguments() {
        assert!(ranges_req("require", &[], &m(&["my_debugger"])).is_empty());
    }

    // `require` with multiple arguments is not a debugger require.
    #[test]
    fn require_multiple_arguments() {
        assert!(ranges_req("require 'my_debugger', 'x'", &[], &m(&["my_debugger"])).is_empty());
    }

    // `require` with a non-string argument is not a debugger require.
    #[test]
    fn require_non_string_argument() {
        assert!(ranges_req("require my_debugger", &[], &m(&["my_debugger"])).is_empty());
    }

    // A `require` of an unlisted path is not reported.
    #[test]
    fn require_unlisted() {
        assert!(ranges_req("require 'other'", &[], &m(&["my_debugger"])).is_empty());
    }
}
