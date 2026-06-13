//! `Layout/ElseAlignment`.
//!
//! Checks that an `else` / `elsif` keyword (the `else` of a `when` / `in`
//! `case`, and the `else` of a `begin` / `def` / block `rescue` chain) is
//! aligned with the construct it belongs to. Stock fires a callback per node
//! type:
//!
//! - `on_if(node, base)` — for `if` / `unless`: when the node has an `else`
//!   (`loc.else`, the `elsif` *or* `else` keyword of the immediate node) that
//!   begins its line, check it against `base_range_of_if`. When the node is an
//!   `elsif` chain (`elsif_conditional?`) recurse into the `elsif` branch
//!   carrying the same base. `base_range_of_if` with no base climbs the `if`
//!   lineage to the enclosing real `if` / `unless` keyword; with a base (set by
//!   `check_assignment`) it is the base node's source range.
//! - `on_case` / `on_case_match` — the `else` aligns with the last `when` / `in`
//!   keyword.
//! - `on_rescue` — the `else` of a `begin` / `def` / block `rescue` chain aligns
//!   with the enclosing `begin` keyword / `def` keyword (or the leading
//!   `private`-style selector) / the block's send (or the assignment LHS when it
//!   is on the block's first line).
//! - `CheckAssignment` — when an `if` / `unless` is on the RHS of an assignment,
//!   the recursion's base becomes the assignment node (`variable`
//!   `EnforcedStyleAlignWith`) or the `if` itself (`keyword`).
//!
//! Each checked keyword is an offense when its `column_offset_between` the base
//! (effective column difference) is non-zero. The autocorrect mirrors
//! `AlignmentCorrector.correct` for a single-line range: shift the keyword's
//! line by that signed column delta (insert spaces at the line start for a
//! positive delta, remove leading whitespace for a negative one).
//!
//! Reconstructed over Prism in one ancestor-frame walk. The `rescue` `else` is
//! handled at the enclosing `BeginNode` (Prism keeps the `else_clause` on the
//! begin, not the rescue).

use std::collections::HashSet;
use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// `Layout/EndAlignment`'s `EnforcedStyleAlignWith`: 0 = keyword, 1 = variable,
/// 2 = start_of_line. Only `keyword` vs not-`keyword` matters here.
const STYLE_KEYWORD: u8 = 0;

#[derive(Clone, Copy)]
pub struct Config {
    /// `Layout/EndAlignment`'s `EnforcedStyleAlignWith`.
    pub style: u8,
}

/// One misaligned keyword (the offense). `else_start..else_end` is the keyword
/// range (the offense location); `column_delta` is the signed shift the
/// autocorrect applies to the keyword's line (`base_col - else_col`).
pub struct ElseAlignmentOffense {
    pub else_start: usize,
    pub else_end: usize,
    pub message: String,
    pub column_delta: i64,
}

pub fn check_else_alignment(source: &[u8], config: Config) -> Vec<ElseAlignmentOffense> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        style: config.style,
        offenses: Vec::new(),
        handled: HashSet::new(),
        frames: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    style: u8,
    pub(crate) offenses: Vec<ElseAlignmentOffense>,
    /// Start offsets of `if` / `unless` nodes already checked through the
    /// recursion / assignment path (`ignore_node`).
    handled: HashSet<usize>,
    /// Ancestor frames (top = parent of the entering node).
    frames: Vec<Frame>,
}

/// One ancestor frame, with the typed facts later levels need.
#[derive(Clone)]
struct Frame {
    kind: FrameKind,
    start: usize,
    end: usize,
    first_line: usize,
}

#[derive(Clone)]
enum FrameKind {
    /// A real `if` / `unless` (keyword is not `elsif`): keyword range.
    RealIf { kw_start: usize, kw_end: usize },
    /// An `elsif` `IfNode`.
    Elsif,
    /// A `def` / `defs`: `def` keyword range.
    Def { kw_start: usize, kw_end: usize },
    /// A `CallNode` (parser `send`): full range, and its selector range when it
    /// has a message (for `private def` and block sends).
    Call {
        sel_start: usize,
        sel_end: usize,
        has_message: bool,
    },
    /// A block node (`BlockNode`).
    Block,
    /// A write node (assignment LHS): the LHS span (start..value_start trimmed),
    /// here approximated by the whole write-node range start and its first line.
    Write,
    /// Anything else.
    Other,
}

