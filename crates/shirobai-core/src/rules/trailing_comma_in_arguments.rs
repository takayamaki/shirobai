//! `Style/TrailingCommaInArguments`.
//!
//! Checks for a trailing comma after the last argument of a parenthesized
//! method call (or an index `[]` call). The `EnforcedStyleForMultiline` config
//! decides whether a multi-line call *should* carry a trailing comma:
//!
//! - `no_comma`: never; a trailing comma is always an offense.
//! - `comma`: required only when every item is on its own line.
//! - `consistent_comma`: required unless the method name and arguments share a
//!   line.
//! - `diff_comma`: required only when the last item immediately precedes a
//!   newline.
//!
//! Regardless of style, a trailing comma in a single-line call is always an
//! offense.
//!
//! Reconstructed over Prism, mirroring stock's `on_send`/`on_csend` (a
//! parser-gem `(send …)` / `(csend …)` is a Prism `CallNode`) and the shared
//! `TrailingComma#check` mixin. The trigger guard is `node.arguments? &&
//! (node.parenthesized? || node.method?(:[]))`:
//!
//! - `arguments?` -> the `CallNode` has a non-empty `ArgumentsNode`.
//! - `parenthesized?` -> the opening loc is exactly `(`. (An index call's
//!   opening is `[`, a no-paren call has none.)
//! - `method?(:[])` -> the call name is exactly `[]`. This deliberately excludes
//!   `[]=` (index assignment), which parser-gem represents as `(indexasgn …)`
//!   and never routes through `on_send`; Prism keeps it a `CallNode` named
//!   `:[]=`, so the exact-name match is what filters it out.
//!
//! `check` builds the range `after_last_item = [last_argument.end, node.end)`
//! and looks for a comma reachable through leading whitespace (the heredoc-aware
//! regex). If found and not inside a comment, it is an offense unless the style
//! wants the comma there (`avoid_comma`). Otherwise, if the style wants a comma
//! and there is none, that is a "put a comma" offense (`put_comma`), skipped
//! when the last argument is a block-pass (`&block`).
//!
//! Division of labour with the Ruby wrapper: Rust decides which calls offend,
//! the caret range, the message selector, and the single corrector op (remove
//! the trailing comma, or insert a comma after a range — stock's
//! `PunctuationCorrector.swap_comma`). The wrapper turns the op into the
//! corrector call and selects the message text. No Ruby string semantics are
//! involved, so the corrector op is fully computed here.

use ruby_prism::Node;

use super::line_index::LineIndex;

/// Configured `EnforcedStyleForMultiline`.
pub const STYLE_NO_COMMA: u8 = 0;
pub const STYLE_COMMA: u8 = 1;
pub const STYLE_CONSISTENT_COMMA: u8 = 2;
pub const STYLE_DIFF_COMMA: u8 = 3;

/// Message selector. `AVOID_*` mirror the style-specific `extra_avoid_comma_info`
/// suffixes; `PUT` is the style-independent "Put a comma" message.
pub const MSG_AVOID_NO_COMMA: u8 = 0;
pub const MSG_AVOID_COMMA: u8 = 1;
pub const MSG_AVOID_CONSISTENT_COMMA: u8 = 2;
pub const MSG_AVOID_DIFF_COMMA: u8 = 3;
pub const MSG_PUT: u8 = 4;

/// Corrector op kind. `AVOID` removes the one-char comma at the caret range
/// (`swap_comma` sees a `,`); `PUT` inserts a comma after the caret range
/// (`swap_comma` sees a non-comma).
pub const FIX_AVOID: u8 = 0;
pub const FIX_PUT: u8 = 1;

/// `Style/TrailingCommaInArguments` configuration.
#[derive(Clone, Copy)]
pub struct Config {
    /// `EnforcedStyleForMultiline`.
    pub style: u8,
}

/// One offense. `[start_offset, end_offset)` is the caret range; it is also the
/// range the corrector op operates on (`FIX_AVOID` removes it, `FIX_PUT`
/// inserts a comma after it).
pub struct TrailingCommaInArgumentsOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: u8,
    pub fix: u8,
}

