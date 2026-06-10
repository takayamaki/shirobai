use ruby_prism::{Visit, parse, visit_call_node};

pub struct DebuggerOffense {
    pub start_offset: usize,
    pub end_offset: usize,
}

/// Detect debugger entry points (`binding.pry`, `byebug`, ...) and return the
/// byte range RuboCop would mark as an offense.
///
/// `methods` is the already-flattened `DebuggerMethods` list (chained names
/// such as `binding.pry` or `Foo::Bar::Baz.debug`).
pub fn check_debugger(source: &[u8], methods: &[String]) -> Vec<DebuggerOffense> {
    let result = parse(source);
    let node = result.node();
    let mut visitor = DebuggerVisitor {
        source,
        methods,
        offenses: Vec::new(),
    };
    visitor.visit(&node);
    visitor.offenses
}

struct DebuggerVisitor<'a> {
    source: &'a [u8],
    methods: &'a [String],
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
                let src = String::from_utf8_lossy(&self.source[loc.start_offset()..loc.end_offset()]);
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
        if let Some(block) = call.block() {
            if let Some(block_node) = block.as_block_node() {
                let start = call.location().start_offset();
                let mut end = block_node.location().start_offset();
                while end > start && self.source[end - 1].is_ascii_whitespace() {
                    end -= 1;
                }
                return end;
            }
        }
        call.location().end_offset()
    }

    fn debugger_method(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        let name = self.chained_method_name(call);
        self.methods.iter().any(|m| m == &name)
    }
}

impl<'pr> Visit<'pr> for DebuggerVisitor<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if self.debugger_method(node) {
            let loc = node.location();
            self.offenses.push(DebuggerOffense {
                start_offset: loc.start_offset(),
                end_offset: self.offense_end(node),
            });
        }
        visit_call_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_methods() -> Vec<String> {
        [
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
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    fn ranges(source: &str, methods: &[String]) -> Vec<(usize, usize)> {
        check_debugger(source.as_bytes(), methods)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset))
            .collect()
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
        assert_eq!(ranges("Kernel.binding.irb", &default_methods()), vec![(0, 18)]);
    }

    // A method-only chain configured by the user.
    #[test]
    fn method_chain_config() {
        let methods = vec!["debugger.foo.bar".to_string()];
        assert_eq!(ranges("debugger.foo.bar", &methods), vec![(0, 16)]);
    }

    // A constant path chain (`Foo::Bar::Baz.debug`).
    #[test]
    fn const_path_chain_config() {
        let methods = vec!["Foo::Bar::Baz.debug".to_string()];
        assert_eq!(ranges("Foo::Bar::Baz.debug", &methods), vec![(0, 19)]);
    }

    // A cbase constant (`::Kernel.debugger`) drops the leading `::` when matching.
    #[test]
    fn cbase_constant() {
        assert_eq!(ranges("::Kernel.debugger", &default_methods()), vec![(0, 17)]);
    }

    // A trailing literal block is excluded from the offense range.
    #[test]
    fn block_excluded_from_range() {
        assert_eq!(ranges("Pry.rescue { puts 1 }", &default_methods()), vec![(0, 10)]);
    }

    // A matching name used as a method (receiver present) is not a bare call.
    #[test]
    fn receiver_method_not_matched() {
        assert!(ranges("code.byebug", &default_methods()).is_empty());
    }

    // Comments are ignored by the parser.
    #[test]
    fn comment_ignored() {
        assert!(ranges("# byebug", &default_methods()).is_empty());
    }
}
