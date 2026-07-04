//! `Layout/SpaceInsideArrayLiteralBrackets`.
//!
//! Checks the space just inside `[` and `]` of array literals and array
//! patterns, per `EnforcedStyle` (`no_space` / `space` / `compact`) and
//! `EnforcedStyleForEmptyBrackets` (`no_space` / `space`).
//!
//! Stock's `on_array` / `on_array_pattern` reads
//! `processed_source.tokens_within(node)`; every token-level fact is
//! reconstructed from bytes and the AST (see `space_scan`):
//!
//! - `empty_brackets?` — the brackets are adjacent *tokens*: only whitespace
//!   and `\`-newline continuations between them (a comment is a token);
//! - `next_to_newline?` / `next_to_comment?` — the next-token scan after `[`;
//! - `end_has_own_line?` — only whitespace between the line start and `]`;
//! - `multi_dimensional_array?` — the token next to the bracket (skipping
//!   `tNL`s, i.e. plain whitespace) is itself a bracket. After `[`, a `[` byte
//!   at token start is always a `tLBRACK`; before `]`, a `]` byte is a
//!   `tRBRACK` only when it closes an array / array pattern / find pattern /
//!   bracketed hash pattern / index call (a `%w[...]` closer is a
//!   `tSTRING_END`, a `?]` character literal is a `tCHARACTER`). Those closer
//!   positions are collected during the walk (compact style only) and pending
//!   right-bracket checks resolve against the set afterwards.
//!
//! Stock's `find_node_with_brackets` redirects every array pattern to its
//! nearest `const_pattern` ancestor and re-derives the pair as *first
//! left-bracket token / last right-bracket token* within that node. Prism has
//! no `const_pattern` node — the constant is a field — so the redirect
//! becomes:
//!
//! - `ADT[...]` (array pattern with constant, bracketed): checked directly —
//!   the first/last bracket tokens are its own delimiters. A bare array
//!   pattern nested below it redirects to the same pair, producing an exact
//!   duplicate that stock's `add_offense` location dedup silently drops, so
//!   nothing is emitted for it.
//! - `ADT(...)` (constant, parenthesized): parser still wraps an
//!   `array_pattern` child, so the redirect fires unconditionally; the pair is
//!   hunted as min/max bracket positions over the subtree (possibly a
//!   mismatched pair spanning two sibling patterns — verified against stock).
//! - `ADT[*, x, *]` / `ADT[k: v]` (find / hash pattern with constant): stock
//!   only reaches these through a bare array pattern somewhere below (the
//!   cop has no `on_find_pattern` and `on_hash_pattern` belongs to the hash
//!   cop), and the check runs once against the ancestor's pair.
//! - A bare array pattern with no constant-bearing ancestor is checked
//!   directly; a braceless top-level pattern (`in a, [b]`) hunts min/max
//!   bracket positions over its own subtree like stock's token find.
//!
//! "Under a constant-bearing pattern" is decided without an ancestor stack:
//! pattern ranges nest or are disjoint and the walk is document-ordered, so a
//! node is inside one iff its start is below the running maximum of
//! constant-pattern end offsets seen so far.
//!
//! Alongside each offense the rule emits the corrector program for the node
//! (`SpaceCorrector` / `compact_corrections` reduced to remove / insert-after
//! / insert-before ops); the wrapper replays it on the node's first offense,
//! mirroring stock's `ignore_node` grouping.

use std::collections::HashSet;

use ruby_prism::{
    Location, Node, Visit, visit_array_pattern_node, visit_find_pattern_node,
    visit_hash_pattern_node,
};

use super::line_index::LineIndex;
use super::space_scan::{
    is_ruby_space, next_token_start, prev_non_space, skip_space_and_newlines_left,
    skip_space_and_newlines_right, skip_space_left, skip_space_right,
};

/// `EnforcedStyle` value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    NoSpace,
    Space,
    Compact,
}

/// Config for `Layout/SpaceInsideArrayLiteralBrackets`.
#[derive(Clone, Copy)]
pub struct Config {
    pub style: Style,
    /// `EnforcedStyleForEmptyBrackets == 'space'`.
    pub space_empty: bool,
}

