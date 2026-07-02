//! Shared body-length calculation for `Metrics/BlockLength` and
//! `Metrics/MethodLength`.
//!
//! Port of `RuboCop::Cop::Metrics::Utils::CodeLengthCalculator` (driven by the
//! `RuboCop::Cop::CodeLength` mixin) over the prism AST. Both cops measure the
//! non-blank / non-comment line count of a definition body, optionally folding
//! `CountAsOne` constructs (`array` / `hash` / `heredoc` / `method_call`) to one
//! line each.
//!
//! Two quirks the naive "count the body node's source lines" approach gets
//! wrong, both verified against a stock probe (`.tmp/2026-06-14/method-length/`):
//!
//! - **Heredoc in the body.** Prism's body-node location ends at the heredoc
//!   *opening* (`x = <<~H`), not its content. Stock's
//!   `source_from_node_with_heredoc` measures whole source *lines* from the
//!   body's first line through the furthest line any descendant reaches,
//!   counting the heredoc body + closing-delimiter line. A body whose only
//!   statement assigns a 6-line heredoc counts as 8 lines, not 1.
//! - **Braceless hash fold.** When a folded `hash` is an unbraced keyword-hash
//!   argument (`foo(a: 1, b: 2)`), stock subtracts `omit_length` — the count of
//!   the enclosing call's `(` / `)` that do not coincide with the hash's own
//!   span — so the fold removes the brace-substitute lines too.

use std::rc::Rc;

use ruby_prism::{Location, Node, Visit};

use super::line_index::{self, LineIndex};

/// Which constructs `CountAsOne` folds. Unknown types are ignored here; the
/// Ruby side raises `RuboCop::Warning` for them.
#[derive(Clone, Copy, Default)]
pub struct Fold {
    array: bool,
    hash: bool,
    heredoc: bool,
    method_call: bool,
}

impl Fold {
    pub fn from_types(types: &[String]) -> Self {
        let mut f = Fold::default();
        for t in types {
            match t.as_str() {
                "array" => f.array = true,
                "hash" => f.hash = true,
                "heredoc" => f.heredoc = true,
                "method_call" => f.method_call = true,
                _ => {}
            }
        }
        f
    }

    pub fn any(&self) -> bool {
        self.array || self.hash || self.heredoc || self.method_call
    }
}

/// Stateless body-length calculator bound to one source.
pub struct CodeLength<'a> {
    source: &'a [u8],
    count_comments: bool,
    fold: Fold,
    index: Rc<LineIndex>,
}

impl<'a> CodeLength<'a> {
    pub fn new(source: &'a [u8], count_comments: bool, fold: Fold) -> Self {
        CodeLength {
            source,
            count_comments,
            fold,
            // Shared with the other cops on this source: BlockLength and
            // MethodLength each build a calculator per file, and a private
            // index would be a redundant full-source scan for each.
            index: line_index::with_line_index(source, Rc::clone),
        }
    }