/// A resolved alignment base: a source range whose effective column and leading
/// non-space token drive the offense.
#[derive(Clone, Copy)]
struct AlignRange {
    start: usize,
    end: usize,
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

    /// `effective_column`: char column, minus one for a line-1 offset when the
    /// source begins with a UTF-8 BOM (matches RangeHelp).
    fn effective_column(&self, off: usize) -> usize {
        let col = self.column(off);
        if self.line_of(off) == 1 && self.source.starts_with(&[0xef, 0xbb, 0xbf]) && col > 0 {
            col - 1
        } else {
            col
        }
    }

    /// `begins_its_line?`: only whitespace precedes `off` on its line.
    fn begins_its_line(&self, off: usize) -> bool {
        let ls = self.line_index.line_start(off);
        self.source[ls..off].iter().all(|&b| is_space_byte(b))
    }

    /// `base_range.source[/^\S*/]`: the leading run of non-space characters of
    /// the base range's source (the label shown in the message).
    fn base_label(&self, base: AlignRange) -> String {
        let mut e = base.start;
        while e < base.end && !is_space_byte_or_nl(self.source[e]) {
            e += 1;
        }
        String::from_utf8_lossy(&self.source[base.start..e]).into_owned()
    }

    fn text(&self, start: usize, end: usize) -> String {
        String::from_utf8_lossy(&self.source[start..end]).into_owned()
    }

    /// `check_alignment(base_range, else_range)`: register an offense when the
    /// keyword begins its line and its effective column differs from the base's.
    fn check_alignment(&mut self, base: AlignRange, kw_start: usize, kw_end: usize) {
        if !self.begins_its_line(kw_start) {
            return;
        }
        let delta =
            self.effective_column(base.start) as i64 - self.effective_column(kw_start) as i64;
        if delta == 0 {
            return;
        }
        let message = format!(
            "Align `{}` with `{}`.",
            self.text(kw_start, kw_end),
            self.base_label(base),
        );
        self.offenses.push(ElseAlignmentOffense {
            else_start: kw_start,
            else_end: kw_end,
            message,
            column_delta: delta,
        });
    }
}

fn is_space_byte(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c)
}

fn is_space_byte_or_nl(b: u8) -> bool {
    b == b'\n' || is_space_byte(b)
}

/// The `else` keyword location of an `if` node: its `subsequent`'s keyword.
fn if_else_loc<'pr>(n: &ruby_prism::IfNode<'pr>) -> Option<Location<'pr>> {
    let sub = n.subsequent()?;
    if let Some(elsif) = sub.as_if_node() {
        elsif.if_keyword_loc()
    } else {
        sub.as_else_node().map(|e| e.else_keyword_loc())
    }
}

// --- `if` / `unless` handling. ---

impl<'a> Visitor<'a> {
    /// `on_if(node, nil)` from the generic walk (the node is a real `if` /
    /// `unless` here, since `elsif`s and assignment RHS are `handled`).
    fn on_if_generic(&mut self, node: &Node<'_>) {
        if self.handled.contains(&node.location().start_offset()) {
            return;
        }
        if let Some(n) = node.as_if_node() {
            if n.if_keyword_loc().is_none() {
                return; // ternary
            }
            self.on_if_node(&n, None);
        } else if let Some(n) = node.as_unless_node() {
            let base = self.unless_keyword_base(&n);
            self.on_unless_node(&n, base);
        }
    }

    fn on_if_node(&mut self, n: &ruby_prism::IfNode<'_>, base: Option<AlignRange>) {
        if let Some(else_loc) = if_else_loc(n) {
            let (else_start, else_end) = loc(&else_loc);
            if self.begins_its_line(else_start) {
                let base = base.unwrap_or_else(|| self.base_range_of_if_lineage(n));
                self.check_alignment(base, else_start, else_end);

                if let Some(sub) = n.subsequent()
                    && let Some(elsif) = sub.as_if_node()
                {
                    self.handled
                        .insert(elsif.as_node().location().start_offset());
                    self.on_if_node(&elsif, Some(base));
                }
            }
        }
    }

