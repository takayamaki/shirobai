//! `Lint/ParenthesesAsGroupedExpression`: warns when a method call has a
//! whitespace between the method name and a `(` that opens its sole argument's
//! parenthesised expression — the parentheses look like a `def f(x)` argument
//! list but the call is actually `f((x))` (a grouped expression) because of the
//! intervening space.
//!
//! Mirrors `vendor/rubocop/lib/rubocop/cop/lint/parentheses_as_grouped_expression.rb`:
//!
//! - On a `send`/`csend` with no closing `)` (outer call is unparenthesised) and
//!   exactly one argument whose `parenthesized_call?` is true and whose source
//!   starts with `(`. In rubocop-ast, `parenthesized_call?` is
//!   `loc_is?(:begin, '(')`; in prism the only kind of argument whose location
//!   begins with a `(` paired with a closing `)` and whose `source.start_with?('(')`
//!   is structurally true is the `ParenthesesNode` — call-like nodes
//!   (`CallNode` chain receivers, `YieldNode`, `SuperNode`, `DefinedNode`)
//!   reach this position only when their source starts with their head token
//!   name (e.g. `b(c)`, `yield(c)`, `super(c)`, `defined?(c)`) and stock's
//!   `source.start_with?('(')` guard fails for them. A bare `ParenthesesNode`
//!   argument is what stock is targeting.
//! - Skip when the outer call's method is an operator (`+`, `-`, …) or a setter
//!   (`name=` with `name` not a comparison operator) — these match stock's
//!   `node.operator_method? || node.setter_method?` short-circuit.
//! - Skip when the space between the selector and the `(` is zero (so
//!   `foo(x)` is not flagged) — this matches stock's
//!   `node.parenthesized? || !first_argument.source.start_with?('(')` early
//!   return: prism's `opening_loc().is_some()` IS the `parenthesized?` check.
//! - The other `valid_context?` branches (`any_block_type?`, `chained_calls?`,
//!   `operator_keyword?`, `hash_type?`, ternary, `compound_range?`) are
//!   structurally excluded by the `ParenthesesNode` gate: a block, chain,
//!   hash, ternary, or unparenthesised range can never appear as a
//!   `ParenthesesNode` itself, and `ParenthesesNode` body types stock's
//!   `valid_first_argument?` cares about (operator keyword / hash / ternary
//!   / range) are *inside* the parens, not the first_argument's own type.
//!   parser-gem agrees: a `(...)` argument shows up as a `begin` node whose
//!   `range_type?` / `hash_type?` / `if_type?` are all false. See the
//!   `valid_context?` walkthrough in `parentheses_as_grouped_expression_edge_cases_spec.rb`.
//!
//! Offense range: the whitespace run between selector and `(`. Autocorrect:
//! `corrector.remove(range)` — drops every space (mirrors stock's
//! `corrector.remove(range)` on the same range).

use ruby_prism::{Node, Visit};

#[derive(Debug, Clone)]
pub struct ParenthesesAsGroupedExpressionOffense {
    /// Start byte of the whitespace range (selector end). Offense highlight and
    /// autocorrect `remove` range begin.
    pub space_start: usize,
    /// End byte of the whitespace range (first argument start). Offense
    /// highlight and autocorrect `remove` range end.
    pub space_end: usize,
    /// Start byte of the first argument (`(`). Used by the wrapper to fetch
    /// the `(...)` source for the `MSG` `%<argument>s` substitution.
    pub arg_start: usize,
    /// End byte of the first argument. Same as above (paired with `arg_start`
    /// the wrapper reads `source[arg_start..arg_end]`).
    pub arg_end: usize,
}