    fn slice(&self, start: usize, end: usize) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.source[start..end])
    }

    /// `irrelevant_line?`: blank, or (without `CountComments`) a comment line.
    fn irrelevant(&self, line: &str) -> bool {
        line.trim().is_empty() || (!self.count_comments && line.trim_start().starts_with('#'))
    }

    /// Sound fast reject: `true` when the measured body length provably cannot
    /// exceed `max`, without measuring. Everything in [`Self::body_length`]
    /// only ever *shrinks* the count relative to the body's physical line
    /// span — irrelevant-line filtering and `CountAsOne` folds subtract, and
    /// the implicit-`begin` / single-statement normalizations narrow the
    /// span — except for the heredoc extension, which measures through lines
    /// past the body's own end. A heredoc reaches those lines only via a
    /// `<<~` / `<<-` / `<<` marker inside the body's source, so a body whose
    /// span already fits in `max` and whose slice has no `<<` cannot come out
    /// over `max`. `false` only means "measure precisely", never "over".
    pub fn cannot_exceed(&self, body: Option<&Node<'_>>, max: usize) -> bool {
        let Some(body) = body else { return true };
        let loc = body.location();
        let (lo, hi) = (loc.start_offset(), loc.end_offset());
        let last = hi.max(lo + 1) - 1;
        let span = self.index.line_of(last) - self.index.line_of(lo) + 1;
        span <= max && !self.source[lo..hi].windows(2).any(|w| w == b"<<")
    }

    /// Body length of a definition body, matching `CodeLengthCalculator#calculate`:
    /// the body's `code_length`, then for every top-level foldable descendant
    /// `length = length - code_length(descendant) + 1` (minus `omit_length` for a
    /// braceless folded hash). Returns 0 for an absent body.
    pub fn body_length(&self, body: Option<Node<'_>>) -> usize {
        let Some(body) = body else { return 0 };
        let base = self.code_length(&body) as isize;
        if !self.fold.any() {
            return base.max(0) as usize;
        }
        let mut fv = FoldVisitor {
            outer: self,
            suppress: 0,
            stack: Vec::new(),
            lift_stack: Vec::new(),
            lift_starts: Vec::new(),
            ancestors: Vec::new(),
            delta: 0,
        };
        fv.visit(&body);
        (base + fv.delta).max(0) as usize
    }

    /// Measure a *body* node (stock's `code_length(@node)` after `extract_body`):
    /// its own source-line count, with the heredoc-in-body extension. A body that
    /// *is* a heredoc string counts its raw source span (just the `<<~MARK`
    /// opening line) — the `heredoc_length` (+2) form is only for a folded
    /// heredoc descendant, never the body itself.
    fn code_length(&self, node: &Node<'_>) -> usize {
        self.length(node, false)
    }

    /// Measure a folded *descendant* (stock's `code_length(descendant)`): a
    /// heredoc collapses via `heredoc_length` (body lines + 2).
    fn descendant_length(&self, node: &Node<'_>) -> usize {
        self.length(node, true)
    }

    fn length(&self, node: &Node<'_>, as_descendant: bool) -> usize {
        // Prism wraps a definition body in a `StatementsNode`; the parser-gem
        // `def.body` is the single statement directly when there is exactly one.
        // Unwrap so a lone top statement is the measured root (not a descendant),
        // matching stock's `extract_body` — otherwise the statement's own `end`
        // line leaks into the heredoc-extension max-line scan.
        if let Some(stmts) = node.as_statements_node() {
            let mut it = stmts.body().iter();
            if let (Some(only), None) = (it.next(), it.next()) {
                return self.length(&only, as_descendant);
            }
        }
        if as_descendant && let Some((start, end)) = self.heredoc_body_span(node) {
            // A folded heredoc descendant: count its body lines + 2 (marker +
            // close), matching stock's `heredoc_length`.
            let text = self.slice(start, end);
            return text.lines().filter(|l| !self.irrelevant(l)).count() + 2;
        }
        // An implicit `begin` (a method/block body with a bare
        // `rescue`/`else`/`ensure`) is a `BeginNode` whose prism location spans
        // the enclosing `def`/block, including the header and the closing `end`.
        // Stock measures only the clause content, so normalize the span to the
        // clauses (matching the parser-gem `rescue`/`ensure` body node).
        let (lo, hi) = self.implicit_begin_span(node).unwrap_or_else(|| {
            let loc = node.location();
            (loc.start_offset(), loc.end_offset())
        });
        if let Some(last_end) = self.heredoc_extended_end(node) {
            // Body contains a heredoc: measure whole source lines from the
            // body's first line through the furthest line any *descendant*
            // reaches (stock's `source_from_node_with_heredoc` derives the last
            // line purely from descendants, never the body node's own `end`).
            let start = self.index.line_start(lo);
            return self.count_lines(start, last_end);
        }
        self.count_lines(lo, hi)
    }

    /// For an implicit-`begin` `BeginNode` (no `begin` keyword), the byte span of
    /// its clause content (statements + `rescue`/`else`/`ensure`), excluding the
    /// enclosing `def`/block header and `end`. `None` for any other node.
    fn implicit_begin_span(&self, node: &Node<'_>) -> Option<(usize, usize)> {
        let begin = node.as_begin_node()?;
        if begin.begin_keyword_loc().is_some() {
            return None; // explicit `begin ... end`
        }
        let mut lo: Option<usize> = None;
        let mut hi: Option<usize> = None;
        let mut span = |loc: Location<'_>| {
            lo = Some(lo.map_or(loc.start_offset(), |v| v.min(loc.start_offset())));
            hi = Some(hi.map_or(loc.end_offset(), |v| v.max(loc.end_offset())));
        };
        if let Some(s) = begin.statements() {
            span(s.location());
        }
        if let Some(r) = begin.rescue_clause() {
            span(r.location());
        }
        if let Some(e) = begin.else_clause() {
            // Prism's `ElseNode` (in a begin/rescue/else context) location
            // extends through the enclosing `end` keyword when no `ensure`
            // follows; with `ensure`, it ends at the `ensure` keyword. The
            // parser-gem `else` body stops before whichever closing keyword
            // comes next. Cap the span at `end_keyword_loc` (which always
            // points to that closing keyword) so the trailing `end` / `ensure`
            // line is not counted — matching the existing `ensure_clause`
            // handling below. Without this, methods that close a `rescue` body
            // with `else` count the `end` line as one extra body line.
            let loc = e.location();
            let content_end = e
                .end_keyword_loc()
                .map_or(loc.end_offset(), |k| k.start_offset());
            lo = Some(lo.map_or(loc.start_offset(), |v| v.min(loc.start_offset())));
            hi = Some(hi.map_or(content_end, |v| v.max(content_end)));
        }
        if let Some(e) = begin.ensure_clause() {
            // Prism's `EnsureNode` location includes the enclosing `end`
            // keyword; the parser-gem `ensure` body stops before it. Cap the
            // span at the `end` keyword so the closing `end` line is not
            // counted.
            let loc = e.location();
            let content_end = e.end_keyword_loc().start_offset();
            lo = Some(lo.map_or(loc.start_offset(), |v| v.min(loc.start_offset())));
            hi = Some(hi.map_or(content_end, |v| v.max(content_end)));
        }
        Some((lo?, hi?))
    }

    /// Count non-irrelevant lines in `source[start..end]`.
    fn count_lines(&self, start: usize, end: usize) -> usize {
        self.slice(start, end)
            .lines()
            .filter(|l| !self.irrelevant(l))
            .count()
    }

    /// If `node` has a `str`/`dstr` heredoc descendant, return the byte offset of
    /// the end of the furthest line any descendant reaches (heredocs reach their
    /// closing-delimiter line). Mirrors stock's `node_with_heredoc?` gate +
    /// `source_from_node_with_heredoc`'s max-last-line scan.
    fn heredoc_extended_end(&self, node: &Node<'_>) -> Option<usize> {
        let mut scan = HeredocScan {
            outer: self,
            has_heredoc: false,
            max_line_start: self.index.line_start(node.location().start_offset()),
            depth: 0,
        };
        scan.visit(node);
        if !scan.has_heredoc {
            return None;
        }
        Some(self.line_end_offset(scan.max_line_start))
    }

    /// Byte offset just past the end of the line whose start is `line_start`
    /// (start of the next line, or the source length at EOF).
    fn line_end_offset(&self, line_start: usize) -> usize {
        match self.source[line_start..].iter().position(|&b| b == b'\n') {
            Some(rel) => line_start + rel + 1,
            None => self.source.len(),
        }
    }

    fn is_str_or_dstr(&self, node: &Node<'_>) -> bool {
        node.as_string_node().is_some() || node.as_interpolated_string_node().is_some()
    }

    /// The closing-delimiter location of a `str`/`dstr` heredoc node, else
    /// `None` (the only heredocs stock's `node_with_heredoc?` reacts to).
    fn heredoc_close_loc<'b>(&self, node: &Node<'b>) -> Option<Location<'b>> {
        if let Some(s) = node.as_string_node()
            && self.opening_is_heredoc(s.opening_loc())
        {
            return s.closing_loc();
        }
        if let Some(s) = node.as_interpolated_string_node()
            && self.opening_is_heredoc(s.opening_loc())
        {
            return s.closing_loc();
        }
        None
    }

    fn opening_is_heredoc(&self, opening: Option<Location<'_>>) -> bool {
        match opening {
            Some(loc) => self
                .source
                .get(loc.start_offset()..loc.end_offset())
                .is_some_and(|s| s.starts_with(b"<<")),
            None => false,
        }
    }

    /// Byte span of a heredoc's body (between the marker line and the closing
    /// delimiter), or `None`. Works for both a plain `str` heredoc and an
    /// interpolated (`<<~`/`<<-` with interpolation or squiggly) heredoc, whose
    /// content prism splits into parts — the span is simply opening-end to
    /// closing-start, matching stock's `loc.heredoc_body.source`.
    fn heredoc_body_span(&self, node: &Node<'_>) -> Option<(usize, usize)> {
        let (opening, closing) = if let Some(s) = node.as_string_node() {
            (s.opening_loc(), s.closing_loc())
        } else if let Some(s) = node.as_interpolated_string_node() {
            (s.opening_loc(), s.closing_loc())
        } else {
            return None;
        };
        let opening = opening?;
        let is_heredoc = self
            .source
            .get(opening.start_offset()..opening.end_offset())
            .is_some_and(|s| s.starts_with(b"<<"));
        if !is_heredoc {
            return None;
        }
        let closing = closing?;
        Some((opening.end_offset(), closing.start_offset()))
    }

    /// `foldable_node?`. A braceless keyword-hash argument is a `hash` in stock
    /// (parser-gem); prism splits it into `KeywordHashNode`, so the `hash` fold
    /// must match both. A braced literal is a `HashNode`.
    fn is_foldable(&self, node: &Node<'_>) -> bool {
        (self.fold.array && node.as_array_node().is_some())
            || (self.fold.hash
                && (node.as_hash_node().is_some() || node.as_keyword_hash_node().is_some()))
            || (self.fold.method_call && node.as_call_node().is_some())
            || (self.fold.heredoc && self.heredoc_close_loc(node).is_some())
    }

    /// When `node` is a `method_call`-foldable `CallNode` carrying a block,
    /// return `(block_opening_start, block_body_start)`. The send part (call
    /// start to the block opening) folds as a unit while the block body is
    /// descended, mirroring stock's `(block (send ...) body)` tree shape. A
    /// `LambdaNode` is *not* a call in parser-gem, so it is excluded.
    fn method_call_block(&self, node: &Node<'_>) -> Option<(usize, Option<usize>)> {
        if !self.fold.method_call {
            return None;
        }
        let call = node.as_call_node()?;
        let block = call.block()?;
        let block_node = block.as_block_node()?;
        let opening = block_node.opening_loc().start_offset();
        let body_start = block_node.body().map(|b| b.location().start_offset());
        Some((opening, body_start))
    }

    /// Line count of the send part of a call-with-block (`[start, send_end]`),
    /// extended through any heredoc reached by the receiver or arguments
    /// (mirroring stock's heredoc-aware `code_length` of the separate `send`
    /// node). The block body is excluded — it is scanned independently.
    fn send_part_length(&self, call: &Node<'_>, start: usize, send_end: usize) -> usize {
        let Some(call) = call.as_call_node() else {
            return self.count_lines(start, send_end);
        };
        let mut scan = HeredocScan {
            outer: self,
            has_heredoc: false,
            max_line_start: self.index.line_start(start),
            depth: 1, // children scanned directly count (no body-root exclusion)
        };
        if let Some(recv) = call.receiver() {
            scan.visit(&recv);
        }
        if let Some(args) = call.arguments() {
            scan.visit(&args.as_node());
        }
        let end = if scan.has_heredoc {
            self.line_end_offset(scan.max_line_start).max(send_end)
        } else {
            send_end
        };
        self.count_lines(self.index.line_start(start), end)
    }

    /// The offset with trailing ASCII whitespace before `end` trimmed away, used
    /// to end the send part before the ` do` / ` {` of its block.
    fn trim_trailing_ws(&self, end: usize) -> usize {
        let mut e = end;
        while e > 0 && self.source[e - 1].is_ascii_whitespace() {
            e -= 1;
        }
        e
    }

    /// `omit_length`: for a folded braceless hash argument, the count of the
    /// enclosing call's `(` / `)` that do not coincide with the hash's span.
    /// Only a `KeywordHashNode` (always braceless) qualifies; a braced
    /// `HashNode` returns 0. `ancestors` is the fold walk's node stack, innermost
    /// last, used to reach the call parent (skipping the `ArgumentsNode` prism
    /// inserts where stock's parser-gem nests the hash directly under the send).
    fn omit_length(&self, node: &Node<'_>, ancestors: &[Node<'_>]) -> usize {
        let Some(kw) = node.as_keyword_hash_node() else {
            return 0;
        };
        // Walk up past an `ArgumentsNode` to the call (stock's `node.parent`).
        let Some(parent) = ancestors.last() else {
            return 0;
        };
        let call_node = if parent.as_arguments_node().is_some() {
            ancestors.get(ancestors.len().wrapping_sub(2))
        } else {
            Some(parent)
        };
        let Some(call) = call_node.and_then(|n| n.as_call_node()) else {
            return 0;
        };
        // `another_args?`: more than one argument -> no omission.
        let arg_count = call
            .arguments()
            .map(|a| a.arguments().iter().count())
            .unwrap_or(0);
        if arg_count > 1 {
            return 0;
        }
        // `parenthesized?`: the call must have explicit parentheses.
        let (Some(open), Some(close)) = (call.opening_loc(), call.closing_loc()) else {
            return 0;
        };
        let span = kw.location();
        let mut n = 0;
        if open.end_offset() != span.start_offset() {
            n += 1;
        }
        if close.start_offset() != span.end_offset() {
            n += 1;
        }
        n
    }
}