/// One offense. `node` indexes into [`ArrayBracketsResult::node_ops`]; the
/// wrapper applies that op list on the node's first offense (stock's
/// `ignore_node` grouping). `suppress_when_disable_uncorrectable` mirrors the
/// `autocorrect_with_disable_uncorrectable? && !start_ok` early return: the
/// wrapper drops the offense when that mode is active.
pub struct SpaceInsideArrayLiteralBracketsOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: MessageId,
    pub node: usize,
    pub suppress_when_disable_uncorrectable: bool,
}

/// The four fixed messages stock emits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MessageId {
    /// `'Use space inside array brackets.'`
    Use,
    /// `'Do not use space inside array brackets.'`
    DoNotUse,
    /// `'Use one space inside empty array brackets.'`
    UseOneEmpty,
    /// `'Do not use space inside empty array brackets.'`
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

/// One corrector call of the node's correction program.
#[derive(Clone, Copy)]
pub enum Op {
    /// `corrector.remove(range)`.
    Remove(usize, usize),
    /// `corrector.insert_after(range, ' ')`.
    InsertAfter(usize, usize),
    /// `corrector.insert_before(range, ' ')`.
    InsertBefore(usize, usize),
}

impl Op {
    /// `(op_code, start, end)` for the wire: 0 remove, 1 insert-after,
    /// 2 insert-before.
    pub fn packed(self) -> (u8, usize, usize) {
        match self {
            Op::Remove(s, e) => (0, s, e),
            Op::InsertAfter(s, e) => (1, s, e),
            Op::InsertBefore(s, e) => (2, s, e),
        }
    }
}

/// Offenses plus the per-node correction programs they reference. `node_ops`
/// holds programs only for nodes that produced at least one offense (indexed
/// by the offenses' `node` field in first-appearance order) — a program is
/// only ever replayed on a node's first offense, and shipping one per checked
/// node made the wire volume scale with the number of array literals instead
/// of the number of offenses.
pub struct ArrayBracketsResult {
    pub offenses: Vec<SpaceInsideArrayLiteralBracketsOffense>,
    pub node_ops: Vec<Vec<Op>>,
}

pub fn check_space_inside_array_literal_brackets(
    source: &[u8],
    config: Config,
) -> ArrayBracketsResult {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.into_result()
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    Visitor {
        source,
        config,
        line_index: LineIndex::new(source),
        items: Vec::new(),
        node_ops: Vec::new(),
        rbrack_closers: HashSet::new(),
        const_scope_end: 0,
    }
}

/// A resolved offense or a compact right-bracket check deferred on the
/// `tRBRACK`-ness of the preceding `]`.
enum Item {
    Ready(SpaceInsideArrayLiteralBracketsOffense),
    PendingCompactRight {
        node: usize,
        rb_s: usize,
        rb_e: usize,
        prev_end: usize,
        end_ok: bool,
    },
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    config: Config,
    line_index: LineIndex,
    items: Vec<Item>,
    node_ops: Vec<Vec<Op>>,
    /// Closing-bracket positions that lex as `tRBRACK`. Only filled under
    /// `compact`.
    rbrack_closers: HashSet<usize>,
    /// Running max end offset of constant-bearing pattern nodes entered so
    /// far; a node starting below it sits inside one (ranges nest or are
    /// disjoint in document order).
    const_scope_end: usize,
}

