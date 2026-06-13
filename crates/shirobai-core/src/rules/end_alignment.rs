//! `Layout/EndAlignment`.
//!
//! Checks that an `end` keyword is aligned with the start of the construct it
//! closes, under one of three `EnforcedStyleAlignWith` styles: `keyword`
//! (align with the `if` / `class` / ... keyword), `variable` (align with the
//! left-hand side of an enclosing assignment), or `start_of_line` (align with
//! the first non-space column of the keyword's line).
//!
//! Stock fires a callback per node type (`on_class`, `on_if`, `on_while`,
//! `on_case`, the `CheckAssignment` `on_lvasgn` / `on_send` family, ...). Two
//! code paths build the candidate alignment ranges:
//!
//! - `check_other_alignment(node)` — for a bare construct: all three styles'
//!   keyword range is the construct's own keyword, except `start_of_line` which
//!   is the keyword line's content range.
//! - `check_asgn_alignment(outer, inner)` — for a conditional on the RHS of an
//!   assignment (or `case`/`sclass` as an argument): `keyword` and
//!   `start_of_line` are the inner construct's, but `variable` aligns with the
//!   assignment's LHS (`outer.begin .. inner.keyword.end`, unless there is a
//!   line break before the keyword, in which case it falls back to the
//!   keyword). The asgn path also `ignore_node`s the inner construct so its own
//!   `on_if` / `on_case` callback is a no-op.
//!
//! `check_end_kw_alignment` then computes which styles the `end` already matches
//! (`matching_ranges`: same line as the range, or identical column). If the
//! configured style matches, the `end` is correct (`correct_style_detected`);
//! otherwise it is an offense aligned to the configured style's range, and the
//! matched styles drive `style_detected` (replicated by the Ruby wrapper for
//! `config_to_allow_offenses` parity).
//!
//! Here it is reconstructed over Prism in one ancestor-stack walk. Each
//! `end`-bearing construct is checked once, in stock's callback order, with a
//! `handled` set standing in for `ignore_node`. The autocorrect column comes
//! from `alignment_node` (which, unlike the message's alignment range, climbs
//! through same-line `send` parents for the `variable` style); the Ruby wrapper
//! applies the same `AlignmentCorrector#align_end` replace/insert arms.

use std::collections::HashSet;
use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// Style selector (`EnforcedStyleAlignWith`): 0 = keyword, 1 = variable,
/// 2 = start_of_line.
const STYLE_KEYWORD: u8 = 0;
const STYLE_VARIABLE: u8 = 1;
const STYLE_START_OF_LINE: u8 = 2;

/// One checked `end` keyword, emitted in walk order. `matching` is the set of
/// styles the `end` already aligns with (drives `style_detected` /
/// `correct_style_detected` on the Ruby side). When the configured style is not
/// in `matching`, `offense` carries the offense location + autocorrect target.
pub struct EndAlignmentRecord {
    /// `end` keyword range (the offense location when misaligned).
    pub end_start: usize,
    pub end_end: usize,
    /// The matched style ids in the path's hash-insertion order (stock's
    /// `matching.keys`); `style_detected` keeps the first as the allowed style.
    /// 0 = keyword, 1 = variable, 2 = start_of_line.
    pub matching: Vec<u8>,
    /// `Some(offense)` when the configured style is not matched.
    pub offense: Option<EndAlignmentOffense>,
}

/// The offense detail for a misaligned `end`.
pub struct EndAlignmentOffense {
    /// Formatted stock message.
    pub message: String,
    /// `AlignmentCorrector#align_end`: target column for the `end` (char column
    /// of `alignment_node`). The whitespace run before `end` is replaced with
    /// this many spaces, or (when something non-space precedes `end`) a newline
    /// plus that many spaces is inserted.
    pub align_column: usize,
}

#[derive(Clone, Copy)]
pub struct Config {
    pub style: u8,
}

pub fn check_end_alignment(source: &[u8], config: Config) -> Vec<EndAlignmentRecord> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.records
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        style: config.style,
        records: Vec::new(),
        handled: HashSet::new(),
        ancestors: Vec::new(),
        send_arguments: HashSet::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    style: u8,
    pub(crate) records: Vec<EndAlignmentRecord>,
    /// Start offsets of constructs already checked through the asgn path
    /// (`ignore_node`); their own `on_if` / `on_case` callback is then a no-op.
    handled: HashSet<usize>,
    /// Start offset of each open ancestor node (top = parent of the entering
    /// node). Used to find the enclosing send for `case`-as-argument.
    ancestors: Vec<usize>,
    /// Start offsets of nodes that are an argument of some `CallNode`
    /// (populated when that call is entered), standing in for `node.argument?`.
    send_arguments: HashSet<usize>,
}