/// Whether a node is a class or module definition (folding stops at these).
fn is_classlike(node: &Node<'_>) -> bool {
    matches!(node, Node::ClassNode { .. } | Node::ModuleNode { .. })
}

/// Applies `CountAsOne` folding over a body, mirroring
/// `each_top_level_descendant`: each outermost foldable construct collapses to
/// one line (minus `omit_length`), and class/module bodies are not descended.
///
/// A call with an attached block needs care for the `method_call` fold: stock's
/// parser tree nests the `send` inside a `block` node, so it folds only the
/// *send part* (receiver + arguments, no block body) and descends into the block
/// body, where inner foldables fold independently and the body lines count. So
/// here a foldable `CallNode` with a block does not collapse the whole call;
/// instead its send part (up to the block opening) folds as a unit (suppressing
/// the receiver/arguments inside it) while the block body is left to fold
/// normally.
struct FoldVisitor<'a, 'b, 'pr> {
    outer: &'b CodeLength<'a>,
    /// Depth of foldable/classlike boundaries we are nested inside.
    suppress: usize,
    /// Per-node record of whether it raised `suppress`, popped on leave.
    stack: Vec<bool>,
    /// Per-node record of whether it lifted suppression (a folded call-with-block
    /// body), popped on leave.
    lift_stack: Vec<bool>,
    /// Start offsets of block bodies whose suppression must be lifted when
    /// entered (the body of a folded call-with-block).
    lift_starts: Vec<usize>,
    /// Ancestor stack (innermost last), so `omit_length` can reach the call.
    ancestors: Vec<Node<'pr>>,
    delta: isize,
}