pub fn check_trailing_comma_in_arguments(
    source: &[u8],
    cfg: &Config,
) -> Vec<TrailingCommaInArgumentsOffense> {
    let mut rule = build_rule(source, cfg);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

pub(crate) fn build_rule<'a>(source: &'a [u8], cfg: &Config) -> Visitor<'a> {
    // Comments and the line index are collected here, before `dispatch::run`
    // enters the shared `parse_cache` walk: re-borrowing the single-RefCell
    // parse cache mid-walk would panic (see the trap table).
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    let comments = super::parse_cache::comment_ranges(source);
    Visitor {
        source,
        cfg: *cfg,
        line_index,
        comments,
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    cfg: Config,
    line_index: std::rc::Rc<LineIndex>,
    /// `(start, end)` byte ranges of every comment, ascending by start.
    comments: Vec<(usize, usize)>,
    pub offenses: Vec<TrailingCommaInArgumentsOffense>,
}

impl Visitor<'_> {
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn src(&self, a: usize, b: usize) -> &[u8] {
        &self.source[a..b]
    }

    fn on_call(&mut self, call: &ruby_prism::CallNode<'_>) {
        // `node.arguments? && (node.parenthesized? || node.method?(:[]))`.
        //
        // In parser-gem a block-pass (`&block`) is the last *argument*; in Prism
        // it is the separate `block` field (a `BlockArgumentNode`), not part of
        // `arguments()`. A `do…end`/`{}` block is a `BlockNode` and is *not* an
        // argument. So the argument list is `arguments()` plus a trailing
        // `BlockArgumentNode` block, if any.
        let mut args: Vec<Node<'_>> = match call.arguments() {
            Some(a) => a.arguments().iter().collect(),
            None => Vec::new(),
        };
        if let Some(block) = call.block()
            && matches!(block, Node::BlockArgumentNode { .. })
        {
            args.push(block);
        }
        if args.is_empty() {
            return;
        }
        let parenthesized = call
            .opening_loc()
            .is_some_and(|o| self.src(o.start_offset(), o.end_offset()) == b"(");
        let is_index = call.name().as_slice() == b"[]";
        if !parenthesized && !is_index {
            return;
        }

        // `node.source_range.end_pos`. Stock's `on_send` sees the *send* node,
        // whose source range ends at the closing bracket — when a `do…end`/`{}`
        // block is attached, parser-gem keeps the block on the parent, so the
        // send still ends at `)`. Prism's `CallNode` location, in contrast,
        // spans the block. Use the closing bracket end to match stock.
        let node_end = self.effective_node_end(call);
        let last_arg = args.last().unwrap();
        let begin_pos = last_arg.location().end_offset();

        self.check(call, &args, begin_pos, node_end);
    }

    /// `node.source_range.end_pos`: the offset just past the closing bracket.
    /// For our trigger set (parenthesized or `[]`) the closing bracket is always
    /// present; the fallback to the node end is defensive.
    fn effective_node_end(&self, call: &ruby_prism::CallNode<'_>) -> usize {
        match call.closing_loc() {
            Some(c) => c.end_offset(),
            None => call.location().end_offset(),
        }
    }

    /// `TrailingComma#check`.
    fn check(
        &mut self,
        call: &ruby_prism::CallNode<'_>,
        args: &[Node<'_>],
        begin_pos: usize,
        end_pos: usize,
    ) {
        let range_src = self.src(begin_pos, end_pos);
        let comma_offset = comma_offset(range_src, any_heredoc(args));

        if let Some(off) = comma_offset {
            let comma_pos = begin_pos + off;
            if !self.inside_comment(begin_pos, comma_pos) {
                self.check_comma(call, args, comma_pos);
            } else if self.should_have_comma(call, args) {
                self.put_comma(args);
            }
        } else if self.should_have_comma(call, args) {
            self.put_comma(args);
        }
    }

    /// `check_comma`: an offense unless the style wants the comma there.
    fn check_comma(&mut self, call: &ruby_prism::CallNode<'_>, args: &[Node<'_>], comma_pos: usize) {
        if self.should_have_comma(call, args) {
            return;
        }
        let message = match self.cfg.style {
            STYLE_COMMA => MSG_AVOID_COMMA,
            STYLE_CONSISTENT_COMMA => MSG_AVOID_CONSISTENT_COMMA,
            STYLE_DIFF_COMMA => MSG_AVOID_DIFF_COMMA,
            _ => MSG_AVOID_NO_COMMA,
        };
        // `avoid_comma`: caret is the one-char comma; the corrector removes it.
        self.offenses.push(TrailingCommaInArgumentsOffense {
            start_offset: comma_pos,
            end_offset: comma_pos + 1,
            message,
            fix: FIX_AVOID,
        });
    }

    /// `put_comma`: insert a comma after the last item's last-line content,
    /// unless that item is a block-pass.
    fn put_comma(&mut self, args: &[Node<'_>]) {
        let last = args.last().unwrap();
        if matches!(last, Node::BlockArgumentNode { .. }) {
            return;
        }
        let (start, end) = (last.location().start_offset(), last.location().end_offset());
        let range = self.autocorrect_range(start, end);
        self.offenses.push(TrailingCommaInArgumentsOffense {
            start_offset: range.0,
            end_offset: range.1,
            message: MSG_PUT,
            fix: FIX_PUT,
        });
    }

    /// `autocorrect_range`: from the last item, drop everything up to and
    /// including its last newline, then skip leading whitespace; the range runs
    /// from there to the item's end.
    fn autocorrect_range(&self, start: usize, end: usize) -> (usize, usize) {
        let src = self.src(start, end);
        // `ix = source.rindex("\n") || 0`. Ruby's `rindex` returns the index of
        // the `\n`; `source[ix..] =~ /\S/` then advances past it (and any
        // further whitespace). When there is no `\n`, `ix` starts at 0.
        let ix = src.iter().rposition(|&b| b == b'\n').unwrap_or(0);
        let advance = src[ix..]
            .iter()
            .position(|&b| !is_space(b))
            .unwrap_or(0);
        (start + ix + advance, end)
    }

    /// `inside_comment?`: the comma lies inside a comment on its line.
    fn inside_comment(&self, range_begin: usize, comma_pos: usize) -> bool {
        // `processed_source.comment_at_line(range.line)`: the comment whose
        // start line equals the range's begin line, if any.
        let line = self.line_of(range_begin);
        self.comments.iter().any(|&(cs, _)| {
            self.line_of(cs) == line && cs < comma_pos
        })
    }

    /// `should_have_comma?(style, node)`.
    fn should_have_comma(&self, call: &ruby_prism::CallNode<'_>, args: &[Node<'_>]) -> bool {
        match self.cfg.style {
            STYLE_COMMA => self.multiline(call, args) && self.no_elements_on_same_line(call, args),
            STYLE_CONSISTENT_COMMA => {
                self.multiline(call, args) && !self.method_name_and_arguments_on_same_line(call, args)
            }
            STYLE_DIFF_COMMA => self.multiline(call, args) && self.last_item_precedes_newline(call, args),
            _ => false,
        }
    }

    /// `multiline?` = `node.multiline? && !allowed_multiline_argument?`.
    fn multiline(&self, call: &ruby_prism::CallNode<'_>, args: &[Node<'_>]) -> bool {
        let s = call.location().start_offset();
        let e = self.effective_node_end(call);
        let node_multiline = self.line_of(s) != self.line_of(e);
        node_multiline && !self.allowed_multiline_argument(call, args)
    }

    /// `allowed_multiline_argument?` = `elements(node).one? &&
    /// !begins_its_line?(node_end_location)`.
    fn allowed_multiline_argument(&self, call: &ruby_prism::CallNode<'_>, args: &[Node<'_>]) -> bool {
        let elems = self.elements(args);
        elems.len() == 1 && !self.begins_its_line(self.node_end_location(call))
    }

    /// `node_end_location` = `node.loc.end || …`. For the calls we trigger on
    /// (parenthesized or `[]`) the closing bracket is always present.
    fn node_end_location(&self, call: &ruby_prism::CallNode<'_>) -> usize {
        match call.closing_loc() {
            Some(c) => c.start_offset(),
            // `node.source_range.end.adjust(begin_pos: -1)`: the byte before the
            // node's end. Defensive; unreachable for our trigger set.
            None => call.location().end_offset().saturating_sub(1),
        }
    }

    /// `elements(node)`: each argument, except a multi-line braceless hash is
    /// expanded into its pairs (`children`). Returns each element's
    /// `(start, end)` source range.
    fn elements(&self, args: &[Node<'_>]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for arg in args {
            if let Node::KeywordHashNode { .. } = arg {
                // A braceless hash argument: `hash_type? && !braces?` is always
                // true for a `KeywordHashNode` (it never has braces). Promote
                // its elements when it is multi-line.
                let kh = arg.as_keyword_hash_node().unwrap();
                let (hs, he) = (kh.location().start_offset(), kh.location().end_offset());
                if self.line_of(hs) != self.line_of(he) {
                    for el in kh.elements().iter() {
                        out.push((el.location().start_offset(), el.location().end_offset()));
                    }
                    continue;
                }
            }
            out.push((arg.location().start_offset(), arg.location().end_offset()));
        }
        out
    }

    /// `no_elements_on_same_line?`: no two consecutive items (the elements, then
    /// the node end location) share a line.
    fn no_elements_on_same_line(&self, call: &ruby_prism::CallNode<'_>, args: &[Node<'_>]) -> bool {
        let mut ranges = self.elements(args);
        let end = self.node_end_location(call);
        ranges.push((end, end));
        // `each_cons(2).none? { |a, b| a.last_line == b.line }`.
        for w in ranges.windows(2) {
            let a_last_line = self.line_of(w[0].1);
            let b_line = self.line_of(w[1].0);
            if a_last_line == b_line {
                return false;
            }
        }
        true
    }

    /// `method_name_and_arguments_on_same_line?`.
    fn method_name_and_arguments_on_same_line(
        &self,
        call: &ruby_prism::CallNode<'_>,
        args: &[Node<'_>],
    ) -> bool {
        // `return false if !node.call_type? || node.last_line != last_argument.last_line`.
        // Every trigger node is call_type, so only the line check applies.
        let last_arg = args.last().unwrap();
        let node_last_line = self.line_of(self.effective_node_end(call));
        let last_arg_last_line = self.line_of(last_arg.location().end_offset());
        if node_last_line != last_arg_last_line {
            return false;
        }
        // `return true if last_argument.hash_type? && last_argument.braces?`.
        if let Node::HashNode { .. } = last_arg {
            return true;
        }
        // `line = selector&.line || node.loc.line; line == last_argument.last_line`.
        let line = match call.message_loc() {
            Some(m) => self.line_of(m.start_offset()),
            None => self.line_of(call.location().start_offset()),
        };
        line == last_arg_last_line
    }

    /// `last_item_precedes_newline?`: the text from the last child's end to the
    /// node's end starts with an optional comma, whitespace, optional comment,
    /// then a newline.
    fn last_item_precedes_newline(&self, call: &ruby_prism::CallNode<'_>, args: &[Node<'_>]) -> bool {
        // `node.children.last` is the last argument for a call node.
        let last_arg = args.last().unwrap();
        let from = last_arg.location().end_offset();
        let to = self.effective_node_end(call);
        starts_with_optional_comma_to_newline(self.src(from, to))
    }

    /// `Util.begins_its_line?(range)`: the first non-whitespace byte on the
    /// range's line is exactly at the range start.
    fn begins_its_line(&self, off: usize) -> bool {
        let line_start = self.line_index.line_start(off);
        let mut p = line_start;
        while p < self.source.len() && self.source[p] != b'\n' && is_space(self.source[p]) {
            p += 1;
        }
        p == off
    }
}