/// Standalone entry point used by the per-cop fallback. This cop is always
/// `bundle_eligible?` (no config), so this path is exercised by tests only.
pub fn check_parentheses_as_grouped_expression(
    source: &[u8],
) -> Vec<ParenthesesAsGroupedExpressionOffense> {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
///
/// `Lint/ParenthesesAsGroupedExpression` is config-less and source-independent
/// (every offset comes from the AST locations), so the builder takes no
/// arguments.
pub(crate) fn build_rule() -> ParenthesesAsGroupedExpressionVisitor {
    ParenthesesAsGroupedExpressionVisitor {
        offenses: Vec::new(),
    }
}

pub(crate) struct ParenthesesAsGroupedExpressionVisitor {
    pub(crate) offenses: Vec<ParenthesesAsGroupedExpressionOffense>,
}

impl ParenthesesAsGroupedExpressionVisitor {
    fn check_call(&mut self, call: &ruby_prism::CallNode<'_>) {
        // `node.parenthesized?` — outer call's closing `)`. Skip parenthesised
        // calls (`foo(x)`).
        if call.opening_loc().is_some() {
            return;
        }

        // `node.arguments.one?` — exactly one argument. prism splits
        // `block_pass` (`&block`) off into `call.block()`; rubocop-ast's
        // `arguments` accessor counts `block_pass` as an argument too, but
        // stock's `arguments.one?` after the `parenthesized_call?` guard never
        // accepts a block-pass-as-first-arg because a block_pass's source is
        // `&...`, never `(...)`. So we can use the prism `arguments` count
        // directly: a single positional argument that is a `ParenthesesNode`.
        let Some(args_node) = call.arguments() else { return; };
        let args: Vec<_> = args_node.arguments().iter().collect();
        if args.len() != 1 {
            return;
        }

        // `node.first_argument.parenthesized_call?` plus
        // `first_argument.source.start_with?('(')` together identify a bare
        // `(...)` argument. In prism that's a `ParenthesesNode` as the first
        // (and only) argument node. Other call-like kinds (`CallNode`,
        // `YieldNode`, `SuperNode`, `DefinedNode`) whose own source starts
        // with their head token always fail stock's `start_with?('(')` guard
        // even when their `loc.begin` is `(`, so we don't need to handle them.
        let first_arg = &args[0];
        let Some(parens) = first_arg.as_parentheses_node() else { return; };

        // `invalid_bare_argument?` (1.91): the parentheses are required when
        // their (possibly re-parenthesised) sole expression could not stand as
        // a bare argument — `and` / `or` / `not`, a modifier `rescue`, or a
        // modifier `if` / `unless` / `while` / `until`.
        if invalid_bare_argument(first_arg) {
            return;
        }

        // `node.operator_method?` / `node.setter_method?` — outer method name.
        // The `ParenthesesNode` gate already rejects block / chain / hash /
        // ternary / unparenthesised range first arguments by construction
        // (see the module docs); the remaining `valid_context?` branches that
        // depend on the *outer call* shape are the operator-method and
        // setter-method names.
        let name_bytes = call.name();
        let name_bytes = name_bytes.as_slice();
        if is_operator_method(name_bytes) || is_setter_name(name_bytes) {
            return;
        }

        // `space_length = first_argument.begin_pos - node.loc.selector.end_pos`.
        // Stock's `loc.selector` is the method-name token range; the prism
        // analogue is `call.message_loc()`. Predicate methods like `is?`
        // include the `?` in both, so the boundary matches byte-for-byte.
        //
        // `space_length.positive?` — skip `foo(x)` (no space; would have been
        // rejected by the outer `parenthesized?` guard anyway, but the no-arg
        // case slips through if a future config exposes it).
        let Some(message_loc) = call.message_loc() else { return; };
        let space_start = message_loc.end_offset();
        let space_end = parens.location().start_offset();
        if space_end <= space_start {
            return;
        }

        self.offenses.push(ParenthesesAsGroupedExpressionOffense {
            space_start,
            space_end,
            arg_start: parens.location().start_offset(),
            arg_end: parens.location().end_offset(),
        });
    }
}

/// Stock's `invalid_bare_argument?`:
///
/// ```ruby
/// node = node.children.first while node&.begin_type? && node.children.one?
/// return false unless node
/// keyword_operator?(node) || modifier_expression?(node) || command_call_operand?(node)
/// ```
///
/// Parser wraps `(...)` in a `begin` whose children are the statements; prism
/// gives a `ParenthesesNode` whose `body` is a `StatementsNode`. Peeling a
/// single-statement parenthesis repeatedly lands on the innermost expression
/// (or on nothing for `()`).
fn invalid_bare_argument(node: &Node<'_>) -> bool {
    if let Some(parens) = node.as_parentheses_node() {
        // `()`: `children.first` is nil.
        let Some(body) = parens.body() else { return false };
        if let Some(stmts) = body.as_statements_node() {
            let children: Vec<Node<'_>> = stmts.body().iter().collect();
            if children.len() == 1 {
                return invalid_bare_argument(&children[0]);
            }
        }
        // A multi-statement `begin` is neither a keyword operator nor a
        // modifier expression.
        return false;
    }
    keyword_operator(node) || modifier_expression(node) || command_call_operand(node)
}

/// `command_call_operand?` (1.92.2, rubocop#16143): an assignment (or an
/// assignment-method call such as `x.y = v` / `x[i] = v`) whose value is a
/// command call, or a prefix-bang `!` send whose receiver is one — both
/// recursively. Parser's `node.children.last` is the assigned value for every
/// `ASSIGNMENTS` type (`lvasgn` … `masgn`, `op_asgn`, `or_asgn`, `and_asgn`)
/// and the last argument for an assignment-method send.
fn command_call_operand(node: &Node<'_>) -> bool {
    if let Some(value) = assignment_value(node) {
        return command_call_value(&value);
    }
    if let Some(c) = node.as_call_node() {
        // `node.call_type? && node.assignment_method?`: `!comparison_method?
        // && method_name.end_with?('=')`; `children.last` is the last
        // argument (a send with no argument would hand stock its method-name
        // symbol, which cannot be a command call).
        if is_setter_name(c.name().as_slice()) {
            let Some(args) = c.arguments() else { return false };
            let Some(last) = args.arguments().iter().last() else { return false };
            return command_call_value(&last);
        }
        // `node.send_type? && node.prefix_bang?`: `send` only (not `csend`),
        // `negation_method?` (`:!`) spelled with a `!` selector.
        if !c.is_safe_navigation()
            && c.name().as_slice() == b"!"
            && c.message_loc().is_some_and(|m| m.as_slice() == b"!")
        {
            return c.receiver().is_some_and(|r| command_call_value(&r));
        }
    }
    false
}

/// The value of a parser `ASSIGNMENTS`-type node, in prism terms.
fn assignment_value<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    macro_rules! value_of {
        ($($as:ident),* $(,)?) => {
            $(if let Some(n) = node.$as() { return Some(n.value()); })*
        };
    }
    value_of!(
        as_local_variable_write_node,
        as_instance_variable_write_node,
        as_class_variable_write_node,
        as_global_variable_write_node,
        as_constant_write_node,
        as_constant_path_write_node,
        as_multi_write_node,
        as_local_variable_operator_write_node,
        as_instance_variable_operator_write_node,
        as_class_variable_operator_write_node,
        as_global_variable_operator_write_node,
        as_constant_operator_write_node,
        as_constant_path_operator_write_node,
        as_call_operator_write_node,
        as_index_operator_write_node,
        as_local_variable_or_write_node,
        as_instance_variable_or_write_node,
        as_class_variable_or_write_node,
        as_global_variable_or_write_node,
        as_constant_or_write_node,
        as_constant_path_or_write_node,
        as_call_or_write_node,
        as_index_or_write_node,
        as_local_variable_and_write_node,
        as_instance_variable_and_write_node,
        as_class_variable_and_write_node,
        as_global_variable_and_write_node,
        as_constant_and_write_node,
        as_constant_path_and_write_node,
        as_call_and_write_node,
        as_index_and_write_node,
    );
    None
}

