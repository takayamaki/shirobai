//! `Layout/FirstArrayElementIndentation`.
//!
//! Checks the indentation of the first element of an array literal whose
//! opening bracket and first element are on separate lines, and of a hanging
//! right bracket. Same `AlignmentCorrector` division of labour as the other
//! alignment cops: Rust computes the offense range (the first element, or the
//! `]` token), the `column_delta` and the message; Ruby applies the
//! realignment via `AlignmentCorrector`.
//!
//! Columns are parser-gem columns (character counts from the line start), not
//! display width: the `MultilineElementIndentation` mixin reads
//! `source_range.column` / `=~ /\S/` directly instead of `display_column`.
//!
//! The trickiest part is replicating which left parenthesis "claims" an array
//! (stock's `each_argument_node` + `ignore_node` dance): `on_send` fires in
//! pre-order and scans the call's arguments with `on_node(:array, arg, :send)`,
//! which descends into everything *except* nested plain `send`s — parser's
//! `:block` node keeps a call's block body scannable, `:csend` is never
//! excluded, and an index call (`foo[...]`) is a `send` with `loc.begin` `nil`
//! so it blocks but can never claim. The ancestor-stack walk reproduces this:
//! the deepest plain-call ancestor (path not entering through its block) is a
//! blocking boundary, and the outermost candidate call at or below it whose
//! `(` is on the array's opening line claims the array.

use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// One misindented first element or right bracket. `[start_offset,
/// end_offset)` is the offense range, which Ruby both reports and realigns by
/// `column_delta` via `AlignmentCorrector` (resolving the element's AST node
/// for the string-taboo handling; the `]` range stays a range, like stock).
pub struct FirstArrayElemIndentOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub message: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    /// `special_inside_parentheses` (default).
    SpecialInsideParens,
    Consistent,
    AlignBrackets,
}

impl Style {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Style::Consistent,
            2 => Style::AlignBrackets,
            _ => Style::SpecialInsideParens,
        }
    }
}

/// `indent_base`'s second return value: what the expected column is based on.
#[derive(Clone, Copy)]
enum BaseType {
    /// `:left_brace_or_bracket` (`align_brackets` style).
    LeftBracket,
    /// `:first_column_after_left_parenthesis`.
    AfterParen,
    /// `:parent_hash_key`.
    ParentHashKey,
    /// `:start_of_line`.
    StartOfLine,
}

pub fn check_first_array_element_indentation(
    source: &[u8],
    style: u8,
    indent_width: usize,
    enforce_fixed_indentation: bool,
) -> Vec<FirstArrayElemIndentOffense> {
    let Some(mut rule) = build_rule(source, style, indent_width, enforce_fixed_indentation) else {
        return Vec::new();
    };
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle. `None` when
/// the cop is disabled outright: `Layout/ArrayAlignment` enforcing
/// `with_fixed_indentation` gates both `on_array` and `on_send` unless the
/// style is `consistent`.
pub(crate) fn build_rule(
    source: &[u8],
    style: u8,
    indent_width: usize,
    enforce_fixed_indentation: bool,
) -> Option<Visitor<'_>> {
    let style = Style::from_u8(style);
    if enforce_fixed_indentation && style != Style::Consistent {
        return None;
    }
    Some(Visitor {
        source,
        line_index: super::line_index::with_line_index(source, |li| li.clone()),
        style,
        indent: indent_width,
        stack: Vec::new(),
        offenses: Vec::new(),
    })
}