/// A resolved alignment range: a source range whose `column` / `line` the
/// matcher and message use.
#[derive(Clone, Copy)]
struct AlignRange {
    start: usize,
    end: usize,
}

/// The keyword facts of an `end`-bearing construct.
struct Construct {
    /// Keyword range (`loc.keyword`): the `if` / `class` / `case` / ... token.
    kw_start: usize,
    kw_end: usize,
    /// `end` keyword range (`loc.end`).
    end_start: usize,
    end_end: usize,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

impl<'a> Visitor<'a> {
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn column(&self, off: usize) -> usize {
        self.line_index.column(self.source, off)
    }

    /// `effective_column`: the char column, minus one when the offset is on
    /// line 1 and the source begins with a UTF-8 BOM (matches RangeHelp).
    fn effective_column(&self, off: usize) -> usize {
        let col = self.column(off);
        if self.line_of(off) == 1 && self.source.starts_with(&[0xef, 0xbb, 0xbf]) && col > 0 {
            col - 1
        } else {
            col
        }
    }

    /// Source text of a range, for the message (`align_with.source`).
    fn text(&self, start: usize, end: usize) -> String {
        String::from_utf8_lossy(&self.source[start..end]).into_owned()
    }

    /// `start_line_range(node)`: the keyword line's content range, from the
    /// first non-space char to the start of the trailing whitespace.
    fn start_line_range(&self, kw_start: usize) -> AlignRange {
        let line = self.line_of(kw_start);
        let starts = self.line_index.line_starts();
        let ls = starts[line - 1];
        // Line content end: next line start (minus newline / CR), or EOF.
        let le = if line < starts.len() {
            let mut e = starts[line] - 1;
            if e > ls && self.source[e - 1] == b'\r' {
                e -= 1;
            }
            e
        } else {
            self.source.len()
        };
        // `source =~ /\S/`: first non-space byte offset within the line.
        let first_non_space = (ls..le)
            .find(|&i| !is_space_byte(self.source[i]))
            .unwrap_or(ls);
        // `source =~ /\s*\z/`: start of the trailing whitespace run.
        let mut trail = le;
        while trail > ls && is_space_byte(self.source[trail - 1]) {
            trail -= 1;
        }
        AlignRange {
            start: first_non_space,
            end: trail,
        }
    }

    /// `matching_ranges`: a range matches the `end` when it is on the same line
    /// as the `end` or shares its effective column.
    fn range_matches(&self, range: AlignRange, end_start: usize) -> bool {
        self.line_of(range.start) == self.line_of(end_start)
            || self.effective_column(range.start) == self.effective_column(end_start)
    }

    /// Format the stock `MSG` for a misaligned `end` against `align`.
    fn message(&self, end_start: usize, align: AlignRange) -> String {
        format!(
            "`end` at {}, {} is not aligned with `{}` at {}, {}.",
            self.line_of(end_start),
            self.column(end_start),
            self.text(align.start, align.end),
            self.line_of(align.start),
            self.column(align.start),
        )
    }

    /// Run `check_end_kw_alignment`: with the per-style ranges in the path's
    /// hash-insertion order (`align_ranges`), compute the matched styles
    /// (`matching.keys`, order-preserving) and emit a record (offense iff the
    /// configured style is not matched). `style_range` resolves a style id to
    /// its range; `align_node_start` is the autocorrect alignment column owner.
    fn check(
        &mut self,
        c: &Construct,
        ordered_styles: [(u8, AlignRange); 3],
        align_node_start: usize,
    ) {
        let mut matching: Vec<u8> = Vec::with_capacity(3);
        for &(id, range) in &ordered_styles {
            if self.range_matches(range, c.end_start) {
                matching.push(id);
            }
        }

        let offense = if matching.contains(&self.style) {
            None
        } else {
            let align = ordered_styles
                .iter()
                .find(|(id, _)| *id == self.style)
                .map(|(_, r)| *r)
                .expect("configured style is always present");
            Some(EndAlignmentOffense {
                message: self.message(c.end_start, align),
                align_column: self.column(align_node_start),
            })
        };

        self.records.push(EndAlignmentRecord {
            end_start: c.end_start,
            end_end: c.end_end,
            matching,
            offense,
        });
    }