impl<'a> Visitor<'a> {
    /// Resolve deferred compact right checks and return offenses + per-node
    /// correction programs in stock's emission order.
    pub(crate) fn into_result(self) -> ArrayBracketsResult {
        let Visitor {
            source,
            items,
            mut node_ops,
            rbrack_closers,
            ..
        } = self;
        let mut offenses = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Item::Ready(o) => offenses.push(o),
                Item::PendingCompactRight {
                    node,
                    rb_s,
                    rb_e,
                    prev_end,
                    end_ok,
                } => {
                    let multi_dim = rbrack_closers.contains(&(prev_end - 1));
                    if multi_dim {
                        // qualifies_for_compact?: multi-dimensional and the
                        // `]` has whitespace before it (`\s`, newline counts).
                        if is_ruby_space(source[rb_s - 1]) {
                            offenses.push(SpaceInsideArrayLiteralBracketsOffense {
                                start_offset: skip_space_left(source, rb_s),
                                end_offset: rb_s,
                                message: MessageId::DoNotUse,
                                node,
                                suppress_when_disable_uncorrectable: false,
                            });
                        }
                        // compact_corrections removes the (possibly empty)
                        // newline-inclusive run before the `]`.
                        node_ops[node]
                            .push(Op::Remove(skip_space_and_newlines_left(source, rb_s), rb_s));
                    } else {
                        // space_offenses(node, nil, right, start_ok: true).
                        if !(matches!(source[rb_s - 1], b' ' | b'\t') || end_ok) {
                            offenses.push(SpaceInsideArrayLiteralBracketsOffense {
                                start_offset: rb_s,
                                end_offset: rb_e,
                                message: MessageId::Use,
                                node,
                                suppress_when_disable_uncorrectable: false,
                            });
                        }
                        if !is_ruby_space(source[rb_s - 1]) {
                            node_ops[node].push(Op::InsertBefore(rb_s, rb_e));
                        }
                    }
                }
            }
        }
        // Keep only the offending nodes' programs; renumber the offenses'
        // node keys to the compacted vector (first-appearance order).
        let mut remap: Vec<Option<usize>> = vec![None; node_ops.len()];
        let mut kept_ops: Vec<Vec<Op>> = Vec::new();
        for offense in &mut offenses {
            let slot = match remap[offense.node] {
                Some(slot) => slot,
                None => {
                    let slot = kept_ops.len();
                    kept_ops.push(std::mem::take(&mut node_ops[offense.node]));
                    remap[offense.node] = Some(slot);
                    slot
                }
            };
            offense.node = slot;
        }
        ArrayBracketsResult {
            offenses,
            node_ops: kept_ops,
        }
    }

    fn offense(
        &mut self,
        start: usize,
        end: usize,
        message: MessageId,
        node: usize,
        suppress: bool,
    ) {
        self.items
            .push(Item::Ready(SpaceInsideArrayLiteralBracketsOffense {
                start_offset: start,
                end_offset: end,
                message,
                node,
                suppress_when_disable_uncorrectable: suppress,
            }));
    }

    /// Stock's bracket check for one node: `node_start..node_end` is the
    /// grouping node's range (`single_line?`), `lb`/`rb` the bracket pair
    /// (usually the node's own delimiters; a redirected const-pattern hunt may
    /// pass a mismatched pair).
    fn check_brackets(
        &mut self,
        node_start: usize,
        node_end: usize,
        lb_s: usize,
        lb_e: usize,
        rb_s: usize,
        rb_e: usize,
    ) {
        let src = self.source;
        let node = self.node_ops.len();
        self.node_ops.push(Vec::new());

        let (tok_pos, crossed) = next_token_start(src, lb_e);

        // empty_brackets?: `[` and `]` are adjacent tokens.
        if tok_pos == rb_s {
            if self.config.space_empty {
                // offending_empty_space?: not exactly one space between.
                if !(rb_s - lb_e == 1 && src[lb_e] == b' ') {
                    self.offense(lb_s, rb_e, MessageId::UseOneEmpty, node, false);
                    self.node_ops[node].push(Op::Remove(lb_e, rb_s));
                    self.node_ops[node].push(Op::InsertAfter(lb_s, lb_e));
                }
            } else if rb_s != lb_e {
                // offending_empty_no_space?: any characters between.
                self.offense(lb_s, rb_e, MessageId::DoNotUseEmpty, node, false);
                self.node_ops[node].push(Op::Remove(lb_e, rb_s));
            }
            return;
        }

        let start_ok_newline = crossed;
        let single_line =
            self.line_index.line_of(node_start) == self.line_index.line_of(node_end - 1);
        let end_ok = if single_line {
            false
        } else {
            self.end_has_own_line(rb_s)
        };

        match self.config.style {
            Style::NoSpace => {
                // start_ok is replaced by next_to_comment? (line-independent).
                let start_ok = src.get(tok_pos) == Some(&b'#');
                // extra_space?(left, :left): the byte after `[` is `[ \t]`.
                if matches!(src[lb_e], b' ' | b'\t') && !start_ok {
                    self.offense(
                        lb_e,
                        skip_space_right(src, lb_e),
                        MessageId::DoNotUse,
                        node,
                        false,
                    );
                }
                if matches!(src[rb_s - 1], b' ' | b'\t') && !end_ok {
                    self.offense(
                        skip_space_left(src, rb_s),
                        rb_s,
                        MessageId::DoNotUse,
                        node,
                        !start_ok,
                    );
                }
                // SpaceCorrector.remove_space (space_after? / space_before?
                // are full `\s`; the removed run is `[ \t]` only).
                if is_ruby_space(src[lb_e]) {
                    self.node_ops[node].push(Op::Remove(lb_e, skip_space_right(src, lb_e)));
                }
                if is_ruby_space(src[rb_s - 1]) {
                    self.node_ops[node].push(Op::Remove(skip_space_left(src, rb_s), rb_s));
                }
            }
            Style::Space => {
                if !(matches!(src[lb_e], b' ' | b'\t') || start_ok_newline) {
                    self.offense(lb_s, lb_e, MessageId::Use, node, false);
                }
                if !(matches!(src[rb_s - 1], b' ' | b'\t') || end_ok) {
                    self.offense(rb_s, rb_e, MessageId::Use, node, !start_ok_newline);
                }
                // SpaceCorrector.add_space.
                if !is_ruby_space(src[lb_e]) {
                    self.node_ops[node].push(Op::InsertAfter(lb_s, lb_e));
                }
                if !is_ruby_space(src[rb_s - 1]) {
                    self.node_ops[node].push(Op::InsertBefore(rb_s, rb_e));
                }
            }
            Style::Compact => {
                // Left: multi-dimensional iff the next token is a `[` (always
                // a genuine tLBRACK in this position).
                let multi_left = src.get(tok_pos) == Some(&b'[');
                if multi_left {
                    if is_ruby_space(src[lb_e]) {
                        // qualifies_for_compact?(left): space_after? is `\s`,
                        // the offense range is the `[ \t]` run (possibly
                        // empty when a newline follows directly).
                        self.offense(
                            lb_e,
                            skip_space_right(src, lb_e),
                            MessageId::DoNotUse,
                            node,
                            false,
                        );
                    }
                    // compact_corrections: newline-inclusive removal.
                    self.node_ops[node]
                        .push(Op::Remove(lb_e, skip_space_and_newlines_right(src, lb_e)));
                } else {
                    // space_offenses(node, left, nil, start_ok:, end_ok: true).
                    if !(matches!(src[lb_e], b' ' | b'\t') || start_ok_newline) {
                        self.offense(lb_s, lb_e, MessageId::Use, node, false);
                    }
                    if !is_ruby_space(src[lb_e]) {
                        self.node_ops[node].push(Op::InsertAfter(lb_s, lb_e));
                    }
                }

                // Right: the previous non-whitespace byte (the token walk
                // skipping tNLs). A `]` byte needs the closer set, which is
                // complete only after the walk.
                let prev_end = prev_non_space(src, rb_s).expect("a `[` precedes");
                if src[prev_end - 1] == b']' {
                    self.items.push(Item::PendingCompactRight {
                        node,
                        rb_s,
                        rb_e,
                        prev_end,
                        end_ok,
                    });
                } else {
                    // Not multi-dimensional: right side of space_offenses with
                    // start_ok: true.
                    if !(matches!(src[rb_s - 1], b' ' | b'\t') || end_ok) {
                        self.offense(rb_s, rb_e, MessageId::Use, node, false);
                    }
                    if !is_ruby_space(src[rb_s - 1]) {
                        self.node_ops[node].push(Op::InsertBefore(rb_s, rb_e));
                    }
                }
            }
        }
    }

    /// `end_has_own_line?`: nothing but whitespace between the line start and
    /// the `]` (the `col == -1` early return is the vacuous empty slice).
    fn end_has_own_line(&self, rb_s: usize) -> bool {
        let ls = self.line_index.line_start(rb_s);
        self.source[ls..rb_s].iter().all(|&b| is_ruby_space(b))
    }

    /// Record a closer position under `compact`.
    fn collect_closer(&mut self, opening: &Location<'_>, closing: &Location<'_>) {
        if self.config.style == Style::Compact
            && self.source.get(opening.start_offset()) == Some(&b'[')
        {
            self.rbrack_closers.insert(closing.start_offset());
        }
    }

    /// The min/max bracket-token hunt of stock's `array_brackets` over a
    /// redirected node (parenthesized const pattern or braceless top-level
    /// array pattern), plus the bare-array-pattern trigger scan for const
    /// find / hash patterns. Runs over the node's children (patterns are tiny
    /// and this only fires on the rare const/braceless forms).
    fn scan_subtree<F>(&self, visit_children: F) -> SubtreeScan<'a>
    where
        F: FnOnce(&mut SubtreeScan<'a>),
    {
        let mut scan = SubtreeScan {
            source: self.source,
            const_depth: 0,
            bare_trigger: false,
            min_lbrack: None,
            max_rbrack: None,
        };
        visit_children(&mut scan);
        scan
    }

    /// Dispatch one redirected check against `pair` (first/last bracket
    /// positions) grouped on the const node's range.
    fn check_hunted(&mut self, node_loc: &Location<'_>, scan: &SubtreeScan<'_>) {
        if let (Some(min_l), Some(max_r)) = (scan.min_lbrack, scan.max_rbrack) {
            self.check_brackets(
                node_loc.start_offset(),
                node_loc.end_offset(),
                min_l,
                min_l + 1,
                max_r,
                max_r + 1,
            );
        }
    }
}