/// Lightweight ancestor frame kind, carrying exactly what the claiming logic
/// and the parent-hash-key base need.
///
/// NOTE: `ArgumentsNode` (like `StatementsNode`) is reached through its
/// parent's concretely-typed field (`visit_arguments_node`), which bypasses
/// the generic branch hooks — it never appears on the stack. "The path enters
/// through the call's arguments" is therefore an offset test against the
/// `ArgumentsNode`'s range stored on the call's own frame; skipped transparent
/// levels cannot change a containment test because node ranges nest.
enum FrameKind {
    /// A parser `send` / `csend`. `paren_start` is the begin offset of a
    /// literal `(` (an index call's `[` is `loc.begin == nil` in parser and
    /// can never claim). `args_range` spans the `ArgumentsNode`, which in
    /// parser is the send's own extent minus receiver/selector/block.
    Call {
        csend: bool,
        paren_start: Option<usize>,
        args_range: Option<(usize, usize)>,
    },
    /// A `&blk` block-pass (part of `node.arguments` in parser, so a path
    /// through it counts as entering the call's arguments).
    BlockArgument,
    /// A `{}`/`do` block attached to a call: parser materialises a separate
    /// `:block` node *containing* the send, so a path through the block is not
    /// excluded by the send.
    Block,
    /// A hash `pair`, for `pair.loc.column` and `pair.last_line` (the frame's
    /// own `start`/`end` span the pair).
    Assoc {
        key_start: usize,
        value_start: usize,
    },
    /// A hash (braced or keyword). `elements` are the children's ranges, for
    /// `pair.right_sibling`.
    Hash {
        elements: Vec<(usize, usize)>,
    },
    /// An attribute / index op-assign (`a.b += x`, `a[i] ||= x`): parser nests
    /// a plain `send` covering everything up to `block_end` (`a.b` / `a[i]`)
    /// inside the assign node, blocking `on_node` scans into that range, while
    /// the assigned value sits outside it and stays scannable.
    AsgnTarget {
        block_end: usize,
    },
    Other,
}

/// Ancestor frame: the node's range plus its classified kind.
struct Frame {
    start: usize,
    end: usize,
    kind: FrameKind,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    style: Style,
    indent: usize,
    stack: Vec<Frame>,
    pub(crate) offenses: Vec<FirstArrayElemIndentOffense>,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

/// Ruby regex `\s` (the line-local subset; `\n` cannot occur inside a line).
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r')
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(a) = node.as_array_node() {
            self.process_array(&a);
        }
        self.stack.push(self.make_frame(node));
    }

    fn leave(&mut self) {
        self.stack.pop();
    }
}

impl<'a> Visitor<'a> {
    fn make_frame(&self, node: &Node<'_>) -> Frame {
        let (start, end) = loc(&node.location());
        let kind = if let Some(c) = node.as_call_node() {
            let csend = c
                .call_operator_loc()
                .map(|l| l.as_slice() == b"&.")
                .unwrap_or(false);
            let paren_start = c
                .opening_loc()
                .filter(|o| o.as_slice() == b"(")
                .map(|o| o.start_offset());
            FrameKind::Call {
                csend,
                paren_start,
                args_range: c.arguments().map(|a| loc(&a.as_node().location())),
            }
        } else if node.as_block_argument_node().is_some() {
            FrameKind::BlockArgument
        } else if node.as_block_node().is_some() {
            FrameKind::Block
        } else if let Some(a) = node.as_assoc_node() {
            FrameKind::Assoc {
                key_start: a.key().location().start_offset(),
                value_start: a.value().location().start_offset(),
            }
        } else if let Some(h) = node.as_hash_node() {
            FrameKind::Hash {
                elements: h.elements().iter().map(|e| loc(&e.location())).collect(),
            }
        } else if let Some(h) = node.as_keyword_hash_node() {
            FrameKind::Hash {
                elements: h.elements().iter().map(|e| loc(&e.location())).collect(),
            }
        } else if let Some(k) = asgn_target_kind(node) {
            k
        } else {
            FrameKind::Other
        };
        Frame { start, end, kind }
    }

    /// 1-based line number of `off`.
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    /// parser-gem `Source::Range#column`: character column within the line.
    fn column(&self, off: usize) -> usize {
        self.line_index.column(self.source, off)
    }

    /// `left_brace.source_line =~ /\S/`: the column of the first non-blank
    /// character of `off`'s line (the leading run is ASCII, so bytes == chars;
    /// the line cannot be all-blank — `off` points at a token on it).
    fn line_first_nonws_column(&self, off: usize) -> usize {
        let ls = self.line_index.line_start(off);
        self.source[ls..].iter().take_while(|&&b| is_ws(b)).count()
    }

