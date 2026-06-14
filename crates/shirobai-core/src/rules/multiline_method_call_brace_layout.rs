//! `Layout/MultilineMethodCallBraceLayout`.
//!
//! Stock checks that the closing `)` of a multiline method call is on the
//! right line per `EnforcedStyle` (`symmetrical` / `new_line` / `same_line`).
//! The cop targets only parenthesized `send` / `csend` nodes (no `super`,
//! `yield`, `def`); empty argument lists, implicit calls (no `loc.begin`) and
//! single-line spans are ignored.
//!
//! `single_line_ignoring_receiver?` (added by `MultilineMethodCallBraceLayout`
//! on top of the shared `MultilineLiteralBraceLayout` mixin): when the cop
//! looks at `(`/`)` lines (not the whole receiver chain), a receiver that
//! spans multiple lines but whose `(` and `)` share a line is still ignored.
//! Prism's `CallNode.opening_loc` / `closing_loc` map to parser's
//! `node.loc.begin` / `node.loc.end` verbatim, so we read the same lines.
//!
//! `last_line_heredoc?`: if the last argument is a heredoc (or contains one)
//! whose `heredoc_end` line is at or after the call's last line, autocorrect
//! around the brace would break the source. Stock returns before emitting.
//!
//! Detection is fully done here. Autocorrect division of labour: this rule
//! returns each offense's parent `send` node `send_node_start` (so the Ruby
//! wrapper can re-resolve the parser-gem node via
//! `processed_source.ast.each_node`) plus the `correctable` flag.
//! The flag is false when `new_line_needed_before_closing_brace?` fires
//! (comment-after-last-element AND the call is chained / an argument). The
//! wrapper then either skips the corrector or delegates to stock's
//! `MultilineLiteralBraceCorrector` which preserves the full byte-exact
//! autocorrect (heredoc-chain relocation, comment-block reflow, trailing-comma
//! handling).
//!
//! The shared walk keeps a stack of `CallNode` ancestors so we can answer
//! `node.chained?` / `node.argument?` at offense time without re-walking.

use ruby_prism::Node;
use std::rc::Rc;

use super::line_index::LineIndex;

/// One misplaced closing `)`. Ruby builds the offense range from
/// `[offense_start, offense_end)` and hands the autocorrect off to stock's
/// `MultilineLiteralBraceCorrector` keyed by `(send_node_start, send_node_end)`.
pub struct MmcblOffense {
    /// Closing brace token (the offense highlight range).
    pub offense_start: usize,
    pub offense_end: usize,
    /// Picks the message string. See module-level `MSG_*` constants.
    pub message_code: u8,
    /// `send` / `csend` node range = `node.source_range.begin_pos / end_pos`.
    /// The wrapper walks `processed_source.ast` to find the parser-gem node
    /// whose source range MATCHES this pair exactly — `begin_pos` alone is not
    /// enough because chained sends (`foo(...).bar`) share a start with the
    /// inner call (pre-order each_node would otherwise return the OUTER send,
    /// which has no own `loc.begin`/`loc.end` for braces). The pair pins us to
    /// the inner brace-bearing send.
    pub send_node_start: usize,
    pub send_node_end: usize,
    /// `false` when stock's `new_line_needed_before_closing_brace?` fires
    /// (comment after the last element AND the call is chained / an argument).
    /// In that case stock emits the offense but the corrector block returns
    /// early before touching `corrector`. Wrappers must replicate that by
    /// skipping the corrector entirely so `correctable?` stays `false`.
    pub correctable: bool,
}

/// EnforcedStyle codes shared with the Ruby wrapper.
pub const STYLE_SYMMETRICAL: u8 = 0;
pub const STYLE_NEW_LINE: u8 = 1;
pub const STYLE_SAME_LINE: u8 = 2;

/// Message codes for `MmcblOffense::message_code`. The wrapper formats the
/// final string from this small table because the messages are class
/// constants on stock's class.
pub const MSG_SAME_LINE: u8 = 0; // `SAME_LINE_MESSAGE` (symmetrical, open-same)
pub const MSG_NEW_LINE: u8 = 1; // `NEW_LINE_MESSAGE`  (symmetrical, open-diff)
pub const MSG_ALWAYS_NEW_LINE: u8 = 2; // `ALWAYS_NEW_LINE_MESSAGE`
pub const MSG_ALWAYS_SAME_LINE: u8 = 3; // `ALWAYS_SAME_LINE_MESSAGE`

