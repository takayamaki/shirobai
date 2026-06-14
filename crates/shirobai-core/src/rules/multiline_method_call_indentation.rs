//! `Layout/MultilineMethodCallIndentation`.
//!
//! Checks the indentation of the method-name part of `.`-chained method calls
//! that span more than one line. The heavy half of the shared
//! `MultilineExpressionIndentation` mixin (the operation half lives in
//! `multiline_operation_indentation`).
//!
//! Coverage: all three styles — `aligned` (semantic dot-above / first-call /
//! multiline-block-chain alignment + syntactic keyword/assignment/operation
//! base), `indented`, and `indented_relative_to_receiver` (receiver-relative
//! base, hash/grouped chained receiver, splat) — regular indentation plus
//! hash-pair alignment (`find_pair_ancestor`, `inside_multiline_chain_arg?`,
//! `first_dot_alignment_base` incl. `after_multiline_block_base`) and
//! block-aware autocorrect. The full vendor spec passes.

use std::rc::Rc;

use ruby_prism::{CallNode, Location, Node};

use super::line_index::LineIndex;

/// One misindented method-call selector. `column_delta` is
/// `correct_column - actual_column`. `block_*` ranges (0 = none) tell the Ruby
/// side to additionally realign a trailing multiline block.
pub struct MethodCallIndentOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub message: String,
    pub block_body_start: usize,
    pub block_body_end: usize,
    pub block_end_start: usize,
    pub block_end_end: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Aligned,
    Indented,
    IndentedRelativeToReceiver,
}

impl Style {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Style::Indented,
            2 => Style::IndentedRelativeToReceiver,
            _ => Style::Aligned,
        }
    }
}

