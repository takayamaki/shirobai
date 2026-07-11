//! `Layout/BlockAlignment`.
//!
//! Checks that a block's closing token (`end` for `do`..`end`, `}` for a brace
//! block) is aligned correctly, under one of three `EnforcedStyleAlignWith`
//! styles: `either` (default — the `end` may align with the *start of the line*
//! the whole expression begins on, or with the *start of the line the `do`
//! appears on*; autocorrect prefers `start_of_line`), `start_of_block` (align
//! with the `do`/`{` line indentation), or `start_of_line` (align with the
//! line the expression starts on).
//!
//! Stock fires `on_block` (and `on_numblock` / `on_itblock`) per block. The
//! alignment target node is chosen by walking the block's lineage
//! (`[node, *node.ancestors]`) with `block_end_align_target`: the first node
//! whose parent is *not* one of `{assignment? any_def splat and or (send _ :<<)
//! (send equal?(child) !:[])}` (or whose parent starts on a different line and
//! is not a mass-assignment) becomes the target. `find_lhs_node` then unwraps
//! `op_asgn` / `masgn` to their LHS for the message and column.
//!
//! Two anchors drive the offense: `start_loc` (the target's source range — the
//! message's preferred alignment) and `do_source_line_column` (the indentation
//! of the `do`/`{` line — the alternative, shown as "or ..." only under
//! `either` when the two differ). An offense fires when the `end` begins its
//! line and its column differs from `start_loc.column` (always, under
//! `start_of_block`), and (unless `start_of_line`) also differs from the `do`
//! line indentation.
//!
//! Autocorrect targets `compute_start_col`: under `start_of_block` the `do`
//! line indentation, otherwise the column of `start_for_line_node` (the target,
//! climbed to the topmost ancestor sharing the target's first line, then
//! `find_lhs_node`).
//!
//! Reconstructed over Prism in one ancestor-stack walk: a parser block wrapper
//! is a Prism call/super with a literal `BlockNode`, or a `LambdaNode`. Its
//! lineage is the Prism ancestor chain; the `(send equal?(child) ...)` matcher
//! is reproduced by comparing the ancestor send's receiver range to the child.

use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// Style selector (`EnforcedStyleAlignWith`): 0 = either, 1 = start_of_block,
/// 2 = start_of_line.
const STYLE_EITHER: u8 = 0;
const STYLE_START_OF_BLOCK: u8 = 1;
const STYLE_START_OF_LINE: u8 = 2;

/// One checked block whose closing token is misaligned (an offense).
pub struct BlockAlignmentOffense {
    /// Closing token range (`end` or `}`) — the offense location.
    pub end_start: usize,
    pub end_end: usize,
    /// Formatted stock message.
    pub message: String,
    /// `compute_start_col`: target column for the closing token. Positive delta
    /// (target > current) inserts spaces before the token; negative removes
    /// them.
    pub align_column: usize,
}

#[derive(Clone, Copy)]
pub struct Config {
    pub style: u8,
}

pub fn check_block_alignment(source: &[u8], config: Config) -> Vec<BlockAlignmentOffense> {
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
        frames: Vec::new(),
        transparent: Vec::new(),
    }
}

/// The parser-equivalent kind of one ancestor frame, carrying exactly what
/// `block_end_align_target` / `find_lhs_node` / `start_for_line_node` need.
#[derive(Clone)]
enum FrameKind {
    /// A parser `send`/`csend` (Prism `CallNode`). `receiver` is the receiver's
    /// range (for the `(send equal?(child) !:[])` matcher); `index` marks the
    /// `[]` method; `lshift` marks `<<`; `setter` marks an assignment-method
    /// send (`name=` / `[]=`), which `SendNode#assignment?` reports as an
    /// `assignment?` and the matcher therefore accepts unconditionally.
    Send {
        receiver: Option<(usize, usize)>,
        index: bool,
        lshift: bool,
        setter: bool,
    },
    /// A parser `splat`.
    Splat,
    /// A parser `and` / `or` (logical operator).
    AndOr,
    /// A parser `def` / `defs` (any_def).
    Def,
    /// A parser assignment whose LHS `find_lhs_node` never unwraps: `lvasgn`
    /// family writes, `or_asgn` / `and_asgn`, and setter sends.
    Assign,
    /// A parser `op_asgn` (the `+=` / `-=` family): `find_lhs_node` unwraps to
    /// its LHS range.
    OpAsgn { lhs: (usize, usize) },
    /// A parser `masgn`: `find_lhs_node` unwraps to its `mlhs` range, and
    /// `disqualified_parent?` treats it specially.
    Masgn { lhs: (usize, usize) },
    /// Anything else (cannot host a relevant role, but still an ancestor).
    Other,
}

