//! `Lint/AmbiguousBlockAssociation`: flags an unparenthesised method call whose
//! last argument is a block-bearing call (`x.f y { |b| … }`), where the block
//! could syntactically associate with either the outer `f` or the inner `y`.
//!
//! Mirrors `vendor/rubocop/lib/rubocop/cop/lint/ambiguous_block_association.rb`:
//!
//! ```ruby
//! def on_send(node)
//!   return unless node.arguments?
//!   return unless ambiguous_block_association?(node)
//!   return if node.parenthesized? || node.last_argument.lambda_or_proc? ||
//!             allowed_method_pattern?(node)
//!
//!   message = message(node)
//!   add_offense(node, message: message) do |corrector|
//!     wrap_in_parentheses(corrector, node)
//!   end
//! end
//! alias on_csend on_send
//!
//! def ambiguous_block_association?(send_node)
//!   send_node.last_argument.any_block_type? &&
//!     !send_node.last_argument.send_node.arguments?
//! end
//!
//! def allowed_method_pattern?(node)
//!   node.assignment? || node.operator_method? || node.method?(:[]) ||
//!     allowed_method?(node.last_argument.method_name) ||
//!     matches_allowed_pattern?(node.last_argument.send_node.source)
//! end
//! ```
//!
//! parser-gem vs prism: parser's `:block` covers BOTH the regular block form
//! `a { … }` AND a lambda literal `-> { … }`; prism splits them. The
//! `last_argument.any_block_type?` guard therefore reaches:
//!
//!   - `CallNode` with `block: Some(BlockNode)` (matches parser
//!     `(block (send …) …)` for `a { … }` / `a do … end`), OR
//!   - `LambdaNode` directly (matches parser `(block (send nil :lambda) …)`
//!     for `->(a) { … }`).
//!
//! `lambda_or_proc?` excludes the LambdaNode form AND a CallNode whose block
//! sender name is `:lambda` / `:proc` (`foo lambda { … }` / `foo proc { … }`),
//! exactly like stock's `:lambda_or_proc -> (block_node) {…}` matcher.
//!
//! `block_pass` arguments (`&blk`) live on `CallNode.block` in prism but NOT
//! in the `arguments` list — so a `foo(&blk)` last argument never trips the
//! any_block_type? gate. parser-gem inlines `:block_pass` into the arguments
//! list but `:block_pass` is not `:block` either, so this is parity.
//!
//! `node.method?(:[])` short-circuits `foo[bar { … }]` (the outer `:[]` call
//! whose last argument IS a block-bearing call). We mirror the name check
//! against the outer CallNode's name.
//!
//! `node.assignment?` is rubocop-ast's `setter_method?` (the alias on
//! `MethodDispatchNode`): true when `loc.operator` is `:=`. In prism that
//! is `equal_loc.is_some()` (set only for `obj.foo = rhs` and `obj[k] = rhs`).
//!
//! `node.operator_method?` flags binary / unary / index operator names
//! (`+`, `==`, `<=>`, `[]`, etc.). The full `OPERATOR_METHODS` table is
//! identical to the one used by `Lint/ParenthesesAsGroupedExpression` and
//! `Style/NestedParenthesizedCalls`; encoded verbatim below.
//!
//! AllowedMethods: a flat list of method names (post-regexp filtering done in
//! Ruby), matched against the INNER block sender's `method_name`
//! (`node.last_argument.method_name`).
//!
//! AllowedPatterns: regexp matched against `last_argument.send_node.source`
//! (the source range of the block sender, e.g. `change` /
//! `receive(:complete).twice`). Regexps cannot ride the bundle path
//! (`bundle_eligible?` is false on the wrapper when AllowedPatterns is
//! non-empty); the standalone entry below takes a pre-applied
//! `allowed_inner_sources` list that the wrapper builds by running each
//! configured regexp against every block-sender candidate. To keep the Rust
//! path simple, the standalone API instead takes the regexp source list and
//! does the match in Rust — there is no cross-cutting regexp engine in the
//! Rust crate, so we KEEP the regexp matching in Ruby and switch off the
//! bundle there: see the wrapper.
//!
//! Offense range: the WHOLE outer call's source range (stock's
//! `add_offense(node)` — `node.source_range` is the full call). Autocorrect:
//! `node.loc.selector.end.join(node.first_argument.source_range.begin)` is
//! `remove`d (the whitespace run between the selector and the first arg) and
//! `(` inserted at its start; `)` inserted after `node.last_argument.end`.
//! In prism: `message_loc().end_offset()` for the selector end, and
//! `arguments[0].location().start_offset()` for the first-arg begin —
//! identical bytes to parser-gem in every observed case (predicate-method
//! selectors include `?` in both).