    fn unless_keyword_base(&self, n: &ruby_prism::UnlessNode<'_>) -> AlignRange {
        let (s, e) = loc(&n.keyword_loc());
        AlignRange { start: s, end: e }
    }

    fn on_unless_node(&mut self, n: &ruby_prism::UnlessNode<'_>, base: AlignRange) {
        let Some(els) = n.else_clause() else {
            return;
        };
        let (es, ee) = loc(&els.else_keyword_loc());
        if !self.begins_its_line(es) {
            return;
        }
        self.check_alignment(base, es, ee);
    }

    /// `base_range_of_if(node, nil)`: the node's own keyword when it is a real
    /// `if` / `unless`, else climb the ancestor `if` stack to the first real one.
    fn base_range_of_if_lineage(&self, n: &ruby_prism::IfNode<'_>) -> AlignRange {
        if let Some(kw) = n.if_keyword_loc()
            && &self.source[kw.start_offset()..kw.end_offset()] != b"elsif"
        {
            let (s, e) = loc(&kw);
            return AlignRange { start: s, end: e };
        }
        for f in self.frames.iter().rev() {
            if let FrameKind::RealIf { kw_start, kw_end } = f.kind {
                return AlignRange {
                    start: kw_start,
                    end: kw_end,
                };
            }
        }
        let kw = n.if_keyword_loc().expect("if node has a keyword");
        let (s, e) = loc(&kw);
        AlignRange { start: s, end: e }
    }
}

// --- `case` / `case_match`. ---

impl<'a> Visitor<'a> {
    fn on_case(&mut self, n: &ruby_prism::CaseNode<'_>) {
        let Some(els) = n.else_clause() else {
            return;
        };
        let Some(last) = n.conditions().iter().last() else {
            return;
        };
        let Some(when_node) = last.as_when_node() else {
            return;
        };
        let (ws, we) = loc(&when_node.keyword_loc());
        let (es, ee) = loc(&els.else_keyword_loc());
        self.check_alignment(AlignRange { start: ws, end: we }, es, ee);
    }

    fn on_case_match(&mut self, n: &ruby_prism::CaseMatchNode<'_>) {
        let Some(els) = n.else_clause() else {
            return;
        };
        let Some(last) = n.conditions().iter().last() else {
            return;
        };
        let Some(in_node) = last.as_in_node() else {
            return;
        };
        let (is, ie) = loc(&in_node.in_loc());
        let (es, ee) = loc(&els.else_keyword_loc());
        self.check_alignment(AlignRange { start: is, end: ie }, es, ee);
    }
}

// --- `begin` / `def` / block `rescue` `else`. ---

impl<'a> Visitor<'a> {
    /// `on_rescue`, reached at the enclosing `BeginNode`.
    fn on_begin_rescue_else(&mut self, begin: &ruby_prism::BeginNode<'_>) {
        if begin.rescue_clause().is_none() {
            return;
        }
        let Some(els) = begin.else_clause() else {
            return;
        };
        let (es, ee) = loc(&els.else_keyword_loc());
        if !self.begins_its_line(es) {
            return;
        }
        let base = self.base_range_of_rescue(begin);
        self.check_alignment(base, es, ee);
    }

