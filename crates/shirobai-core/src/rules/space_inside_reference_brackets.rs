//! `Layout/SpaceInsideReferenceBrackets`.
//!
//! Checks the space just inside `[` and `]` of index references (`foo[:key]`,
//! `foo[1] = 2`, `foo[1] += 2`, `foo[1], x = …`), per `EnforcedStyle`
//! (`no_space` / `space`) and `EnforcedStyleForEmptyBrackets`.
//!
//! Stock's `on_send` (RESTRICT_ON_SEND `[]` / `[]=`) hunts the bracket token
//! pair inside `tokens_within(node)`; prism hands the pair over directly as
//! `opening_loc` / `closing_loc` on `CallNode` (`[]` / `[]=`) and the
//! `IndexOperatorWrite` / `IndexOrWrite` / `IndexAndWrite` / `IndexTarget`
//! nodes (legacy parser folds those back into `send :[]` / `send :[]=`).
//!
//! Probed behavior pinned here:
//!
//! - "empty brackets" is a token-adjacency test (`[`, `]` consecutive), so a
//!   `\`-newline continuation is still empty but a comment is not; empty
//!   offenses fire BEFORE the multiline guard (`foo[\n]` is flagged).
//! - the non-empty checks skip when stock's NODE is multiline. Stock's node
//!   is the legacy send: `a[1] += \n 2` keeps the inner `send a :[] 1`
//!   single-line (checked) while `a[1] = \n 2` is the whole assignment send
//!   (skipped). The extent therefore ends at the closing `]` for the
//!   `Index*Write` family but at the node end for `CallNode` / `IndexTarget`.
//! - `extra_space?` reads `[ \t]` right inside a bracket (`\f` fools
//!   `space_after?` into a no-op empty removal, reproduced in the ops).
//! - a `[]` / `[]=` CallNode whose `opening_loc` is not a `[` byte is the
//!   explicit form (`a.[](1)`) and stock finds no `tLBRACK2`: skipped.
//! - safe navigation (`a&.[](…)`) has no `on_csend`: `CallNode`s with the
//!   safe-navigation flag are skipped (they cannot carry real brackets).
//!
//! Alongside each offense the rule emits the corrector program for the node
//! (`SpaceCorrector` reduced to remove / insert-after / insert-before ops);
//! the wrapper replays it on the node's first offense, mirroring stock's
//! `ignore_node` grouping (same wire shape as
//! `space_inside_array_literal_brackets`).

use ruby_prism::Node;

use super::line_index::LineIndex;
use super::space_scan::{is_ruby_space, next_token_start, skip_space_left, skip_space_right};

/// `EnforcedStyle` value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    NoSpace,
    Space,
}

/// Config for `Layout/SpaceInsideReferenceBrackets`.
#[derive(Clone, Copy)]
pub struct Config {
    pub style: Style,
    /// `EnforcedStyleForEmptyBrackets == 'space'`.
    pub space_empty: bool,
}

/// One offense. `node` indexes into [`ReferenceBracketsResult::node_ops`];
/// the wrapper applies that op list on the node's first offense (stock's
/// `ignore_node` grouping). `suppress_when_disable_uncorrectable` mirrors the
/// `autocorrect_with_disable_uncorrectable? && !start_ok` early return on the
/// right-bracket offense.
pub struct SpaceInsideReferenceBracketsOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: MessageId,
    pub node: usize,
    pub suppress_when_disable_uncorrectable: bool,
}

/// The four fixed messages stock emits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MessageId {
    /// `'Use space inside reference brackets.'`
    Use,
    /// `'Do not use space inside reference brackets.'`
    DoNotUse,
    /// `'Use one space inside empty reference brackets.'`
    UseOneEmpty,
    /// `'Do not use space inside empty reference brackets.'`
    DoNotUseEmpty,
}

impl MessageId {
    /// The numeric tag carried over the wire to the Ruby wrapper.
    pub fn code(self) -> u8 {
        match self {
            MessageId::Use => 0,
            MessageId::DoNotUse => 1,
            MessageId::UseOneEmpty => 2,
            MessageId::DoNotUseEmpty => 3,
        }
    }
}

/// A corrector op: `(kind, start, end)` with kind 0 = remove the range,
/// 1 = insert `" "` after the range, 2 = insert `" "` before the range.
pub type CorrectorOp = (u8, usize, usize);

/// The wire result: offenses plus one corrector program per offending node.
#[derive(Default)]
pub struct ReferenceBracketsResult {
    pub offenses: Vec<SpaceInsideReferenceBracketsOffense>,
    pub node_ops: Vec<Vec<CorrectorOp>>,
}