/// One ancestor frame: its parser kind, full range, and first line.
#[derive(Clone)]
struct Frame {
    kind: FrameKind,
    start: usize,
    end: usize,
    first_line: usize,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    style: u8,
    pub(crate) offenses: Vec<BlockAlignmentOffense>,
    /// Ancestor frames (top = parent of the entering node).
    frames: Vec<Frame>,
    /// Per-entered-node flag: `true` when the node was made transparent (no
    /// frame pushed), so `leave` knows whether to pop a frame.
    transparent: Vec<bool>,
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

    /// First line of a node range (`node.first_line`).
    fn first_line_of(&self, start: usize) -> usize {
        self.line_of(start)
    }

    /// `begins_its_line?`: only whitespace precedes `off` on its line.
    fn begins_its_line(&self, off: usize) -> bool {
        let ls = self.line_index.line_start(off);
        self.source[ls..off].iter().all(|&b| is_space_byte(b))
    }

    /// The first line (chomped) of a range, for the message (`loc.source`'s
    /// first line / `start_loc.source.lines.first.chomp`).
    fn first_line_text(&self, start: usize, end: usize) -> String {
        let slice = &self.source[start..end];
        let line_end = slice.iter().position(|&b| b == b'\n').unwrap_or(slice.len());
        let mut e = line_end;
        if e > 0 && slice[e - 1] == b'\r' {
            e -= 1;
        }
        String::from_utf8_lossy(&slice[..e]).into_owned()
    }

    /// `match = /\S.*/.match(anchor_loc.source_line)`: returns `(source, line,
    /// column)` from the anchor line's first non-space char. The anchor is
    /// normally the `do`/`{` line, but shifts to the method dispatch line
    /// when the `do` line is a continuation of multiline arguments.
    fn do_source_line(&self, do_start: usize, anchor_start: Option<usize>) -> (String, usize, usize) {
        let ref_offset = anchor_start.unwrap_or(do_start);
        let line = self.line_of(ref_offset);
        let ls = self.line_index.line_start(ref_offset);
        let starts = self.line_index.line_starts();
        let le = if line < starts.len() {
            let mut e = starts[line] - 1;
            if e > ls && self.source[e - 1] == b'\r' {
                e -= 1;
            }
            e
        } else {
            self.source.len()
        };
        let first_non_space = (ls..le)
            .find(|&i| !is_space_byte(self.source[i]))
            .unwrap_or(ls);
        let column = self.column(first_non_space);
        let text = String::from_utf8_lossy(&self.source[first_non_space..le]).into_owned();
        (text, line, column)
    }

    /// The indentation of the original `do`/`{` line (not the anchor).
    fn do_line_indentation(&self, do_start: usize) -> usize {
        let ls = self.line_index.line_start(do_start);
        let starts = self.line_index.line_starts();
        let line = self.line_of(do_start);
        let le = if line < starts.len() {
            starts[line] - 1
        } else {
            self.source.len()
        };
        let first_non_space = (ls..le)
            .find(|&i| !is_space_byte(self.source[i]))
            .unwrap_or(ls);
        self.column(first_non_space)
    }
}

fn is_space_byte(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c)
}

