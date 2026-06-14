//! `Style/NestedParenthesizedCalls`: flags an unparenthesized method call that
//! appears as an argument of a parenthesized method call, and adds parentheses
//! around the inner call's arguments.
//!
//! Mirrors `vendor/rubocop/lib/rubocop/cop/style/nested_parenthesized_calls.rb`:
//!
//! - On a `send`/`csend` whose `parenthesized?` is true (loc_is?(:end, ')')) —
//!   in prism, the call's `closing_loc` is `Some` and starts with `)`.
//! - Iterate the call's argument list and pick every direct `send`/`csend`
//!   child (parser-gem `each_child_node(:call)` — calls only, not blocks/ifs).
//! - Skip when the child is "allowed":
//!   - has no arguments,
//!   - is itself parenthesized,
//!   - is a setter (`name=`, but not the comparison operators),
//!   - is an operator method (rubocop-ast `OPERATOR_METHODS`),
//!   - is on the `AllowedMethods` list AND both the inner call and the outer
//!     argument list contain exactly one argument.
//! - Offense range: the child's full source range.
//! - Autocorrect (always attached, matches stock):
//!   - Replace the range from the trailing whitespace before the first inner
//!     argument up to its start with `(` (stock's
//!     `range_with_surrounding_space(first_arg.begin, side: :left,
//!     whitespace: true, continuations: true)`).
//!   - Insert `)` right after the last inner argument's range.
//!
//! `AllowedMethods` is per-cop config; the rule takes it as a slice of names.

use ruby_prism::{Node, Visit};

#[derive(Debug, Clone)]
pub struct NestedParenthesizedCallsOffense {
    /// Start byte of the inner call's full source range (offense highlight).
    pub start_offset: usize,
    /// End byte of the inner call's full source range (also the message source).
    pub end_offset: usize,
    /// Start byte of the autocorrect replacement region (the whitespace run
    /// before the first inner argument, computed by stock's
    /// `range_with_surrounding_space(side: :left, whitespace: true,
    /// continuations: true)`). The wrapper replaces `[ac_open_start, ac_open_end)`
    /// with `(`.
    pub ac_open_start: usize,
    /// End byte of the autocorrect replacement region (the first inner
    /// argument's `begin_pos`).
    pub ac_open_end: usize,
    /// Byte offset of the last inner argument's `end_pos`; the wrapper inserts
    /// `)` at this point.
    pub ac_close_pos: usize,
}

pub fn check_nested_parenthesized_calls(
    source: &[u8],
    allowed_methods: &[String],
) -> Vec<NestedParenthesizedCallsOffense> {
    let mut visitor = build_rule(source, allowed_methods);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

pub(crate) fn build_rule<'s>(
    source: &'s [u8],
    allowed_methods: &[String],
) -> NestedParenthesizedCallsVisitor<'s> {
    NestedParenthesizedCallsVisitor {
        source,
        allowed: allowed_methods
            .iter()
            .map(|s| s.as_bytes().to_vec())
            .collect(),
        offenses: Vec::new(),
    }
}

pub(crate) struct NestedParenthesizedCallsVisitor<'s> {
    source: &'s [u8],
    allowed: Vec<Vec<u8>>,
    pub(crate) offenses: Vec<NestedParenthesizedCallsOffense>,
}

impl<'s> NestedParenthesizedCallsVisitor<'s> {
    fn check_call(&mut self, call: &ruby_prism::CallNode<'_>) {
        // `node.parenthesized?` — rubocop-ast checks `loc_is?(:end, ')')`. In
        // prism, the call has a `closing_loc` of `Some` when a closing token is
        // present, and the closing token of a regular paren'd call is `)`. An
        // index call (`a[x]`) also has a closing_loc whose first byte is `]`,
        // which is correctly rejected here.
        let Some(closing) = call.closing_loc() else {
            return;
        };
        let closing_start = closing.start_offset();
        if closing_start >= self.source.len() || self.source[closing_start] != b')' {
            return;
        }

        let parent_arg_count = call
            .arguments()
            .map(|args| args.arguments().iter().count())
            .unwrap_or(0);
        let Some(args) = call.arguments() else { return };

        for child in args.arguments().iter() {
            let Some(nested) = direct_call(&child) else {
                continue;
            };
            if self.allowed_omission(&nested, parent_arg_count) {
                continue;
            }
            self.emit(&nested);
        }
    }