pub fn check_multiline_method_call_brace_layout(
    source: &[u8],
    style: u8,
) -> Vec<MmcblOffense> {
    let mut rule = build_rule(source, style);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

pub(crate) fn build_rule(source: &[u8], style: u8) -> Visitor {
    // `parse_cache::comment_ranges` borrows the cache too, and we can't reach it
    // from inside the walk (single RefCell, see the `parse_cache panic` row in
    // the trap table). Collect comment-start lines BEFORE entering the walk;
    // descents only need to ask "did any comment start on line L?".
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    let comment_lines: std::collections::HashSet<usize> = super::parse_cache::comment_ranges(source)
        .into_iter()
        .map(|(s, _)| line_index.line_of(s))
        .collect();
    Visitor {
        line_index,
        style,
        comment_lines,
        // Marker stack: every `enter` pushes one entry, every `leave` pops one.
        // `Some(frame)` for CallNode (parser `send`/`csend`), `None` for other
        // branch nodes — keeps push/pop balanced under the shared walk.
        frame_stack: Vec::new(),
        offenses: Vec::new(),
    }
}

#[derive(Clone)]
struct CallFrame {
    /// `node.source_range.begin_pos` for this call.
    node_start: usize,
    /// `node.source_range.end_pos`.
    node_end: usize,
    /// `receiver.source_range.begin_pos / end_pos` (None when no receiver).
    receiver_range: Option<(usize, usize)>,
    /// Argument start/end offsets. Parser includes the `block_pass` (`&blk`)
    /// as the LAST argument — Prism keeps it in `block` so we splice it back
    /// here. Used to answer `node.argument?` for a descendant call.
    arg_starts: Vec<usize>,
    arg_ends: Vec<usize>,
}

pub(crate) struct Visitor {
    line_index: Rc<LineIndex>,
    style: u8,
    /// Set of 1-based line numbers that contain at least one comment-start.
    /// Pre-collected before the walk to avoid re-borrowing `parse_cache`
    /// (single RefCell, panics on re-entrant `with_parsed`).
    comment_lines: std::collections::HashSet<usize>,
    /// Marker stack — one entry per branch enter, popped on leave. `Some(_)`
    /// only for CallNode; other branches push `None`.
    frame_stack: Vec<Option<CallFrame>>,
    pub(crate) offenses: Vec<MmcblOffense>,
}

impl super::dispatch::Rule for Visitor {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(c) = node.as_call_node() {
            let frame = self.build_frame(&c);
            self.check(&c, &frame);
            self.frame_stack.push(Some(frame));
        } else {
            self.frame_stack.push(None);
        }
    }

    fn leave(&mut self) {
        self.frame_stack.pop();
    }
}

impl Visitor {
    fn build_frame(&self, c: &ruby_prism::CallNode<'_>) -> CallFrame {
        let node_start = c.as_node().location().start_offset();
        let node_end = c.as_node().location().end_offset();
        let receiver_range = c
            .receiver()
            .map(|r| (r.location().start_offset(), r.location().end_offset()));
        let mut arg_starts = Vec::new();
        let mut arg_ends = Vec::new();
        if let Some(args) = c.arguments() {
            for a in args.arguments().iter() {
                arg_starts.push(a.location().start_offset());
                arg_ends.push(a.location().end_offset());
            }
        }
        if let Some(block) = c.block()
            && let Some(bp) = block.as_block_argument_node()
        {
            arg_starts.push(bp.as_node().location().start_offset());
            arg_ends.push(bp.as_node().location().end_offset());
        }
        CallFrame {
            node_start,
            node_end,
            receiver_range,
            arg_starts,
            arg_ends,
        }
    }