    fn base_range_of_rescue(&self, begin: &ruby_prism::BeginNode<'_>) -> AlignRange {
        // Explicit `begin` (kwbegin): align with the `begin` keyword.
        if let Some(begin_kw) = begin.begin_keyword_loc() {
            let (s, e) = loc(&begin_kw);
            return AlignRange { start: s, end: e };
        }
        // Implicit begin: parent is `frames.last()`.
        let nf = self.frames.len();
        if nf >= 1 {
            let parent = &self.frames[nf - 1];
            match &parent.kind {
                FrameKind::Def { kw_start, kw_end } => {
                    // `private def`: the def's parent is a `Call` with a message;
                    // align with that selector. (grandparent = frames[nf-2].)
                    if nf >= 2
                        && let FrameKind::Call {
                            sel_start,
                            sel_end,
                            has_message: true,
                        } = self.frames[nf - 2].kind
                    {
                        return AlignRange {
                            start: sel_start,
                            end: sel_end,
                        };
                    }
                    return AlignRange {
                        start: *kw_start,
                        end: *kw_end,
                    };
                }
                // The block's owning call is the grandparent; an enclosing
                // assignment (great-grandparent) on the same line wins.
                FrameKind::Block if nf >= 2 => {
                    let call = &self.frames[nf - 2];
                    if nf >= 3 {
                        let gg = &self.frames[nf - 3];
                        if matches!(gg.kind, FrameKind::Write) && gg.first_line == call.first_line {
                            return AlignRange {
                                start: gg.start,
                                end: gg.end,
                            };
                        }
                    }
                    return AlignRange {
                        start: call.start,
                        end: call.end,
                    };
                }
                _ => {}
            }
        }
        // Fallback (`else node.loc.keyword`): the rescue keyword.
        if let Some(rescue) = begin.rescue_clause() {
            let (s, e) = loc(&rescue.keyword_loc());
            return AlignRange { start: s, end: e };
        }
        let (s, e) = loc(&begin.location());
        AlignRange { start: s, end: e }
    }
}

// --- check_assignment chain. ---

/// `first_part_of_call_chain`: descend through call receivers to the leading
/// receiver (a block node's send is its enclosing call, which in Prism is the
/// parent, so it never appears standalone as an RHS here).
fn first_part_of_call_chain<'pr>(mut node: Node<'pr>) -> Option<Node<'pr>> {
    loop {
        match node.as_call_node().and_then(|c| c.receiver()) {
            Some(r) => node = r,
            None => return Some(node),
        }
    }
}

impl<'a> Visitor<'a> {
    /// `CheckAssignment#check_assignment`: when the RHS (after the call chain) is
    /// an `if` / `unless`, run `on_if` carrying the base.
    fn check_assignment(&mut self, asgn_start: usize, asgn_end: usize, rhs: Node<'_>) {
        let Some(rhs) = first_part_of_call_chain(rhs) else {
            return;
        };
        if let Some(if_node) = rhs.as_if_node() {
            if if_node.if_keyword_loc().is_none() {
                return; // ternary
            }
            let base = self.assignment_base(asgn_start, asgn_end, &rhs);
            self.handled.insert(rhs.location().start_offset());
            self.on_if_node(&if_node, Some(base));
        } else if let Some(unless_node) = rhs.as_unless_node() {
            let base = self.assignment_base(asgn_start, asgn_end, &rhs);
            self.handled.insert(rhs.location().start_offset());
            self.on_unless_node(&unless_node, base);
        }
    }

    /// `variable_alignment?(node.loc, rhs, style) ? node : rhs`.
    fn assignment_base(&self, asgn_start: usize, asgn_end: usize, rhs: &Node<'_>) -> AlignRange {
        let rhs_start = rhs.location().start_offset();
        let variable =
            self.style != STYLE_KEYWORD && self.line_of(rhs_start) <= self.line_of(asgn_start);
        if variable {
            AlignRange {
                start: asgn_start,
                end: asgn_end,
            }
        } else {
            let kw = self.if_keyword_of(rhs);
            AlignRange {
                start: kw.0,
                end: kw.1,
            }
        }
    }

    fn if_keyword_of(&self, rhs: &Node<'_>) -> (usize, usize) {
        if let Some(n) = rhs.as_if_node()
            && let Some(kw) = n.if_keyword_loc()
        {
            return loc(&kw);
        }
        if let Some(n) = rhs.as_unless_node() {
            return loc(&n.keyword_loc());
        }
        let s = rhs.location().start_offset();
        (s, s)
    }
}

/// The last argument of a `CallNode` (parser `send.last_argument`).
fn call_last_argument<'pr>(call: &ruby_prism::CallNode<'pr>) -> Option<Node<'pr>> {
    call.arguments()?.arguments().iter().last()
}

/// `extract_rhs` for the `CheckAssignment` write-node family: the assignment's
/// RHS value, or `None` when the node is not such a write.
fn extract_rhs<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
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

// --- Walk driver. ---

