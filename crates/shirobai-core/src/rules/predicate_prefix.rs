//! `Naming/PredicatePrefix`.

use ruby_prism::Node;

/// A method-definition site whose name literally starts with a configured
/// `NamePrefix` entry.
///
/// This mirrors the stock cop's own cheap gate (`method_name.start_with?(prefix)`,
/// which guards its `/^#{prefix}[^0-9]/` regex): a name that matches no prefix
/// literally can never be an offense, so only these rare candidates cross back
/// into Ruby, where the per-prefix filtering (`allowed_method_name?`,
/// `AllowedMethods`, `UseSorbetSigs`) runs verbatim.
pub struct PredicatePrefixCandidate {
    /// Offense range: the `def` name token, or the macro's symbol argument.
    pub start_offset: usize,
    pub end_offset: usize,
    /// The bare method name (no sigil, no colon).
    pub name: String,
    /// Whether this is a `def`/`defs` site (the `UseSorbetSigs` exemption only
    /// applies to those, not to `MethodDefinitionMacros` calls).
    pub is_def: bool,
    /// Whether the definition's left sibling is a `sig` block returning
    /// `T::Boolean` (`sorbet_sig?(node, return_type: 'T::Boolean')`).
    pub sorbet_boolean_sig: bool,
}

pub fn check_predicate_prefix(
    source: &[u8],
    prefixes: &[String],
    macros: &[String],
) -> Vec<PredicatePrefixCandidate> {
    let mut rule = build_rule(source, prefixes, macros);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule<'a>(
    source: &'a [u8],
    prefixes: &'a [String],
    macros: &'a [String],
) -> PredicatePrefixRule<'a> {
    PredicatePrefixRule {
        source,
        prefixes,
        macros,
        levels: vec![Level::default()],
        offenses: Vec::new(),
    }
}

/// Sibling bookkeeping for one nesting level of the walk, tracking what the
/// previously-closed sibling was. This reproduces `node.left_sibling` for the
/// Sorbet-sig lookup without parent pointers: when a `def` is entered, the
/// level on top of the stack remembers whether the node closed right before it
/// was a boolean `sig` block.
#[derive(Default, Clone, Copy)]
struct Level {
    /// Whether the previously-closed sibling at this level is a
    /// `sig { returns(T::Boolean) }` block.
    prev_is_bool_sig: bool,
    /// Whether the node that opened this level is itself such a block
    /// (transferred to the parent's `prev_is_bool_sig` on `leave`).
    self_is_bool_sig: bool,
}

pub(crate) struct PredicatePrefixRule<'a> {
    source: &'a [u8],
    prefixes: &'a [String],
    macros: &'a [String],
    levels: Vec<Level>,
    pub(crate) offenses: Vec<PredicatePrefixCandidate>,
}

impl PredicatePrefixRule<'_> {
    fn matches_prefix(&self, name: &[u8]) -> bool {
        self.prefixes.iter().any(|p| name.starts_with(p.as_bytes()))
    }

    /// Port of the `sorbet_return_type` node matcher
    /// `(block (send nil? :sig) args (send _ :returns $_type))` combined with
    /// the `type.source == 'T::Boolean'` check: a `sig` call without arguments
    /// whose plain block body is exactly one `returns(...)` call (no block of
    /// its own) with a single argument whose source text is `T::Boolean`.
    fn is_bool_sig_block(&self, node: &Node<'_>) -> bool {
        let Some(call) = node.as_call_node() else {
            return false;
        };
        if call.name().as_slice() != b"sig"
            || call.receiver().is_some()
            || call.arguments().is_some()
        {
            return false;
        }
        let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
            return false;
        };
        // The parser-AST pattern's `args` slot only matches a plain (possibly
        // empty) parameter list; numbered/`it` parameters are other node types.
        if block
            .parameters()
            .is_some_and(|p| p.as_block_parameters_node().is_none())
        {
            return false;
        }
        let Some(body) = block.body().and_then(|b| b.as_statements_node()) else {
            return false;
        };
        let mut statements = body.body().iter();
        let (Some(only), None) = (statements.next(), statements.next()) else {
            return false;
        };
        let Some(returns) = only.as_call_node() else {
            return false;
        };
        // A `returns` carrying its own block would be a `block` node in the
        // parser AST and fail the `(send _ :returns $_type)` pattern.
        if returns.name().as_slice() != b"returns" || returns.block().is_some() {
            return false;
        }
        let Some(arguments) = returns.arguments() else {
            return false;
        };
        let mut arguments = arguments.arguments().iter();
        let (Some(type_arg), None) = (arguments.next(), arguments.next()) else {
            return false;
        };
        let loc = type_arg.location();
        &self.source[loc.start_offset()..loc.end_offset()] == b"T::Boolean"
    }

    /// `dynamic_method_define`: `(send nil? #method_definition_macro? (sym $_) ...)`.
    /// The offense range is the symbol argument (`node.first_argument`).
    fn check_macro(&mut self, call: &ruby_prism::CallNode<'_>) {
        if call.receiver().is_some() {
            return;
        }
        let method = call.name().as_slice();
        if !self.macros.iter().any(|m| m.as_bytes() == method) {
            return;
        }
        let Some(arguments) = call.arguments() else {
            return;
        };
        let Some(first) = arguments.arguments().iter().next() else {
            return;
        };
        let Some(sym) = first.as_symbol_node() else {
            return;
        };
        // Interpolated / dynamic symbols have no static value.
        if sym.value_loc().is_none() {
            return;
        }
        let name = sym.unescaped();
        if !self.matches_prefix(name) {
            return;
        }
        let loc = first.location();
        self.offenses.push(PredicatePrefixCandidate {
            start_offset: loc.start_offset(),
            end_offset: loc.end_offset(),
            name: String::from_utf8_lossy(name).into_owned(),
            is_def: false,
            sorbet_boolean_sig: false,
        });
    }
}

