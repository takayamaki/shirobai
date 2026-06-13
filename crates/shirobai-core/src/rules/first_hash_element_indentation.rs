//! `Layout/FirstHashElementIndentation`.
//!
//! Checks the indentation of the first key in a hash literal whose opening
//! brace and first key are on separate lines, and of a hanging right brace.
//! The sibling of `Layout/FirstArrayElementIndentation`: same
//! `MultilineElementIndentation` mixin, same `AlignmentCorrector` division of
//! labour (Rust computes the offense range, `column_delta` and message; Ruby
//! applies the realignment), and the same `each_argument_node` / `ignore_node`
//! claiming of a hash by a method call's left parenthesis.
//!
//! Hash-specific differences from the array cop:
//!
//! - The checked node is always a braced [`HashNode`]; a braceless keyword hash
//!   (`func a: 1`) is a `KeywordHashNode` with no `opening_loc` and is never
//!   checked (stock's `on_hash` requires `node.loc.begin`).
//! - The first element is `hash_node.pairs.first` — the first `AssocNode`,
//!   skipping a leading `**kwsplat` (`AssocSplatNode`), exactly like parser's
//!   `.pairs`.
//! - When `Layout/HashAlignment` enforces the `separator` style for the first
//!   pair's separator, `check_based_on_longest_key` shifts the expected column
//!   right by `max_key_length - first_key_length` (`key.source_range.length`).
//! - The `enforce` gate watches `Layout/ArgumentAlignment` (not ArrayAlignment)
//!   and stands down only the paren claim (`on_send`), not the whole cop, with
//!   no style exemption: an enforced run simply never lets a `(` claim a hash.
//!
//! Columns are parser-gem columns (character counts from the line start), not
//! display width.

use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// One misindented first pair or right brace. `[start_offset, end_offset)` is
/// the offense range (the first pair node, or the `}` token), which Ruby
/// reports and realigns by `column_delta` via `AlignmentCorrector`.
pub struct FirstHashElemIndentOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub message: String,
    /// For a first-pair offense whose value begins on the same line as (or
    /// before) its key, Ruby corrects the whole pair node; otherwise it
    /// corrects only the key's line. `None` for a right-brace offense (always
    /// a plain range correction). `Some((value_line_le_key_line))`.
    pub correct_whole_pair: Option<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    /// `special_inside_parentheses` (default).
    SpecialInsideParens,
    Consistent,
    AlignBraces,
}

impl Style {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Style::Consistent,
            2 => Style::AlignBraces,
            _ => Style::SpecialInsideParens,
        }
    }
}

/// `indent_base`'s second return value: what the expected column is based on.
#[derive(Clone, Copy)]
enum BaseType {
    /// `:left_brace_or_bracket` (`align_braces` style).
    LeftBrace,
    /// `:first_column_after_left_parenthesis`.
    AfterParen,
    /// `:parent_hash_key`.
    ParentHashKey,
    /// `:start_of_line`.
    StartOfLine,
}

/// Which separator style `Layout/HashAlignment` enforces for the colon / hash
/// rocket flavours. `1` means `separator`; everything else (`key`, `table`,
/// etc.) means "not separator".
#[derive(Clone, Copy)]
pub struct SeparatorConfig {
    pub colon_separator: bool,
    pub rocket_separator: bool,
}

pub fn check_first_hash_element_indentation(
    source: &[u8],
    style: u8,
    indent_width: usize,
    enforce_fixed_indentation: bool,
    separators: SeparatorConfig,
) -> Vec<FirstHashElemIndentOffense> {
    let mut rule = build_rule(
        source,
        style,
        indent_width,
        enforce_fixed_indentation,
        separators,
    );
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for standalone or bundled (shared-walk) use. Unlike the array
/// cop, the hash cop never disables itself outright — the `enforce` flag only
/// suppresses paren claiming — so this always returns a `Visitor`.
pub(crate) fn build_rule(
    source: &[u8],
    style: u8,
    indent_width: usize,
    enforce_fixed_indentation: bool,
    separators: SeparatorConfig,
) -> Visitor<'_> {
    Visitor {
        source,
        line_index: super::line_index::with_line_index(source, |li| li.clone()),
        style: Style::from_u8(style),
        indent: indent_width,
        enforce_fixed: enforce_fixed_indentation,
        separators,
        stack: Vec::new(),
        offenses: Vec::new(),
    }
}