use ruby_prism::{Node, Visit};

#[derive(Debug, Clone, Default)]
pub struct Config {
    /// `AllowedMethods` (regexp entries filtered out by the wrapper). Matched
    /// against the INNER block sender's method name.
    pub allowed_methods: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AmbiguousBlockAssociationOffense {
    /// Start byte of the OUTER call's source range (offense highlight).
    pub start_offset: usize,
    /// End byte of the OUTER call's source range.
    pub end_offset: usize,
    /// `param` — the last argument's source (the block-bearing inner call,
    /// e.g. `a { |val| puts val }`). Used by the wrapper to format MSG.
    pub param_start: usize,
    pub param_end: usize,
    /// `method` — the inner block sender's source (e.g. `a` or
    /// `receive(:complete).twice`). Used by the wrapper to format MSG.
    pub inner_send_start: usize,
    pub inner_send_end: usize,
    /// Autocorrect: `remove` the whitespace run `[ac_open_start, ac_open_end)`
    /// AND `insert_before` it with `(` (stock's `corrector.remove(range)` +
    /// `corrector.insert_before(range, '(')`).
    pub ac_open_start: usize,
    pub ac_open_end: usize,
    /// Autocorrect: `insert_after` at `ac_close_pos` with `)` (stock's
    /// `corrector.insert_after(node.last_argument, ')')`).
    pub ac_close_pos: usize,
}

/// Standalone entry point used by the per-cop fallback (e.g. when
/// `AllowedPatterns` carries `Regexp` entries; the wrapper builds a
/// pre-applied list of "inner-sender sources to skip" and passes it as
/// `allowed_inner_sources`).
pub fn check_ambiguous_block_association(
    source: &[u8],
    allowed_methods: &[String],
    allowed_inner_sources: &[String],
) -> Vec<AmbiguousBlockAssociationOffense> {
    let mut visitor = build_rule_with_extras(source, allowed_methods, allowed_inner_sources);
    super::parse_cache::with_parsed(source, |_src, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for shared-walk bundles. The bundle path never carries
/// `AllowedPatterns` regexp matches (the wrapper falls back to standalone when
/// any regexp is configured), so `allowed_inner_sources` is empty here.
pub(crate) fn build_rule<'s>(source: &'s [u8], cfg: Config) -> AmbiguousBlockAssociationVisitor<'s> {
    AmbiguousBlockAssociationVisitor {
        source,
        allowed_methods: cfg.allowed_methods.into_iter().map(|s| s.into_bytes()).collect(),
        allowed_inner_sources: Vec::new(),
        offenses: Vec::new(),
    }
}

fn build_rule_with_extras<'s>(
    source: &'s [u8],
    allowed_methods: &[String],
    allowed_inner_sources: &[String],
) -> AmbiguousBlockAssociationVisitor<'s> {
    AmbiguousBlockAssociationVisitor {
        source,
        allowed_methods: allowed_methods.iter().map(|s| s.as_bytes().to_vec()).collect(),
        allowed_inner_sources: allowed_inner_sources
            .iter()
            .map(|s| s.as_bytes().to_vec())
            .collect(),
        offenses: Vec::new(),
    }
}