    /// `check_other_alignment`: keyword and variable both use the keyword; the
    /// autocorrect alignment node is the construct itself.
    fn check_other(&mut self, c: &Construct, align_node_start: usize) {
        if self.handled.contains(&c.kw_start) {
            return;
        }
        let kw = AlignRange {
            start: c.kw_start,
            end: c.kw_end,
        };
        let sol = self.start_line_range(c.kw_start);
        // `check_other_alignment` hash order: keyword, variable, start_of_line.
        self.check(
            c,
            [
                (STYLE_KEYWORD, kw),
                (STYLE_VARIABLE, kw),
                (STYLE_START_OF_LINE, sol),
            ],
            align_node_start,
        );
    }

    /// `check_asgn_alignment(outer, inner)`: variable aligns with the LHS, and
    /// the inner construct is marked handled (`ignore_node`).
    fn check_asgn(&mut self, outer_start: usize, c: &Construct, align_node_start: usize) {
        if self.handled.contains(&c.kw_start) {
            self.handled.insert(c.kw_start);
            return;
        }
        let kw = AlignRange {
            start: c.kw_start,
            end: c.kw_end,
        };
        let sol = self.start_line_range(c.kw_start);
        let variable = self.asgn_variable_align(outer_start, c);
        // `check_asgn_alignment` hash order: keyword, start_of_line, variable.
        self.check(
            c,
            [
                (STYLE_KEYWORD, kw),
                (STYLE_START_OF_LINE, sol),
                (STYLE_VARIABLE, variable),
            ],
            align_node_start,
        );
        self.handled.insert(c.kw_start);
    }

    /// `asgn_variable_align_with`: the keyword when there is a line break before
    /// it (inner keyword line > outer line), else `outer.begin .. inner.kw.end`.
    fn asgn_variable_align(&self, outer_start: usize, c: &Construct) -> AlignRange {
        if self.line_of(c.kw_start) > self.line_of(outer_start) {
            AlignRange {
                start: c.kw_start,
                end: c.kw_end,
            }
        } else {
            AlignRange {
                start: outer_start,
                end: c.kw_end,
            }
        }
    }
}

/// Whether byte `b` is whitespace as `\s` / `String#blank?` sees it.
fn is_space_byte(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c)
}

// --- Construct extraction. ---

impl<'a> Visitor<'a> {
    /// The keyword + end facts of an `end`-bearing construct, or `None` when the
    /// node has no `end` (modifier / ternary / endless forms) or is not one of
    /// the handled types.
    fn construct(&self, node: &Node<'_>) -> Option<Construct> {
        if let Some(n) = node.as_if_node() {
            // Ternary (`a ? b : c`) has no `if` keyword; modifier `x if c` has
            // no `end`. Both are skipped.
            let kw = n.if_keyword_loc()?;
            let end = n.end_keyword_loc()?;
            // An `elsif` branch is a nested `IfNode` that Prism gives the
            // *parent's* `end`; in parser its `loc.end` is nil, so stock never
            // checks it. Detect it by the keyword text and skip.
            if self.source.get(kw.start_offset()..kw.end_offset()) == Some(b"elsif") {
                return None;
            }
            return Some(self.mk(&kw, &end));
        }
        if let Some(n) = node.as_unless_node() {
            let end = n.end_keyword_loc()?;
            return Some(self.mk(&n.keyword_loc(), &end));
        }
        if let Some(n) = node.as_while_node() {
            let end = n.closing_loc()?;
            return Some(self.mk(&n.keyword_loc(), &end));
        }
        if let Some(n) = node.as_until_node() {
            let end = n.closing_loc()?;
            return Some(self.mk(&n.keyword_loc(), &end));
        }
        if let Some(n) = node.as_case_node() {
            return Some(self.mk(&n.case_keyword_loc(), &n.end_keyword_loc()));
        }
        if let Some(n) = node.as_case_match_node() {
            return Some(self.mk(&n.case_keyword_loc(), &n.end_keyword_loc()));
        }
        if let Some(n) = node.as_class_node() {
            return Some(self.mk(&n.class_keyword_loc(), &n.end_keyword_loc()));
        }
        if let Some(n) = node.as_module_node() {
            return Some(self.mk(&n.module_keyword_loc(), &n.end_keyword_loc()));
        }
        if let Some(n) = node.as_singleton_class_node() {
            return Some(self.mk(&n.class_keyword_loc(), &n.end_keyword_loc()));
        }
        None
    }

