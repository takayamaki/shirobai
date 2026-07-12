//! Parser-gem `send` node bounds for prism `CallNode`s.
//!
//! Prism attaches a literal `{ }` / `do ... end` block to the `CallNode`
//! itself, so `call.location()` spans through the block's closing `}` /
//! `end`. The parser gem models the same code as `(block (send ...) ...)`:
//! the `send` node stops at the closing paren, else the last argument, else
//! the method name. Rules that mirror stock behaviour keyed on the SEND
//! node's range (not the whole block expression) use this helper to
//! reproduce the parser-gem end offset.

/// End offset of the parser-gem `send` node for `call`.
///
/// Identical to `call.location().end_offset()` unless a literal block
/// (`BlockNode`) is attached. A `BlockArgumentNode` (`&blk`) is NOT a literal
/// block: parser treats it as the trailing argument, and prism's location
/// already ends there.
pub(crate) fn send_node_end_offset(call: &ruby_prism::CallNode<'_>) -> usize {
    let has_literal_block = call
        .block()
        .is_some_and(|b| b.as_block_node().is_some());
    if !has_literal_block {
        return call.as_node().location().end_offset();
    }
    if let Some(close) = call.closing_loc() {
        return close.end_offset();
    }
    if let Some(last) = call
        .arguments()
        .and_then(|a| a.arguments().iter().last())
    {
        return last.location().end_offset();
    }
    if let Some(msg) = call.message_loc() {
        return msg.end_offset();
    }
    call.as_node().location().end_offset()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Offset the send node of the FIRST call in `src` ends at, computed via
    /// a parse of the snippet.
    fn first_call_send_end(src: &str) -> usize {
        let parsed = ruby_prism::parse(src.as_bytes());
        let node = parsed.node();
        find_call_end(&node).expect("no call node in snippet")
    }

    fn find_call_end(node: &ruby_prism::Node<'_>) -> Option<usize> {
        if let Some(c) = node.as_call_node() {
            return Some(send_node_end_offset(&c));
        }
        if let Some(p) = node.as_program_node() {
            for s in p.statements().body().iter() {
                if let Some(e) = find_call_end(&s) {
                    return Some(e);
                }
            }
        }
        None
    }

    #[test]
    fn no_block_is_location_end() {
        let src = "foo(a, b)\n";
        assert_eq!(first_call_send_end(src), src.find(')').unwrap() + 1);
    }

    #[test]
    fn paren_call_with_do_block_ends_at_close_paren() {
        let src = "foo(a, b) do |x|\n  x\nend\n";
        assert_eq!(first_call_send_end(src), src.find(')').unwrap() + 1);
    }

    #[test]
    fn parenless_call_with_do_block_ends_at_last_arg() {
        let src = "foo a, bb do |x|\n  x\nend\n";
        assert_eq!(first_call_send_end(src), src.find("bb").unwrap() + 2);
    }

    #[test]
    fn bare_call_with_brace_block_ends_at_method_name() {
        let src = "foo.bar { |x| x }\n";
        assert_eq!(first_call_send_end(src), src.find("bar").unwrap() + 3);
    }

    #[test]
    fn block_pass_is_not_a_literal_block() {
        let src = "foo a, &blk\n";
        assert_eq!(first_call_send_end(src), src.find("&blk").unwrap() + 4);
    }
}