pub(crate) struct AmbiguousBlockAssociationVisitor<'s> {
    source: &'s [u8],
    allowed_methods: Vec<Vec<u8>>,
    /// Source strings (`last_argument.send_node.source`) the wrapper already
    /// matched against `AllowedPatterns`; we skip emissions whose inner-sender
    /// source bytes equal any of these. Empty on the bundle path.
    allowed_inner_sources: Vec<Vec<u8>>,
    pub(crate) offenses: Vec<AmbiguousBlockAssociationOffense>,
}

impl<'s> AmbiguousBlockAssociationVisitor<'s> {
    fn check_call(&mut self, call: &ruby_prism::CallNode<'_>) {
        // `node.arguments?` — at least one regular argument. block_pass (`&blk`)
        // lives on `CallNode.block` in prism, NOT in the arguments list, so
        // an arguments-only count is correct.
        let Some(args_node) = call.arguments() else { return };
        let arg_vec: Vec<_> = args_node.arguments().iter().collect();
        let Some(last_arg) = arg_vec.last() else { return };

        // `ambiguous_block_association?`: last_argument is a block-bearing
        // construct AND its sender takes no arguments. parser-gem `:block`
        // covers both forms; prism splits them.
        //
        // `inner_send_loc` is `[location.start_offset, block.location.start_offset)`
        // with trailing whitespace stripped — matching parser-gem's
        // `block.send_node.source` which excludes the `{` / `do` token and
        // the whitespace immediately before it. For `change { … }`:
        // `change` (6 bytes). For `posts.find { … }`: `posts.find`. For
        // `receive(:complete).twice { … }`: `receive(:complete).twice`.
        let (inner_send_loc, inner_method_name): (Option<(usize, usize)>, Option<Vec<u8>>) =
            if let Some(inner) = last_arg.as_call_node() {
                // `a { … }` form: parser `(block (send nil :a) …)`, prism
                // CallNode(a) with `block: Some(BlockNode | LambdaNode)`.
                let Some(block) = inner.block() else { return };
                match block {
                    Node::BlockNode { .. } => {}
                    Node::LambdaNode { .. } => {}
                    _ => return,
                }
                // `!send_node.arguments?` — inner CallNode has no arguments.
                // prism `CallNode.arguments()` is `None` when there are none.
                if inner.arguments().is_some()
                    && inner.arguments().unwrap().arguments().iter().count() > 0
                {
                    return;
                }
                // `lambda_or_proc?` on a parser `:block` — rubocop-ast
                // matchers `lambda?` / `proc?`:
                //   - `(block (send nil? :lambda) …)` — `lambda { … }`
                //   - `(block (send nil? :proc) …)` — `proc { … }`
                //   - `(block (send Proc :new) …)` — `Proc.new { … }`
                // (The bare `Proc.new` case has no block in parser, so it is
                // not a block argument to begin with.)
                let name = inner.name();
                let name = name.as_slice();
                let receiver = inner.receiver();
                if receiver.is_none() && matches!(name, b"lambda" | b"proc") {
                    return;
                }
                if name == b"new"
                    && receiver
                        .as_ref()
                        .and_then(|r| r.as_constant_read_node())
                        .is_some_and(|c| c.name().as_slice() == b"Proc")
                {
                    return;
                }
                let loc = inner.location();
                let block_start = block.location().start_offset();
                let mut send_end = block_start;
                while send_end > 0 && matches!(self.source[send_end - 1], b' ' | b'\t') {
                    send_end -= 1;
                }
                (
                    Some((loc.start_offset(), send_end)),
                    Some(name.to_vec()),
                )
            } else if let Some(_lambda) = last_arg.as_lambda_node() {
                // `foo ->(a) { … }` form: parser `(block (send nil :lambda) …)`
                // with `lambda_or_proc?` true → excluded by stock. We mirror
                // by bailing without emitting.
                return;
            } else {
                return;
            };

        // `node.parenthesized?` — outer call has `(` `)`. Skip those.
        if call.opening_loc().is_some() {
            return;
        }

        // `allowed_method_pattern?` short-circuits:
        //   - `node.assignment?` (setter): prism `equal_loc.is_some()`.
        //   - `node.operator_method?`: name in OPERATOR_METHODS.
        //   - `node.method?(:[])`: outer name is `[]`.
        if call.equal_loc().is_some() {
            return;
        }
        let outer_name = call.name();
        let outer_name = outer_name.as_slice();
        if is_operator_method(outer_name) || outer_name == b"[]" {
            return;
        }

        // `allowed_method?(node.last_argument.method_name)` — inner block
        // sender's name on the AllowedMethods list.
        if let Some(inner_name) = inner_method_name.as_ref()
            && self
                .allowed_methods
                .iter()
                .any(|n| n.as_slice() == inner_name.as_slice())
        {
            return;
        }

        // `matches_allowed_pattern?(node.last_argument.send_node.source)` —
        // the wrapper pre-applied the regexp list and passed any matching
        // inner-sender source strings here. (Empty on the bundle path.)
        if let Some((is_start, is_end)) = inner_send_loc
            && self
                .allowed_inner_sources
                .iter()
                .any(|s| s.as_slice() == &self.source[is_start..is_end])
        {
            return;
        }

        // Offense / autocorrect anchors.
        let outer_loc = call.location();
        let last_loc = last_arg.location();
        let first_arg_loc = arg_vec[0].location();
        let Some(msg_loc) = call.message_loc() else { return };

        let (inner_send_start, inner_send_end) =
            inner_send_loc.expect("inner_send_loc set for the CallNode arm");

        self.offenses.push(AmbiguousBlockAssociationOffense {
            start_offset: outer_loc.start_offset(),
            end_offset: outer_loc.end_offset(),
            param_start: last_loc.start_offset(),
            param_end: last_loc.end_offset(),
            inner_send_start,
            inner_send_end,
            ac_open_start: msg_loc.end_offset(),
            ac_open_end: first_arg_loc.start_offset(),
            ac_close_pos: last_loc.end_offset(),
        });
    }
}