/// Whether a method name is an assignment setter (`SendNode#assignment?`): it
/// ends with `=` but is not a comparison/operator-with-equals
/// (`==`, `!=`, `<=`, `>=`, `===`), or it is the index setter `[]=`.
fn is_setter_method(name: &[u8]) -> bool {
    if name == b"[]=" {
        return true;
    }
    if !name.ends_with(b"=") {
        return false;
    }
    !matches!(name, b"==" | b"!=" | b"<=" | b">=" | b"===")
}

// --- Block detection. ---

/// The closing-token location (`loc.end`) and opening (`loc.begin`) of a parser
/// block wrapper, or `None` when `node` is not one.
struct BlockLoc {
    open_start: usize,
    close_start: usize,
    close_end: usize,
    /// When the `do`/`{` line starts inside a multiline method argument,
    /// anchor on the method dispatch line instead of the do line.
    anchor_start: Option<usize>,
}

impl<'a> Visitor<'a> {
    fn block_loc(&self, node: &Node<'_>) -> Option<BlockLoc> {
        // call / super / forwarding-super with a literal BlockNode.
        if let Some(call) = node.as_call_node() {
            if let Some(bn) = call.block().and_then(|b| b.as_block_node()) {
                let mut arg_ranges: Vec<(usize, usize)> = Vec::new();
                if let Some(args) = call.arguments() {
                    for arg in args.arguments().iter() {
                        arg_ranges.push(loc(&arg.location()));
                    }
                }
                let selector = call
                    .message_loc()
                    .map(|l| l.start_offset())
                    .unwrap_or(call.location().start_offset());
                return self.block_node_loc_with_anchor(node.location().start_offset(), &bn, &arg_ranges, selector);
            }
            return None;
        }
        if let Some(sup) = node.as_super_node() {
            if let Some(bn) = sup.block().and_then(|b| b.as_block_node()) {
                let mut arg_ranges: Vec<(usize, usize)> = Vec::new();
                if let Some(args) = sup.arguments() {
                    for arg in args.arguments().iter() {
                        arg_ranges.push(loc(&arg.location()));
                    }
                }
                let selector = sup.keyword_loc().start_offset();
                return self.block_node_loc_with_anchor(node.location().start_offset(), &bn, &arg_ranges, selector);
            }
            return None;
        }
        if let Some(fsup) = node.as_forwarding_super_node() {
            if let Some(bn) = fsup.block() {
                let selector = node.location().start_offset();
                return self.block_node_loc_with_anchor(node.location().start_offset(), &bn, &[], selector);
            }
            return None;
        }
        // Stabby lambda `-> { }` / `-> do end` is a parser block.
        if let Some(lam) = node.as_lambda_node() {
            let (open_start, _) = loc(&lam.opening_loc());
            let (close_start, close_end) = loc(&lam.closing_loc());
            // Lambda parameters are the "arguments" for the anchor check.
            let mut arg_ranges: Vec<(usize, usize)> = Vec::new();
            if let Some(params) = lam.parameters() {
                let ploc = params.location();
                arg_ranges.push(loc(&ploc));
            }
            let selector = lam.operator_loc().start_offset();
            let anchor_start =
                self.do_line_anchor(node.location().start_offset(), open_start, &arg_ranges, &[], selector);
            return Some(BlockLoc {
                open_start,
                close_start,
                close_end,
                anchor_start,
            });
        }
        None
    }

    fn block_node_loc_with_anchor(
        &self,
        node_start: usize,
        bn: &ruby_prism::BlockNode<'_>,
        send_arg_ranges: &[(usize, usize)],
        selector_start: usize,
    ) -> Option<BlockLoc> {
        let (open_start, _) = loc(&bn.opening_loc());
        let (close_start, close_end) = loc(&bn.closing_loc());
        let mut block_param_ranges: Vec<(usize, usize)> = Vec::new();
        if let Some(params) = bn.parameters() {
            let ploc = params.location();
            block_param_ranges.push(loc(&ploc));
        }
        let anchor_start = self.do_line_anchor(node_start, open_start, send_arg_ranges, &block_param_ranges, selector_start);
        Some(BlockLoc {
            open_start,
            close_start,
            close_end,
            anchor_start,
        })
    }