    fn check(&mut self, c: &ruby_prism::CallNode<'_>, frame: &CallFrame) {
        // Stock's `check_brace_layout` (base) is:
        //   return if ignored_literal?(node)
        //   return if last_line_heredoc?(node.children.last)
        //   check(node)
        // and MMCBL overrides `ignored_literal?` to also short-circuit on
        // `single_line_ignoring_receiver?` (begin/end on same line).
        //
        // implicit_literal? = no `loc.begin` (= no `(`).
        let (Some(open), Some(close)) = (c.opening_loc(), c.closing_loc()) else {
            return;
        };
        // Opening must be `(`. Other CallNode shapes (`foo[1]`, `foo&.[]=1`,
        // operator sends) carry different opening sources, and the cop is
        // about method-call braces specifically.
        if open.as_slice() != b"(" || close.as_slice() != b")" {
            return;
        }
        let open_line = self.line_index.line_of(open.start_offset());
        let close_line = self.line_index.line_of(close.start_offset());
        // `single_line_ignoring_receiver?`: begin/end on the same line.
        if open_line == close_line {
            return;
        }
        // `empty_literal?`: no children. Children for MMCBL = `node.arguments`.
        if frame.arg_starts.is_empty() {
            return;
        }
        let first_arg_start = frame.arg_starts[0];
        let last_arg_end = *frame.arg_ends.last().expect("non-empty checked above");
        let last_arg_start = *frame.arg_starts.last().expect("non-empty");

        // `last_line_heredoc?(node.children.last)`: if the last argument or
        // anything it contains is a heredoc whose `heredoc_end` line is at or
        // after the call's last line. (Parent is pinned at the call when the
        // walk recurses.)
        if self.last_line_heredoc(c, last_arg_start, last_arg_end) {
            return;
        }

        // `opening_brace_on_same_line?` = same_line?(loc.begin, children.first)
        // `closing_brace_on_same_line?` = loc.end.line == children.last.last_line
        let opening_same = open_line == self.line_index.line_of(first_arg_start);
        // children.last.last_line: the last line spanned by the last argument.
        let last_arg_last_byte = last_arg_end.saturating_sub(1);
        let last_arg_last_line = self.line_index.line_of(last_arg_last_byte);
        let closing_same = close_line == last_arg_last_line;

        let (should_offense, message_code) = match self.style {
            STYLE_SYMMETRICAL => {
                if opening_same {
                    (!closing_same, MSG_SAME_LINE)
                } else {
                    (closing_same, MSG_NEW_LINE)
                }
            }
            STYLE_NEW_LINE => (closing_same, MSG_ALWAYS_NEW_LINE),
            STYLE_SAME_LINE => (!closing_same, MSG_ALWAYS_SAME_LINE),
            _ => return,
        };
        if !should_offense {
            return;
        }
        let correctable = self.is_correctable(frame, last_arg_last_line, closing_same);
        self.offenses.push(MmcblOffense {
            offense_start: close.start_offset(),
            offense_end: close.end_offset(),
            message_code,
            send_node_start: frame.node_start,
            send_node_end: frame.node_end,
            correctable,
        });
    }

    /// `new_line_needed_before_closing_brace?` returns true (i.e. corrector
    /// should skip) when:
    /// - the last argument's last line carries a comment, AND
    /// - the call is `chained?` (parent is `send` and `parent.receiver == THIS`)
    ///   OR `argument?` (parent is `send` and THIS is one of parent's arguments).
    ///
    /// Stock checks this only inside the `correct_next_line_brace` branch (i.e.
    /// when we'd fold to same-line). For `correct_same_line_brace` (close on
    /// SAME line, insert `\n` before `)`), no comment block check is needed.
    fn is_correctable(
        &self,
        frame: &CallFrame,
        last_arg_last_line: usize,
        closing_same: bool,
    ) -> bool {
        // Only the fold-to-same-line branch can be skipped by the comment guard.
        if closing_same {
            return true;
        }
        if !self.comment_lines.contains(&last_arg_last_line) {
            return true;
        }
        // The frame stack hasn't pushed this call's own frame yet (push happens
        // AFTER `check`). So `frame_stack.last()` is the immediate parent.
        let parent = match self.frame_stack.iter().rev().find_map(|x| x.as_ref()) {
            None => return true, // no parent send -> not chained/argument
            Some(p) => p,
        };
        // chained?: parent.receiver == this node (range equality).
        if let Some((rs, re)) = parent.receiver_range
            && rs == frame.node_start
            && re == frame.node_end
        {
            return false;
        }
        // argument?: this node matches one of parent's arg ranges.
        for (i, &s) in parent.arg_starts.iter().enumerate() {
            if s == frame.node_start && parent.arg_ends[i] == frame.node_end {
                return false;
            }
        }
        true
    }

    /// `last_line_heredoc?(node)`: walk the subtree of the last argument and
    /// return true if any heredoc's terminator line is at or after the
    /// originally-passed argument's last line. Note: stock pins `parent` to
    /// the FIRST argument the helper receives (`parent ||= node`), not the
    /// outer call — so the check is "does the last argument or its children
    /// contain a heredoc that ends at or after the last argument's own end?".
    /// A heredoc-as-last-argument always trips it because the heredoc's
    /// `heredoc_end.last_line` equals its own `last_line`.
    fn last_line_heredoc(
        &self,
        c: &ruby_prism::CallNode<'_>,
        _last_arg_start: usize,
        last_arg_end: usize,
    ) -> bool {
        let parent_last_byte = last_arg_end.saturating_sub(1);
        let parent_last_line = self.line_index.line_of(parent_last_byte);
        let Some(args) = c.arguments() else {
            return false;
        };
        let args_vec: Vec<_> = args.arguments().iter().collect();
        let Some(last) = args_vec.last() else {
            return false;
        };
        self.last_line_heredoc_walk(last, parent_last_line)
    }