    /// The `(` that claims this array via stock's `on_send` path, if any: the
    /// outermost eligible ancestor call not separated from the array by a
    /// plain `send` (`on_node(:array, arg, :send)` stops at nested sends), with
    /// parentheses on the same line as the array's opening bracket. Pre-order
    /// `on_send` + `ignore_node` make the outermost match win.
    fn claimed_paren(&self, array_open_start: usize) -> Option<usize> {
        let array_line = self.line_of(array_open_start);
        let n = self.stack.len();
        // Start offset of the next node on the path from frame `i` towards the
        // array (the array itself when `i` is the direct parent).
        let next_start =
            |i: usize| -> usize { self.stack.get(i + 1).map_or(array_open_start, |f| f.start) };
        // The deepest blocking ancestor: a plain (non-`&.`) call whose path
        // towards the array does not enter through its block, or an op-assign
        // target whose blocked range (parser's nested plain `send`) the path
        // enters. Calls *below* the boundary can still claim.
        let mut start = 0;
        for i in 0..n {
            match self.stack[i].kind {
                FrameKind::Call { csend: false, .. } => {
                    let child_is_block =
                        i + 1 < n && matches!(self.stack[i + 1].kind, FrameKind::Block);
                    if !child_is_block {
                        start = i;
                    }
                }
                FrameKind::AsgnTarget { block_end } if next_start(i) < block_end => {
                    start = i;
                }
                _ => {}
            }
        }
        for i in start..n {
            let FrameKind::Call {
                paren_start: Some(p),
                args_range,
                ..
            } = self.stack[i].kind
            else {
                continue;
            };
            // `each_argument_node` scans only the call's own arguments (which
            // in parser include a `&blk` block-pass — Prism's separate
            // `BlockArgumentNode` field), never its receiver.
            let ns = next_start(i);
            let via_arguments = args_range.is_some_and(|(s, e)| s <= ns && ns < e)
                || (i + 1 < n && matches!(self.stack[i + 1].kind, FrameKind::BlockArgument));
            if via_arguments && self.line_of(p) == array_line {
                return Some(p);
            }
        }
        None
    }

    /// `check(array_node, left_parenthesis)`.
    fn process_array(&mut self, a: &ruby_prism::ArrayNode<'_>) {
        let Some(open) = a.opening_loc() else { return };
        let open_start = open.start_offset();
        let paren = self.claimed_paren(open_start);
        let first = a.elements().iter().next().map(|e| loc(&e.location()));

        if let Some(f) = first {
            if self.line_of(f.0) == self.line_of(open_start) {
                return;
            }
            self.check_first(f, open_start, paren);
        }
        if let Some(close) = a.closing_loc() {
            self.check_right_bracket(loc(&close), first, open_start, paren);
        }
    }

    /// `check_first(first, left_bracket, left_parenthesis, 0)`.
    fn check_first(&mut self, first: (usize, usize), open_start: usize, paren: Option<usize>) {
        let actual_column = self.column(first.0);
        let (base_column, base_type) = self.indent_base(open_start, Some(first), paren);
        let expected_column = base_column + self.indent;
        let column_delta = expected_column as isize - actual_column as isize;
        if column_delta == 0 {
            return;
        }
        let message = format!(
            "Use {} spaces for indentation in an array, relative to {}.",
            self.indent,
            base_description(base_type)
        );
        self.offenses.push(FirstArrayElemIndentOffense {
            start_offset: first.0,
            end_offset: first.1,
            column_delta,
            message,
        });
    }

    /// `check_right_bracket(right_bracket, first_elem, left_bracket,
    /// left_parenthesis)`.
    fn check_right_bracket(
        &mut self,
        close: (usize, usize),
        first: Option<(usize, usize)>,
        open_start: usize,
        paren: Option<usize>,
    ) {
        // Accept a right bracket that does not begin its line.
        let ls = self.line_index.line_start(close.0);
        if self.source[ls..close.0].iter().any(|&b| !is_ws(b)) {
            return;
        }
        let (expected_column, base_type) = self.indent_base(open_start, first, paren);
        let column_delta = expected_column as isize - self.column(close.0) as isize;
        if column_delta == 0 {
            return;
        }
        let message = match base_type {
            BaseType::LeftBracket => "Indent the right bracket the same as the left bracket.",
            BaseType::AfterParen => {
                "Indent the right bracket the same as the first position \
                 after the preceding left parenthesis."
            }
            BaseType::ParentHashKey => "Indent the right bracket the same as the parent hash key.",
            BaseType::StartOfLine => {
                "Indent the right bracket the same as the start of the line \
                 where the left bracket is."
            }
        };
        self.offenses.push(FirstArrayElemIndentOffense {
            start_offset: close.0,
            end_offset: close.1,
            column_delta,
            message: message.to_string(),
        });
    }