    /// When the `do`/`{` line begins inside one of the call's arguments or the
    /// block's parameters, return the method selector start offset as the
    /// alignment anchor. rubocop#15312 additionally requires the do-line's first
    /// char to sit inside `(` / `[`: a bare argument list without parentheses
    /// puts the continuation indentation under the author's control, so the
    /// anchor must not move to the dispatch line.
    fn do_line_anchor(
        &self,
        node_start: usize,
        open_start: usize,
        send_arg_ranges: &[(usize, usize)],
        block_param_ranges: &[(usize, usize)],
        selector_start: usize,
    ) -> Option<usize> {
        let ls = self.line_index.line_start(open_start);
        let first_char_pos = (ls..open_start)
            .find(|&i| !is_space_byte(self.source[i]))
            .unwrap_or(open_start);

        let inside = send_arg_ranges
            .iter()
            .chain(block_param_ranges.iter())
            .any(|&(s, e)| s <= first_char_pos && first_char_pos < e);

        if inside && self.inside_parentheses(node_start, first_char_pos) {
            Some(selector_start)
        } else {
            None
        }
    }

    /// `inside_parentheses?(node, pos)`: whether the `(` / `[` tokens opened
    /// between `node_start` and `pos` outnumber the `)` / `]` closed — i.e. `pos`
    /// sits inside an unclosed round/square bracket. Block braces `{` / `}` are
    /// not counted (stock checks `left_parens?` / `left_bracket?` only).
    fn inside_parentheses(&self, node_start: usize, pos: usize) -> bool {
        let mut depth: i32 = 0;
        for &b in &self.source[node_start..pos] {
            match b {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                _ => {}
            }
        }
        depth > 0
    }
}

// --- Lineage / alignment-target selection. ---

/// A resolved alignment target: the source range whose column/line/text the
/// message and autocorrect use.
#[derive(Clone, Copy)]
struct Target {
    start: usize,
    end: usize,
    first_line: usize,
}

impl<'a> Visitor<'a> {
    /// `block_end_align_target(node)` + `find_lhs_node`: walk the lineage
    /// `[block, *ancestors]` and return the first node that is its own
    /// alignment target, then unwrap op_asgn/masgn LHS. `block_range` is the
    /// block wrapper's full range; `block_first_line` its first line.
    ///
    /// The lineage pairs are `(current, parent)`; `block` is the first
    /// `current`, and the frames (top-down) supply the parents. The matcher
    /// `block_end_align_target?(parent, current)` decides whether `current`
    /// (the child) should keep climbing.
    fn start_for_block_node(&self, block: (usize, usize), block_first_line: usize) -> Target {
        // Build the lineage as (range, first_line, frame-kind-of-this-node).
        // The block itself has no FrameKind role as a *parent*, so we represent
        // it with `Other`; only its range/first_line matter as a `current`.
        let mut lineage: Vec<(usize, usize, usize, FrameKind)> =
            Vec::with_capacity(self.frames.len() + 1);
        lineage.push((block.0, block.1, block_first_line, FrameKind::Other));
        for f in self.frames.iter().rev() {
            lineage.push((f.start, f.end, f.first_line, f.kind.clone()));
        }

        let target = self.first_target(&lineage);
        self.find_lhs(target)
    }

    /// Mirror of `each_cons(2) { return current if end_align_target? }; last`.
    fn first_target(
        &self,
        lineage: &[(usize, usize, usize, FrameKind)],
    ) -> (usize, usize, usize, FrameKind) {
        for pair in lineage.windows(2) {
            if self.end_align_target(&pair[0], &pair[1]) {
                return pair[0].clone();
            }
        }
        lineage.last().unwrap().clone()
    }

