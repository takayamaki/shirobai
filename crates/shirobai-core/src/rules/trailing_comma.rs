//! Shared logic for the `Style/TrailingCommaIn*` cop family.
//!
//! Mirrors stock's `RuboCop::Cop::Mixin::TrailingComma`, which is included by
//! `Style/TrailingCommaInArguments`, `Style/TrailingCommaInHashLiteral` and
//! `Style/TrailingCommaInArrayLiteral`. The style/message/fix wire codes, the
//! comma discovery (`comma_offset`), the heredoc detection (`heredoc?`), the
//! `put_comma` autocorrect range and the comment guard live here; the per-cop
//! modules keep only their trigger guards and the call-specific predicates.
//!
//! [`LiteralChecker`] is the literal-node flavour of the mixin's `check`:
//! stock's `check_literal` runs on a bracketed hash/array node, where
//! `method_name_and_arguments_on_same_line?` is always false (the node is not
//! a call) and `elements(node)` is simply `node.children` (the braceless-hash
//! promotion only applies to call arguments).

use std::rc::Rc;

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

/// Configuration shared by the literal cops (`EnforcedStyleForMultiline`).
#[derive(Clone, Copy)]
pub struct Config {
    pub style: u8,
}

/// One offense. `[start_offset, end_offset)` is the caret range; it is also the
/// range the corrector op operates on (`FIX_AVOID` removes it, `FIX_PUT`
/// inserts a comma after it).
pub struct TrailingCommaOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: u8,
    pub fix: u8,
}

/// The literal-node flavour of `TrailingComma#check`, shared by
/// `Style/TrailingCommaInHashLiteral` and `Style/TrailingCommaInArrayLiteral`.
/// The per-cop visitors extract the node pieces and call [`check_literal`]
/// (Self::check_literal).
pub(crate) struct LiteralChecker<'a> {
    source: &'a [u8],
    cfg: Config,
    line_index: Rc<LineIndex>,
    /// `(start, end)` byte ranges of every comment, ascending by start.
    comments: Vec<(usize, usize)>,
    pub offenses: Vec<TrailingCommaOffense>,
}

impl<'a> LiteralChecker<'a> {
    /// Comments and the line index are collected here, before `dispatch::run`
    /// enters the shared `parse_cache` walk: re-borrowing the single-RefCell
    /// parse cache mid-walk would panic (see the trap table).
    pub(crate) fn new(source: &'a [u8], cfg: Config) -> Self {
        let line_index = super::line_index::with_line_index(source, |li| li.clone());
        let comments = super::parse_cache::comment_ranges(source);
        LiteralChecker {
            source,
            cfg,
            line_index,
            comments,
            offenses: Vec::new(),
        }
    }

    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn src(&self, a: usize, b: usize) -> &[u8] {
        &self.source[a..b]
    }