pub fn check_multiline_method_call_indentation(
    source: &[u8],
    style: u8,
    indent_width: usize,
    base_indent_width: usize,
) -> Vec<MethodCallIndentOffense> {
    let mut rule = build_rule(source, style, indent_width, base_indent_width);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule(
    source: &[u8],
    style: u8,
    indent_width: usize,
    base_indent_width: usize,
) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        style: Style::from_u8(style),
        indent: indent_width,
        base: base_indent_width,
        stack: Vec::new(),
        offenses: Vec::new(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KwKind {
    If,
    Unless,
    While,
    Until,
    For,
    Return,
}

impl KwKind {
    fn keyword(self) -> &'static str {
        match self {
            KwKind::If => "if",
            KwKind::Unless => "unless",
            KwKind::While => "while",
            KwKind::Until => "until",
            KwKind::For => "for",
            KwKind::Return => "return",
        }
    }
}

#[derive(Clone, Copy)]
struct KwInfo {
    kind: KwKind,
    is_modifier: bool,
    /// `indented_keyword_expression` range (for the syntactic alignment base).
    expr: (usize, usize),
}

enum FrameKind {
    Keyword {
        kind: KwKind,
        expr: Option<(usize, usize)>,
        is_modifier: bool,
        is_ternary: bool,
    },
    Assignment {
        rhs: (usize, usize),
    },
    Send {
        setter: bool,
        paren: Option<(usize, usize)>,
        args: Vec<(usize, usize)>,
        /// `operator_method? && arguments?`: a binary-operator call (`a + b`),
        /// for the `operation_rhs` syntactic alignment base.
        binary_op: bool,
        /// `loc.dot` offset and the selector range, for `get_dot_right_above`
        /// and `left_hand_side` climbing.
        dot: Option<usize>,
        selector: Option<(usize, usize)>,
    },
    Block {
        body: Option<(usize, usize)>,
    },
    /// `splat` / `kwsplat`: shrinks the `indented_relative_to_receiver` extra
    /// indentation by the operator length.
    Splat {
        op_len: usize,
    },
    /// A hash `pair` (`key: value`). `key` is the key's range.
    Pair {
        key: (usize, usize),
    },
    Paren,
    Unaligned,
    Other,
}

struct Frame {
    start: usize,
    end: usize,
    kind: FrameKind,
}

/// A call in a receiver chain, with the fields the hash-pair alignment helpers
/// read off it.
#[derive(Clone, Copy)]
struct CallInfo {
    range: (usize, usize),
    dot: Option<usize>,
    sel: Option<(usize, usize)>,
    recv: Option<(usize, usize)>,
    has_multiline_block: bool,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    style: Style,
    indent: usize,
    base: usize,
    stack: Vec<Frame>,
    pub(crate) offenses: Vec<MethodCallIndentOffense>,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

fn within((is, ie): (usize, usize), (os, oe): (usize, usize)) -> bool {
    is >= os && ie <= oe
}

impl<'a> Visitor<'a> {
    fn line_start(&self, off: usize) -> usize {
        self.line_index.line_start(off)
    }

    fn col(&self, off: usize) -> usize {
        self.line_index.column(self.source, off)
    }

    /// 1-based line number of `off`.
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn indent_col(&self, off: usize) -> usize {
        let ls = self.line_start(off);
        self.source[ls..]
            .iter()
            .take_while(|&&b| b == b' ' || b == b'\t')
            .count()
    }

    fn begins_its_line(&self, off: usize) -> bool {
        let ls = self.line_start(off);
        self.source[ls..off]
            .iter()
            .all(|&b| b == b' ' || b == b'\t')
    }

    fn text(&self, s: usize, e: usize) -> &'a str {
        std::str::from_utf8(&self.source[s..e]).unwrap_or("")
    }

    fn not_for_this_cop(&self, op: (usize, usize)) -> bool {
        self.stack.iter().rev().any(|f| match f.kind {
            FrameKind::Paren => true,
            FrameKind::Send {
                paren: Some((pb, pe)),
                ..
            } => op.0 > pb && op.1 < pe,
            _ => false,
        })
    }

    fn kw_special(&self, range: (usize, usize)) -> Option<KwInfo> {
        for f in self.stack.iter().rev() {
            if let FrameKind::Keyword {
                kind,
                expr,
                is_modifier,
                is_ternary,
            } = f.kind
            {
                if is_ternary {
                    continue;
                }
                if let Some(e) = expr
                    && within(range, e)
                {
                    return Some(KwInfo {
                        kind,
                        is_modifier,
                        expr: e,
                    });
                }
            }
        }
        None
    }

    fn part_of_assignment_rhs(&self, candidate: (usize, usize)) -> Option<(usize, usize)> {
        for f in self.stack.iter().rev() {
            match &f.kind {
                FrameKind::Keyword { .. } | FrameKind::Unaligned => return None,
                FrameKind::Block { body } => {
                    if body.is_some_and(|b| within(candidate, b)) {
                        return None;
                    }
                }
                FrameKind::Assignment { rhs } => {
                    if within(candidate, *rhs) {
                        return Some(*rhs);
                    }
                }
                FrameKind::Send { setter, args, .. } => {
                    if *setter
                        && let Some(last) = args.last()
                        && within(candidate, *last)
                    {
                        return Some(*last);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// `argument_in_method_call(node, :with_parentheses)`: is the call inside the
    /// parenthesized argument list of an enclosing (non-setter) call?
    fn argument_in_parenthesized_call(&self, op: (usize, usize)) -> bool {
        for f in self.stack.iter().rev() {
            match &f.kind {
                FrameKind::Block { .. } => return false,
                FrameKind::Send {
                    setter,
                    paren,
                    args,
                    ..
                } => {
                    if *setter || paren.is_none() {
                        continue;
                    }
                    if args.iter().any(|a| within(op, *a)) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// `get_dot_right_above`: an ancestor call whose dot is exactly one line
    /// above, at the same column as `node_dot`. Returns its `dot.join(selector)`.
    fn get_dot_right_above(&self, node_dot: usize) -> Option<(usize, usize)> {
        let want_line = self.line_of(node_dot);
        let want_col = self.col(node_dot);
        for f in self.stack.iter().rev() {
            if let FrameKind::Send {
                dot: Some(d),
                selector: Some(sel),
                ..
            } = &f.kind
                && self.line_of(*d) + 1 == want_line
                && self.col(*d) == want_col
            {
                return Some((*d, sel.1));
            }
        }
        None
    }

    /// `left_hand_side`: climb to the outermost `.`-chained, non-setter call.
    fn left_hand_side(&self, node_range: (usize, usize)) -> (usize, usize) {
        let mut lhs = node_range;
        for f in self.stack.iter().rev() {
            match &f.kind {
                FrameKind::Send {
                    dot: Some(_),
                    setter: false,
                    ..
                } => lhs = (f.start, f.end),
                _ => break,
            }
        }
        lhs
    }

    /// `extra_indentation`: 0 for `aligned`/`indented`; for
    /// `indented_relative_to_receiver`, the indent width minus the splat operator
    /// length when the call is a direct splat/kwsplat element.
    fn extra_indentation(&self) -> isize {
        if self.style != Style::IndentedRelativeToReceiver {
            return 0;
        }
        if let Some(FrameKind::Splat { op_len }) = self.stack.last().map(|f| &f.kind) {
            self.indent as isize - *op_len as isize
        } else {
            self.indent as isize
        }
    }

    fn correct_indentation(&self, kw: Option<KwInfo>) -> usize {
        let special = kw.is_some_and(|k| !k.is_modifier);
        if special {
            self.indent + self.base
        } else {
            self.indent
        }
    }

    fn operation_description(
        &self,
        kw: Option<KwInfo>,
        assign_rhs: Option<(usize, usize)>,
    ) -> String {
        if let Some(k) = kw {
            let keyword = k.kind.keyword();
            let kind = if k.kind == KwKind::For {
                "collection"
            } else {
                "condition"
            };
            let article = if keyword.starts_with('i') || keyword.starts_with('u') {
                "an"
            } else {
                "a"
            };
            format!("a {kind} in {article} `{keyword}` statement")
        } else if assign_rhs.is_some() {
            "an expression in an assignment".to_string()
        } else {
            "an expression".to_string()
        }
    }

    /// `semantic_alignment_base`: the dot range to align a `.`-prefixed selector
    /// with — `get_dot_right_above`, then the multiline-block chain, then the
    /// first-call fallback (`semantic_alignment_node`).
    fn semantic_alignment_base(
        &self,
        node: &CallNode<'_>,
        node_range: (usize, usize),
        rhs: (usize, usize),
    ) -> Option<(usize, usize)> {
        let rhs_text = self.text(rhs.0, rhs.1);
        if !rhs_text.starts_with('.') && !rhs_text.starts_with("&.") {
            return None;
        }
        if self.argument_in_parenthesized_call(node_range) {
            return None;
        }
        if let Some(dot) = node.call_operator_loc()
            && let Some(above) = self.get_dot_right_above(dot.start_offset())
        {
            return Some(above);
        }
        self.find_multiline_block_chain_base(node)
            .or_else(|| self.first_call_alignment_base(node))
    }

    /// `first_call_alignment_node` reduced to the base range it yields: the
    /// `dot.join(selector)` of the bottom-most `.`-call in the receiver chain,
    /// subject to the array/begin receiver special cases.
    fn first_call_alignment_base(&self, node: &CallNode<'_>) -> Option<(usize, usize)> {
        // Walk the receiver chain, recording the deepest `.`-call.
        let mut rec: Option<(usize, usize, usize)> = None; // (dot_start, selector_end, call_start)
        let mut base: Option<((usize, usize), bool, bool)> = None; // (loc, is_array, is_begin)
        let mut current = node.as_node();
        while let Some(c) = current.as_call_node() {
            if let (Some(dot), Some(sel)) = (c.call_operator_loc(), c.message_loc()) {
                rec = Some((
                    dot.start_offset(),
                    sel.end_offset(),
                    c.as_node().location().start_offset(),
                ));
                base = c.receiver().map(bottom_receiver_info);
            }
            match c.receiver() {
                Some(r) => current = r,
                None => break,
            }
        }

        let (dot_start, sel_end, call_start) = rec?;
        let dot_line = self.line_of(dot_start);

        if let Some((bl, true, _)) = base
            && dot_line == self.line_of(bl.1)
        {
            return Some((dot_start, sel_end)); // method on an array's last line
        }
        if dot_line != self.line_of(call_start) {
            return None;
        }
        if let Some((bl, _, true)) = base
            && dot_line == self.line_of(bl.1)
        {
            return None; // method on a `begin`/grouped receiver's last line
        }
        Some((dot_start, sel_end))
    }

    /// `find_multiline_block_chain_node` reduced to its base range. When the call
    /// carries a block, continuation alignment; otherwise the descendant-block
    /// case.
    fn find_multiline_block_chain_base(&self, node: &CallNode<'_>) -> Option<(usize, usize)> {
        if let Some(blk) = node.block().and_then(|b| b.as_block_node()) {
            let block_single_line = self.is_single_line(loc(&blk.as_node().location()));
            self.find_continuation_base(node, block_single_line)
        } else {
            self.handle_descendant_base(node)
        }
    }

    fn is_single_line(&self, range: (usize, usize)) -> bool {
        self.line_of(range.0) == self.line_of(range.1)
    }

    /// `single_line_block_receiver?` → `receiver.send_node`'s `dot.join(selector)`:
    /// a single-line call carrying a block. In stock the Parser AST wraps the
    /// `send` in a `:block` whose `single_line?` covers `dot ~ block-end` only —
    /// not the full receiver chain. In prism a `CallNode` location spans the
    /// whole chain, so we measure single-line-ness from the call's dot (or its
    /// message when there is no dot) to its block's end.
    fn block_receiver_dot(&self, receiver: &Node<'_>) -> Option<(usize, usize)> {
        let c = receiver.as_call_node()?;
        let blk = c.block().and_then(|b| b.as_block_node())?;
        let dot = c.call_operator_loc()?;
        let sel = c.message_loc()?;
        let span_start = dot.start_offset();
        let span_end = blk.as_node().location().end_offset();
        if self.line_of(span_start) != self.line_of(span_end) {
            return None;
        }
        Some((span_start, sel.end_offset()))
    }

    fn find_continuation_base(
        &self,
        node: &CallNode<'_>,
        block_single_line: bool,
    ) -> Option<(usize, usize)> {
        let receiver = node.receiver()?;
        if let Some(b) = self.block_receiver_dot(&receiver) {
            return Some(b);
        }
        let rc = receiver.as_call_node()?;
        // A receiver that carries a (multiline) block is a parser `:block`, not
        // `call_type?` — `receiver.call_type? && receiver.loc.dot` is false.
        if rc.block().and_then(|b| b.as_block_node()).is_some() {
            return None;
        }
        let dot = rc.call_operator_loc()?;
        let sel = rc.message_loc()?;
        let base = (dot.start_offset(), sel.end_offset());
        let rr = rc.receiver()?;
        let rr_is_begin = rr.as_parentheses_node().is_some() || rr.as_begin_node().is_some();
        if rr_is_begin && block_single_line {
            return Some(base);
        }
        if self.line_of(dot.start_offset()) > self.line_of(rr.location().end_offset()) {
            return Some(base);
        }
        None
    }

    fn handle_descendant_base(&self, node: &CallNode<'_>) -> Option<(usize, usize)> {
        let receiver = node.receiver()?;
        if let Some(b) = self.block_receiver_dot(&receiver) {
            return Some(b);
        }
        // `node.each_descendant(:any_block).first&.multiline?` over the receiver
        // chain: the deepest call carrying a multiline block.
        let owner_dot = self.deepest_block_owner_dot(node)?;
        if let Some(rc) = receiver.as_call_node()
            && let (Some(dot), Some(sel)) = (rc.call_operator_loc(), rc.message_loc())
        {
            return Some((dot.start_offset(), sel.end_offset()));
        }
        Some(owner_dot)
    }

    /// The bottom-most call in the receiver chain that carries a multiline block;
    /// returns its `dot.join(selector)`. `None` if there is no such block.
    fn deepest_block_owner_dot(&self, node: &CallNode<'_>) -> Option<(usize, usize)> {
        let mut found: Option<(usize, usize)> = None;
        let mut current = node.as_node();
        while let Some(c) = current.as_call_node() {
            if let Some(blk) = c.block().and_then(|b| b.as_block_node())
                && !self.is_single_line(loc(&blk.as_node().location()))
                && let (Some(dot), Some(sel)) = (c.call_operator_loc(), c.message_loc())
            {
                found = Some((dot.start_offset(), sel.end_offset()));
            }
            match c.receiver() {
                Some(r) => current = r,
                None => break,
            }
        }
        found
    }

    /// `syntactic_alignment_base`: keyword expression / assignment rhs /
    /// operation rhs.
    fn syntactic_alignment_base(
        &self,
        lhs: (usize, usize),
        rhs: (usize, usize),
    ) -> Option<(usize, usize)> {
        if let Some(kw) = self.kw_special(lhs) {
            return Some(kw.expr);
        }
        if let Some(arhs) = self.part_of_assignment_rhs_for(lhs, rhs) {
            return Some(arhs);
        }
        self.operation_rhs(lhs)
    }

    /// `operation_rhs(lhs)`: if `lhs` is the right operand of an enclosing binary
    /// operator (`a + lhs`), align with that operand. Returns its range.
    fn operation_rhs(&self, lhs: (usize, usize)) -> Option<(usize, usize)> {
        for f in self.stack.iter().rev() {
            if let FrameKind::Send {
                binary_op: true,
                args,
                ..
            } = &f.kind
                && let Some(first) = args.first()
                && within(lhs, *first)
            {
                return Some(*first);
            }
        }
        None
    }

    /// `part_of_assignment_rhs(lhs, rhs)`: walk from `lhs` (not the node) but with
    /// `rhs` as the candidate — matches the mixin's `syntactic_alignment_base`.
    fn part_of_assignment_rhs_for(
        &self,
        _lhs: (usize, usize),
        rhs: (usize, usize),
    ) -> Option<(usize, usize)> {
        self.part_of_assignment_rhs(rhs)
    }

    fn alignment_base(
        &self,
        node: &CallNode<'_>,
        node_range: (usize, usize),
        lhs: (usize, usize),
        rhs: (usize, usize),
    ) -> Option<(usize, usize)> {
        match self.style {
            Style::Aligned => self
                .semantic_alignment_base(node, node_range, rhs)
                .or_else(|| self.syntactic_alignment_base(lhs, rhs)),
            Style::IndentedRelativeToReceiver => self.receiver_alignment_base(node),
            Style::Indented => None,
        }
    }

    /// `find_hash_method_base_in_receiver_chain`: the dot of the first receiver
    /// call whose own receiver is a hash, or a `begin`/grouped node ending on the
    /// call's dot line. (Despite the name it also handles grouped receivers.)
    fn find_hash_method_base(&self, node: &CallNode<'_>) -> Option<(usize, usize)> {
        let mut chain = node.receiver();
        while let Some(cur) = chain {
            let Some(c) = cur.as_call_node() else { break };
            let base_receiver = c.receiver();
            let matched = base_receiver.as_ref().is_some_and(|br| {
                if br.as_hash_node().is_some() {
                    return true;
                }
                let begin = br.as_parentheses_node().is_some() || br.as_begin_node().is_some();
                begin
                    && c.call_operator_loc().is_some_and(|dot| {
                        self.line_of(dot.start_offset()) == self.line_of(br.location().end_offset())
                    })
            });
            if matched
                && let Some(dot) = c.call_operator_loc()
                && let Some(sel) = c.message_loc()
            {
                return Some((dot.start_offset(), sel.end_offset()));
            }
            chain = base_receiver;
        }
        None
    }

    /// `receiver_alignment_base`: hash/grouped chained receiver base, else the
    /// receiver of the bottom-most `.`-call.
    fn receiver_alignment_base(&self, node: &CallNode<'_>) -> Option<(usize, usize)> {
        if let Some(b) = self.find_hash_method_base(node) {
            return Some(b);
        }
        let mut rec: Option<(usize, usize)> = None;
        let mut current = node.as_node();
        while let Some(c) = current.as_call_node() {
            if c.call_operator_loc().is_some() {
                rec = c.receiver().map(|r| loc(&r.location()));
            }
            match c.receiver() {
                Some(r) => current = r,
                None => break,
            }
        }
        rec
    }

    /// The method-call cop's `on_send`.
    fn process_send(&mut self, call: &CallNode<'_>, node_range: (usize, usize)) {
        let Some(_receiver) = call.receiver() else {
            return;
        };
        if call.name().as_slice() == b"[]" {
            return;
        }
        let Some(dot) = call.call_operator_loc() else {
            return; // relevant_node?: only `.`/`&.` calls.
        };
        let Some(rhs) = self.right_hand_side(call, &dot) else {
            return;
        };

        if !self.begins_its_line(rhs.0) {
            return;
        }

        // Stock's `left_hand_side` walks `lhs.parent` while it is a call_type
        // with a dot. In Parser AST a call carrying a block is wrapped in a
        // `:block` (not call_type), so the walk terminates at the call itself.
        // In prism the same call has the block as a *child*, so a naive parent
        // walk happily climbs through to the enclosing send. Mirror stock by
        // stopping at `node_range` when this call has a block.
        let call_has_block = call.block().and_then(|b| b.as_block_node()).is_some();
        let lhs = if call_has_block {
            node_range
        } else {
            self.left_hand_side(node_range)
        };
        let Some((correct_column, message)) = self.offending(call, node_range, lhs, rhs) else {
            return;
        };
        let column_delta = correct_column - self.col(rhs.0) as isize;
        if column_delta == 0 {
            return;
        }

        // Block-aware autocorrect: when the offending call carries a block, the
        // Ruby side realigns the block body and `end` line too.
        let (mut bbs, mut bbe, mut bes, mut bee) = (0, 0, 0, 0);
        if let Some(blk) = call.block().and_then(|b| b.as_block_node()) {
            if let Some(body) = blk.body() {
                let b = loc(&body.location());
                bbs = b.0;
                bbe = b.1;
            }
            let close = blk.closing_loc();
            bes = close.start_offset();
            bee = close.end_offset();
        }

        self.offenses.push(MethodCallIndentOffense {
            start_offset: rhs.0,
            end_offset: rhs.1,
            column_delta,
            message,
            block_body_start: bbs,
            block_body_end: bbe,
            block_end_start: bes,
            block_end_end: bee,
        });
    }

    /// `offending_range` + `check`: returns `(correct_column, message)`, or
    /// `None` to skip. Routes hash-pair contexts before regular indentation.
    fn offending(
        &self,
        call: &CallNode<'_>,
        node_range: (usize, usize),
        lhs: (usize, usize),
        rhs: (usize, usize),
    ) -> Option<(isize, String)> {
        let pair_key = self.find_pair_ancestor();

        if pair_key.is_some() && self.style == Style::Aligned {
            return self.check_hash_pair_indentation(call, node_range, lhs, rhs);
        }
        if let Some(key) = pair_key
            && self.style == Style::Indented
            && self.base_receiver_is_hash(call)
        {
            return Some(self.check_hash_pair_indented_style(node_range, rhs, key));
        }

        // skip_for_context?
        if pair_key.is_some() {
            if self.inside_multiline_chain_arg(node_range) {
                return None;
            }
        } else if self.not_for_this_cop(node_range) {
            return None;
        }

        // check_regular_indentation
        let base = self.alignment_base(call, node_range, lhs, rhs);
        let correct_column = match base {
            Some(b) => self.col(b.0) as isize + self.extra_indentation(),
            None => {
                let kw = self.kw_special(node_range);
                (self.indent_col(lhs.0) + self.correct_indentation(kw)) as isize
            }
        };
        Some((
            correct_column,
            self.build_message(node_range, lhs, rhs, base),
        ))
    }

    /// `find_pair_ancestor`: the key range of the nearest hash-`pair` ancestor.
    fn find_pair_ancestor(&self) -> Option<(usize, usize)> {
        self.stack.iter().rev().find_map(|f| match f.kind {
            FrameKind::Pair { key } => Some(key),
            _ => None,
        })
    }

    fn base_receiver_is_hash(&self, call: &CallNode<'_>) -> bool {
        call.receiver()
            .map(|r| bottom_receiver_node_is_hash(r))
            .unwrap_or(false)
    }

    /// `check_hash_pair_indentation` (aligned style inside a hash pair).
    fn check_hash_pair_indentation(
        &self,
        call: &CallNode<'_>,
        node_range: (usize, usize),
        lhs: (usize, usize),
        rhs: (usize, usize),
    ) -> Option<(isize, String)> {
        let mut base = self.find_hash_pair_alignment_base(call);
        if base.is_none() && self.inside_multiline_chain_arg(node_range) {
            return None;
        }
        let base = base
            .take()
            .or_else(|| self.first_dot_alignment_base(call, node_range, rhs))
            .unwrap_or(lhs);
        if self.aligned_with_first_line_dot(call, node_range, rhs) {
            return None;
        }
        let base_source = self.text(base.0, base.1).split('\n').next().unwrap_or("");
        let message = format!(
            "Align `{}` with `{}` on line {}.",
            self.text(rhs.0, rhs.1),
            base_source,
            self.line_of(base.0)
        );
        Some((self.col(base.0) as isize, message))
    }

    /// `check_hash_pair_indented_style` (indented style, hash-literal receiver).
    fn check_hash_pair_indented_style(
        &self,
        node_range: (usize, usize),
        rhs: (usize, usize),
        key: (usize, usize),
    ) -> (isize, String) {
        let key_col = self.col(key.0);
        let correct_column = (key_col + self.indent * 2) as isize;
        let hash_pair_base_column = (key_col + self.indent) as isize;
        let used = self.col(rhs.0) as isize - hash_pair_base_column;
        let what = self.operation_description(
            self.kw_special(node_range),
            self.part_of_assignment_rhs(rhs),
        );
        let message = format!(
            "Use {} (not {used}) spaces for indenting {what} spanning multiple lines.",
            self.indent
        );
        (correct_column, message)
    }

    /// `find_hash_pair_alignment_base`: when the chain bottoms out in a hash
    /// literal, align with the first dotted call.
    fn find_hash_pair_alignment_base(&self, call: &CallNode<'_>) -> Option<(usize, usize)> {
        let receiver = call.receiver()?;
        if !bottom_receiver_node_is_hash(receiver) {
            return None;
        }
        self.first_call_dot_join(call)
    }

    /// `first_dot_alignment_base`, incl. `after_multiline_block_base`.
    fn first_dot_alignment_base(
        &self,
        call: &CallNode<'_>,
        node_range: (usize, usize),
        rhs: (usize, usize),
    ) -> Option<(usize, usize)> {
        let rhs_text = self.text(rhs.0, rhs.1);
        if !rhs_text.starts_with('.') && !rhs_text.starts_with("&.") {
            return None;
        }
        let calls = self.chain_calls(call);
        let fc_idx = calls.iter().rposition(|c| c.dot.is_some())?;
        let fc = calls[fc_idx];
        let dot = fc.dot?;
        if fc.range == node_range {
            return None;
        }
        // after_multiline_block_base: align with the call chained onto the block.
        if fc.has_multiline_block && fc_idx > 0 {
            let ab = calls[fc_idx - 1];
            if let (Some(d), Some(sel)) = (ab.dot, ab.sel)
                && ab.range != node_range
            {
                return Some((d, sel.1));
            }
        }
        let recv = fc.recv?;
        if self.line_of(dot) != self.line_of(recv.0) {
            return None;
        }
        Some((dot, fc.sel?.1))
    }

    /// `aligned_with_first_line_dot?`.
    fn aligned_with_first_line_dot(
        &self,
        call: &CallNode<'_>,
        node_range: (usize, usize),
        rhs: (usize, usize),
    ) -> bool {
        let rhs_text = self.text(rhs.0, rhs.1);
        if !rhs_text.starts_with('.') && !rhs_text.starts_with("&.") {
            return false;
        }
        let calls = self.chain_calls(call);
        let Some(fc) = calls.iter().rfind(|c| c.dot.is_some()) else {
            return false;
        };
        if let Some(recv) = call.receiver()
            && fc.range == loc(&recv.location())
        {
            return false;
        }
        let Some(dot) = fc.dot else { return false };
        self.line_of(dot) == self.line_of(node_range.0) && self.col(dot) == self.col(rhs.0)
    }

    /// `inside_multiline_chain_arg?`: the hash holding this pair is an argument of
    /// an enclosing dotted call whose selector and receiver sit on different
    /// lines. (Prism interposes `KeywordHashNode`/`ArgumentsNode` between the
    /// pair and the call, where parser links the hash straight to the send.)
    fn inside_multiline_chain_arg(&self, _node_range: (usize, usize)) -> bool {
        let mut it = self.stack.iter().rev();
        for f in it.by_ref() {
            if matches!(f.kind, FrameKind::Pair { .. }) {
                break;
            }
        }
        // The hash holding the pair (`pair.parent`).
        let Some(hash_start) = it.next().map(|f| f.start) else {
            return false;
        };
        // Nearest enclosing call above the pair (skipping hash/arguments wrappers).
        for f in it {
            if let FrameKind::Send { dot, selector, .. } = &f.kind {
                return match (dot, selector) {
                    // `hash_arg_in_chain?`: the call must not have the hash as its
                    // receiver, and its selector/receiver span different lines.
                    (Some(_), Some(sel)) => {
                        f.start != hash_start && self.line_of(sel.0) != self.line_of(f.start)
                    }
                    _ => false,
                };
            }
        }
        false
    }

    /// The chain of calls reachable through `.receiver`, `node` first and the
    /// deepest receiver last.
    fn chain_calls(&self, node: &CallNode<'_>) -> Vec<CallInfo> {
        let mut v = Vec::new();
        let mut current = node.as_node();
        while let Some(c) = current.as_call_node() {
            let has_multiline_block = c
                .block()
                .and_then(|b| b.as_block_node())
                .is_some_and(|b| !self.is_single_line(loc(&b.as_node().location())));
            v.push(CallInfo {
                range: loc(&c.as_node().location()),
                dot: c.call_operator_loc().map(|d| d.start_offset()),
                sel: c.message_loc().as_ref().map(loc),
                recv: c.receiver().map(|r| loc(&r.location())),
                has_multiline_block,
            });
            match c.receiver() {
                Some(r) => current = r,
                None => break,
            }
        }
        v
    }

    /// `first_call_has_a_dot`'s `dot.join(selector)`: the bottom-most `.`-call.
    fn first_call_dot_join(&self, node: &CallNode<'_>) -> Option<(usize, usize)> {
        let calls = self.chain_calls(node);
        let fc = calls.iter().rfind(|c| c.dot.is_some())?;
        Some((fc.dot?, fc.sel?.1))
    }

    fn right_hand_side(&self, call: &CallNode<'_>, dot: &Location<'_>) -> Option<(usize, usize)> {
        let selector = call.message_loc();
        if let Some(sel) = &selector {
            if self.line_of(dot.start_offset()) == self.line_of(sel.start_offset()) {
                return Some((dot.start_offset(), sel.end_offset()));
            }
            return Some(loc(sel));
        }
        // implicit call `a.(args)`: dot.join(loc.begin)
        call.opening_loc()
            .map(|open| (dot.start_offset(), open.end_offset()))
    }

    fn build_message(
        &self,
        node_range: (usize, usize),
        lhs: (usize, usize),
        rhs: (usize, usize),
        base: Option<(usize, usize)>,
    ) -> String {
        if let Some(b) = base {
            let base_source = self.text(b.0, b.1).split('\n').next().unwrap_or("");
            if self.style == Style::Aligned {
                return format!(
                    "Align `{}` with `{}` on line {}.",
                    self.text(rhs.0, rhs.1),
                    base_source,
                    self.line_of(b.0)
                );
            }
            if self.style == Style::IndentedRelativeToReceiver {
                return format!(
                    "Indent `{}` {} spaces more than `{}` on line {}.",
                    self.text(rhs.0, rhs.1),
                    self.indent,
                    base_source,
                    self.line_of(b.0)
                );
            }
        }
        let kw = self.kw_special(node_range);
        let assign = self.part_of_assignment_rhs(rhs);
        let what = self.operation_description(kw, assign);
        let used = self.col(rhs.0) as isize - self.indent_col(lhs.0) as isize;
        let expected = self.correct_indentation(kw);
        format!("Use {expected} (not {used}) spaces for indenting {what} spanning multiple lines.")
    }

    fn frame_for(&self, node: &Node<'_>) -> FrameKind {
        if let Some(n) = node.as_if_node() {
            let is_ternary = n.if_keyword_loc().is_none();
            let is_modifier = !is_ternary && n.end_keyword_loc().is_none();
            return FrameKind::Keyword {
                kind: KwKind::If,
                expr: Some(loc(&n.predicate().location())),
                is_modifier,
                is_ternary,
            };
        }
        if let Some(n) = node.as_unless_node() {
            return FrameKind::Keyword {
                kind: KwKind::Unless,
                expr: Some(loc(&n.predicate().location())),
                is_modifier: n.end_keyword_loc().is_none(),
                is_ternary: false,
            };
        }
        if let Some(n) = node.as_while_node() {
            return FrameKind::Keyword {
                kind: KwKind::While,
                expr: Some(loc(&n.predicate().location())),
                is_modifier: false,
                is_ternary: false,
            };
        }
        if let Some(n) = node.as_until_node() {
            return FrameKind::Keyword {
                kind: KwKind::Until,
                expr: Some(loc(&n.predicate().location())),
                is_modifier: false,
                is_ternary: false,
            };
        }
        if let Some(n) = node.as_for_node() {
            return FrameKind::Keyword {
                kind: KwKind::For,
                expr: Some(loc(&n.collection().location())),
                is_modifier: false,
                is_ternary: false,
            };
        }
        if let Some(n) = node.as_return_node() {
            let expr = n
                .arguments()
                .and_then(|a| a.arguments().iter().next().map(|f| loc(&f.location())));
            return FrameKind::Keyword {
                kind: KwKind::Return,
                expr,
                is_modifier: false,
                is_ternary: false,
            };
        }
        if node.as_parentheses_node().is_some() {
            return FrameKind::Paren;
        }
        if node.as_array_node().is_some() {
            return FrameKind::Unaligned;
        }
        if let Some(n) = node.as_begin_node() {
            if n.begin_keyword_loc().is_some() {
                return FrameKind::Unaligned;
            }
            return FrameKind::Other;
        }
        if let Some(n) = node.as_block_node() {
            return FrameKind::Block {
                body: n.body().map(|b| loc(&b.location())),
            };
        }
        if let Some(n) = node.as_splat_node() {
            let op = n.operator_loc();
            return FrameKind::Splat {
                op_len: op.end_offset() - op.start_offset(),
            };
        }
        if let Some(n) = node.as_assoc_splat_node() {
            let op = n.operator_loc();
            return FrameKind::Splat {
                op_len: op.end_offset() - op.start_offset(),
            };
        }
        if let Some(n) = node.as_assoc_node() {
            return FrameKind::Pair {
                key: loc(&n.key().location()),
            };
        }
        if let Some(rhs) = assignment_value(node) {
            return FrameKind::Assignment { rhs };
        }
        if let Some(c) = node.as_call_node() {
            let paren = match c.closing_loc() {
                Some(close) if close.as_slice() == b")" => c
                    .opening_loc()
                    .map(|o| (o.start_offset(), close.end_offset())),
                _ => None,
            };
            let args: Vec<(usize, usize)> = c
                .arguments()
                .map(|a| {
                    a.arguments()
                        .iter()
                        .map(|arg| loc(&arg.location()))
                        .collect()
                })
                .unwrap_or_default();
            let binary_op = c.receiver().is_some()
                && c.call_operator_loc().is_none()
                && c.name().as_slice() != b"[]"
                && !args.is_empty();
            return FrameKind::Send {
                setter: c.is_attribute_write(),
                paren,
                args,
                binary_op,
                dot: c.call_operator_loc().map(|d| d.start_offset()),
                selector: c.message_loc().as_ref().map(loc),
            };
        }
        FrameKind::Other
    }
}

/// Whether `find_base_receiver(node)` is a hash literal.
fn bottom_receiver_node_is_hash(node: Node<'_>) -> bool {
    let mut cur = node;
    while let Some(r) = cur.as_call_node().and_then(|c| c.receiver()) {
        cur = r;
    }
    cur.as_hash_node().is_some()
}

/// `find_base_receiver`: walk `.receiver` to the bottom of the chain, returning
/// its range and whether it is an `array` / `begin`(grouped) node.
fn bottom_receiver_info(node: Node<'_>) -> ((usize, usize), bool, bool) {
    let mut cur = node;
    while let Some(r) = cur.as_call_node().and_then(|c| c.receiver()) {
        cur = r;
    }
    let l = loc(&cur.location());
    let is_array = cur.as_array_node().is_some();
    let is_begin = cur.as_parentheses_node().is_some() || cur.as_begin_node().is_some();
    (l, is_array, is_begin)
}

fn assignment_value(node: &Node<'_>) -> Option<(usize, usize)> {
    macro_rules! try_value {
        ($($m:ident),* $(,)?) => {
            $(if let Some(n) = node.$m() { return Some(loc(&n.value().location())); })*
        };
    }
    try_value!(
        as_local_variable_write_node,
        as_instance_variable_write_node,
        as_class_variable_write_node,
        as_global_variable_write_node,
        as_constant_write_node,
        as_constant_path_write_node,
        as_multi_write_node,
        as_local_variable_operator_write_node,
        as_local_variable_or_write_node,
        as_local_variable_and_write_node,
        as_instance_variable_operator_write_node,
        as_instance_variable_or_write_node,
        as_instance_variable_and_write_node,
        as_class_variable_operator_write_node,
        as_class_variable_or_write_node,
        as_class_variable_and_write_node,
        as_global_variable_operator_write_node,
        as_global_variable_or_write_node,
        as_global_variable_and_write_node,
        as_constant_operator_write_node,
        as_constant_or_write_node,
        as_constant_and_write_node,
        as_constant_path_operator_write_node,
        as_constant_path_or_write_node,
        as_constant_path_and_write_node,
        as_index_operator_write_node,
        as_index_or_write_node,
        as_index_and_write_node,
        as_call_operator_write_node,
        as_call_or_write_node,
        as_call_and_write_node,
    );
    None
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(c) = node.as_call_node() {
            self.process_send(&c, loc(&node.location()));
        }
        let kind = self.frame_for(node);
        let l = node.location();
        self.stack.push(Frame {
            start: l.start_offset(),
            end: l.end_offset(),
            kind,
        });
    }

    fn leave(&mut self) {
        self.stack.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: Style) -> Vec<(usize, usize, isize, String)> {
        let s = match style {
            Style::Aligned => 0,
            Style::Indented => 1,
            Style::IndentedRelativeToReceiver => 2,
        };
        check_multiline_method_call_indentation(source.as_bytes(), s, 2, 2)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.message))
            .collect()
    }

    #[test]
    fn aligned_chain_misaligned() {
        // `.c` at col 6 should align under the chain's first dot `.a` (col 5).
        let got = run("Thing.a\n     .b\n      .c\n", Style::Aligned);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, -1);
        assert!(got[0].3.starts_with("Align `.c` with `.a`"), "{}", got[0].3);
    }

    #[test]
    fn accepts_aligned_chain() {
        assert!(run("Thing.a\n     .b\n     .c\n", Style::Aligned).is_empty());
    }

    #[test]
    fn indented_style() {
        let got = run("a\n.b\n", Style::Indented);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, 2);
        assert!(got[0].3.contains("Use 2 (not 0)"), "{}", got[0].3);
    }
}