    /// `end_align_target?(node, parent)` =
    /// `disqualified_parent?(parent, node) || !block_end_align_target?(parent, node)`.
    fn end_align_target(
        &self,
        current: &(usize, usize, usize, FrameKind),
        parent: &(usize, usize, usize, FrameKind),
    ) -> bool {
        self.disqualified_parent(parent, current) || !self.matcher(parent, current)
    }

    /// `disqualified_parent?(parent, node)`: parent has a location, starts on a
    /// different first line than `node`, and is not a mass-assignment.
    fn disqualified_parent(
        &self,
        parent: &(usize, usize, usize, FrameKind),
        current: &(usize, usize, usize, FrameKind),
    ) -> bool {
        let is_masgn = matches!(parent.3, FrameKind::Masgn { .. });
        // `parent&.loc` — every real node here has a location; `Other`/`Program`
        // roots do too. There is no synthetic parent.
        parent.2 != current.2 && !is_masgn
    }

    /// `block_end_align_target?(parent, node)` matcher:
    /// `{assignment? any_def splat and or (send _ :<<) (send equal?(node) !:[])}`.
    fn matcher(
        &self,
        parent: &(usize, usize, usize, FrameKind),
        current: &(usize, usize, usize, FrameKind),
    ) -> bool {
        match &parent.3 {
            FrameKind::Assign | FrameKind::OpAsgn { .. } | FrameKind::Masgn { .. } => true,
            FrameKind::Def => true,
            FrameKind::Splat => true,
            FrameKind::AndOr => true,
            FrameKind::Send {
                receiver,
                index,
                lshift,
                setter,
            } => {
                // A setter send (`name=` / `[]=`) is `assignment?`.
                if *setter || *lshift {
                    return true;
                }
                // `(send equal?(node) !:[] ...)`: receiver is the child node and
                // the method is not `[]`.
                if *index {
                    return false;
                }
                receiver.map(|r| r.0 == current.0 && r.1 == current.1) == Some(true)
            }
            FrameKind::Other => false,
        }
    }

    /// `find_lhs_node`: `node = node.lhs while node.type?(:op_asgn, :masgn)`.
    fn find_lhs(&self, node: (usize, usize, usize, FrameKind)) -> Target {
        match node.3 {
            FrameKind::OpAsgn { lhs } | FrameKind::Masgn { lhs } => {
                // The LHS replaces the node; the LHS is a leaf for our purposes
                // (no further op_asgn/masgn nesting arises in this lineage).
                Target {
                    start: lhs.0,
                    end: lhs.1,
                    first_line: self.first_line_of(lhs.0),
                }
            }
            _ => Target {
                start: node.0,
                end: node.1,
                first_line: node.2,
            },
        }
    }

    /// `start_for_line_node`: from `start_for_block_node`, climb to the topmost
    /// ancestor that still shares the start node's first line, then
    /// `find_lhs_node`. We approximate the climb using the frame stack: among
    /// the frames that *contain* `start` and share its first line, pick the
    /// outermost (lowest in the stack).
    fn start_for_line_node(&self, start: Target) -> Target {
        let mut best: Option<&Frame> = None;
        for f in self.frames.iter() {
            // ancestor of the start node: contains it.
            if f.start <= start.start && f.end >= start.end && f.first_line == start.first_line {
                // outermost wins (frames are pushed parent-first, so the first
                // matching frame from the bottom is the outermost).
                if best.is_none() {
                    best = Some(f);
                }
            }
        }
        match best {
            Some(f) => self.find_lhs((f.start, f.end, f.first_line, f.kind.clone())),
            None => start,
        }
    }
}

// --- The check. ---

impl<'a> Visitor<'a> {
    fn on_block(&mut self, block_range: (usize, usize), bl: &BlockLoc) {
        let block_first_line = self.first_line_of(block_range.0);
        let start = self.start_for_block_node(block_range, block_first_line);
        let start = if self.style == STYLE_START_OF_LINE {
            self.start_for_line_node(start)
        } else {
            start
        };
        self.check_block_alignment(start, block_range, bl);
    }