/// Child scan for the redirected const-pattern checks: min/max bracket-token
/// positions over the subtree (every bracket source is a node delimiter) and
/// whether a bare bracketed array pattern sits below with no closer
/// constant-bearing pattern in between.
pub(crate) struct SubtreeScan<'a> {
    source: &'a [u8],
    const_depth: u32,
    bare_trigger: bool,
    min_lbrack: Option<usize>,
    max_rbrack: Option<usize>,
}

impl SubtreeScan<'_> {
    fn record_pair(&mut self, opening: usize, closing: usize) {
        if self.min_lbrack.is_none_or(|m| opening < m) {
            self.min_lbrack = Some(opening);
        }
        if self.max_rbrack.is_none_or(|m| closing > m) {
            self.max_rbrack = Some(closing);
        }
    }

    fn record_bracketed(
        &mut self,
        opening: Option<Location<'_>>,
        closing: Option<Location<'_>>,
    ) -> bool {
        if let (Some(o), Some(c)) = (opening, closing)
            && self.source.get(o.start_offset()) == Some(&b'[')
        {
            self.record_pair(o.start_offset(), c.start_offset());
            return true;
        }
        false
    }
}

impl<'pr> Visit<'pr> for SubtreeScan<'_> {
    fn visit_array_pattern_node(&mut self, node: &ruby_prism::ArrayPatternNode<'pr>) {
        let bracketed = self.record_bracketed(node.opening_loc(), node.closing_loc());
        if node.constant().is_some() {
            self.const_depth += 1;
            visit_array_pattern_node(self, node);
            self.const_depth -= 1;
        } else {
            if bracketed && self.const_depth == 0 {
                self.bare_trigger = true;
            }
            visit_array_pattern_node(self, node);
        }
    }

    fn visit_find_pattern_node(&mut self, node: &ruby_prism::FindPatternNode<'pr>) {
        self.record_bracketed(node.opening_loc(), node.closing_loc());
        if node.constant().is_some() {
            self.const_depth += 1;
            visit_find_pattern_node(self, node);
            self.const_depth -= 1;
        } else {
            visit_find_pattern_node(self, node);
        }
    }

    fn visit_hash_pattern_node(&mut self, node: &ruby_prism::HashPatternNode<'pr>) {
        self.record_bracketed(node.opening_loc(), node.closing_loc());
        if node.constant().is_some() {
            self.const_depth += 1;
            visit_hash_pattern_node(self, node);
            self.const_depth -= 1;
        } else {
            visit_hash_pattern_node(self, node);
        }
    }

    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        self.record_bracketed(node.opening_loc(), node.closing_loc());
        ruby_prism::visit_array_node(self, node);
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.record_bracketed(node.opening_loc(), node.closing_loc());
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_index_target_node(&mut self, node: &ruby_prism::IndexTargetNode<'pr>) {
        self.record_pair(
            node.opening_loc().start_offset(),
            node.closing_loc().start_offset(),
        );
        ruby_prism::visit_index_target_node(self, node);
    }

    fn visit_index_operator_write_node(&mut self, node: &ruby_prism::IndexOperatorWriteNode<'pr>) {
        self.record_pair(
            node.opening_loc().start_offset(),
            node.closing_loc().start_offset(),
        );
        ruby_prism::visit_index_operator_write_node(self, node);
    }

    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        self.record_pair(
            node.opening_loc().start_offset(),
            node.closing_loc().start_offset(),
        );
        ruby_prism::visit_index_or_write_node(self, node);
    }

    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        self.record_pair(
            node.opening_loc().start_offset(),
            node.closing_loc().start_offset(),
        );
        ruby_prism::visit_index_and_write_node(self, node);
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(array) = node.as_array_node() {
            if let (Some(o), Some(c)) = (array.opening_loc(), array.closing_loc())
                && self.source.get(o.start_offset()) == Some(&b'[')
            {
                self.check_brackets(
                    node.location().start_offset(),
                    node.location().end_offset(),
                    o.start_offset(),
                    o.end_offset(),
                    c.start_offset(),
                    c.end_offset(),
                );
                self.collect_closer(&o, &c);
            }
        } else if let Some(pat) = node.as_array_pattern_node() {
            let loc = node.location();
            match (pat.opening_loc(), pat.closing_loc()) {
                (Some(o), Some(c)) => {
                    self.collect_closer(&o, &c);
                    if pat.constant().is_some() {
                        let end = loc.end_offset();
                        if end > self.const_scope_end {
                            self.const_scope_end = end;
                        }
                        if self.source.get(o.start_offset()) == Some(&b'[') {
                            // `ADT[...]`: first/last bracket tokens are its
                            // own delimiters.
                            self.check_brackets(
                                loc.start_offset(),
                                loc.end_offset(),
                                o.start_offset(),
                                o.end_offset(),
                                c.start_offset(),
                                c.end_offset(),
                            );
                        } else {
                            // `ADT(...)`: parser wraps an array_pattern child
                            // whose redirect hunts the min/max bracket tokens.
                            let scan = self.scan_subtree(|s| visit_array_pattern_node(s, &pat));
                            self.check_hunted(&loc, &scan);
                        }
                    } else if self.source.get(o.start_offset()) == Some(&b'[')
                        && loc.start_offset() >= self.const_scope_end
                    {
                        // A bare bracketed pattern outside any const pattern is
                        // checked directly; inside one, the redirect is either
                        // an exact duplicate (dedup no-op) or handled by the
                        // const find/hash pattern's own enter below.
                        self.check_brackets(
                            loc.start_offset(),
                            loc.end_offset(),
                            o.start_offset(),
                            o.end_offset(),
                            c.start_offset(),
                            c.end_offset(),
                        );
                    }
                }
                (None, None) if loc.start_offset() >= self.const_scope_end => {
                    // Braceless top-level pattern (`in a, [b]`): stock hunts
                    // the first/last bracket tokens within the node range.
                    let scan = self.scan_subtree(|s| visit_array_pattern_node(s, &pat));
                    self.check_hunted(&loc, &scan);
                }
                _ => {}
            }
        } else if let Some(pat) = node.as_find_pattern_node() {
            if let (Some(o), Some(c)) = (pat.opening_loc(), pat.closing_loc()) {
                self.collect_closer(&o, &c);
                if pat.constant().is_some() {
                    let loc = node.location();
                    let end = loc.end_offset();
                    if end > self.const_scope_end {
                        self.const_scope_end = end;
                    }
                    // Reached only through a bare array pattern below (the cop
                    // has no on_find_pattern); the redirect then checks this
                    // node's first/last bracket pair once.
                    let scan = self.scan_subtree(|s| visit_find_pattern_node(s, &pat));
                    if scan.bare_trigger {
                        if self.source.get(o.start_offset()) == Some(&b'[') {
                            self.check_brackets(
                                loc.start_offset(),
                                loc.end_offset(),
                                o.start_offset(),
                                o.end_offset(),
                                c.start_offset(),
                                c.end_offset(),
                            );
                        } else {
                            self.check_hunted(&loc, &scan);
                        }
                    }
                }
            }
        } else if let Some(pat) = node.as_hash_pattern_node() {
            if let (Some(o), Some(c)) = (pat.opening_loc(), pat.closing_loc()) {
                self.collect_closer(&o, &c);
                if pat.constant().is_some() {
                    let loc = node.location();
                    let end = loc.end_offset();
                    if end > self.const_scope_end {
                        self.const_scope_end = end;
                    }
                    // Same as find patterns: only a bare array pattern below
                    // pulls the array cop onto this node's brackets.
                    let scan = self.scan_subtree(|s| visit_hash_pattern_node(s, &pat));
                    if scan.bare_trigger {
                        if self.source.get(o.start_offset()) == Some(&b'[') {
                            self.check_brackets(
                                loc.start_offset(),
                                loc.end_offset(),
                                o.start_offset(),
                                o.end_offset(),
                                c.start_offset(),
                                c.end_offset(),
                            );
                        } else {
                            self.check_hunted(&loc, &scan);
                        }
                    }
                }
            }
        } else if self.config.style == Style::Compact {
            // Index closers are tRBRACKs for the compact multi-dimension test.
            if let Some(call) = node.as_call_node() {
                if let (Some(o), Some(c)) = (call.opening_loc(), call.closing_loc()) {
                    self.collect_closer(&o, &c);
                }
            } else if let Some(n) = node.as_index_target_node() {
                self.collect_closer(&n.opening_loc(), &n.closing_loc());
            } else if let Some(n) = node.as_index_operator_write_node() {
                self.collect_closer(&n.opening_loc(), &n.closing_loc());
            } else if let Some(n) = node.as_index_or_write_node() {
                self.collect_closer(&n.opening_loc(), &n.closing_loc());
            } else if let Some(n) = node.as_index_and_write_node() {
                self.collect_closer(&n.opening_loc(), &n.closing_loc());
            }
        }
    }

    fn leave(&mut self) {}

    fn interest(&self) -> super::dispatch::Interest {
        // ArrayNode is ENTER_LITERAL; the pattern nodes are ENTER_OTHER. The
        // compact closer collection additionally reads CallNode (ENTER_CALL)
        // and the Index*Write/Target nodes (ENTER_WRITE). `enter` is a pure
        // kind match with an empty fall-through and `leave` / leaf / rescue
        // are unused, so the mask is exact.
        let mut mask =
            super::dispatch::Interest::ENTER_LITERAL | super::dispatch::Interest::ENTER_OTHER;
        if self.config.style == Style::Compact {
            mask |= super::dispatch::Interest::ENTER_CALL | super::dispatch::Interest::ENTER_WRITE;
        }
        super::dispatch::Interest(mask)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: Style, space_empty: bool) -> Vec<(usize, usize, u8, usize, bool)> {
        check_space_inside_array_literal_brackets(source.as_bytes(), Config { style, space_empty })
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

    fn ops(source: &str, style: Style, space_empty: bool) -> Vec<Vec<(u8, usize, usize)>> {
        check_space_inside_array_literal_brackets(source.as_bytes(), Config { style, space_empty })
            .node_ops
            .into_iter()
            .map(|v| v.into_iter().map(Op::packed).collect())
            .collect()
    }

    #[test]
    fn no_space_flags_leading_and_trailing() {
        // "a = [ 2, 3 ]": left space [5,6), right space [10,11).
        assert_eq!(
            run("a = [ 2, 3 ]\n", Style::NoSpace, false),
            vec![(5, 6, 1, 0, false), (10, 11, 1, 0, true)]
        );
        assert_eq!(
            ops("a = [ 2, 3 ]\n", Style::NoSpace, false),
            vec![vec![(0, 5, 6), (0, 10, 11)]]
        );
    }

    #[test]
    fn no_space_accepts_comment_after_bracket() {
        assert!(run("a = [ # c\n  1]\n", Style::NoSpace, false).is_empty());
    }

    #[test]
    fn no_space_flags_space_before_newline() {
        // "[ \n  1]": the space run after `[` offends even across the break.
        assert_eq!(
            run("a = [ \n  1]\n", Style::NoSpace, false),
            vec![(5, 6, 1, 0, false)]
        );
    }

    #[test]
    fn no_space_accepts_own_line_bracket() {
        assert!(run("a = [\n  1, 2\n    ]\n", Style::NoSpace, false).is_empty());
        assert!(run("a = [\n  1, 2, nil\n\t].compact\n", Style::NoSpace, false).is_empty());
    }

    #[test]
    fn percent_arrays_are_skipped() {
        assert!(run("a = %w[ a b ]\n", Style::NoSpace, false).is_empty());
        assert!(run("a = %i[ a b ]\n", Style::Space, false).is_empty());
    }

    #[test]
    fn reference_brackets_are_skipped() {
        assert!(run("b[ 3]\nc[ foo ]\n", Style::NoSpace, false).is_empty());
    }

    #[test]
    fn space_style_flags_missing() {
        assert_eq!(
            run("a = [2, 3 ]\n", Style::Space, false),
            vec![(4, 5, 0, 0, false)]
        );
        assert_eq!(
            run("a = [ 2, 3]\n", Style::Space, false),
            vec![(10, 11, 0, 0, true)]
        );
    }

    #[test]
    fn empty_brackets() {
        assert_eq!(
            run("a = [ ]\n", Style::NoSpace, false),
            vec![(4, 7, 3, 0, false)]
        );
        assert_eq!(
            run("a = [\n]\n", Style::NoSpace, false),
            vec![(4, 7, 3, 0, false)]
        );
        assert!(run("a = []\n", Style::NoSpace, false).is_empty());
        assert_eq!(
            run("a = []\n", Style::NoSpace, true),
            vec![(4, 6, 2, 0, false)]
        );
        assert_eq!(
            run("a = [  ]\n", Style::NoSpace, true),
            vec![(4, 8, 2, 0, false)]
        );
        assert!(run("a = [ ]\n", Style::NoSpace, true).is_empty());
        // Comment between: not empty brackets, and no non-empty offense fires.
        assert!(run("a = [ # c\n]\n", Style::NoSpace, true).is_empty());
    }

    #[test]
    fn compact_nested_arrays() {
        // "[ [1] ]": left compact removal offense + inner Use pair + right
        // compact removal offense.
        assert_eq!(
            run("d = [ [1] ]\n", Style::Compact, false),
            vec![
                (5, 6, 1, 0, false),
                (9, 10, 1, 0, false),
                (6, 7, 0, 1, false),
                (8, 9, 0, 1, false)
            ]
        );
        assert!(run("d = [[ 1 ]]\n", Style::Compact, false).is_empty());
    }

    #[test]
    fn compact_zero_width_left_offense_on_newline() {
        // "[\n  [1], [2]]": qualifies via `\n` after `[`, offense range empty.
        let offs = run("f = [\n  [1], [2]]\n", Style::Compact, false);
        assert_eq!(offs[0], (5, 5, 1, 0, false));
        // compact correction removes the newline run.
        assert_eq!(
            ops("f = [\n  [1], [2]]\n", Style::Compact, false)[0][0],
            (0, 5, 8)
        );
    }

    #[test]
    fn compact_percent_and_index_closers() {
        // %w closer is not a tRBRACK: a space is required before `]`.
        assert_eq!(
            run("a = [x, %w[a]]\n", Style::Compact, false),
            vec![(4, 5, 0, 0, false), (13, 14, 0, 0, false)]
        );
        // an index call closer is a tRBRACK: multi-dimensional, no offense.
        assert_eq!(
            run("c = [x, h[0]]\n", Style::Compact, false),
            vec![(4, 5, 0, 0, false)]
        );
    }

    #[test]
    fn array_pattern_checked() {
        assert_eq!(
            run("case v\nin [ a, b ]\n  1\nend\n", Style::NoSpace, false),
            vec![(11, 12, 1, 0, false), (16, 17, 1, 0, true)]
        );
    }

    #[test]
    fn const_pattern_bracket_form_redirect_swallows_inner() {
        // `ADT[[a, b ]]`: the bare inner pattern redirects to the const pair,
        // duplicating the (offense-free) outer check — nothing fires.
        assert!(run("case v\nin ADT[[a, b ]]\n  1\nend\n", Style::NoSpace, false).is_empty());
        // Without the constant the inner pattern is checked normally.
        assert_eq!(
            run("case v\nin [[a, b ]]\n  1\nend\n", Style::NoSpace, false),
            vec![(16, 17, 1, 0, true)]
        );
    }

    #[test]
    fn const_pattern_paren_form_hunts_min_max_pair() {
        // `ADT([g ], [h ])`: the pair is `[` of [g ] / `]` of [h ] — only the
        // space before the LAST `]` offends.
        assert_eq!(
            run(
                "case v\nin ADT([g ], [h ])\n  1\nend\n",
                Style::NoSpace,
                false
            ),
            vec![(22, 23, 1, 0, true)]
        );
    }

    #[test]
    fn const_pattern_bracket_form_checks_own_pair() {
        assert_eq!(
            run(
                "case v\nin ADT[ i, [j ]]\n  1\nend\n",
                Style::NoSpace,
                false
            ),
            vec![(14, 15, 1, 0, false)]
        );
    }
}