    fn allowed_omission(&self, nested: &ruby_prism::CallNode<'_>, parent_arg_count: usize) -> bool {
        // `!send_node.arguments?` — parser-gem's `:send` node treats a block
        // argument (`&block`) as a regular argument; prism splits it off into
        // a `block: Option<BlockArgumentNode>` field. Count the BlockArgument
        // as an argument here so `qux &block_var` matches parser-gem's
        // `arguments? == true`.
        let nested_arg_count = arg_count(nested);
        if nested_arg_count == 0 {
            return true;
        }
        // `send_node.parenthesized?`
        if let Some(closing) = nested.closing_loc() {
            let start = closing.start_offset();
            if start < self.source.len() && self.source[start] == b')' {
                return true;
            }
        }
        let name = nested.name();
        let name = name.as_slice();
        // `send_node.setter_method?` — name ends with `=` and is not a comparison
        // operator. rubocop-ast `setter_method?` checks `loc.operator` is `:=`.
        if is_setter_name(name) {
            return true;
        }
        // `send_node.operator_method?` — name is in `OPERATOR_METHODS`.
        if is_operator_method(name) {
            return true;
        }
        // `allowed?(send_node)` — `parent.arguments.one? && allowed_method? &&
        // arguments.one?`. `parent.arguments.one?` is the OUTER call's argument
        // count being exactly one.
        if parent_arg_count == 1 && nested_arg_count == 1 && self.is_allowed(name) {
            return true;
        }
        false
    }

    fn is_allowed(&self, name: &[u8]) -> bool {
        self.allowed.iter().any(|n| n.as_slice() == name)
    }

    fn emit(&mut self, nested: &ruby_prism::CallNode<'_>) {
        let loc = nested.location();
        let start_offset = loc.start_offset();
        let end_offset = loc.end_offset();

        // `first_argument.source_range` / `last_argument.source_range` in
        // parser-gem's view: the BlockArgument (`&block`) counts as an
        // argument and is included in the list of children. We collect the
        // regular arguments first, then append the BlockArgument's location
        // at the end (block_pass always appears last in parser-gem's order).
        let (first_begin, last_end) = arg_range_bounds(nested);

        // `range_with_surrounding_space(first_arg.begin, side: :left,
        // whitespace: true, continuations: true)`. The starting range is
        // zero-width at first_begin; we strip [ \t] then \\\n then \n then \s
        // moving left, matching stock's `final_pos` exactly (newlines default
        // true; whitespace requested).
        let ac_open_start = strip_leading_whitespace(self.source, first_begin);
        let ac_open_end = first_begin;
        let ac_close_pos = last_end;

        self.offenses.push(NestedParenthesizedCallsOffense {
            start_offset,
            end_offset,
            ac_open_start,
            ac_open_end,
            ac_close_pos,
        });
    }
}

/// Parser-gem `node.arguments.size` for a `:send`/`:csend` CallNode: the
/// number of regular arguments plus 1 if a `block_pass` (`&block`) is present
/// (parser-gem inlines block_pass into the arguments list while prism splits
/// it into a separate `block: BlockArgumentNode` field).
fn arg_count(call: &ruby_prism::CallNode<'_>) -> usize {
    let mut count = call
        .arguments()
        .map(|args| args.arguments().iter().count())
        .unwrap_or(0);
    if matches!(call.block(), Some(ruby_prism::Node::BlockArgumentNode { .. })) {
        count += 1;
    }
    count
}