    fn check_block_alignment(
        &mut self,
        start: Target,
        block_range: (usize, usize),
        bl: &BlockLoc,
    ) {
        let end_start = bl.close_start;
        if !self.begins_its_line(end_start) {
            return;
        }
        let start_col = self.column(start.start);
        let end_col = self.column(end_start);
        if start_col == end_col && self.style != STYLE_START_OF_BLOCK {
            return;
        }
        // compute_do_source_line_column (using anchor if available).
        let (do_text, do_line, do_col) = self.do_source_line(bl.open_start, bl.anchor_start);
        // permitted_do_line_columns: under `either`, both the anchor line's
        // indentation and the original do line's indentation are accepted.
        let mut permitted = vec![do_col];
        if self.style == STYLE_EITHER && bl.anchor_start.is_some() {
            permitted.push(self.do_line_indentation(bl.open_start));
        }
        if permitted.contains(&end_col) && self.style != STYLE_START_OF_LINE {
            return;
        }
        self.register_offense(start, block_range, bl, (do_text, do_line, do_col));
    }

    #[allow(clippy::too_many_arguments)]
    fn register_offense(
        &mut self,
        start: Target,
        block_range: (usize, usize),
        bl: &BlockLoc,
        do_slc: (String, usize, usize),
    ) {
        let end_start = bl.close_start;
        let end_end = bl.close_end;
        let end_line = self.line_of(end_start);
        let end_col = self.column(end_start);
        // `current` source: the closing token text (its single line).
        let current_text = self.first_line_text(end_start, end_end);

        // error_source_line_column.
        let (err_text, err_line, err_col) = if self.style == STYLE_START_OF_BLOCK {
            do_slc.clone()
        } else {
            (
                self.first_line_text(start.start, start.end),
                self.line_of(start.start),
                self.column(start.start),
            )
        };

        // alt_prefer: " or ..." only under `either` when start_loc differs from
        // do_source_line_column.
        let start_line = self.line_of(start.start);
        let start_col = self.column(start.start);
        let alt = if self.style != STYLE_EITHER
            || (start_line == do_slc.1 && start_col == do_slc.2)
        {
            String::new()
        } else {
            format!(" or `{}` at {}, {}", do_slc.0, do_slc.1, do_slc.2)
        };

        let message = format!(
            "`{}` at {}, {} is not aligned with `{}` at {}, {}{}.",
            current_text, end_line, end_col, err_text, err_line, err_col, alt,
        );

        // compute_start_col (autocorrect target column).
        let align_column = if self.style == STYLE_START_OF_BLOCK {
            do_slc.2
        } else {
            let line_node = self.start_for_line_node(start);
            self.column(line_node.start)
        };

        let _ = block_range;
        self.offenses.push(BlockAlignmentOffense {
            end_start,
            end_end,
            message,
            align_column,
        });
    }
}

// --- Frame construction. ---

impl<'a> Visitor<'a> {
    fn make_frame(&self, node: &Node<'_>) -> Frame {
        let (start, end) = loc(&node.location());
        let first_line = self.line_of(start);
        let kind = self.frame_kind(node);
        Frame {
            kind,
            start,
            end,
            first_line,
        }
    }

    fn frame_kind(&self, node: &Node<'_>) -> FrameKind {
        if let Some(call) = node.as_call_node() {
            let receiver = call.receiver().map(|r| loc(&r.location()));
            let name = call.name();
            let name = name.as_slice();
            return FrameKind::Send {
                receiver,
                index: name == b"[]",
                lshift: name == b"<<",
                setter: is_setter_method(name),
            };
        }
        if node.as_splat_node().is_some() {
            return FrameKind::Splat;
        }
        if node.as_and_node().is_some() || node.as_or_node().is_some() {
            return FrameKind::AndOr;
        }
        if node.as_def_node().is_some() {
            return FrameKind::Def;
        }
        // masgn.
        if let Some(m) = node.as_multi_write_node() {
            let lhs = self.masgn_lhs_range(&m);
            return FrameKind::Masgn { lhs };
        }
        // op_asgn / or_asgn / and_asgn (find_lhs unwraps these).
        if let Some(lhs) = self.op_asgn_lhs(node) {
            return FrameKind::OpAsgn { lhs };
        }
        // Plain assignment writes (lhs not unwrapped).
        if self.is_plain_assignment(node) {
            return FrameKind::Assign;
        }
        FrameKind::Other
    }