/// Lightweight ancestor frame kind, mirroring the array cop's. See that file
/// for the `ArgumentsNode` typed-field-bypass note: containment tests use
/// offset ranges, which nest, so transparent levels never change them.
enum FrameKind {
    Call {
        csend: bool,
        paren_start: Option<usize>,
        args_range: Option<(usize, usize)>,
    },
    BlockArgument,
    Block,
    /// A hash `pair` (`AssocNode`), for the parent-hash-key base.
    Assoc {
        key_start: usize,
        value_start: usize,
    },
    /// A hash (braced or keyword), carrying its pair-only element ranges for
    /// `pair.right_sibling` (parser's `.pairs` excludes kwsplat).
    Hash {
        pairs: Vec<(usize, usize)>,
    },
    AsgnTarget {
        block_end: usize,
    },
    Other,
}

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
    enforce_fixed: bool,
    separators: SeparatorConfig,
    stack: Vec<Frame>,
    pub(crate) offenses: Vec<FirstHashElemIndentOffense>,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

/// Ruby regex `\s` (the line-local subset).
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r')
}

/// Pair-only ranges of a hash node's elements (its `AssocNode`s), filtering out
/// `**kwsplat` (`AssocSplatNode`), matching parser's `.pairs`.
fn pair_ranges<'a>(elements: impl Iterator<Item = Node<'a>>) -> Vec<(usize, usize)> {
    elements
        .filter(|e| e.as_assoc_node().is_some())
        .map(|e| loc(&e.location()))
        .collect()
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(h) = node.as_hash_node() {
            self.process_hash(&h);
        }
        self.stack.push(self.make_frame(node));
    }

    fn leave(&mut self) {
        self.stack.pop();
    }
}