/// `(first_argument.source_range.begin_pos, last_argument.source_range.end_pos)`:
/// the byte offsets stock pulls off `node.first_argument` / `node.last_argument`
/// for the autocorrect anchors. Mirrors parser-gem's argument list ordering
/// (regular arguments first, BlockArgument last).
fn arg_range_bounds(call: &ruby_prism::CallNode<'_>) -> (usize, usize) {
    let mut first_begin: Option<usize> = None;
    let mut last_end: Option<usize> = None;
    if let Some(args) = call.arguments() {
        for arg in args.arguments().iter() {
            let loc = arg.location();
            first_begin.get_or_insert(loc.start_offset());
            last_end = Some(loc.end_offset());
        }
    }
    if let Some(ruby_prism::Node::BlockArgumentNode { .. }) = call.block() {
        let block = call.block().unwrap();
        let loc = block.location();
        first_begin.get_or_insert(loc.start_offset());
        last_end = Some(loc.end_offset());
    }
    (
        first_begin.expect("emit only called when arg_count > 0"),
        last_end.expect("emit only called when arg_count > 0"),
    )
}

/// rubocop-ast `each_child_node(:call)` yields direct children whose type is
/// `:send` or `:csend`. In prism, the corresponding node kind is `CallNode`. A
/// braceless-hash keyword-hash argument is wrapped in `KeywordHashNode` /
/// `HashNode` (not CallNode), block arguments / splat / kwsplat are
/// `BlockArgumentNode` / `SplatNode` / `KeywordHashNode` — none are
/// CallNode-typed, so this filter naturally drops them. The outer call's own
/// arguments are accessed via prism `ArgumentsNode::arguments`, mirroring
/// rubocop-ast's `node.arguments` accessor.
fn direct_call<'a>(node: &Node<'a>) -> Option<ruby_prism::CallNode<'a>> {
    node.as_call_node()
}

/// `name.end_with?('=')` && not a comparison operator (`==`, `===`, `!=`,
/// `<=`, `>=`). Matches `RuboCop::AST::Node#assignment_method?` semantics
/// used by `setter_method?` (which additionally requires `loc.operator == :=`,
/// trivially true for a normal `x.foo = 1` call).
fn is_setter_name(name: &[u8]) -> bool {
    if !name.ends_with(b"=") {
        return false;
    }
    !matches!(name, b"==" | b"===" | b"!=" | b"<=" | b">=")
}

/// rubocop-ast `OPERATOR_METHODS` — every operator that can be defined as a
/// method. Includes the unary / binary arithmetic / comparison / bitwise
/// operators plus `[]`, `[]=`, `!`, `!~`, the negative-unary
/// pseudo-names `-@` / `+@`, etc. The list is closed (no Ruby version adds new
/// ones in practice), so we encode it verbatim here.
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

/// Stock `range_with_surrounding_space(side: :left, whitespace: true,
/// continuations: true)` (with the default `newlines: true`): from a
/// zero-width range at `pos`, strip leftward `[ \t]`, then `\\\n`, then `\n`,
/// then `\s` — in that order. Returns the new start byte. The strip never
/// crosses a multibyte boundary because every consumed character is ASCII.
fn strip_leading_whitespace(src: &[u8], mut pos: usize) -> usize {
    // 1. `[ \t]` moving left.
    while pos > 0 {
        let c = src[pos - 1];
        if c == b' ' || c == b'\t' {
            pos -= 1;
        } else {
            break;
        }
    }
    // 2. `\\\n` pairs moving left.
    while pos >= 2 && src[pos - 2] == b'\\' && src[pos - 1] == b'\n' {
        pos -= 2;
    }
    // 3. `\n` moving left.
    while pos > 0 && src[pos - 1] == b'\n' {
        pos -= 1;
    }
    // 4. `\s` moving left. Ruby `\s` is [ \t\r\n\f\v]; bytes already in the
    // ASCII set so byte-level comparison is exact.
    while pos > 0 {
        let c = src[pos - 1];
        if matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0x0b) {
            pos -= 1;
        } else {
            break;
        }
    }
    pos
}

impl<'pr, 's> Visit<'pr> for NestedParenthesizedCallsVisitor<'s> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        // Continue into the call's children so a nested call's own arguments
        // get checked too (`puts(foo(bar baz))` reports `bar baz`, not just
        // failing because the outer paren covered the inner).
        ruby_prism::visit_call_node(self, node);
    }
}

impl<'s> super::dispatch::Rule for NestedParenthesizedCallsVisitor<'s> {
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