    /// The `mlhs` range of a `MultiWriteNode` (parser `masgn`'s LHS). When the
    /// targets are parenthesized (`(a, b) =`) the range is `lparen..rparen`;
    /// otherwise it spans from the node start to the last target component's end
    /// (excluding any trailing comma / the `=`), matching parser's `mlhs.source`
    /// (`processed,` -> `processed`, `a, b,` -> `a, b`).
    fn masgn_lhs_range(&self, m: &ruby_prism::MultiWriteNode<'_>) -> (usize, usize) {
        if let (Some(lp), Some(rp)) = (m.lparen_loc(), m.rparen_loc()) {
            return (lp.start_offset(), rp.end_offset());
        }
        let start = m.location().start_offset();
        let mut end = start;
        for left in m.lefts().iter() {
            end = end.max(left.location().end_offset());
        }
        if let Some(rest) = m.rest() {
            // An `ImplicitRestNode` is just the trailing comma (`processed,`):
            // parser excludes it from the `mlhs` range. An explicit splat
            // (`a, *b`) does extend the range.
            if rest.as_implicit_rest_node().is_none() {
                end = end.max(rest.location().end_offset());
            }
        }
        for right in m.rights().iter() {
            end = end.max(right.location().end_offset());
        }
        (start, end)
    }

    /// The LHS range for the parser `op_asgn` node types only (the `+=` / `-=`
    /// family — `find_lhs_node` unwraps these). `or_asgn` / `and_asgn`
    /// (`||=` / `&&=`) are *not* unwrapped by stock, so they are plain
    /// assignments. `None` for non-op-asgn nodes.
    fn op_asgn_lhs(&self, node: &Node<'_>) -> Option<(usize, usize)> {
        if let Some(n) = node.as_local_variable_operator_write_node() {
            return Some(loc(&n.name_loc()));
        }
        if let Some(n) = node.as_instance_variable_operator_write_node() {
            return Some(loc(&n.name_loc()));
        }
        if let Some(n) = node.as_class_variable_operator_write_node() {
            return Some(loc(&n.name_loc()));
        }
        if let Some(n) = node.as_global_variable_operator_write_node() {
            return Some(loc(&n.name_loc()));
        }
        if let Some(n) = node.as_constant_operator_write_node() {
            return Some(loc(&n.name_loc()));
        }
        // Constant-path / index / call op-asgn: the LHS is the target node.
        if let Some(n) = node.as_constant_path_operator_write_node() {
            return Some(loc(&n.target().as_node().location()));
        }
        if let Some(n) = node.as_index_operator_write_node() {
            return Some(self.index_target_range(&n.as_node()));
        }
        if let Some(n) = node.as_call_operator_write_node() {
            return Some(loc(&n.as_node().location()));
        }
        None
    }