/// `command_call_value?`: `command_call?(node) || command_call_operand?(node)`.
fn command_call_value(node: &Node<'_>) -> bool {
    command_call(node) || command_call_operand(node)
}

/// `command_call?`: unwrap a block to its send node, then a `send` / `csend`
/// / `super` / `yield` with arguments, no `(...)`, and not an operator
/// method. Prism keeps a `do … end` / `{ }` block on its call (the call IS
/// parser's `send_node`), and splits a block-pass (`&blk`, an argument in
/// parser) into the `block` field, so a `BlockArgumentNode` block counts as an
/// argument. A lambda literal (`-> {}`) is a `LambdaNode` here; in parser its
/// send node is the argument-less `lambda`, so both say no. `zsuper`
/// (`ForwardingSuperNode`) is not `super`-typed.
fn command_call(node: &Node<'_>) -> bool {
    let is_block_arg = |b: Option<Node<'_>>| b.is_some_and(|b| b.as_block_argument_node().is_some());
    if let Some(c) = node.as_call_node() {
        let has_args = c.arguments().is_some_and(|a| a.arguments().iter().next().is_some())
            || is_block_arg(c.block());
        let parenthesized = c.closing_loc().is_some_and(|l| l.as_slice() == b")");
        return has_args && !parenthesized && !is_operator_method(c.name().as_slice());
    }
    if let Some(s) = node.as_super_node() {
        let has_args = s.arguments().is_some_and(|a| a.arguments().iter().next().is_some())
            || is_block_arg(s.block());
        return has_args && s.rparen_loc().is_none();
    }
    if let Some(y) = node.as_yield_node() {
        let has_args = y.arguments().is_some_and(|a| a.arguments().iter().next().is_some());
        return has_args && y.rparen_loc().is_none();
    }
    false
}