impl super::dispatch::Rule for PredicatePrefixRule<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(def) = node.as_def_node() {
            let name = def.name().as_slice();
            if self.matches_prefix(name) {
                let prev_is_bool_sig = self.levels.last().is_some_and(|l| l.prev_is_bool_sig);
                let loc = def.name_loc();
                self.offenses.push(PredicatePrefixCandidate {
                    start_offset: loc.start_offset(),
                    end_offset: loc.end_offset(),
                    name: String::from_utf8_lossy(name).into_owned(),
                    is_def: true,
                    sorbet_boolean_sig: prev_is_bool_sig,
                });
            }
        } else if let Some(call) = node.as_call_node() {
            self.check_macro(&call);
        }
        self.levels.push(Level {
            prev_is_bool_sig: false,
            self_is_bool_sig: self.is_bool_sig_block(node),
        });
    }

    fn leave(&mut self) {
        let closed = self.levels.pop().unwrap_or_default();
        if let Some(top) = self.levels.last_mut() {
            top.prev_is_bool_sig = closed.self_is_bool_sig;
        }
    }

    fn enter_leaf(&mut self, _node: &Node<'_>) {
        // A leaf sibling (symbol, constant, ...) is never a `sig` block.
        if let Some(top) = self.levels.last_mut() {
            top.prev_is_bool_sig = false;
        }
    }

    fn enter_rescue(&mut self, _node: &Node<'_>) {
        // A rescue clause opens its own sibling level: statements before the
        // `rescue` must not leak a sig flag into the clause's first statement.
        self.levels.push(Level::default());
    }

    fn leave_rescue(&mut self) {
        self.levels.pop();
        if let Some(top) = self.levels.last_mut() {
            top.prev_is_bool_sig = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> (Vec<String>, Vec<String>) {
        (
            ["is_", "has_", "have_", "does_"].map(String::from).to_vec(),
            ["define_method", "define_singleton_method"]
                .map(String::from)
                .to_vec(),
        )
    }

    fn candidates(source: &str) -> Vec<(String, bool, bool)> {
        let (prefixes, macros) = defaults();
        check_predicate_prefix(source.as_bytes(), &prefixes, &macros)
            .into_iter()
            .map(|c| (c.name, c.is_def, c.sorbet_boolean_sig))
            .collect()
    }

    #[test]
    fn def_with_prefix_is_candidate() {
        assert_eq!(
            candidates("def is_attr; end"),
            vec![("is_attr".to_string(), true, false)]
        );
    }

    #[test]
    fn def_without_prefix_is_skipped() {
        assert!(candidates("def attr?; end\ndef hello; end").is_empty());
    }

    #[test]
    fn defs_name_range_covers_name_only() {
        let got = check_predicate_prefix(b"def self.has_attr; end", &defaults().0, &defaults().1);
        assert_eq!(got.len(), 1);
        // `def self.` is 9 bytes; the name token is `has_attr`.
        assert_eq!((got[0].start_offset, got[0].end_offset), (9, 17));
        assert!(got[0].is_def);
    }

    #[test]
    fn macro_symbol_is_candidate_with_colon_range() {
        let got = check_predicate_prefix(
            b"define_method(:is_hello) do |x|\nend",
            &defaults().0,
            &defaults().1,
        );
        assert_eq!(got.len(), 1);
        // The range spans `:is_hello` including the colon.
        assert_eq!((got[0].start_offset, got[0].end_offset), (14, 23));
        assert_eq!(got[0].name, "is_hello");
        assert!(!got[0].is_def);
    }

    #[test]
    fn macro_with_receiver_or_unknown_name_is_skipped() {
        assert!(candidates("obj.define_method(:is_hello) { }").is_empty());
        assert!(candidates("def_node_matcher(:is_hello) { }").is_empty());
        assert!(candidates("define_method(\"is_hello\") { }").is_empty());
    }

    #[test]
    fn custom_macro_list() {
        let prefixes = vec!["is_".to_string()];
        let macros = vec!["def_node_matcher".to_string()];
        let got = check_predicate_prefix(b"def_node_matcher :is_hello, 'x'", &prefixes, &macros);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "is_hello");
    }

    #[test]
    fn sorbet_sig_brace_and_do_end() {
        assert_eq!(
            candidates("sig { returns(T::Boolean) }\ndef is_attr; end"),
            vec![("is_attr".to_string(), true, true)]
        );
        assert_eq!(
            candidates("sig do\n  returns(T::Boolean)\nend\ndef is_attr; end"),
            vec![("is_attr".to_string(), true, true)]
        );
    }

    #[test]
    fn sorbet_sig_survives_comments_and_blank_lines() {
        assert_eq!(
            candidates("sig { returns(T::Boolean) }\n# Comment.\n\ndef is_attr; end"),
            vec![("is_attr".to_string(), true, true)]
        );
    }

    #[test]
    fn sorbet_sig_chained_on_params() {
        assert_eq!(
            candidates("sig { params(x: Integer).returns(T::Boolean) }\ndef is_even(x); end"),
            vec![("is_even".to_string(), true, true)]
        );
    }

    #[test]
    fn sorbet_sig_with_other_return_type_does_not_count() {
        assert_eq!(
            candidates("sig { returns(T::Array[String]) }\ndef has_caused_error; end"),
            vec![("has_caused_error".to_string(), true, false)]
        );
    }

    #[test]
    fn sorbet_sig_pairs_with_adjacent_def_only() {
        let src = "sig { returns(T::Boolean) }\n\
                   def is_attr?; end\n\
                   sig { returns(String) }\n\
                   def has_caused_error; end\n";
        assert_eq!(
            candidates(src),
            vec![
                ("is_attr?".to_string(), true, true),
                ("has_caused_error".to_string(), true, false)
            ]
        );
    }

    #[test]
    fn def_without_sig_has_no_flag() {
        assert_eq!(
            candidates("x = 1\ndef is_attr; end"),
            vec![("is_attr".to_string(), true, false)]
        );
    }

    #[test]
    fn sig_with_multi_statement_body_does_not_match() {
        assert_eq!(
            candidates("sig { foo\nreturns(T::Boolean) }\ndef is_attr; end"),
            vec![("is_attr".to_string(), true, false)]
        );
    }

    #[test]
    fn sig_flag_works_inside_class_body() {
        let src = "class Foo\n\
                   \x20 sig { returns(T::Boolean) }\n\
                   \x20 def is_attr; end\n\
                   end\n";
        assert_eq!(candidates(src), vec![("is_attr".to_string(), true, true)]);
    }

    #[test]
    fn sig_flag_does_not_leak_across_rescue_boundary() {
        let src = "begin\n\
                   \x20 sig { returns(T::Boolean) }\n\
                   rescue\n\
                   \x20 def is_attr; end\n\
                   end\n";
        assert_eq!(candidates(src), vec![("is_attr".to_string(), true, false)]);
    }

    #[test]
    fn sig_flag_pairs_inside_rescue_body() {
        let src = "begin\n\
                   \x20 x\n\
                   rescue\n\
                   \x20 sig { returns(T::Boolean) }\n\
                   \x20 def is_attr; end\n\
                   end\n";
        assert_eq!(candidates(src), vec![("is_attr".to_string(), true, true)]);
    }

    #[test]
    fn empty_prefixes_yield_no_candidates() {
        let got = check_predicate_prefix(b"def is_attr; end", &[], &defaults().1);
        assert!(got.is_empty());
    }
}