impl<'pr> FoldVisitor<'_, '_, 'pr> {
    fn count_top_level(&mut self, node: &Node<'pr>) {
        self.delta += 1 - self.outer.descendant_length(node) as isize;
        self.delta -= self.outer.omit_length(node, &self.ancestors) as isize;
    }

    /// Fold a `method_call` foldable that carries a block: collapse the send
    /// part (call start to the block opening, trailing whitespace trimmed) and
    /// register the block body so suppression lifts there. The send part is
    /// measured with the heredoc-in-body extension (stock's separate `send`
    /// node folds via the heredoc-aware `code_length`), scanning the receiver
    /// and arguments but not the block body.
    fn fold_call_with_block(&mut self, node: &Node<'pr>, block_open: usize, body_start: Option<usize>) {
        let start = node.location().start_offset();
        let send_end = self.outer.trim_trailing_ws(block_open);
        let lines = self.outer.send_part_length(node, start, send_end);
        self.delta += 1 - lines as isize;
        if let Some(bs) = body_start {
            self.lift_starts.push(bs);
        }
    }
}

impl<'pr> Visit<'pr> for FoldVisitor<'_, '_, 'pr> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        // Lift suppression when entering a registered block body.
        let lifted = self.suppress > 0
            && self
                .lift_starts
                .last()
                .is_some_and(|&s| s == node.location().start_offset());
        if lifted {
            self.suppress -= 1;
            self.lift_starts.pop();
        }
        self.lift_stack.push(lifted);

        let boundary = if self.suppress == 0 {
            if let Some((block_open, body_start)) = self.outer.method_call_block(&node) {
                // A `method_call` foldable carrying a block: fold the send part
                // and suppress its receiver/arguments, lifting in the body.
                self.fold_call_with_block(&node, block_open, body_start);
                true
            } else if self.outer.is_foldable(&node) {
                self.count_top_level(&node);
                true
            } else {
                is_classlike(&node)
            }
        } else {
            false
        };
        if boundary {
            self.suppress += 1;
        }
        self.stack.push(boundary);
        self.ancestors.push(node);
    }

    fn visit_branch_node_leave(&mut self) {
        if self.stack.pop().unwrap_or(false) {
            self.suppress -= 1;
        }
        if self.lift_stack.pop().unwrap_or(false) {
            self.suppress += 1;
        }
        self.ancestors.pop();
    }

    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        // A leaf has no children, so no lift bookkeeping is needed.
        self.lift_stack.push(false);
        if self.suppress == 0 && self.outer.is_foldable(&node) {
            self.count_top_level(&node);
        }
    }

    fn visit_leaf_node_leave(&mut self) {
        self.lift_stack.pop();
    }
}