/// `comma_offset(items, range)`: the index of the first comma reachable through
/// leading whitespace, or `None`. With a heredoc among the items the leading
/// whitespace may not include a newline (`/\A[^\S\n]*,/`); otherwise any
/// whitespace is allowed (`/\A\s*,/`).
fn comma_offset(range_src: &[u8], has_heredoc: bool) -> Option<usize> {
    let mut i = 0;
    while i < range_src.len() {
        match range_src[i] {
            b',' => return Some(i),
            b'\n' if has_heredoc => return None,
            b if is_space(b) => i += 1,
            _ => return None,
        }
    }
    None
}

/// `/,?\s*(#.*)?\n/` anchored at the start of `src`: optional comma, any
/// whitespace, an optional `#`-comment running to end of line, then a newline.
fn starts_with_optional_comma_to_newline(src: &[u8]) -> bool {
    let mut i = 0;
    // Optional single comma.
    if src.first() == Some(&b',') {
        i += 1;
    }
    // `\s*` — but a `\n` here both satisfies `\s*` greedily and the trailing
    // `\n`. Ruby's regex backtracks, so the simplest faithful port is: scan
    // `\s*`, and a match succeeds if we land on a `\n`, or on a `#` whose line
    // ends in `\n`. We mirror the regex by scanning whitespace, then allowing an
    // optional comment, then requiring a `\n`.
    //
    // Because `\s` includes `\n`, the regex can match by consuming whitespace up
    // to and including a `\n`. Equivalent test: somewhere in the leading
    // whitespace run (optionally followed by a `# …` comment) there is a `\n`.
    let start = i;
    while i < src.len() && is_space_no_newline(src[i]) {
        i += 1;
    }
    match src.get(i) {
        Some(b'\n') => true,
        Some(b'#') => {
            // `(#.*)?\n`: a comment to end of line, then `\n`.
            while i < src.len() && src[i] != b'\n' {
                i += 1;
            }
            i < src.len() && src[i] == b'\n'
        }
        _ => {
            // No comment and no inline `\n`; the whitespace run itself must have
            // contained a `\n` (consumed above only if `is_space_no_newline`
            // failed on it). Re-scan the run allowing `\n`.
            let mut j = start;
            while j < src.len() && is_space(src[j]) {
                if src[j] == b'\n' {
                    return true;
                }
                j += 1;
            }
            false
        }
    }
}