pub fn check_space_inside_reference_brackets(
    source: &[u8],
    config: Config,
) -> ReferenceBracketsResult {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.result
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        config,
        line_index,
        result: ReferenceBracketsResult::default(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    config: Config,
    line_index: std::rc::Rc<LineIndex>,
    pub(crate) result: ReferenceBracketsResult,
}

impl<'a> Visitor<'a> {
    /// One reference-bracket node: `lb`/`rb` are the bracket byte offsets,
    /// `extent_end` is the end of stock's legacy node (for `multiline?`).
    fn check(&mut self, node_start: usize, extent_end: usize, lb: usize, rb_s: usize, rb_e: usize) {
        let src = self.source;
        // `empty_brackets?`: `[` and `]` are adjacent tokens (whitespace and
        // continuations between only).
        let (tok, _crossed) = next_token_start(src, lb + 1);
        if tok == rb_s {
            self.check_empty(node_start, lb, rb_s, rb_e);
            return;
        }
        // `return if node.multiline?` — on stock's legacy node extent.
        if self.line_index.line_of(node_start) != self.line_index.line_of(extent_end) {
            return;
        }
        let left_inner = src.get(lb + 1).copied();
        let right_inner = if rb_s > 0 { src.get(rb_s - 1).copied() } else { None };
        let left_space = matches!(left_inner, Some(b' ' | b'\t'));
        let right_space = matches!(right_inner, Some(b' ' | b'\t'));
        let node_id = self.result.node_ops.len();
        let mut any = false;
        match self.config.style {
            Style::NoSpace => {
                if left_space {
                    // `side_space_range(side: :right)` from the `[`.
                    self.result.offenses.push(SpaceInsideReferenceBracketsOffense {
                        start_offset: lb + 1,
                        end_offset: skip_space_right(src, lb + 1),
                        message: MessageId::DoNotUse,
                        node: node_id,
                        suppress_when_disable_uncorrectable: false,
                    });
                    any = true;
                }
                if right_space {
                    // `side_space_range(side: :left)` from the `]`.
                    self.result.offenses.push(SpaceInsideReferenceBracketsOffense {
                        start_offset: skip_space_left(src, rb_s),
                        end_offset: rb_s,
                        message: MessageId::DoNotUse,
                        node: node_id,
                        suppress_when_disable_uncorrectable: true,
                    });
                    any = true;
                }
                if any {
                    // `SpaceCorrector.remove_space`: `space_after?` /
                    // `space_before?` are `\s` tests, the removal ranges are
                    // the `[ \t]` runs (a lone `\f` removes an empty range).
                    let mut ops: Vec<CorrectorOp> = Vec::new();
                    if left_inner.is_some_and(is_ruby_space) {
                        ops.push((0, lb + 1, skip_space_right(src, lb + 1)));
                    }
                    if right_inner.is_some_and(is_ruby_space) {
                        ops.push((0, skip_space_left(src, rb_s), rb_s));
                    }
                    self.result.node_ops.push(ops);
                }
            }
            Style::Space => {
                if !left_space {
                    // `side: :none` — the `[` itself.
                    self.result.offenses.push(SpaceInsideReferenceBracketsOffense {
                        start_offset: lb,
                        end_offset: lb + 1,
                        message: MessageId::Use,
                        node: node_id,
                        suppress_when_disable_uncorrectable: false,
                    });
                    any = true;
                }
                if !right_space {
                    self.result.offenses.push(SpaceInsideReferenceBracketsOffense {
                        start_offset: rb_s,
                        end_offset: rb_e,
                        message: MessageId::Use,
                        node: node_id,
                        suppress_when_disable_uncorrectable: true,
                    });
                    any = true;
                }
                if any {
                    // `SpaceCorrector.add_space` (guards are `\s` tests).
                    let mut ops: Vec<CorrectorOp> = Vec::new();
                    if !left_inner.is_some_and(is_ruby_space) {
                        ops.push((1, lb, lb + 1));
                    }
                    if !right_inner.is_some_and(is_ruby_space) {
                        ops.push((2, rb_s, rb_e));
                    }
                    self.result.node_ops.push(ops);
                }
            }
        }
        if !any {
            debug_assert_eq!(node_id, self.result.node_ops.len());
        }
    }

    /// `empty_offenses` + `SpaceCorrector.empty_corrections`. Runs before the
    /// multiline guard, so `foo[\n]` is still flagged.
    fn check_empty(&mut self, _node_start: usize, lb: usize, rb_s: usize, rb_e: usize) {
        let src = self.source;
        let gap_one_space = lb + 2 == rb_s && src.get(lb + 1) == Some(&b' ');
        let no_character = lb + 1 == rb_s;
        let node_id = self.result.node_ops.len();
        if self.config.space_empty {
            // `offending_empty_space?`: not exactly one `' '`.
            if !gap_one_space {
                self.result.offenses.push(SpaceInsideReferenceBracketsOffense {
                    start_offset: lb,
                    end_offset: rb_e,
                    message: MessageId::UseOneEmpty,
                    node: node_id,
                    suppress_when_disable_uncorrectable: false,
                });
                self.result
                    .node_ops
                    .push(vec![(0, lb + 1, rb_s), (1, lb, lb + 1)]);
            }
        } else if !no_character {
            // `offending_empty_no_space?`: any characters between.
            self.result.offenses.push(SpaceInsideReferenceBracketsOffense {
                start_offset: lb,
                end_offset: rb_e,
                message: MessageId::DoNotUseEmpty,
                node: node_id,
                suppress_when_disable_uncorrectable: false,
            });
            self.result.node_ops.push(vec![(0, lb + 1, rb_s)]);
        }
    }

    fn check_pair(
        &mut self,
        node_start: usize,
        extent_end: usize,
        opening: Option<ruby_prism::Location<'_>>,
        closing: Option<ruby_prism::Location<'_>>,
    ) {
        if let (Some(o), Some(c)) = (opening, closing)
            && self.source.get(o.start_offset()) == Some(&b'[')
        {
            self.check(
                node_start,
                extent_end,
                o.start_offset(),
                c.start_offset(),
                c.end_offset(),
            );
        }
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        match node {
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if call.is_safe_navigation() {
                    return;
                }
                let name = call.name().as_slice();
                if name != b"[]" && name != b"[]=" {
                    return;
                }
                let loc = node.location();
                // Stock's node is the whole legacy send (`a[1]` for reads,
                // `a[1] = 2` for writes): the extent is the node range.
                self.check_pair(
                    loc.start_offset(),
                    loc.end_offset(),
                    call.opening_loc(),
                    call.closing_loc(),
                );
            }
            Node::IndexOperatorWriteNode { .. } => {
                let n = node.as_index_operator_write_node().unwrap();
                // Legacy: `(op-asgn (send a :[] 1) …)` — the inner send ends
                // at the closing `]`.
                self.check_pair(
                    node.location().start_offset(),
                    n.closing_loc().end_offset(),
                    Some(n.opening_loc()),
                    Some(n.closing_loc()),
                );
            }
            Node::IndexOrWriteNode { .. } => {
                let n = node.as_index_or_write_node().unwrap();
                self.check_pair(
                    node.location().start_offset(),
                    n.closing_loc().end_offset(),
                    Some(n.opening_loc()),
                    Some(n.closing_loc()),
                );
            }
            Node::IndexAndWriteNode { .. } => {
                let n = node.as_index_and_write_node().unwrap();
                self.check_pair(
                    node.location().start_offset(),
                    n.closing_loc().end_offset(),
                    Some(n.opening_loc()),
                    Some(n.closing_loc()),
                );
            }
            Node::IndexTargetNode { .. } => {
                let n = node.as_index_target_node().unwrap();
                // Legacy: a valueless `send :[]=` covering `a[1]` exactly.
                let loc = node.location();
                self.check_pair(
                    loc.start_offset(),
                    loc.end_offset(),
                    Some(n.opening_loc()),
                    Some(n.closing_loc()),
                );
            }
            _ => {}
        }
    }

    fn leave(&mut self) {}

    fn interest(&self) -> super::dispatch::Interest {
        // CallNode plus the Index*Write / IndexTarget assignment forms
        // (WRITE bucket). `enter` is a pure kind match; nothing else is read.
        super::dispatch::Interest(
            super::dispatch::Interest::ENTER_CALL | super::dispatch::Interest::ENTER_WRITE,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: Style, space_empty: bool) -> Vec<(usize, usize, u8, usize, bool)> {
        check_space_inside_reference_brackets(source.as_bytes(), Config { style, space_empty })
            .offenses
            .into_iter()
            .map(|o| {
                (
                    o.start_offset,
                    o.end_offset,
                    o.message.code(),
                    o.node,
                    o.suppress_when_disable_uncorrectable,
                )
            })
            .collect()
    }

    fn ops(source: &str, style: Style, space_empty: bool) -> Vec<Vec<CorrectorOp>> {
        check_space_inside_reference_brackets(source.as_bytes(), Config { style, space_empty })
            .node_ops
    }

    #[test]
    fn no_space_flags_inner_spaces() {
        assert_eq!(
            run("hash[ :key ]\n", Style::NoSpace, false),
            vec![(5, 6, 1, 0, false), (10, 11, 1, 0, true)]
        );
        assert_eq!(
            ops("hash[ :key ]\n", Style::NoSpace, false),
            vec![vec![(0, 5, 6), (0, 10, 11)]]
        );
        assert_eq!(
            run("hash[\t:key\t]\n", Style::NoSpace, false),
            vec![(5, 6, 1, 0, false), (10, 11, 1, 0, true)]
        );
        assert!(run("hash[:key]\n", Style::NoSpace, false).is_empty());
    }

    #[test]
    fn space_style_flags_missing_spaces() {
        assert_eq!(
            run("hash[:key]\n", Style::Space, false),
            vec![(4, 5, 0, 0, false), (9, 10, 0, 0, true)]
        );
        assert_eq!(
            ops("hash[:key]\n", Style::Space, false),
            vec![vec![(1, 4, 5), (2, 9, 10)]]
        );
        assert!(run("hash[ :key ]\n", Style::Space, false).is_empty());
    }

    #[test]
    fn empty_brackets() {
        assert_eq!(
            run("foo[ ]\n", Style::NoSpace, false),
            vec![(3, 6, 3, 0, false)]
        );
        assert!(run("foo[]\n", Style::NoSpace, false).is_empty());
        assert_eq!(
            run("foo[]\n", Style::NoSpace, true),
            vec![(3, 5, 2, 0, false)]
        );
        assert_eq!(
            ops("foo[]\n", Style::NoSpace, true),
            vec![vec![(0, 4, 4), (1, 3, 4)]]
        );
        assert!(run("foo[ ]\n", Style::NoSpace, true).is_empty());
        assert_eq!(
            run("foo[  ]\n", Style::NoSpace, true),
            vec![(3, 7, 2, 0, false)]
        );
        // Empty offenses fire before the multiline guard.
        assert_eq!(
            run("foo[\n]\n", Style::NoSpace, false),
            vec![(3, 6, 3, 0, false)]
        );
        assert_eq!(
            run("foo[\n]\n", Style::NoSpace, true),
            vec![(3, 6, 2, 0, false)]
        );
    }

    #[test]
    fn write_forms_are_checked() {
        assert_eq!(
            run("a[ 1 ] = 2\n", Style::NoSpace, false).len(),
            2,
            "index write"
        );
        assert_eq!(
            run("a[ 1 ] += 2\n", Style::NoSpace, false).len(),
            2,
            "index op write"
        );
        assert_eq!(
            run("a[ 1 ] ||= 2\n", Style::NoSpace, false).len(),
            2,
            "index or write"
        );
        assert_eq!(
            run("a[ 1 ], b = 1, 2\n", Style::NoSpace, false).len(),
            2,
            "masgn target"
        );
    }

    #[test]
    fn multiline_extents_follow_the_legacy_node() {
        // `a[1] = …` is one legacy send: a next-line value skips it.
        assert!(run("a[ 1 ] =\n  2\n", Style::NoSpace, false).is_empty());
        // `a[1] += …` keeps the inner send single-line: still flagged.
        assert_eq!(run("a[ 1 ] +=\n  2\n", Style::NoSpace, false).len(), 2);
        // Multiline brackets skip.
        assert!(run("a[\n1\n]\n", Style::NoSpace, false).is_empty());
        // Multiline receiver makes the node multiline.
        assert!(run("(a\n)[ 1 ]\n", Style::NoSpace, false).is_empty());
    }

    #[test]
    fn non_reference_brackets_are_skipped() {
        assert!(run("x = [ 1 ]\n", Style::NoSpace, false).is_empty());
        assert!(run("a.[](1)\n", Style::NoSpace, false).is_empty());
        assert!(run("a&.[](1)\n", Style::NoSpace, false).is_empty());
        assert!(run("x = %w[ a ]\n", Style::NoSpace, false).is_empty());
    }

    #[test]
    fn comment_inside_brackets_is_multiline() {
        assert!(run("a[ # c\n1]\n", Style::NoSpace, false).is_empty());
        assert!(run("foo[ # c\n ]\n", Style::NoSpace, false).is_empty());
    }

    #[test]
    fn nested_and_chained_indexes() {
        assert_eq!(run("a[ b[ 1 ] ]\n", Style::NoSpace, false).len(), 4);
        assert_eq!(run("a[ 1 ][ 2 ]\n", Style::NoSpace, false).len(), 4);
    }

    #[test]
    fn heredoc_argument_stays_single_line() {
        assert_eq!(
            run("a[ <<~EOS ]\n  b\nEOS\n", Style::NoSpace, false).len(),
            2
        );
    }
}