    fn default_allowed() -> Vec<String> {
        ["be", "be_a", "be_an", "be_between", "be_falsey", "be_kind_of", "be_instance_of",
         "be_truthy", "be_within", "eq", "eql", "end_with", "include", "match", "raise_error",
         "respond_to", "start_with"]
            .map(String::from)
            .to_vec()
    }

    fn detect(src: &str) -> Vec<(usize, usize)> {
        let allowed = default_allowed();
        check_nested_parenthesized_calls(src.as_bytes(), &allowed)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset))
            .collect()
    }

    fn detect_full(src: &str) -> Vec<NestedParenthesizedCallsOffense> {
        let allowed = default_allowed();
        check_nested_parenthesized_calls(src.as_bytes(), &allowed)
    }

    #[test]
    fn flags_simple_nested_call() {
        let off = detect("puts(compute something)\n");
        assert_eq!(off, vec![(5, 22)]);
    }

    #[test]
    fn flags_multi_arg_nested() {
        let off = detect("puts(compute first, second)\n");
        assert_eq!(off, vec![(5, 26)]);
    }

    #[test]
    fn flags_safe_navigation_inner() {
        let off = detect("puts(receiver&.compute something)\n");
        assert_eq!(off, vec![(5, 32)]);
    }

    #[test]
    fn accepts_unparenthesized_outer() {
        assert!(detect("puts compute something\n").is_empty());
    }

    #[test]
    fn accepts_no_argument_inner() {
        assert!(detect("puts(compute)\n").is_empty());
    }

    #[test]
    fn accepts_parenthesized_inner() {
        assert!(detect("puts(compute(something))\n").is_empty());
    }

    #[test]
    fn accepts_aref() {
        assert!(detect("method(obj[1])\n").is_empty());
    }

    #[test]
    fn accepts_block_arg() {
        assert!(detect("method(block_taker { another_method 1 })\n").is_empty());
    }

    #[test]
    fn accepts_allowed_method_single_arg() {
        assert!(detect("expect(obj).to(be true)\n").is_empty());
        assert!(detect("expect(obj).to(eq 1)\n").is_empty());
    }

    #[test]
    fn flags_allowed_method_multi_arg() {
        // eq is allowed but multi arg => fall through to flag
        let off = detect("expect(obj).to(eq 1, 2)\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn accepts_setter() {
        assert!(detect("expect(object1.attr = 1).to eq 1\n").is_empty());
    }

    #[test]
    fn accepts_operator_method() {
        assert!(detect("foo(a + b)\n").is_empty());
        assert!(detect("foo(a == b)\n").is_empty());
        assert!(detect("foo(a <=> b)\n").is_empty());
    }

    #[test]
    fn autocorrect_replaces_space_and_closes() {
        let off = detect_full("puts(compute something)\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        // first_arg = `something` starts at 13; the `(` replace range is the
        // single space at position 12.
        assert_eq!(o.ac_open_start, 12);
        assert_eq!(o.ac_open_end, 13);
        assert_eq!(o.ac_close_pos, 22);
    }

    #[test]
    fn autocorrect_eats_backslash_newline() {
        let src = "puts(nex \\\n      5)\n";
        let off = detect_full(src);
        assert_eq!(off.len(), 1);
        let o = &off[0];
        // first_arg `5` starts after the spaces on the next line. AC range
        // begins at the space after `nex` (position 8).
        let arg_start = src.find('5').unwrap();
        assert_eq!(o.ac_open_end, arg_start);
        // AC start should strip the leading space + `\\\n` + spaces.
        assert_eq!(&src[o.ac_open_start..o.ac_open_end], " \\\n      ");
        assert_eq!(o.ac_close_pos, arg_start + 1);
    }

    #[test]
    fn flags_csend_outer() {
        let off = detect("a&.puts(compute foo)\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn deeply_nested_flags_inner() {
        let off = detect("a(b(c d))\n");
        // a(b(c d)) -> a's child b is paren'd, allowed.
        // b's child c is paren'd? No, `c d` is `c(d)` unparen'd. Flag c d.
        assert_eq!(off, vec![(4, 7)]);
    }

    #[test]
    fn ternary_arg_not_a_call_child() {
        assert!(detect("puts(cond ? a b : c)\n").is_empty());
    }
}