impl<'a> Visitor<'a> {
    fn enter_node(&mut self, node: &Node<'_>) {
        // 1. Assignment / send nodes: the `check_assignment` path.
        if let Some(rhs) = extract_rhs(node) {
            let (s, e) = (node.location().start_offset(), node.location().end_offset());
            self.check_assignment(s, e, rhs);
        } else if let Some(call) = node.as_call_node()
            && let Some(rhs) = call_last_argument(&call)
        {
            let (s, e) = (node.location().start_offset(), node.location().end_offset());
            self.check_assignment(s, e, rhs);
        }

        // 2. The construct's own callback.
        if node.as_if_node().is_some() || node.as_unless_node().is_some() {
            self.on_if_generic(node);
        } else if let Some(c) = node.as_case_node() {
            self.on_case(&c);
        } else if let Some(c) = node.as_case_match_node() {
            self.on_case_match(&c);
        } else if let Some(b) = node.as_begin_node() {
            self.on_begin_rescue_else(&b);
        }
    }

    /// The ancestor frame for `node`.
    fn frame_for(&self, node: &Node<'_>) -> Frame {
        let start = node.location().start_offset();
        let end = node.location().end_offset();
        let first_line = self.line_of(start);
        let kind = if let Some(n) = node.as_if_node() {
            match n.if_keyword_loc() {
                Some(kw) if &self.source[kw.start_offset()..kw.end_offset()] != b"elsif" => {
                    let (s, e) = loc(&kw);
                    FrameKind::RealIf {
                        kw_start: s,
                        kw_end: e,
                    }
                }
                Some(_) => FrameKind::Elsif,
                None => FrameKind::Other, // ternary
            }
        } else if let Some(n) = node.as_unless_node() {
            let (s, e) = loc(&n.keyword_loc());
            FrameKind::RealIf {
                kw_start: s,
                kw_end: e,
            }
        } else if let Some(n) = node.as_def_node() {
            let (s, e) = loc(&n.def_keyword_loc());
            FrameKind::Def {
                kw_start: s,
                kw_end: e,
            }
        } else if let Some(call) = node.as_call_node() {
            match call.message_loc() {
                Some(sel) => {
                    let (s, e) = loc(&sel);
                    FrameKind::Call {
                        sel_start: s,
                        sel_end: e,
                        has_message: true,
                    }
                }
                None => FrameKind::Call {
                    sel_start: start,
                    sel_end: start,
                    has_message: false,
                },
            }
        } else if node.as_block_node().is_some() {
            FrameKind::Block
        } else if is_write_node(node) {
            FrameKind::Write
        } else {
            FrameKind::Other
        };
        Frame {
            kind,
            start,
            end,
            first_line,
        }
    }
}

/// Whether `node` is an assignment write node (LHS = some target, value = RHS).
fn is_write_node(node: &Node<'_>) -> bool {
    node.as_local_variable_write_node().is_some()
        || node.as_instance_variable_write_node().is_some()
        || node.as_class_variable_write_node().is_some()
        || node.as_global_variable_write_node().is_some()
        || node.as_constant_write_node().is_some()
        || node.as_constant_path_write_node().is_some()
        || node.as_multi_write_node().is_some()
        || node.as_local_variable_operator_write_node().is_some()
        || node.as_local_variable_or_write_node().is_some()
        || node.as_local_variable_and_write_node().is_some()
        || node.as_instance_variable_operator_write_node().is_some()
        || node.as_instance_variable_or_write_node().is_some()
        || node.as_instance_variable_and_write_node().is_some()
        || node.as_class_variable_operator_write_node().is_some()
        || node.as_class_variable_or_write_node().is_some()
        || node.as_class_variable_and_write_node().is_some()
        || node.as_global_variable_operator_write_node().is_some()
        || node.as_global_variable_or_write_node().is_some()
        || node.as_global_variable_and_write_node().is_some()
        || node.as_constant_operator_write_node().is_some()
        || node.as_constant_or_write_node().is_some()
        || node.as_constant_and_write_node().is_some()
        || node.as_constant_path_operator_write_node().is_some()
        || node.as_constant_path_or_write_node().is_some()
        || node.as_constant_path_and_write_node().is_some()
        || node.as_index_operator_write_node().is_some()
        || node.as_index_or_write_node().is_some()
        || node.as_index_and_write_node().is_some()
        || node.as_call_operator_write_node().is_some()
        || node.as_call_or_write_node().is_some()
        || node.as_call_and_write_node().is_some()
}