/// Ruby's `/\s/` over a single byte.
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// `/\s/` without the newline (the `[^\S\n]` class).
fn is_space_no_newline(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c)
}

/// `any_heredoc?(items)`: any argument is (or wraps) a heredoc.
fn any_heredoc(args: &[Node<'_>]) -> bool {
    args.iter().any(heredoc)
}

/// `heredoc?(node)`, ported to Prism node types.
fn heredoc(node: &Node<'_>) -> bool {
    // `node.loc?(:heredoc_body)`: a string/xstring literal whose opening is a
    // heredoc marker (`<<…`).
    if is_heredoc_literal(node) {
        return true;
    }
    match node {
        // `heredoc_send?`: parser-gem `(send recv meth *args)`. children.size==2
        // (`recv.meth`, no args) -> check the receiver; size>2 (has args) ->
        // check the last argument.
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            let args: Vec<Node<'_>> = match call.arguments() {
                Some(a) => a.arguments().iter().collect(),
                None => Vec::new(),
            };
            if !args.is_empty() {
                heredoc(args.last().unwrap())
            } else if let Some(recv) = call.receiver() {
                heredoc(&recv)
            } else {
                false
            }
        }
        // `node.type?(:pair, :hash)` -> `heredoc?(node.children.last)` (the
        // value of the last pair / the last hash element's value). A braceless
        // hash argument is a `KeywordHashNode`; a braced one is a `HashNode`.
        Node::HashNode { .. } => last_hash_value(node.as_hash_node().unwrap().elements()),
        Node::KeywordHashNode { .. } => {
            last_hash_value(node.as_keyword_hash_node().unwrap().elements())
        }
        Node::AssocNode { .. } => {
            let pair = node.as_assoc_node().unwrap();
            heredoc(&pair.value())
        }
        _ => false,
    }
}

