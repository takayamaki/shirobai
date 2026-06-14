//! `Lint/RequireParentheses`: warns when a predicate-style call without
//! parentheses takes an `&&`/`||` expression as its (first or last) argument,
//! or a ternary whose condition is an `&&`/`||` expression, where the missing
//! parentheses make the precedence ambiguous.
//!
//! Mirrors `vendor/rubocop/lib/rubocop/cop/lint/require_parentheses.rb`:
//!
//! - On a `send`/`csend` with arguments and no closing `)`:
//!   - If the first argument is a ternary `if` whose condition is `and`/`or`
//!     (parser-gem node type — which on prism is `AndNode`/`OrNode`, covering
//!     both `&&`/`||` and the keyword forms once they reach this position),
//!     and the method is neither `[]` nor an assignment method (a name ending
//!     with `=` other than the comparison operators), the offense range
//!     spans from the call's start to the ternary condition's end.
//!   - Otherwise, if the method name ends with `?` (predicate) and the last
//!     argument is `AndNode`/`OrNode`, the whole call is flagged.
//!
//! No autocorrect (matches stock).

use ruby_prism::{Node, Visit};

pub struct RequireParenthesesOffense {
    pub start_offset: usize,
    pub end_offset: usize,
}

/// Standalone entry point used by the per-cop fallback (`bundle_eligible?` is
/// always true for this cop, so this path is exercised by tests only).
pub fn check_require_parentheses(source: &[u8]) -> Vec<RequireParenthesesOffense> {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
///
/// `Lint/RequireParentheses` is config-less, source-independent (every offset
/// comes from the AST locations), and uses no source text directly, so the
/// builder takes no arguments.
pub(crate) fn build_rule() -> RequireParenthesesVisitor {
    RequireParenthesesVisitor {
        offenses: Vec::new(),
    }
}

pub(crate) struct RequireParenthesesVisitor {
    pub(crate) offenses: Vec<RequireParenthesesOffense>,
}

impl RequireParenthesesVisitor {
    fn check_call(&mut self, call: &ruby_prism::CallNode<'_>) {
        // `node.arguments?` and `!node.parenthesized?`.
        let Some(args) = call.arguments() else { return };
        let args: Vec<_> = args.arguments().iter().collect();
        if args.is_empty() {
            return;
        }
        // `parenthesized?` is `loc_is?(:end, ')')` in rubocop-ast; the prism
        // analogue is "the call's closing token exists and is `)`". The
        // closing_loc is only `Some` when a closing `)` is present.
        if call.closing_loc().is_some() {
            return;
        }

        let first = &args[0];
        if let Some(if_node) = first.as_if_node() {
            // `node.first_argument.if_type? && node.first_argument.ternary?`
            // — a ternary `IfNode` has no `if`/`then`/`end` keywords.
            if if_node.if_keyword_loc().is_none() {
                self.check_ternary(call, &if_node);
                return;
            }
        }

        // `predicate_method?` is `method_name.to_s.end_with?('?')`.
        let method_name = call.name();
        if !method_name.as_slice().ends_with(b"?") {
            return;
        }
        let last = args.last().unwrap();
        if is_logical_operator(last) {
            // Whole call range — matches `add_offense(node)` on a send node,
            // whose source_range is `call.location()`.
            let loc = call.location();
            self.offenses.push(RequireParenthesesOffense {
                start_offset: loc.start_offset(),
                end_offset: loc.end_offset(),
            });
        }
    }

    fn check_ternary(&mut self, call: &ruby_prism::CallNode<'_>, ternary: &ruby_prism::IfNode<'_>) {
        // `check_ternary` bails when `method?(:[])`, when the method is an
        // assignment method (name ends with `=` but isn't a comparison
        // operator), or when the ternary condition isn't `and`/`or`.
        let method_name = call.name();
        let name = method_name.as_slice();
        if name == b"[]" {
            return;
        }
        if is_assignment_method(name) {
            return;
        }
        let predicate = ternary.predicate();
        if !is_logical_operator(&predicate) {
            return;
        }

        // `range_between(node.source_range.begin_pos, ternary.condition.source_range.end_pos)`.
        let start = call.location().start_offset();
        let end = predicate.location().end_offset();
        self.offenses.push(RequireParenthesesOffense {
            start_offset: start,
            end_offset: end,
        });
    }
}

/// `operator_keyword?` in rubocop-ast: true when the parser-gem node type is
/// `:and` or `:or`. Prism splits these by token kind into `AndNode`/`OrNode`,
/// both of which map back to the parser type set.
fn is_logical_operator(node: &Node<'_>) -> bool {
    matches!(node, Node::AndNode { .. } | Node::OrNode { .. })
}

/// `assignment_method?` in rubocop-ast:
/// `!comparison_method? && method_name.to_s.end_with?('=')`. The comparison
/// operators (`== === != <= >= > <`) trivially can't end with `=` and `!=`
/// is the only one whose name does; we keep the full predicate to mirror
/// stock so anything that ends in `=` other than `!=` qualifies.
fn is_assignment_method(name: &[u8]) -> bool {
    if !name.ends_with(b"=") {
        return false;
    }
    !matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=")
}

impl<'pr> Visit<'pr> for RequireParenthesesVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        // Recurse so nested calls (a call's arguments, a block body, etc.) are
        // also checked.
        ruby_prism::visit_call_node(self, node);
    }
}

impl super::dispatch::Rule for RequireParenthesesVisitor {
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
        check_require_parentheses(src.as_bytes())
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset))
            .collect()
    }

    #[test]
    fn flags_predicate_with_amp_amp_last_arg() {
        let off = detect("day.is? 'monday' && month == :jan\n");
        assert_eq!(off.len(), 1);
        assert_eq!(off[0], (0, 33));
    }

    #[test]
    fn flags_predicate_with_or_or_last_arg() {
        let off = detect("day_is? 'tuesday' || true\n");
        assert_eq!(off, vec![(0, 25)]);
    }

    #[test]
    fn flags_ternary_with_and_condition() {
        let off = detect("wd.include? 'tuesday' && true == true ? a : b\n");
        // span: call start..ternary condition end.
        assert_eq!(off.len(), 1);
        let (s, e) = off[0];
        assert_eq!(&"wd.include? 'tuesday' && true == true ? a : b"[s..e], "wd.include? 'tuesday' && true == true");
    }

    #[test]
    fn flags_csend_predicate() {
        let off = detect("day&.is? 'monday' && month == :jan\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn accepts_keyword_and_or() {
        assert!(detect("day.is? 'tuesday' and month == :jan\nday.is? 'tuesday' or month == :jan\n").is_empty());
    }

    #[test]
    fn accepts_non_predicate_method() {
        assert!(detect("weekdays.foo 'tuesday' && true == true\n").is_empty());
    }

    #[test]
    fn accepts_assignment_method_with_or_or() {
        assert!(detect("s.version = @version || \">= 1.8.5\"\n").is_empty());
    }

    #[test]
    fn accepts_index_method_with_or_or() {
        assert!(detect("a[b || c]\n").is_empty());
    }

    #[test]
    fn accepts_parenthesized_call() {
        assert!(detect("day.is?('tuesday' && true == true)\n").is_empty());
    }

    #[test]
    fn accepts_index_with_ternary() {
        assert!(detect("do_something[foo && bar ? baz : qux]\n").is_empty());
    }

    #[test]
    fn accepts_setter_with_ternary() {
        assert!(detect("self.foo = bar && baz ? qux : quux\n").is_empty());
    }
}