impl<'a> super::dispatch::Rule for Visitor<'a> {
    fn enter(&mut self, node: &Node<'_>) {
        self.enter_node(node);
        let frame = self.frame_for(node);
        self.frames.push(frame);
    }

    fn leave(&mut self) {
        self.frames.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Vec<ElseAlignmentOffense> {
        check_else_alignment(source.as_bytes(), Config { style })
    }

    fn variable(source: &str) -> Vec<ElseAlignmentOffense> {
        run(source, 1)
    }

    fn keyword(source: &str) -> Vec<ElseAlignmentOffense> {
        run(source, STYLE_KEYWORD)
    }

    #[test]
    fn if_misaligned_else() {
        let r = variable("if cond\n  func1\n else\n func2\nend\n");
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("Align `else` with `if`."));
        assert_eq!(r[0].column_delta, -1);
    }

    #[test]
    fn elsif_misaligned() {
        let r = variable("    if a1\n      b1\n  elsif a2\n      b2\n    end\n");
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("Align `elsif` with `if`."));
        assert_eq!(r[0].column_delta, 2);
    }

    #[test]
    fn deeply_nested_elsif_chain() {
        let r = variable("def m\n  if a\n    b\n   elsif c\n    d\n   else\n    g\n  end\nend\n");
        assert_eq!(r.len(), 2);
        assert!(r[0].message.contains("`elsif`"));
        assert!(r[1].message.contains("`else`"));
        assert_eq!(r[0].column_delta, -1);
        assert_eq!(r[1].column_delta, -1);
    }

    #[test]
    fn case_else_aligns_with_when() {
        let r = variable("case a\nwhen b\n  c\nwhen d\n  e\n else\n  f\nend\n");
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("Align `else` with `when`."));
    }

    #[test]
    fn case_match_in_else() {
        let r = variable("case 0\nin 0\n  foo\nin Integer\n  baz\n else\n  qux\nend\n");
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("Align `else` with `in`."));
    }

    #[test]
    fn def_rescue_else_ensure() {
        let r = variable(
            "def my_func(string)\n  puts string\nrescue => e\n  puts e\n  else\n  puts e\nensure\n  puts 'x'\nend\n",
        );
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("Align `else` with `def`."));
    }

    #[test]
    fn explicit_begin_rescue_else() {
        let r =
            variable("def m\n  begin\n    x\n  rescue\n    y\nelse\n    z\n  ensure\n    w\n  end\nend\n");
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("Align `else` with `begin`."));
    }

    #[test]
    fn private_def_else() {
        let r = variable(
            "private def test\n          something\n        rescue\n          handling\n        else\n          something_else\n        end\n",
        );
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("Align `else` with `private`."));
    }

    #[test]
    fn block_arg_rescue_else_send() {
        let r = variable("array_like.each do |n|\n  x\nrescue\n  y\n  else\n  z\nend\n");
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("Align `else` with `array_like.each`."));
    }

    #[test]
    fn block_assign_sameline_aligns_with_lhs() {
        let r = variable("result = array_like.each do |n|\n  x\nrescue\n  y\n  else\n  z\nend\n");
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("Align `else` with `result`."));
    }

    #[test]
    fn assignment_keyword_style_aligns_with_if() {
        let r = keyword("var = if a\n        0\nelse\n  1\n    end\n");
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("Align `else` with `if`."));
    }

    #[test]
    fn oneline_if_no_offense() {
        assert!(variable("if cond then func1 else func2 end\n").is_empty());
    }

    #[test]
    fn ternary_no_offense() {
        assert!(variable("cond ? func1 : func2\n").is_empty());
    }

    #[test]
    fn rescue_inside_if_branches_no_offense() {
        assert!(variable("if a\n  a rescue nil\nelse\n  a rescue nil\nend\n").is_empty());
    }
}