    /// `indent_base(left_brace, first, left_parenthesis)`.
    fn indent_base(
        &self,
        open_start: usize,
        first: Option<(usize, usize)>,
        paren: Option<usize>,
    ) -> (usize, BaseType) {
        if self.style == Style::AlignBrackets {
            return (self.column(open_start), BaseType::LeftBracket);
        }
        if first.is_some()
            && let Some(col) = self.parent_hash_key_column()
        {
            return (col, BaseType::ParentHashKey);
        }
        if let Some(p) = paren
            && self.style == Style::SpecialInsideParens
        {
            return (self.column(p) + 1, BaseType::AfterParen);
        }
        (
            self.line_first_nonws_column(open_start),
            BaseType::StartOfLine,
        )
    }

    /// `hash_pair_where_value_beginning_with` + the two pair conditions: the
    /// array's direct parent is a hash pair whose key and value begin on the
    /// same line and whose right sibling begins on a subsequent line. Returns
    /// `pair.loc.column`.
    fn parent_hash_key_column(&self) -> Option<usize> {
        let n = self.stack.len();
        let pair = self.stack.last()?;
        let FrameKind::Assoc {
            key_start,
            value_start,
        } = pair.kind
        else {
            return None;
        };
        let (start, end) = (pair.start, pair.end);
        // `key_and_value_begin_on_same_line?(pair)`.
        if self.line_of(key_start) != self.line_of(value_start) {
            return None;
        }
        // `right_sibling_begins_on_subsequent_line?(pair)`.
        let FrameKind::Hash { elements } = &self.stack[n.checked_sub(2)?].kind else {
            return None;
        };
        let idx = elements.iter().position(|&(s, _)| s == start)?;
        let sibling = elements.get(idx + 1)?;
        if self.line_of(end - 1) >= self.line_of(sibling.0) {
            return None;
        }
        Some(self.column(start))
    }
}

/// Classifies `a.b op= x` / `a[i] op= x` as an [`FrameKind::AsgnTarget`]:
/// parser nests a plain `send` spanning everything before the assignment
/// operator (`a.b` / `a[i]`), which blocks `on_node` scans into that range.
/// Safe-navigation forms nest a `csend` instead, which `on_node` does not
/// exclude, so they stay transparent (`None`).
fn asgn_target_kind(node: &Node<'_>) -> Option<FrameKind> {
    let (safe, op_start) = if let Some(n) = node.as_call_operator_write_node() {
        (
            n.is_safe_navigation(),
            n.binary_operator_loc().start_offset(),
        )
    } else if let Some(n) = node.as_call_and_write_node() {
        (n.is_safe_navigation(), n.operator_loc().start_offset())
    } else if let Some(n) = node.as_call_or_write_node() {
        (n.is_safe_navigation(), n.operator_loc().start_offset())
    } else if let Some(n) = node.as_index_operator_write_node() {
        (
            n.is_safe_navigation(),
            n.binary_operator_loc().start_offset(),
        )
    } else if let Some(n) = node.as_index_and_write_node() {
        (n.is_safe_navigation(), n.operator_loc().start_offset())
    } else {
        let n = node.as_index_or_write_node()?;
        (n.is_safe_navigation(), n.operator_loc().start_offset())
    };
    (!safe).then_some(FrameKind::AsgnTarget {
        block_end: op_start,
    })
}