    fn last_line_heredoc_walk(&self, node: &Node<'_>, parent_last_line: usize) -> bool {
        // Heredoc shapes: StringNode / XStringNode / InterpolatedStringNode /
        // InterpolatedXStringNode whose `opening_loc` source starts with "<<".
        // Their `closing_loc` is the terminator line.
        if let Some(s) = node.as_string_node()
            && let Some(open) = s.opening_loc()
            && let Some(close) = s.closing_loc()
            && open.as_slice().starts_with(b"<<")
            && self.line_index.line_of(close.end_offset().saturating_sub(1))
                >= parent_last_line
        {
            return true;
        }
        if let Some(s) = node.as_x_string_node() {
            let open = s.opening_loc();
            if open.as_slice().starts_with(b"<<") {
                let close = s.closing_loc();
                if self.line_index.line_of(close.end_offset().saturating_sub(1))
                    >= parent_last_line
                {
                    return true;
                }
            }
        }
        if let Some(s) = node.as_interpolated_string_node()
            && let Some(open) = s.opening_loc()
            && let Some(close) = s.closing_loc()
            && open.as_slice().starts_with(b"<<")
            && self.line_index.line_of(close.end_offset().saturating_sub(1))
                >= parent_last_line
        {
            return true;
        }
        if let Some(s) = node.as_interpolated_x_string_node() {
            let open = s.opening_loc();
            if open.as_slice().starts_with(b"<<") {
                let close = s.closing_loc();
                if self.line_index.line_of(close.end_offset().saturating_sub(1))
                    >= parent_last_line
                {
                    return true;
                }
            }
        }
        for child in child_nodes(node) {
            if self.last_line_heredoc_walk(&child, parent_last_line) {
                return true;
            }
        }
        false
    }
}