/// rubocop-ast `OPERATOR_METHODS`. Identical to the table in
/// `nested_parenthesized_calls` / `parentheses_as_grouped_expression`.
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

impl<'pr, 's> Visit<'pr> for AmbiguousBlockAssociationVisitor<'s> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        ruby_prism::visit_call_node(self, node);
    }
}

impl<'s> super::dispatch::Rule for AmbiguousBlockAssociationVisitor<'s> {
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

    fn detect(src: &str) -> Vec<AmbiguousBlockAssociationOffense> {
        check_ambiguous_block_association(src.as_bytes(), &[], &[])
    }

    fn detect_with_allowed(src: &str, allowed: &[&str]) -> Vec<AmbiguousBlockAssociationOffense> {
        let allowed: Vec<String> = allowed.iter().map(|s| s.to_string()).collect();
        check_ambiguous_block_association(src.as_bytes(), &allowed, &[])
    }

    #[test]
    fn flags_unparenthesised_block_arg_without_receiver() {
        let off = detect("some_method a { |el| puts el }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!(o.start_offset, 0);
        assert_eq!(o.end_offset, 30);
        // selector `some_method` ends at 11, first_arg starts at 12.
        assert_eq!(o.ac_open_start, 11);
        assert_eq!(o.ac_open_end, 12);
        assert_eq!(o.ac_close_pos, 30);
    }

    #[test]
    fn flags_unparenthesised_block_arg_with_receiver() {
        let off = detect("Foo.some_method a { |el| puts el }\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_csend_outer() {
        let off = detect("Foo&.some_method a { |el| puts el }\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_rspec_change_default() {
        let off = detect("expect { order.expire }.to change { order.events }\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_hash_inner_some_method() {
        // Outer `Hash[…]` is `:[]` (excluded). The INNER `some_method a { … }`
        // IS flagged on its own — the visitor recurses.
        let off = detect("Hash[some_method a { |el| el }]\n");
        assert_eq!(off.len(), 1);
        // Inner range starts at 5 (after `Hash[`).
        assert_eq!(off[0].start_offset, 5);
    }

    #[test]
    fn flags_under_lvasgn() {
        // `foo = some_method a { … }` — outer lvasgn, INNER call is flagged.
        let off = detect("foo = some_method a { |el| puts el }\n");
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].start_offset, 6);
    }

    #[test]
    fn accepts_parenthesised_outer() {
        assert!(detect("some_method(a) { |el| puts el }\n").is_empty());
        assert!(detect("some_method(a) { puts _1 }\n").is_empty());
        assert!(detect("Foo.bar(a) { |el| puts el }\n").is_empty());
    }

    #[test]
    fn accepts_do_end_form() {
        // `do…end` binds tighter than `{}` so there is no ambiguity; stock
        // accepts. The inner CallNode `a do … end` still has `block:
        // Some(BlockNode)` in prism, but stock's spec says these are accepted.
        // Looking at stock more closely: the cop fires whether the block is
        // `{}` or `do…end` — the spec `it_behaves_like 'accepts',
        // 'some_method a do;puts "dev";end'` only accepts because the OUTER
        // takes the do/end (not the `a`). The parser parse is
        // `(block (send nil :some_method (send nil :a)) ...)` — the outer
        // call's last_argument IS `a` (a plain send, not a block). The block
        // wraps the outer call.
        assert!(detect("some_method a do;puts \"dev\";end\n").is_empty());
        assert!(detect("some_method a do |e|;puts e;end\n").is_empty());
        assert!(detect("some_method(a) do;puts a;end\n").is_empty());
    }

    #[test]
    fn accepts_lambda_arg() {
        // `foo ->(a) { bar a }` — last_arg is a LambdaNode; lambda_or_proc?
        // excludes it.
        assert!(detect("foo ->(a) { bar a }\n").is_empty());
    }

    #[test]
    fn accepts_operator_method_outer() {
        assert!(detect("foo == bar { baz a }\n").is_empty());
    }

    #[test]
    fn accepts_index_method_outer() {
        // `foo[bar { … }]` — outer name is `:[]`.
        assert!(detect("foo[bar { |a| a }]\n").is_empty());
    }

    #[test]
    fn accepts_inner_call_with_args() {
        // The inner block sender HAS arguments (`fetch(:a)`), so the
        // `!send_node.arguments?` guard rejects.
        assert!(detect("env ENV.fetch(\"ENV\") { \"dev\" }\n").is_empty());
        assert!(detect("{ f: \"b\"}.fetch(:a) do |e|;puts e;end\n").is_empty());
    }

    #[test]
    fn accepts_assignment_outer() {
        // Setter method call: `foo = …` is lvasgn, not a setter. A real
        // setter call: `obj.foo = bar { baz }` — but parser binds `bar { baz }`
        // differently here. This is a placeholder; setter calls are tested in
        // the edge-case spec.
        assert!(detect("Proc.new { puts \"proc\" }\n").is_empty());
    }

    #[test]
    fn accepts_allowed_method_change() {
        // With `change` allowed.
        assert!(detect_with_allowed(
            "expect { order.expire }.to change { order.events }\n",
            &["change"],
        )
        .is_empty());
    }

    #[test]
    fn flags_when_allowed_doesnt_match() {
        // `change` allowed, but the inner sender is `update`.
        let off = detect_with_allowed(
            "expect { order.expire }.to update { order.events }\n",
            &["change"],
        );
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn accepts_lambda_assigned_to_var() {
        assert!(detect("foo = lambda do |diagnostic|;end\n").is_empty());
    }

    #[test]
    fn flags_under_assignment_rhs() {
        // `foo = some_method a { … }` — same as flags_under_lvasgn (already
        // covered) but pinned here as the corresponding vendor accept fixture
        // counterpart.
        let off = detect("foo = some_method a { |el| puts el }\n");
        assert_eq!(off.len(), 1);
    }
}