impl Visitor<'_> {
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
                pairs: pair_ranges(h.elements().iter()),
            }
        } else if let Some(h) = node.as_keyword_hash_node() {
            FrameKind::Hash {
                pairs: pair_ranges(h.elements().iter()),
            }
        } else if let Some(k) = asgn_target_kind(node) {
            k
        } else {
            FrameKind::Other
        };
        Frame { start, end, kind }
    }

    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn column(&self, off: usize) -> usize {
        self.line_index.column(self.source, off)
    }

    /// Column of the first non-blank character of `off`'s line.
    fn line_first_nonws_column(&self, off: usize) -> usize {
        let ls = self.line_index.line_start(off);
        self.source[ls..].iter().take_while(|&&b| is_ws(b)).count()
    }

    /// The `(` that claims this hash, or `None`. Identical to the array cop's
    /// logic; an enforced run (`Layout/ArgumentAlignment: with_fixed_indentation`)
    /// suppresses claiming entirely.
    fn claimed_paren(&self, hash_open_start: usize) -> Option<usize> {
        if self.enforce_fixed {
            return None;
        }
        let hash_line = self.line_of(hash_open_start);
        let n = self.stack.len();
        let next_start =
            |i: usize| -> usize { self.stack.get(i + 1).map_or(hash_open_start, |f| f.start) };
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
            let ns = next_start(i);
            let via_arguments = args_range.is_some_and(|(s, e)| s <= ns && ns < e)
                || (i + 1 < n && matches!(self.stack[i + 1].kind, FrameKind::BlockArgument));
            if via_arguments && self.line_of(p) == hash_line {
                return Some(p);
            }
        }
        None
    }

    /// `check(hash_node, left_parenthesis)`.
    fn process_hash(&mut self, h: &ruby_prism::HashNode<'_>) {
        let open = h.opening_loc();
        let open_start = open.start_offset();
        let paren = self.claimed_paren(open_start);

        // `pairs.first`: the first AssocNode, skipping a leading kwsplat.
        let first_pair = h.elements().iter().find_map(|e| {
            e.as_assoc_node().map(|a| {
                let key = a.key().location();
                let value = a.value().location();
                PairInfo {
                    range: loc(&a.as_node().location()),
                    key_range: loc(&key),
                    value_start: value.start_offset(),
                }
            })
        });

        if let Some(pair) = &first_pair {
            if self.line_of(pair.range.0) == self.line_of(open_start) {
                // Still check the right brace below.
            } else if self.separator_style(pair) {
                let offset = self.longest_key_offset(h, pair);
                self.check_first(pair, open_start, paren, offset);
            } else {
                self.check_first(pair, open_start, paren, 0);
            }
        }
        let close = h.closing_loc();
        self.check_right_brace(loc(&close), first_pair.as_ref(), open_start, paren);
    }

    /// `separator_style?(first_pair)`: is the configured `Layout/HashAlignment`
    /// style `separator` for the first pair's separator?
    fn separator_style(&self, pair: &PairInfo) -> bool {
        // The separator is the operator between key and value. For a colon
        // pair (`a: 1`) the key's source ends with `:`; for a rocket pair
        // (`'a' => 1`) it does not. `is?(':')` in stock tests the operator
        // token text. We approximate via the key's trailing `:` which is how
        // prism lays out symbol keys, falling back to scanning for `=>`.
        let is_colon = self.source.get(pair.key_range.1 - 1) == Some(&b':');
        if is_colon {
            self.separators.colon_separator
        } else {
            self.separators.rocket_separator
        }
    }

    /// `check_based_on_longest_key`'s offset: `max(key lengths) - first key
    /// length`, over the hash's pairs (`key.source_range.length`).
    fn longest_key_offset(&self, h: &ruby_prism::HashNode<'_>, first: &PairInfo) -> usize {
        let first_len = first.key_range.1 - first.key_range.0;
        let max_len = h
            .elements()
            .iter()
            .filter_map(|e| e.as_assoc_node())
            .map(|a| {
                let k = a.key().location();
                k.end_offset() - k.start_offset()
            })
            .max()
            .unwrap_or(first_len);
        max_len - first_len
    }

    /// `check_first(first_pair, left_brace, left_parenthesis, offset)`.
    fn check_first(
        &mut self,
        pair: &PairInfo,
        open_start: usize,
        paren: Option<usize>,
        offset: usize,
    ) {
        let actual_column = self.column(pair.range.0);
        let (base_column, base_type) = self.indent_base(open_start, true, paren);
        let expected_column = base_column + self.indent + offset;
        let column_delta = expected_column as isize - actual_column as isize;
        if column_delta == 0 {
            return;
        }
        let message = format!(
            "Use {} spaces for indentation in a hash, relative to {}.",
            self.indent,
            base_description(base_type)
        );
        // Autocorrect target: whole pair when the value begins on the same
        // line as (or before) the key, else only the key's line.
        let value_le_key = self.line_of(pair.value_start) <= self.line_of(pair.key_range.0);
        self.offenses.push(FirstHashElemIndentOffense {
            start_offset: pair.range.0,
            end_offset: pair.range.1,
            column_delta,
            message,
            correct_whole_pair: Some(value_le_key),
        });
    }

    /// `check_right_brace(right_brace, first_pair, left_brace, left_parenthesis)`.
    fn check_right_brace(
        &mut self,
        close: (usize, usize),
        first: Option<&PairInfo>,
        open_start: usize,
        paren: Option<usize>,
    ) {
        let ls = self.line_index.line_start(close.0);
        if self.source[ls..close.0].iter().any(|&b| !is_ws(b)) {
            return;
        }
        let (expected_column, base_type) = self.indent_base(open_start, first.is_some(), paren);
        let column_delta = expected_column as isize - self.column(close.0) as isize;
        if column_delta == 0 {
            return;
        }
        let message = match base_type {
            BaseType::LeftBrace => "Indent the right brace the same as the left brace.",
            BaseType::AfterParen => {
                "Indent the right brace the same as the first position \
                 after the preceding left parenthesis."
            }
            BaseType::ParentHashKey => "Indent the right brace the same as the parent hash key.",
            BaseType::StartOfLine => {
                "Indent the right brace the same as the start of the line \
                 where the left brace is."
            }
        };
        self.offenses.push(FirstHashElemIndentOffense {
            start_offset: close.0,
            end_offset: close.1,
            column_delta,
            message: message.to_string(),
            correct_whole_pair: None,
        });
    }

    /// `indent_base(left_brace, first, left_parenthesis)`. `has_first` mirrors
    /// stock passing `first` (a truthy pair) vs `first_pair` (possibly nil) —
    /// the parent-hash-key base only applies when there is a first pair.
    fn indent_base(
        &self,
        open_start: usize,
        has_first: bool,
        paren: Option<usize>,
    ) -> (usize, BaseType) {
        if self.style == Style::AlignBraces {
            return (self.column(open_start), BaseType::LeftBrace);
        }
        if has_first
            && let Some(col) = self.parent_hash_key_column(open_start)
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

    /// `hash_pair_where_value_beginning_with` + the two pair conditions. The
    /// hash's direct parent is a pair whose value begins exactly at this hash's
    /// left brace, whose key and value begin on the same line, and whose right
    /// sibling begins on a subsequent line. Returns `pair.loc.column`.
    fn parent_hash_key_column(&self, open_start: usize) -> Option<usize> {
        let n = self.stack.len();
        // `first.parent` is the pair; `first.parent.loc.begin == left_brace`
        // means the hash is the pair's value (begins at the brace).
        let pair = self.stack.last()?;
        let FrameKind::Assoc {
            key_start,
            value_start,
        } = pair.kind
        else {
            return None;
        };
        if value_start != open_start {
            return None;
        }
        let (start, end) = (pair.start, pair.end);
        if self.line_of(key_start) != self.line_of(value_start) {
            return None;
        }
        let FrameKind::Hash { pairs } = &self.stack[n.checked_sub(2)?].kind else {
            return None;
        };
        let idx = pairs.iter().position(|&(s, _)| s == start)?;
        let sibling = pairs.get(idx + 1)?;
        if self.line_of(end - 1) >= self.line_of(sibling.0) {
            return None;
        }
        Some(self.column(start))
    }
}