/// `keyword_operator?`: `(operator_keyword? && semantic_operator?)` — an
/// `and` / `or` spelled with the keyword — or a modifier `rescue`, or a `not`
/// call (`send_type? && prefix_not?`).
fn keyword_operator(node: &Node<'_>) -> bool {
    if let Some(n) = node.as_and_node() {
        return n.operator_loc().as_slice() == b"and";
    }
    if let Some(n) = node.as_or_node() {
        return n.operator_loc().as_slice() == b"or";
    }
    if node.as_rescue_modifier_node().is_some() {
        return true;
    }
    if let Some(c) = node.as_call_node() {
        // `prefix_not?` = `keyword_not?`: the selector is spelled `not`.
        return c.message_loc().is_some_and(|m| m.as_slice() == b"not");
    }
    false
}

/// `modifier_expression?`: `type?(:if, :while, :until) && modifier_form?`. In
/// prism a modifier form has its keyword after the node's start (the body
/// comes first). `begin ... end while x` is parser's `while_post`, which
/// `type?(:while)` does not match, so the begin-modifier flag excludes it.
fn modifier_expression(node: &Node<'_>) -> bool {
    let node_start = node.location().start_offset();
    if let Some(n) = node.as_if_node() {
        return n
            .if_keyword_loc()
            .is_some_and(|k| k.start_offset() > node_start);
    }
    if let Some(n) = node.as_unless_node() {
        return n.keyword_loc().start_offset() > node_start;
    }
    if let Some(n) = node.as_while_node() {
        return !n.is_begin_modifier() && n.keyword_loc().start_offset() > node_start;
    }
    if let Some(n) = node.as_until_node() {
        return !n.is_begin_modifier() && n.keyword_loc().start_offset() > node_start;
    }
    false
}

/// rubocop-ast `OPERATOR_METHODS` — every operator that can be defined as a
/// method. Identical to the table in `nested_parenthesized_calls`. The list is
/// closed (no Ruby version adds new ones in practice), so we encode it
/// verbatim here.
fn is_operator_method(name: &[u8]) -> bool {
    matches!(
        name,
        b"|" | b"^"
            | b"&"
            | b"<=>"
            | b"=="
            | b"==="
            | b"=~"
            | b">"
            | b">="
            | b"<"
            | b"<="
            | b"<<"
            | b">>"
            | b"+"
            | b"-"
            | b"*"
            | b"/"
            | b"%"
            | b"**"
            | b"~"
            | b"+@"
            | b"-@"
            | b"!"
            | b"!="
            | b"!~"
            | b"`"
            | b"[]"
            | b"[]="
    )
}

/// `name.end_with?('=')` && not a comparison operator (`==`, `===`, `!=`,
/// `<=`, `>=`). Matches `setter_method?` for an attribute-write call (whose
/// `loc.operator == :=`, trivially true in this position). Identical to
/// `nested_parenthesized_calls::is_setter_name`.
fn is_setter_name(name: &[u8]) -> bool {
    if !name.ends_with(b"=") {
        return false;
    }
    !matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=")
}

impl<'pr> Visit<'pr> for ParenthesesAsGroupedExpressionVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        // Recurse so nested calls (a call's arguments, a block body, etc.)
        // are also checked.
        ruby_prism::visit_call_node(self, node);
    }
}