/// `base_description(indent_base_type)`.
fn base_description(base_type: BaseType) -> &'static str {
    match base_type {
        BaseType::LeftBracket => "the position of the opening bracket",
        BaseType::AfterParen => "the first position after the preceding left parenthesis",
        BaseType::ParentHashKey => "the parent hash key",
        BaseType::StartOfLine => "the start of the line where the left square bracket is",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Vec<(usize, usize, isize, String)> {
        check_first_array_element_indentation(source.as_bytes(), style, 2, false)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.message))
            .collect()
    }

    #[test]
    fn operand_array_first_element_and_right_bracket() {
        let got = run("a << [\n 1\n  ]\n", 0);
        assert_eq!(got.len(), 2);
        assert_eq!(
            (got[0].2, got[0].3.as_str()),
            (
                1,
                "Use 2 spaces for indentation in an array, relative to the \
                 start of the line where the left square bracket is."
            )
        );
        assert_eq!(
            (got[1].2, got[1].3.as_str()),
            (
                -2,
                "Indent the right bracket the same as the start of the line \
                 where the left bracket is."
            )
        );
    }

    #[test]
    fn accepts_correct_indentation_and_same_line_first_element() {
        assert!(run("a = [\n  1\n]\n", 0).is_empty());
        assert!(run("a = [1,\n     2]\n", 0).is_empty());
        assert!(run("a = [1, 2]\n", 0).is_empty());
        assert!(run("a = []\n", 0).is_empty());
        assert!(run("a, b = b, a\n", 0).is_empty());
    }

    #[test]
    fn special_inside_parentheses_claims_via_send() {
        let got = run("func([\n  1\n])\n", 0);
        assert_eq!(got.len(), 2);
        assert!(
            got[0]
                .3
                .contains("the first position after the preceding left parenthesis"),
            "{}",
            got[0].3
        );
        assert_eq!(got[0].2, 5);
        assert_eq!(got[1].2, 5);
    }

    #[test]
    fn consistent_style_ignores_parentheses() {
        let got = run("func([\n       1\n     ])\n", 1);
        assert_eq!(got.len(), 2);
        assert!(got[0].3.contains("the start of the line"), "{}", got[0].3);
        assert_eq!(got[0].2, -5);
    }

    #[test]
    fn align_brackets_uses_bracket_column() {
        let got = run("var = [\n  1\n]\n", 2);
        assert_eq!(got.len(), 2);
        assert!(
            got[0].3.contains("the position of the opening bracket"),
            "{}",
            got[0].3
        );
        assert_eq!(got[0].2, 6); // expected 6 + 2 = 8, actual 2
        assert_eq!(got[1].2, 6);
    }

    #[test]
    fn parent_hash_key_base_for_multi_pair_hash() {
        let got = run(
            "func(x: [\n  :a,\n       :b\n],\n     y: [\n       :c\n     ])\n",
            0,
        );
        assert_eq!(got.len(), 2);
        assert!(got[0].3.contains("the parent hash key"), "{}", got[0].3);
        assert_eq!(got[0].2, 5); // pair col 5 + 2 = 7, actual 2
        assert!(got[1].3.contains("parent hash key"), "{}", got[1].3);
        assert_eq!(got[1].2, 5);
    }

    #[test]
    fn index_send_never_claims() {
        // `foo[...]` has no `(`; the inner array falls back to start-of-line.
        assert!(run("foo[[\n  1]]\n", 0).is_empty());
    }

    #[test]
    fn block_body_is_scannable_through_the_send() {
        // parser's `:block` node contains the send, so `func`'s scan reaches
        // the array inside the block body.
        let got = run("func(foo { [\n  1\n] })\n", 0);
        assert_eq!(got.len(), 2);
        assert!(
            got[0].3.contains("preceding left parenthesis"),
            "{}",
            got[0].3
        );
        assert_eq!(got[0].2, 5);
    }

    #[test]
    fn nested_plain_send_blocks_outer_claim() {
        // `inner` (a plain send) stops `outer`'s scan; `inner` itself claims:
        // base is inner's `(` col 11 + 1, expected 14, not outer's 5 + 1 + 2.
        let got = run("outer(inner([\n  1\n]))\n", 0);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].2, 12); // 14 - actual col 2
        assert_eq!(got[1].2, 12); // `]` at col 0
    }

    #[test]
    fn op_assign_value_is_claimable_but_target_is_not() {
        // The assigned value sits outside parser's nested `a.b` send, so the
        // outer `(` claims it (after-paren base, col 5 + 1 + 2 = 8).
        let got = run("func(a.b += [\n  1\n])\n", 0);
        assert_eq!(got.len(), 2);
        assert!(
            got[0].3.contains("preceding left parenthesis"),
            "{}",
            got[0].3
        );
        assert_eq!(got[0].2, 5);
        assert_eq!(got[1].2, 5);
        // An array inside the target (the receiver) is blocked from the outer
        // `(` and falls back to start-of-line (col 0 + 2 = 2: no offense).
        assert!(run("func([\n  1\n].b += x)\n", 0).is_empty());
    }

    #[test]
    fn disabled_when_array_alignment_enforces_fixed_indentation() {
        assert!(
            check_first_array_element_indentation("func([\n  1\n])\n".as_bytes(), 0, 2, true)
                .is_empty()
        );
        // ...unless the style is `consistent`.
        assert!(
            !check_first_array_element_indentation(
                "func([\n       1\n     ])\n".as_bytes(),
                1,
                2,
                true
            )
            .is_empty()
        );
    }
}