/// Scans a body for a heredoc descendant and the furthest line any descendant
/// reaches. Mirrors `node_with_heredoc?` + `source_from_node_with_heredoc`.
struct HeredocScan<'a, 'b> {
    outer: &'b CodeLength<'a>,
    has_heredoc: bool,
    max_line_start: usize,
    /// Depth from the scanned body (the body itself is depth 0 and excluded,
    /// matching `each_descendant`).
    depth: usize,
}

impl HeredocScan<'_, '_> {
    fn record(&mut self, node: &Node<'_>) {
        if self.depth == 0 {
            return; // exclude the body node itself
        }
        // Prism inserts wrappers whose location includes a trailing structural
        // keyword that the parser-gem tree attributes to a different node:
        // `ElseNode` / `EnsureNode` include the enclosing `end`; a `BlockNode`
        // duplicates the span of its `CallNode` (parser's block node), which is
        // already scanned. Skip the wrapper's own span and let its children
        // contribute their real lines (so a `foo do ... end` block's `end` line
        // comes from the `CallNode`, and the receiver's heredocs are still seen).
        if matches!(
            node,
            Node::ElseNode { .. } | Node::EnsureNode { .. } | Node::BlockNode { .. }
        ) {
            return;
        }
        // An `elsif` is a prism `IfNode` nested in the `subsequent` position; its
        // location includes the outer `if`'s shared `end` keyword. The parser-gem
        // tree attributes that `end` to the outer node, so skip the `elsif`'s own
        // span (its children still contribute their lines). Detect it by its
        // `if_keyword_loc` reading `elsif` rather than `if`.
        if let Some(if_node) = node.as_if_node()
            && let Some(kw) = if_node.if_keyword_loc()
            && self
                .outer
                .source
                .get(kw.start_offset()..kw.end_offset())
                == Some(b"elsif")
        {
            return;
        }
        let line_byte = match self.outer.heredoc_close_loc(node) {
            Some(close) => {
                if self.outer.is_str_or_dstr(node) {
                    self.has_heredoc = true;
                }
                close.start_offset()
            }
            None => {
                let e = node.location().end_offset();
                if e == 0 { 0 } else { e - 1 }
            }
        };
        let ls = self.outer.index.line_start(line_byte);
        if ls > self.max_line_start {
            self.max_line_start = ls;
        }
    }
}

impl<'pr> Visit<'pr> for HeredocScan<'_, '_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        self.record(&node);
        self.depth += 1;
    }

    fn visit_branch_node_leave(&mut self) {
        self.depth -= 1;
    }

    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        self.record(&node);
    }
}