    /// `TrailingComma#check_literal` + `check`. The caller guarantees the node
    /// has brackets (`node.loc.end`): `node_start`/`node_end` are the node's
    /// source range and `closing_start` is the closing bracket's begin.
    pub(crate) fn check_literal(
        &mut self,
        elements: &[Node<'_>],
        node_start: usize,
        node_end: usize,
        closing_start: usize,
    ) {
        if elements.is_empty() {
            return;
        }
        // `check(node, node.children, kind, children.last.source_range.end_pos,
        //        node.loc.end.begin_pos)`.
        let begin_pos = elements.last().unwrap().location().end_offset();
        let range_src = self.src(begin_pos, closing_start);
        let comma_offset = comma_offset(range_src, any_heredoc(elements));

        if let Some(off) = comma_offset {
            let comma_pos = begin_pos + off;
            if !inside_comment(&self.line_index, &self.comments, begin_pos, comma_pos) {
                self.check_comma(elements, node_start, node_end, closing_start, comma_pos);
            } else if self.should_have_comma(elements, node_start, node_end, closing_start) {
                self.put_comma(elements);
            }
        } else if self.should_have_comma(elements, node_start, node_end, closing_start) {
            self.put_comma(elements);
        }
    }

    /// `check_comma`: an offense unless the style wants the comma there.
    fn check_comma(
        &mut self,
        elements: &[Node<'_>],
        node_start: usize,
        node_end: usize,
        closing_start: usize,
        comma_pos: usize,
    ) {
        if self.should_have_comma(elements, node_start, node_end, closing_start) {
            return;
        }
        let message = match self.cfg.style {
            STYLE_COMMA => MSG_AVOID_COMMA,
            STYLE_CONSISTENT_COMMA => MSG_AVOID_CONSISTENT_COMMA,
            STYLE_DIFF_COMMA => MSG_AVOID_DIFF_COMMA,
            _ => MSG_AVOID_NO_COMMA,
        };
        // `avoid_comma`: caret is the one-char comma; the corrector removes it.
        self.offenses.push(TrailingCommaOffense {
            start_offset: comma_pos,
            end_offset: comma_pos + 1,
            message,
            fix: FIX_AVOID,
        });
    }

    /// `put_comma`: insert a comma after the last item's last-line content.
    /// Stock's block-pass guard is call-only: a literal element is never a
    /// block-pass node.
    fn put_comma(&mut self, elements: &[Node<'_>]) {
        let last = elements.last().unwrap();
        let (start, end) = (last.location().start_offset(), last.location().end_offset());
        let range = autocorrect_range(self.source, start, end);
        self.offenses.push(TrailingCommaOffense {
            start_offset: range.0,
            end_offset: range.1,
            message: MSG_PUT,
            fix: FIX_PUT,
        });
    }

    /// `should_have_comma?(style, node)` for a literal node:
    /// `method_name_and_arguments_on_same_line?` is always false (the node is
    /// not `call_type?`), so `consistent_comma` reduces to `multiline?`.
    fn should_have_comma(
        &self,
        elements: &[Node<'_>],
        node_start: usize,
        node_end: usize,
        closing_start: usize,
    ) -> bool {
        match self.cfg.style {
            STYLE_COMMA => {
                self.multiline(elements, node_start, node_end, closing_start)
                    && self.no_elements_on_same_line(elements, closing_start)
            }
            STYLE_CONSISTENT_COMMA => self.multiline(elements, node_start, node_end, closing_start),
            STYLE_DIFF_COMMA => {
                self.multiline(elements, node_start, node_end, closing_start)
                    && self.last_item_precedes_newline(elements, node_end)
            }
            _ => false,
        }
    }

    /// `multiline?` = `node.multiline? && !allowed_multiline_argument?`.
    fn multiline(
        &self,
        elements: &[Node<'_>],
        node_start: usize,
        node_end: usize,
        closing_start: usize,
    ) -> bool {
        let node_multiline = self.line_of(node_start) != self.line_of(node_end);
        node_multiline && !self.allowed_multiline_argument(elements, closing_start)
    }

    /// `allowed_multiline_argument?` = `elements(node).one? &&
    /// !begins_its_line?(node_end_location)`. For a literal, `elements(node)`
    /// is `node.children` and `node_end_location` is the closing bracket.
    fn allowed_multiline_argument(&self, elements: &[Node<'_>], closing_start: usize) -> bool {
        elements.len() == 1 && !begins_its_line(self.source, &self.line_index, closing_start)
    }

    /// `no_elements_on_same_line?`: no two consecutive items (the elements,
    /// then the closing bracket) share a line.
    fn no_elements_on_same_line(&self, elements: &[Node<'_>], closing_start: usize) -> bool {
        // `each_cons(2).none? { |a, b| a.last_line == b.line }`.
        let mut prev_end = elements[0].location().end_offset();
        for el in &elements[1..] {
            if self.line_of(prev_end) == self.line_of(el.location().start_offset()) {
                return false;
            }
            prev_end = el.location().end_offset();
        }
        self.line_of(prev_end) != self.line_of(closing_start)
    }

    /// `last_item_precedes_newline?`: the text from the last child's end to the
    /// node's end (past the closing bracket) starts with an optional comma,
    /// whitespace, optional comment, then a newline.
    fn last_item_precedes_newline(&self, elements: &[Node<'_>], node_end: usize) -> bool {
        let from = elements.last().unwrap().location().end_offset();
        starts_with_optional_comma_to_newline(self.src(from, node_end))
    }
}

/// `comma_offset(items, range)`: the index of the first comma reachable through
/// leading whitespace, or `None`. With a heredoc among the items the leading
/// whitespace may not include a newline (`/\A[^\S\n]*,/`); otherwise any
/// whitespace is allowed (`/\A\s*,/`).
pub(crate) fn comma_offset(range_src: &[u8], has_heredoc: bool) -> Option<usize> {
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

/// `inside_comment?`: the comma lies inside a comment on the range's line.
/// `range_begin` is the after-last-item range's begin (stock's `range.line`).
pub(crate) fn inside_comment(
    line_index: &LineIndex,
    comments: &[(usize, usize)],
    range_begin: usize,
    comma_pos: usize,
) -> bool {
    // `processed_source.comment_at_line(range.line)`: the comment whose start
    // line equals the range's begin line, if any.
    let line = line_index.line_of(range_begin);
    comments
        .iter()
        .any(|&(cs, _)| line_index.line_of(cs) == line && cs < comma_pos)
}

/// `autocorrect_range(item)`: from the last item, drop everything up to and
/// including its last newline, then skip leading whitespace; the range runs
/// from there to the item's end.
pub(crate) fn autocorrect_range(source: &[u8], start: usize, end: usize) -> (usize, usize) {
    let src = &source[start..end];
    // `ix = source.rindex("\n") || 0`. Ruby's `rindex` returns the index of
    // the `\n`; `source[ix..] =~ /\S/` then advances past it (and any
    // further whitespace). When there is no `\n`, `ix` starts at 0.
    let ix = src.iter().rposition(|&b| b == b'\n').unwrap_or(0);
    let advance = src[ix..].iter().position(|&b| !is_space(b)).unwrap_or(0);
    (start + ix + advance, end)
}

/// `Util.begins_its_line?(range)`: the first non-whitespace byte on the
/// range's line is exactly at the range start.
pub(crate) fn begins_its_line(source: &[u8], line_index: &LineIndex, off: usize) -> bool {
    let line_start = line_index.line_start(off);
    let mut p = line_start;
    while p < source.len() && source[p] != b'\n' && is_space(source[p]) {
        p += 1;
    }
    p == off
}

/// `/,?\s*(#.*)?\n/` anchored at the start of `src`: optional comma, any
/// whitespace, an optional `#`-comment running to end of line, then a newline.
pub(crate) fn starts_with_optional_comma_to_newline(src: &[u8]) -> bool {
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
pub(crate) fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// `/\s/` without the newline (the `[^\S\n]` class).
pub(crate) fn is_space_no_newline(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c)
}

/// `any_heredoc?(items)`: any item is (or wraps) a heredoc.
pub(crate) fn any_heredoc(items: &[Node<'_>]) -> bool {
    items.iter().any(heredoc)
}

/// `heredoc?(node)`, ported to Prism node types.
pub(crate) fn heredoc(node: &Node<'_>) -> bool {
    // `node.loc?(:heredoc_body)`: a string/xstring literal whose opening is a
    // heredoc marker (`<<…`).
    if is_heredoc_literal(node) {
        return true;
    }
    match node {
        // `heredoc_send?` — guarded by `node.send_type?`, which is `:send`
        // ONLY: a safe-navigation call is `(csend …)` and stock's `heredoc?`
        // falls through to `false` for it (probed: the comma regex then
        // crosses newlines into the heredoc body). Prism folds both into
        // `CallNode`, so mirror the guard with the safe-navigation flag.
        // parser-gem `(send recv meth *args)`: children.size==2 (`recv.meth`,
        // no args) -> check the receiver; size>2 (has args) -> check the last
        // argument.
        Node::CallNode { .. } => {
            let call = node.as_call_node().unwrap();
            if call.is_safe_navigation() {
                return false;
            }
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