impl super::dispatch::Rule for ParenthesesAsGroupedExpressionVisitor {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_CALL,
        )
    }
    
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

    fn detect(src: &str) -> Vec<(usize, usize, usize, usize)> {
        check_parentheses_as_grouped_expression(src.as_bytes())
            .into_iter()
            .map(|o| (o.space_start, o.space_end, o.arg_start, o.arg_end))
            .collect()
    }

    #[test]
    fn flags_dot_call_with_space_before_paren() {
        let off = detect("a.func (x)\n");
        // selector `func` ends at 6, first arg `(x)` starts at 7.
        assert_eq!(off, vec![(6, 7, 7, 10)]);
    }

    #[test]
    fn flags_predicate_with_space_before_paren() {
        let off = detect("is? (x)\n");
        // selector `is?` ends at 3, arg `(x)` starts at 4.
        assert_eq!(off, vec![(3, 4, 4, 7)]);
    }

    #[test]
    fn flags_csend_with_space_before_paren() {
        let off = detect("a&.func (x)\n");
        // selector `func` ends at 7, arg `(x)` starts at 8.
        assert_eq!(off, vec![(7, 8, 8, 11)]);
    }

    #[test]
    fn flags_block_argument_in_parens() {
        let off = detect("a.concat ((1..1).map { |i| i * 10 })\n");
        // selector `concat` ends at 8, arg `((1..1).map { |i| i * 10 })` starts at 9.
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].0, 8);
        assert_eq!(off[0].1, 9);
    }

    #[test]
    fn accepts_unparenthesized_block_argument() {
        // `a.concat (1..1).map { |i| i * 10 }` — first_arg is a CallNode
        // (`.map {…}`), not a ParenthesesNode.
        assert!(detect("a.concat (1..1).map { |i| i * 10 }\n").is_empty());
    }

    #[test]
    fn accepts_chain_following_paren() {
        assert!(detect("func (x).func.func.func.func.func\n").is_empty());
    }

    #[test]
    fn accepts_chain_with_safe_navigation() {
        assert!(detect("func (x).func.func.func.func&.func\n").is_empty());
    }

    #[test]
    fn accepts_math_expression() {
        assert!(detect("puts (2 + 3) * 4\n").is_empty());
    }

    #[test]
    fn accepts_math_with_chain() {
        assert!(detect("do_something.eq (foo * bar).to_i\n").is_empty());
    }

    #[test]
    fn accepts_hash_kwarg_first_arg() {
        // `transition (foo - bar) => value` — first_arg is a HashNode, not
        // ParenthesesNode.
        assert!(detect("transition (foo - bar) => value\n").is_empty());
    }

    #[test]
    fn accepts_ternary_first_arg() {
        assert!(detect("foo (cond) ? 1 : 2\n").is_empty());
    }

    #[test]
    fn accepts_no_argument() {
        assert!(detect("func\n").is_empty());
    }

    #[test]
    fn accepts_unparenthesized_call() {
        assert!(detect("puts x\n").is_empty());
    }

    #[test]
    fn accepts_chain_of_method_calls() {
        assert!(detect("a.b\na.b 1\na.b(1)\n").is_empty());
    }

    #[test]
    fn accepts_method_with_parens_as_arg_to_method_without() {
        // `a b(c)` — first_arg is a CallNode (`b(c)`), not ParenthesesNode.
        assert!(detect("a b(c)\n").is_empty());
    }

    #[test]
    fn accepts_yield_arg() {
        // `a yield(c)` — first_arg is a YieldNode, not ParenthesesNode.
        assert!(detect("a yield(c)\n").is_empty());
    }

    #[test]
    fn accepts_super_arg() {
        // `a super(c)` — first_arg is a SuperNode.
        assert!(detect("a super(c)\n").is_empty());
    }

    #[test]
    fn accepts_defined_arg() {
        // `a defined?(c)` — first_arg is a DefinedNode.
        assert!(detect("a defined?(c)\n").is_empty());
    }

    #[test]
    fn accepts_operator_method_with_paren_arg() {
        // `a % (b + c)` — outer method is `%` (operator_method).
        assert!(detect("a % (b + c)\n").is_empty());
    }

    #[test]
    fn accepts_setter_with_paren_arg() {
        // `a.b = (c == d)` — outer method is `b=` (setter_method).
        assert!(detect("a.b = (c == d)\n").is_empty());
    }

    #[test]
    fn accepts_space_inside_call_followed_by_paren() {
        // `a( (b) )` — outer is parenthesised (closing `)` present); inner
        // `(b)` is the first argument but outer's `opening_loc.is_some()`
        // short-circuits the check.
        assert!(detect("a( (b) )\n").is_empty());
    }

    #[test]
    fn accepts_compound_range_first_arg() {
        // `rand (a - b)..(c - d)` — first_arg is a RangeNode whose left/right
        // are ParenthesesNodes; the first_arg itself is not a ParenthesesNode.
        assert!(detect("rand (a - b)..(c - d)\n").is_empty());
    }

    #[test]
    fn flags_simple_range_in_parens() {
        // `rand (1..10)` — first_arg IS `ParenthesesNode(RangeNode)`, flagged.
        let off = detect("rand (1..10)\n");
        // selector `rand` ends at 4, arg `(1..10)` starts at 5.
        assert_eq!(off, vec![(4, 5, 5, 12)]);
    }

    #[test]
    fn accepts_multi_arg_call() {
        // `assert_equal (0..1.9), acceleration.domain` — 2 arguments.
        assert!(detect("assert_equal (0..1.9), acceleration.domain\n").is_empty());
    }

    #[test]
    fn accepts_heredoc_paren_lookalike() {
        // Heredoc contents are inside a `StringNode`, the call's first_arg is
        // the heredoc string, not a ParenthesesNode.
        let src = "foo(\n  <<~EOS\n    foo (\n    )\n  EOS\n)\n";
        assert!(detect(src).is_empty());
    }

    #[test]
    fn flags_within_def_body() {
        // The visitor recurses into method bodies; a flagged call deep inside
        // a `def` is still emitted.
        let off = detect("def f\n  a.func (x)\nend\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn space_can_be_multiple_chars() {
        // Multi-space between selector and `(` collapses into a single offense
        // covering the whole run.
        let off = detect("a.func   (x)\n");
        // selector `func` ends at 6, arg `(x)` starts at 9.
        assert_eq!(off, vec![(6, 9, 9, 12)]);
    }

    // 1.91 `invalid_bare_argument?`: parentheses around an `and` / `or` /
    // `not`, a modifier `rescue` or a modifier conditional are required, so
    // they are not a grouped expression; `&&` still is.
    #[test]
    fn accepts_parentheses_required_by_keyword_operators_and_modifiers() {
        assert!(detect("false? (foo and bar)\n").is_empty());
        assert!(detect("false? (foo or bar)\n").is_empty());
        assert!(detect("false? (not foo)\n").is_empty());
        assert!(detect("false? (foo rescue bar)\n").is_empty());
        assert!(detect("false? (foo if bar)\n").is_empty());
        assert!(detect("false? (foo unless bar)\n").is_empty());
        assert!(detect("false? (foo while bar)\n").is_empty());
        assert!(detect("false? ((foo and bar))\n").is_empty());
        assert_eq!(detect("false? (foo && bar)\n"), vec![(6, 7, 7, 19)]);
        assert_eq!(detect("false? (!foo)\n"), vec![(6, 7, 7, 13)]);
    }

    // 1.92.2 `command_call_operand?` (rubocop#16143): removing the space would
    // turn the parentheses into the argument list of the outer call and
    // change how the inner command call binds.
    #[test]
    fn accepts_command_call_operands() {
        for src in [
            "obj.do_something (x = foo.bar 'baz')\n",
            "obj.do_something (x ||= foo 'baz')\n",
            "obj.do_something (x.y = foo 'baz')\n",
            "obj.do_something (x[1] = foo 'baz')\n",
            "obj.do_something (x = y = foo 'baz')\n",
            "obj.do_something (x = foo 'baz' do end)\n",
            "obj.do_something (!foo 'baz')\n",
            "obj.do_something (!foo 'baz' do end)\n",
            "obj.do_something (x += foo 'baz')\n",
            "obj.do_something (x = a&.foo 'baz')\n",
            "obj.do_something (x = foo &blk)\n",
            "obj.do_something (a, b = foo 'baz')\n",
            "def m\n  obj.do_something (x = super 'baz')\nend\n",
            "def m\n  obj.do_something (x = yield 'baz')\nend\n",
        ] {
            assert!(detect(src).is_empty(), "{src:?}");
        }
    }

    #[test]
    fn flags_non_command_call_operands() {
        for src in [
            "obj.do_something (foo 'baz')\n",
            "obj.do_something (x = foo(1))\n",
            "obj.do_something (x = a + b)\n",
            "obj.do_something (x = foo)\n",
            "obj.do_something (x = -> {})\n",
            "obj.do_something (x = (foo 'baz'))\n",
            "obj.do_something (x == foo 'baz')\n",
            "def m\n  obj.do_something (x = super)\nend\n",
            "def m\n  obj.do_something (x = yield(1))\nend\n",
        ] {
            assert_eq!(detect(src).len(), 1, "{src:?}");
        }
    }
}