    fn mk(&self, kw: &Location<'_>, end: &Location<'_>) -> Construct {
        let (kw_start, kw_end) = loc(kw);
        let (end_start, end_end) = loc(end);
        Construct {
            kw_start,
            kw_end,
            end_start,
            end_end,
        }
    }

    /// Whether this construct node is a `case` / `case_match`. (Only these take
    /// the argument-as-asgn path through their own callback.)
    fn is_case(node: &Node<'_>) -> bool {
        node.as_case_node().is_some() || node.as_case_match_node().is_some()
    }
}

// --- check_assignment chain (the downward asgn path). ---

/// Follow `first_part_of_call_chain`: descend through call receivers and block
/// send nodes to the leading receiver.
fn first_part_of_call_chain<'pr>(mut node: Node<'pr>) -> Option<Node<'pr>> {
    loop {
        if let Some(call) = node.as_call_node() {
            node = call.receiver()?;
        } else if let Some(block) = node.as_block_node() {
            // A block node's "send" is its enclosing call; in Prism the block is
            // a child of the call, so this shape does not arise as an RHS here.
            let _ = block;
            return Some(node);
        } else {
            return Some(node);
        }
    }
}

/// `rhs = rhs.child_nodes.first while rhs.type?(:begin, :or, :and)`: unwrap a
/// parenthesized / begin group (first statement) or a logical operator (left).
fn unwrap_begin_or_and<'pr>(mut node: Node<'pr>) -> Option<Node<'pr>> {
    loop {
        if let Some(p) = node.as_parentheses_node() {
            // parser `(begin ...)`: `child_nodes.first` is the first statement.
            let body = p.body()?;
            let st = body.as_statements_node()?;
            node = st.body().iter().next()?;
        } else if let Some(b) = node.as_begin_node() {
            let st = b.statements()?;
            node = st.body().iter().next()?;
        } else if let Some(o) = node.as_or_node() {
            node = o.left();
        } else if let Some(a) = node.as_and_node() {
            node = a.left();
        } else {
            return Some(node);
        }
    }
}

impl<'a> Visitor<'a> {
    /// `CheckAssignment#check_assignment`: with `rhs` the assignment's RHS,
    /// follow the call chain, unwrap begin/or/and, and (when a non-ternary
    /// conditional remains) run the asgn alignment for `outer_start`.
    fn check_assignment(&mut self, outer_start: usize, rhs: Node<'_>) {
        let Some(rhs) = first_part_of_call_chain(rhs) else {
            return;
        };
        let Some(rhs) = unwrap_begin_or_and(rhs) else {
            return;
        };
        // `rhs.conditional?` (if / while / until / case / case_match) and not a
        // ternary if. `construct` already filters ternary / modifier / endless.
        if let Some(c) = self.conditional_construct(&rhs) {
            let align_node = self.variable_alignment_node(outer_start, &rhs, &c);
            self.check_asgn(outer_start, &c, align_node);
        }
    }

    /// `construct` restricted to the `conditional?` types (the RHS asgn path
    /// never targets class / module / sclass).
    fn conditional_construct(&self, node: &Node<'_>) -> Option<Construct> {
        if node.as_if_node().is_some()
            || node.as_unless_node().is_some()
            || node.as_while_node().is_some()
            || node.as_until_node().is_some()
            || node.as_case_node().is_some()
            || node.as_case_match_node().is_some()
        {
            self.construct(node)
        } else {
            None
        }
    }