/// `heredoc?` of the last element's value (for a hash node).
fn last_hash_value(elements: ruby_prism::NodeList<'_>) -> bool {
    match elements.iter().last() {
        Some(Node::AssocNode { .. }) => {
            let last = elements.iter().last().unwrap();
            let pair = last.as_assoc_node().unwrap();
            heredoc(&pair.value())
        }
        Some(other) => heredoc(&other),
        None => false,
    }
}

/// A string/xstring literal whose opening marker is `<<…` (a heredoc).
fn is_heredoc_literal(node: &Node<'_>) -> bool {
    let opening = match node {
        Node::StringNode { .. } => node.as_string_node().unwrap().opening_loc(),
        Node::InterpolatedStringNode { .. } => {
            node.as_interpolated_string_node().unwrap().opening_loc()
        }
        // `XStringNode`'s opening is always present (the backtick / `<<` marker).
        Node::XStringNode { .. } => Some(node.as_x_string_node().unwrap().opening_loc()),
        Node::InterpolatedXStringNode { .. } => {
            Some(node.as_interpolated_x_string_node().unwrap().opening_loc())
        }
        _ => return false,
    };
    opening.is_some_and(|o| o.as_slice().starts_with(b"<<"))
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.on_call(&call);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Vec<(usize, usize, u8, u8)> {
        check_trailing_comma_in_arguments(source.as_bytes(), &Config { style })
            .iter()
            .map(|o| (o.start_offset, o.end_offset, o.message, o.fix))
            .collect()
    }

    #[test]
    fn no_comma_single_line_trailing() {
        // `some_method(a, b,)` -> avoid comma at the `,`.
        let r = run("some_method(a, b,)\n", STYLE_NO_COMMA);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].2, MSG_AVOID_NO_COMMA);
        assert_eq!(r[0].3, FIX_AVOID);
        // The comma is the one-char range.
        assert_eq!(&"some_method(a, b,)\n".as_bytes()[r[0].0..r[0].1], b",");
    }

    #[test]
    fn index_call_trailing() {
        let r = run("object[1, 2,]\n", STYLE_NO_COMMA);
        assert_eq!(r.len(), 1);
        assert_eq!(&"object[1, 2,]\n".as_bytes()[r[0].0..r[0].1], b",");
    }

    #[test]
    fn index_assignment_not_triggered() {
        // `obj[1, 2,] = x` is `:[]=`, not `:[]` -> no offense.
        assert!(run("obj[1, 2,] = x\n", STYLE_NO_COMMA).is_empty());
    }

    #[test]
    fn no_paren_call_not_triggered() {
        assert!(run("puts a, b,\n", STYLE_NO_COMMA).is_empty());
    }

    #[test]
    fn dot_call_trailing() {
        let r = run("func.(1, 2,)\n", STYLE_NO_COMMA);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn put_comma_two_per_line_consistent() {
        // consistent_comma wants a comma after `b`.
        let r = run("some_method(a, b\n)\n", STYLE_CONSISTENT_COMMA);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].2, MSG_PUT);
        assert_eq!(r[0].3, FIX_PUT);
    }

    #[test]
    fn comma_style_each_own_line_put() {
        let r = run("m(\n  a,\n  b\n)\n", STYLE_COMMA);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].2, MSG_PUT);
    }

    #[test]
    fn comma_style_two_on_line_no_put() {
        assert!(run("m(\n  a, b\n)\n", STYLE_COMMA).is_empty());
    }

    #[test]
    fn block_pass_not_put() {
        // consistent_comma but last item is `&block` -> skipped.
        assert!(run("m(\n  a,\n  &block\n)\n", STYLE_CONSISTENT_COMMA).is_empty());
    }

    #[test]
    fn single_multiline_braced_arg_allowed() {
        // A single multi-line argument with closing bracket on its own line is
        // multiline, but with closing not beginning its line it is allowed.
        assert!(run("EmailWorker.perform_async({\n  a: 1\n})\n", STYLE_CONSISTENT_COMMA).is_empty());
    }

    #[test]
    fn diff_comma_last_precedes_newline() {
        let r = run("m(\n  a,\n  b\n)\n", STYLE_DIFF_COMMA);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].2, MSG_PUT);
    }

    #[test]
    fn diff_comma_last_on_close_line_avoid() {
        // last argument on same line as closing bracket, with trailing comma.
        let r = run("m(a: 1,\n  c: 2,)\n", STYLE_DIFF_COMMA);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].2, MSG_AVOID_DIFF_COMMA);
        assert_eq!(r[0].3, FIX_AVOID);
    }

    #[test]
    fn block_pass_last_arg_no_false_comma() {
        // `Dir.chdir(dir, &block)`: the `&block` is the last argument (in
        // parser-gem terms), so the `,` before it is not trailing.
        assert!(run("Dir.chdir(dir, &block)\n", STYLE_NO_COMMA).is_empty());
    }

    #[test]
    fn block_pass_only_arg_no_offense() {
        assert!(run("m(&block)\n", STYLE_NO_COMMA).is_empty());
    }

    #[test]
    fn do_end_block_not_an_argument() {
        // A `do…end` block is not an argument; `m(a,)` has a trailing comma.
        let r = run("m(a,) do\nend\n", STYLE_NO_COMMA);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn heredoc_arg_no_offense() {
        assert!(run("route(1, <<-HELP.chomp)\n...\nHELP\n", STYLE_NO_COMMA).is_empty());
    }

    #[test]
    fn no_offense_when_already_clean() {
        assert!(run("some_method(a, b, c)\n", STYLE_NO_COMMA).is_empty());
    }
}