/// First-pair geometry needed by the checks.
struct PairInfo {
    range: (usize, usize),
    key_range: (usize, usize),
    value_start: usize,
}

/// Classifies `a.b op= x` / `a[i] op= x` as an [`FrameKind::AsgnTarget`] (see
/// the array cop for the rationale).
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
    } else if let Some(n) = node.as_index_or_write_node() {
        (n.is_safe_navigation(), n.operator_loc().start_offset())
    } else {
        return None;
    };
    (!safe).then_some(FrameKind::AsgnTarget {
        block_end: op_start,
    })
}

/// `base_description(indent_base_type)`.
fn base_description(base_type: BaseType) -> &'static str {
    match base_type {
        BaseType::LeftBrace => "the position of the opening brace",
        BaseType::AfterParen => "the first position after the preceding left parenthesis",
        BaseType::ParentHashKey => "the parent hash key",
        BaseType::StartOfLine => "the start of the line where the left curly brace is",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_SEP: SeparatorConfig = SeparatorConfig {
        colon_separator: false,
        rocket_separator: false,
    };

    fn run(source: &str, style: u8) -> Vec<(usize, usize, isize, String)> {
        check_first_hash_element_indentation(source.as_bytes(), style, 2, false, KEY_SEP)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.message))
            .collect()
    }

    #[test]
    fn operand_hash_first_pair_and_right_brace() {
        let got = run("a << {\n a: 1\n}\n", 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, 1);
        assert!(
            got[0].3.contains("the start of the line where the left curly brace is"),
            "{}",
            got[0].3
        );
    }

    #[test]
    fn accepts_correct_and_same_line() {
        assert!(run("a = {\n  a: 1\n}\n", 0).is_empty());
        assert!(run("a = { a: 1,\n      b: 2 }\n", 0).is_empty());
        assert!(run("a = { a: 1, b: 2 }\n", 0).is_empty());
        assert!(run("a = {}\n", 0).is_empty());
    }

    #[test]
    fn special_inside_parentheses_claims_via_send() {
        let got = run("func({\n  a: 1\n})\n", 0);
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
    fn consistent_ignores_parentheses() {
        let got = run("func({\n       a: 1\n     })\n", 1);
        assert_eq!(got.len(), 2);
        assert!(got[0].3.contains("the start of the line"), "{}", got[0].3);
        assert_eq!(got[0].2, -5);
    }

    #[test]
    fn align_braces_uses_brace_column() {
        let got = run("var = {\n  a: 1\n}\n", 2);
        assert_eq!(got.len(), 2);
        assert!(
            got[0].3.contains("the position of the opening brace"),
            "{}",
            got[0].3
        );
        assert_eq!(got[0].2, 6);
        assert_eq!(got[1].2, 6);
    }

    #[test]
    fn parent_hash_key_base_for_multi_pair_hash() {
        let got = run(
            "func(x: {\n  a: 1,\n       b: 2\n},\n     y: {\n       c: 1,\n       d: 2\n     })\n",
            0,
        );
        assert!(got.iter().any(|o| o.3.contains("the parent hash key")));
    }

    #[test]
    fn separator_colon_shifts_by_longest_key() {
        let sep = SeparatorConfig {
            colon_separator: true,
            rocket_separator: false,
        };
        // keys `a` (len 2 incl colon) and `aaa` (len 4): offset = 4 - 2 = 2.
        let got = check_first_hash_element_indentation(
            "a << {\n       a: 1,\n     aaa: 222\n}\n".as_bytes(),
            0,
            2,
            false,
            sep,
        );
        assert_eq!(got.len(), 1);
        // base 0 + indent 2 + offset 2 = 4; actual col 7 -> delta -3.
        assert_eq!(got[0].column_delta, -3);
    }

    #[test]
    fn kwsplat_first_is_skipped() {
        // `pairs.first` is `b: 2`, on line 3 at col 2 = base 0 + 2: no offense.
        assert!(run("a = {\n    **opts,\n  b: 2\n}\n", 0).is_empty());
    }

    #[test]
    fn enforce_fixed_suppresses_paren_claim_only() {
        // With enforce, the paren no longer claims; start-of-line base col 0 +
        // 2 == actual col 2: no offense. The cop is NOT disabled (consistent
        // and operand hashes still checked).
        let got =
            check_first_hash_element_indentation("func({\n  a: 1\n})\n".as_bytes(), 0, 2, true, KEY_SEP);
        assert!(got.is_empty());
        // An operand hash is still checked under enforce.
        let got2 =
            check_first_hash_element_indentation("a << {\n a: 1\n}\n".as_bytes(), 0, 2, true, KEY_SEP);
        assert_eq!(got2.len(), 1);
    }

    #[test]
    fn autocorrect_target_flag() {
        // value on next line -> key-line-only correction.
        let got = check_first_hash_element_indentation(
            "a = {\n    a:\n  {\n    b: 1\n  }\n}\n".as_bytes(),
            0,
            2,
            false,
            KEY_SEP,
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].correct_whole_pair, Some(false));
        // value on same line -> whole-pair correction.
        let got2 = check_first_hash_element_indentation(
            "a = {\n    a: {\n      b: 1\n    }\n}\n".as_bytes(),
            0,
            2,
            false,
            KEY_SEP,
        );
        assert_eq!(got2.len(), 1);
        assert_eq!(got2[0].correct_whole_pair, Some(true));
    }
}