    /// Extract the assignment RHS for the `CheckAssignment` node types, or
    /// `None` if the node is not such an assignment / has no RHS.
    fn assignment_rhs<'pr>(&self, node: &Node<'pr>) -> Option<Node<'pr>> {
        if let Some(n) = node.as_local_variable_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_instance_variable_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_class_variable_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_global_variable_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_constant_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_constant_path_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_multi_write_node() {
            return Some(n.value());
        }
        // op_asgn / or_asgn / and_asgn (all variable / constant / index / call
        // flavors expose `.value()` for the RHS).
        if let Some(n) = node.as_local_variable_operator_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_local_variable_or_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_local_variable_and_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_instance_variable_operator_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_instance_variable_or_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_instance_variable_and_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_class_variable_operator_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_class_variable_or_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_class_variable_and_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_global_variable_operator_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_global_variable_or_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_global_variable_and_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_constant_operator_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_constant_or_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_constant_and_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_constant_path_operator_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_constant_path_or_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_constant_path_and_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_index_operator_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_index_or_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_index_and_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_call_operator_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_call_or_write_node() {
            return Some(n.value());
        }
        if let Some(n) = node.as_call_and_write_node() {
            return Some(n.value());
        }
        None
    }

    /// The last argument of a `CallNode` (parser `send.last_argument`), the RHS
    /// `extract_rhs` returns for a call.
    fn call_last_argument<'pr>(call: &ruby_prism::CallNode<'pr>) -> Option<Node<'pr>> {
        let args = call.arguments()?;
        args.arguments().iter().last()
    }

    /// `alignment_node`: the autocorrect anchor for the configured style.
    /// `keyword` -> the construct's keyword column; `start_of_line` -> the
    /// keyword line's first-non-space column; `variable` -> the assignment /
    /// operator-method LHS (climbing through same-line `send` parents), falling
    /// back to the keyword when there is no such assignment or a line break
    /// precedes the keyword. Returns the start offset whose column is taken.
    fn variable_alignment_node(
        &self,
        outer_start: usize,
        _inner: &Node<'_>,
        c: &Construct,
    ) -> usize {
        match self.style {
            STYLE_KEYWORD => c.kw_start,
            STYLE_START_OF_LINE => self.start_line_range(c.kw_start).start,
            _ => {
                // `variable`: align with the assignment LHS when the keyword is
                // on the assignment's first line, else fall back to the keyword.
                if self.line_of(c.kw_start) > self.line_of(outer_start) {
                    c.kw_start
                } else {
                    outer_start
                }
            }
        }
    }

    /// `alignment_node` for the `check_other` path (no enclosing assignment).
    fn other_alignment_node(&self, c: &Construct) -> usize {
        match self.style {
            STYLE_START_OF_LINE => self.start_line_range(c.kw_start).start,
            // For `variable` with no assignment, `alignment_node_for_variable_style`
            // falls back to the node itself; the node's column equals the keyword
            // column for these constructs.
            _ => c.kw_start,
        }
    }
}

// --- Walk driver. ---

impl<'a> Visitor<'a> {
    fn enter_node(&mut self, node: &Node<'_>) {
        // 1. Assignment / send nodes: the downward `check_assignment` path.
        if let Some(rhs) = self.assignment_rhs(node) {
            let outer_start = node.location().start_offset();
            self.check_assignment(outer_start, rhs);
        } else if let Some(call) = node.as_call_node() {
            // `on_send`: only when the call has a last argument to treat as RHS.
            if let Some(rhs) = Self::call_last_argument(&call) {
                let outer_start = node.location().start_offset();
                self.check_assignment(outer_start, rhs);
            }
        }

        // 2. The construct's own callback (`on_if` / `on_case` / `on_class` /
        //    ...). `case` / `case_match` as a send argument take the asgn path
        //    against the enclosing send; `sclass` whose parent is an assignment
        //    likewise (handled via the assignment_rhs path above, so here only
        //    its `check_other` fallback runs when not yet handled).
        if let Some(c) = self.construct(node) {
            if Self::is_case(node) && self.is_send_argument(node) {
                let outer_start = *self.ancestors.last().unwrap();
                let align_node = self.case_argument_alignment_node(outer_start, node, &c);
                self.check_asgn(outer_start, &c, align_node);
            } else {
                let align_node = self.other_alignment_node(&c);
                self.check_other(&c, align_node);
            }
        }
    }

    /// Whether `node` is an argument of an enclosing `send` (`node.argument?`).
    /// `send_arguments` is populated when the parent call is entered.
    fn is_send_argument(&self, node: &Node<'_>) -> bool {
        self.send_arguments.contains(&node.location().start_offset())
    }