    /// The parser `send :[]` target range of an `IndexOperatorWriteNode`
    /// (`a[i] += x` → `a[i]`): from the node start to the closing `]`.
    fn index_target_range(&self, node: &Node<'_>) -> (usize, usize) {
        let (start, _) = loc(&node.location());
        // The op-asgn node's own range starts at `a`. The LHS `a[i]` ends at
        // the `]`. Find the matching `]` after the opening `[`.
        let bytes = self.source;
        let mut depth = 0i32;
        let mut i = start;
        let mut end = start;
        while i < bytes.len() {
            match bytes[i] {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        (start, end)
    }

    /// Whether `node` is a parser plain assignment write or `or_asgn`/`and_asgn`
    /// (all `assignment?` but not unwrapped by `find_lhs_node`).
    fn is_plain_assignment(&self, node: &Node<'_>) -> bool {
        node.as_local_variable_write_node().is_some()
            || node.as_instance_variable_write_node().is_some()
            || node.as_class_variable_write_node().is_some()
            || node.as_global_variable_write_node().is_some()
            || node.as_constant_write_node().is_some()
            || node.as_constant_path_write_node().is_some()
            // or_asgn / and_asgn (`||=` / `&&=`): assignment? but not unwrapped.
            || node.as_local_variable_or_write_node().is_some()
            || node.as_local_variable_and_write_node().is_some()
            || node.as_instance_variable_or_write_node().is_some()
            || node.as_instance_variable_and_write_node().is_some()
            || node.as_class_variable_or_write_node().is_some()
            || node.as_class_variable_and_write_node().is_some()
            || node.as_global_variable_or_write_node().is_some()
            || node.as_global_variable_and_write_node().is_some()
            || node.as_constant_or_write_node().is_some()
            || node.as_constant_and_write_node().is_some()
            || node.as_constant_path_or_write_node().is_some()
            || node.as_constant_path_and_write_node().is_some()
            || node.as_index_or_write_node().is_some()
            || node.as_index_and_write_node().is_some()
            || node.as_call_or_write_node().is_some()
            || node.as_call_and_write_node().is_some()
    }
}

// --- Walk driver. ---

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(bl) = self.block_loc(node) {
            let block_range = loc(&node.location());
            self.on_block(block_range, &bl);
        }
        // A single-statement `StatementsNode` has no parser counterpart (parser
        // only emits a `begin` for multi-statement sequences), so it must be
        // transparent in the lineage — otherwise an endless `def foo = blk`
        // body would hide the `def` from the block's ancestor walk. A
        // multi-statement `StatementsNode` is a parser `begin` and stays as a
        // boundary `Other` frame.
        if let Some(stmts) = node.as_statements_node()
            && stmts.body().iter().count() <= 1
        {
            self.transparent.push(true);
            return;
        }
        self.transparent.push(false);
        let frame = self.make_frame(node);
        self.frames.push(frame);
    }

    fn leave(&mut self) {
        if !self.transparent.pop().unwrap_or(false) {
            self.frames.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Vec<BlockAlignmentOffense> {
        check_block_alignment(source.as_bytes(), Config { style })
    }

    #[test]
    fn plain_block_aligned() {
        assert!(run("test do\nend\n", STYLE_EITHER).is_empty());
    }

    #[test]
    fn plain_block_misaligned() {
        let r = run("test do\n  end\n", STYLE_EITHER);
        assert_eq!(r.len(), 1);
        assert!(r[0]
            .message
            .contains("`end` at 2, 2 is not aligned with `test do` at 1, 0."));
        assert_eq!(r[0].align_column, 0);
    }

    #[test]
    fn assignment_anchor() {
        let r = run("variable = test do |a|\n  end\n", STYLE_EITHER);
        assert_eq!(r.len(), 1);
        assert!(r[0]
            .message
            .contains("`variable = test do |a|` at 1, 0."));
    }

    #[test]
    fn brace_block_message() {
        let r = run("test {\n  }\n", STYLE_EITHER);
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("`}` at 2, 2 is not aligned with `test {` at 1, 0."));
    }

    #[test]
    fn op_asgn_unwraps_lhs() {
        let r = run("rb += files.select do |f|\n  x\n  end\n", STYLE_EITHER);
        assert_eq!(r.len(), 1);
        assert!(r[0].message.contains("`rb` at 1, 0."));
    }

    #[test]
    fn or_asgn_does_not_unwrap() {
        let r = run("variable ||= test do |a|\n  end\n", STYLE_EITHER);
        assert_eq!(r.len(), 1);
        assert!(r[0]
            .message
            .contains("`variable ||= test do |a|` at 1, 0."));
    }
}