/// Direct children of `node` for the heredoc-recursion fallback. Only the
/// composite shapes that can plausibly nest a heredoc inside an argument are
/// covered (array / hash / pair / call / block / parens / splat etc.).
fn child_nodes<'pr>(node: &Node<'pr>) -> Vec<Node<'pr>> {
    let mut out = Vec::new();
    if let Some(arr) = node.as_array_node() {
        for e in arr.elements().iter() {
            out.push(e);
        }
    } else if let Some(h) = node.as_hash_node() {
        for e in h.elements().iter() {
            out.push(e);
        }
    } else if let Some(h) = node.as_keyword_hash_node() {
        for e in h.elements().iter() {
            out.push(e);
        }
    } else if let Some(p) = node.as_assoc_node() {
        out.push(p.key());
        out.push(p.value());
    } else if let Some(p) = node.as_assoc_splat_node() {
        if let Some(v) = p.value() {
            out.push(v);
        }
    } else if let Some(c) = node.as_call_node() {
        if let Some(r) = c.receiver() {
            out.push(r);
        }
        if let Some(args) = c.arguments() {
            for a in args.arguments().iter() {
                out.push(a);
            }
        }
        if let Some(b) = c.block() {
            out.push(b);
        }
    } else if let Some(b) = node.as_block_argument_node() {
        if let Some(e) = b.expression() {
            out.push(e);
        }
    } else if let Some(p) = node.as_parentheses_node() {
        if let Some(b) = p.body() {
            out.push(b);
        }
    } else if let Some(st) = node.as_statements_node() {
        for s in st.body().iter() {
            out.push(s);
        }
    } else if let Some(i) = node.as_if_node() {
        out.push(i.predicate());
        if let Some(s) = i.statements() {
            out.push(s.as_node());
        }
        if let Some(s) = i.subsequent() {
            out.push(s);
        }
    } else if let Some(u) = node.as_unless_node() {
        out.push(u.predicate());
        if let Some(s) = u.statements() {
            out.push(s.as_node());
        }
        if let Some(s) = u.else_clause() {
            out.push(s.as_node());
        }
    } else if let Some(s) = node.as_splat_node()
        && let Some(e) = s.expression()
    {
        out.push(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Vec<(usize, usize, u8, usize, usize, bool)> {
        check_multiline_method_call_brace_layout(source.as_bytes(), style)
            .into_iter()
            .map(|o| {
                (
                    o.offense_start,
                    o.offense_end,
                    o.message_code,
                    o.send_node_start,
                    o.send_node_end,
                    o.correctable,
                )
            })
            .collect()
    }

    #[test]
    fn symmetrical_open_same_close_diff() {
        // open `(` after `foo`, args `a`, `b` on different lines, close on its own line.
        let got = run("foo(a,\n  b\n)\n", STYLE_SYMMETRICAL);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, MSG_SAME_LINE);
        assert!(got[0].5); // correctable (no comment)
    }

    #[test]
    fn symmetrical_open_diff_close_same() {
        // open after `(` on its own newline; closing brace on same line as last arg.
        let got = run("foo(\n  a,\n  b)\n", STYLE_SYMMETRICAL);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, MSG_NEW_LINE);
        assert!(got[0].5, "correctable (close-same, insert newline)");
    }

    #[test]
    fn symmetrical_open_same_close_same_ok() {
        assert!(run("foo(a,\n  b)\n", STYLE_SYMMETRICAL).is_empty());
    }

    #[test]
    fn symmetrical_open_diff_close_diff_ok() {
        assert!(run("foo(\n  a,\n  b\n)\n", STYLE_SYMMETRICAL).is_empty());
    }

    #[test]
    fn new_line_close_same_is_offense() {
        let got = run("foo(\n  a,\n  b)\n", STYLE_NEW_LINE);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, MSG_ALWAYS_NEW_LINE);
    }

    #[test]
    fn same_line_close_diff_is_offense() {
        let got = run("foo(a,\n  b\n)\n", STYLE_SAME_LINE);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, MSG_ALWAYS_SAME_LINE);
    }

    #[test]
    fn implicit_call_ignored() {
        assert!(run("foo a,\n  b\n", STYLE_SYMMETRICAL).is_empty());
    }

    #[test]
    fn single_line_ignored() {
        assert!(run("foo(1, 2)\n", STYLE_SYMMETRICAL).is_empty());
    }

    #[test]
    fn empty_parens_ignored() {
        assert!(run("puts()\n", STYLE_SYMMETRICAL).is_empty());
        assert!(run("puts(\n)\n", STYLE_SYMMETRICAL).is_empty());
    }

    #[test]
    fn csend_basic() {
        let got = run("foo&.bar(a,\n  b\n)\n", STYLE_SYMMETRICAL);
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn self_call() {
        let got = run("self.foo(a,\n  b\n)\n", STYLE_SYMMETRICAL);
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn block_pass_is_argument() {
        let got = run("foo(&blk\n)\n", STYLE_SYMMETRICAL);
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn super_yield_not_targeted() {
        assert!(run("super(a,\n  b\n)\n", STYLE_SYMMETRICAL).is_empty());
        assert!(run("yield(a,\n  b\n)\n", STYLE_SYMMETRICAL).is_empty());
    }

    #[test]
    fn heredoc_last_arg_chain_safe_skip() {
        // The closing `)` is on the line AFTER the heredoc terminator, but the
        // heredoc's `heredoc_end` line equals the call's last line — `chain`
        // shape `foo(<<~EOM, x\n  text\nEOM\n).do_something` would normally
        // trigger a same-line fold but stock skips because the heredoc end
        // shares the last line with the call. Stock STILL reports the offense
        // here — `last_line_heredoc?` only looks at children.last (the str
        // arg). If the str arg's heredoc ends ON the parent's last line, it
        // returns true (line >= parent_last_line) and stock returns BEFORE
        // emitting. Verify shirobai matches.
        // The vendor spec is heredoc-chain-with-`.do_something` — that's a
        // chained call, where `node` is the inner `foo(...)`, parent is
        // `.do_something`, parent.last_line is the SAME as foo's last line
        // (parser-side). Either way: shirobai must emit because stock emits in
        // the vendor spec's first test (line 33). Validate that:
        let got = run(
            "foo(<<~EOS, arg\n  text\nEOS\n).do_something\n",
            STYLE_SYMMETRICAL,
        );
        // Stock DOES emit + autocorrect here per vendor spec line 33. Confirm.
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn chained_with_comment_is_uncorrectable() {
        // From `chained_comment` probe: `foo(a,\n  b # comment\n).any?`
        // stock detects but skips autocorrect (no [Correctable]).
        let got = run("foo(a,\n  b # comment\n).any?\n", STYLE_SYMMETRICAL);
        assert_eq!(got.len(), 1);
        assert!(!got[0].5, "expected non-correctable (comment + chained)");
    }

    #[test]
    fn nested_inner_call_detected() {
        // Outer `super(...)` is not a send; inner `bar(baz, ham)` is the only
        // send/csend target. Stock reports 1 offense on the inner brace.
        let got = run("super(bar(baz,\n  ham\n))\n", STYLE_SYMMETRICAL);
        assert_eq!(got.len(), 1);
    }
}