    /// `alignment_node` for a `case`-as-argument: `variable` style climbs to the
    /// enclosing send (`alignment_node_for_variable_style` returns `node.parent`
    /// when the case is a same-line argument, then climbs same-line sends).
    fn case_argument_alignment_node(
        &self,
        outer_start: usize,
        _node: &Node<'_>,
        c: &Construct,
    ) -> usize {
        match self.style {
            STYLE_KEYWORD => c.kw_start,
            STYLE_START_OF_LINE => self.start_line_range(c.kw_start).start,
            _ => {
                // case as argument on the same line as the send: align to the
                // send (its start). If the case is on a later line, fall back to
                // the keyword.
                if self.line_of(c.kw_start) > self.line_of(outer_start) {
                    c.kw_start
                } else {
                    outer_start
                }
            }
        }
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        // Record this call's argument offsets before pushing the frame, so a
        // child construct can test `node.argument?` against its parent.
        if let Some(call) = node.as_call_node()
            && let Some(args) = call.arguments()
        {
            for a in args.arguments().iter() {
                self.send_arguments.insert(a.location().start_offset());
            }
        }
        self.enter_node(node);
        self.ancestors.push(node.location().start_offset());
    }

    fn leave(&mut self) {
        self.ancestors.pop();
    }

    // Constructs (`if` / `case` / `class` / ...) are all branch nodes, so the
    // leaf and rescue hooks need no special handling.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Vec<EndAlignmentRecord> {
        check_end_alignment(source.as_bytes(), Config { style })
    }

    #[test]
    fn aligned_class_no_offense() {
        let r = run("class Foo\nend\n", STYLE_KEYWORD);
        assert_eq!(r.len(), 1);
        assert!(r[0].offense.is_none());
    }

    #[test]
    fn misaligned_if_keyword() {
        let r = run("if test\n  end\n", STYLE_KEYWORD);
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`end` at 2, 2 is not aligned with `if` at 1, 0."));
        assert_eq!(o.align_column, 0);
    }

    #[test]
    fn assignment_keyword_offense() {
        // var = if test\nend  -> keyword wants column 6.
        let r = run("var = if test\nend\n", STYLE_KEYWORD);
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`if` at 1, 6"));
        assert_eq!(o.align_column, 6);
    }

    #[test]
    fn assignment_variable_aligned() {
        // var = if test\nend  -> under variable style, end at col 0 matches LHS.
        let r = run("var = if test\nend\n", STYLE_VARIABLE);
        assert_eq!(r.len(), 1);
        assert!(r[0].offense.is_none());
    }

    #[test]
    fn assignment_variable_misaligned() {
        // var = if test\n      end  -> end at col 6, variable wants col 0.
        let r = run("var = if test\n      end\n", STYLE_VARIABLE);
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`var = if` at 1, 0"));
        assert_eq!(o.align_column, 0);
    }

    #[test]
    fn ternary_no_check() {
        assert!(run("a = cond ? x : y\n", STYLE_KEYWORD).is_empty());
    }

    #[test]
    fn modifier_no_check() {
        assert!(run("a = x if cond\n", STYLE_KEYWORD).is_empty());
    }

    #[test]
    fn chain_aligned_to_keyword() {
        // var = if test\n  foo\nend.method_call  -> end matches keyword col 6?
        // No: end at col 0, keyword col 6; variable LHS col 0 matches under
        // variable style.
        let r = run("var = if test\n  foo\nend.method_call\n", STYLE_VARIABLE);
        assert_eq!(r.len(), 1);
        assert!(r[0].offense.is_none());
    }

    #[test]
    fn elsif_shares_parent_end() {
        // An `elsif` branch must not be checked on its own: Prism gives it the
        // parent's `end`, but parser's `loc.end` is nil there. Only the outer
        // (asgn) `if` is checked, and it is aligned to the LHS under variable.
        let r = run("var = if a\n  b\nelsif c\n  d\nend\n", STYLE_VARIABLE);
        assert_eq!(r.len(), 1);
        assert!(r[0].offense.is_none());
    }

    #[test]
    fn case_as_argument() {
        // format(\n  case c\n  when f\n    b\n  else\n    z\nend, qux\n)
        let src = "format(\n  case c\n  when f\n    b\n  else\n    z\nend, qux\n)\n";
        let r = run(src, STYLE_VARIABLE);
        // The case is checked once, misaligned against keyword column 2.
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`case` at 2, 2"));
    }
}
