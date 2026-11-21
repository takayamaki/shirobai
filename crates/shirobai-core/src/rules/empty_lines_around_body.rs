//! The `EmptyLinesAroundBody` mixin family:
//! `Layout/EmptyLinesAroundMethodBody`, `Layout/EmptyLinesAroundClassBody`,
//! `Layout/EmptyLinesAroundModuleBody`, `Layout/EmptyLinesAroundBlockBody`,
//! `Layout/EmptyLinesAroundBeginBody` and
//! `Layout/EmptyLinesAroundExceptionHandlingKeywords`.
//!
//! All six stock cops share the `EmptyLinesAroundBody` mixin: pure line
//! arithmetic around a node's first/last line (`check_beginning` /
//! `check_ending` / `check_deferred_empty_line`), plus the keyword-adjacent
//! variant for `rescue` / `else` / `ensure`. One rule computes all six in a
//! single shared walk; each cop reads its own offense vector.
//!
//! Faithfulness notes (verified against stock probes):
//!
//! - A "line" is `ProcessedSource#lines[i]`: the raw line chomped (`\n` /
//!   trailing `\r` removed). Only a **zero-length** line is empty; a
//!   whitespace-only line is not.
//! - The offense range is `source_range(buffer, line, 0)` = the first
//!   *character* of the 1-based line (`[start, start + len_utf8(first char))`
//!   in bytes; the Ruby wrapper converts to character offsets).
//! - Stock `add_offense` dedupes by identical range (`class Foo\n\nend` keeps
//!   only the "beginning" offense), which the Ruby wrapper inherits for free
//!   by emitting in stock's order; this rule emits both.
//! - `EmptyLinesAroundExceptionHandlingKeywords` also checks `it` blocks
//!   since 1.92.3 (rubocop#16325 added the `on_itblock` alias).

use ruby_prism::Node;

/// Styles, in `SupportedStyles` order of `Layout/EmptyLinesAroundClassBody`.
pub const NO_EMPTY_LINES: u8 = 0;
pub const EMPTY_LINES: u8 = 1;
pub const EMPTY_LINES_EXCEPT_NAMESPACE: u8 = 2;
pub const EMPTY_LINES_SPECIAL: u8 = 3;
pub const BEGINNING_ONLY: u8 = 4;
pub const ENDING_ONLY: u8 = 5;

/// One offense. `[start_offset, end_offset)` is the first character of the
/// offense line (column 0, length 1, exactly stock's `source_range(buffer,
/// line, 0)`); `insert` distinguishes the two `EmptyLineCorrector` arms:
/// `false` removes the range, `true` inserts `"\n"` before it.
#[derive(Debug)]
pub struct EmptyLineOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub insert: bool,
    pub message: String,
    /// EHK only (1.92, rubocop#15814): an "after the keyword" offense whose
    /// keyword sits in a block body and is followed only by blank lines up to
    /// the block's closing line. Stock leaves it to
    /// `Layout/EmptyLinesAroundBlockBody` when that cop is enabled with
    /// `empty_lines`; the wrapper checks that config and drops the offense.
    pub block_body_deferred: bool,
}

/// The per-cop `EnforcedStyle`s (the other three cops have a fixed style).
#[derive(Clone, Copy)]
pub struct Config {
    pub class_style: u8,
    pub module_style: u8,
    pub block_style: u8,
}

/// All six cops' offenses for one source.
#[derive(Default)]
pub struct FamilyOffenses {
    pub method_body: Vec<EmptyLineOffense>,
    pub class_body: Vec<EmptyLineOffense>,
    pub module_body: Vec<EmptyLineOffense>,
    pub block_body: Vec<EmptyLineOffense>,
    pub begin_body: Vec<EmptyLineOffense>,
    pub exception_keywords: Vec<EmptyLineOffense>,
}

pub fn check_empty_lines_around_body(source: &[u8], cfg: Config) -> FamilyOffenses {
    let mut rule = build_rule(source, cfg);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.into_offenses()
}

/// Build the rule for use standalone or in a shared-walk bundle. The checks
/// are stateless per node (no ancestor stack), so the plain full walk fits.
pub(crate) fn build_rule(source: &[u8], cfg: Config) -> Visitor<'_> {
    let mut line_starts = Vec::with_capacity(source.len() / 32 + 1);
    line_starts.push(0);
    for (i, &b) in source.iter().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    Visitor {
        source,
        line_starts,
        cfg,
        out: FamilyOffenses {
            method_body: Vec::new(),
            class_body: Vec::new(),
            module_body: Vec::new(),
            block_body: Vec::new(),
            begin_body: Vec::new(),
            exception_keywords: Vec::new(),
        },
        ehk_block: None,
    }
}

/// Which cop an offense belongs to (selects the output vector and the
/// `%<kind>s` interpolation of the shared messages).
#[derive(Clone, Copy)]
enum Kind {
    Method,
    Class,
    Module,
    Block,
    Begin,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Method => "method",
            Kind::Class => "class",
            Kind::Module => "module",
            Kind::Block => "block",
            Kind::Begin => "`begin`",
        }
    }
}

/// `first_empty_line_required_child` / `first_child_requires_empty_line?`
/// data for one statement of a class/module body.
struct ChildInfo {
    /// `constant_definition?`: `{class module}` (not `sclass`).
    is_constant_def: bool,
    /// `empty_line_required?`: `{any_def class module #access_modifier_send?
    /// [{(if _ #access_modifier_send? nil?) (if _ nil? #access_modifier_send?)}
    /// modifier_form?]}` (1.92.3 added the modifier arm;
    /// `access_modifier_send?` is the *bare* modifier: no receiver, no
    /// arguments, no block).
    required: bool,
    /// 1-based first line (for `check_deferred_empty_line`).
    first_line: usize,
    /// 1-based parser `last_line` (for `same_line_as_previous_statement?`).
    last_line: usize,
    /// Parser node type interpolated into `MSG_DEFERRED`.
    type_name: &'static str,
}

/// Parser-equivalent body of a class/module: `None` is a nil body; `is_begin`
/// is `body.begin_type?` (more than one statement).
struct BodyInfo {
    is_begin: bool,
    children: Vec<ChildInfo>,
    /// Parser `body.first_line` / `body.last_line` (1.92.3, rubocop#16305).
    lines: (usize, usize),
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    /// Byte offset of the start of each line (`line_starts[0] == 0`).
    line_starts: Vec<usize>,
    cfg: Config,
    out: FamilyOffenses,
    /// Body start line and closing line of the block whose body EHK is
    /// checking (`body.parent` is a block); `None` for def / kwbegin bodies.
    ehk_block: Option<(usize, usize)>,
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_ALL,
        )
    }
    
    fn enter(&mut self, node: &Node<'_>) {
        self.dispatch(node);
    }

    fn leave(&mut self) {}
}

impl<'a> Visitor<'a> {
    pub(crate) fn into_offenses(self) -> FamilyOffenses {
        self.out
    }

    // --- Line helpers (ProcessedSource#lines semantics). ---

    /// 1-based line number of byte offset `off`.
    fn line_of(&self, off: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= off)
    }

    /// Content of the 0-based line `idx0`, chomped (`\n` and a trailing `\r`
    /// stripped). `None` when the line does not exist.
    fn line_content0(&self, idx0: usize) -> Option<&'a [u8]> {
        let start = *self.line_starts.get(idx0)?;
        if start >= self.source.len() {
            // A line start just past a trailing `\n` is not a line in
            // `ProcessedSource#lines` (no empty last entry).
            return None;
        }
        let end = self
            .line_starts
            .get(idx0 + 1)
            .map(|&next| next - 1) // strip the `\n`
            .unwrap_or(self.source.len());
        let mut line = &self.source[start..end];
        if let Some((b'\r', rest)) = line.split_last() {
            line = rest;
        }
        Some(line)
    }

    /// `lines[idx0].empty?` (a missing line is treated as non-empty; stock
    /// would `NoMethodError` there, and no reachable check goes out of range).
    fn line_empty0(&self, idx0: usize) -> bool {
        self.line_content0(idx0).is_some_and(|l| l.is_empty())
    }

    /// `comment_line?(lines[idx0])`: `/^\s*#/`.
    fn line_comment0(&self, idx0: usize) -> bool {
        let Some(line) = self.line_content0(idx0) else {
            return false;
        };
        for &b in line {
            match b {
                b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r' => {}
                b'#' => return true,
                _ => return false,
            }
        }
        false
    }

    /// `source_range(buffer, line1, 0)`: the first character of the 1-based
    /// line `line1`, as a byte range ending on the next character boundary
    /// (so the wrapper's byte→char conversion yields exactly chars `[0, 1)`).
    fn line_head_range(&self, line1: usize) -> Option<(usize, usize)> {
        let start = *self.line_starts.get(line1.checked_sub(1)?)?;
        let len = match self.source.get(start) {
            None => 1, // zero-width tail; unreachable from the checks
            Some(&b) if b < 0x80 || (0x80..0xC0).contains(&b) => 1,
            Some(&b) if b < 0xE0 => 2,
            Some(&b) if b < 0xF0 => 3,
            Some(_) => 4,
        };
        Some((start, start + len))
    }

    fn push(&mut self, kind: Kind, line1: usize, insert: bool, message: String) {
        let Some((start, end)) = self.line_head_range(line1) else {
            return;
        };
        let vec = match kind {
            Kind::Method => &mut self.out.method_body,
            Kind::Class => &mut self.out.class_body,
            Kind::Module => &mut self.out.module_body,
            Kind::Block => &mut self.out.block_body,
            Kind::Begin => &mut self.out.begin_body,
        };
        vec.push(EmptyLineOffense {
            start_offset: start,
            end_offset: end,
            insert,
            message,
            block_body_deferred: false,
        });
    }

    // --- Node dispatch. ---

    fn dispatch(&mut self, node: &Node<'_>) {
        if let Some(d) = node.as_def_node() {
            self.on_def(&d);
        } else if let Some(c) = node.as_class_node() {
            let adjusted = c
                .superclass()
                .map(|s| self.line_of(s.location().end_offset()));
            let body = self.body_info(c.body());
            self.check(
                Kind::Class,
                self.cfg.class_style,
                self.line_of(c.location().start_offset()),
                self.line_of(c.end_keyword_loc().start_offset()),
                adjusted,
                &body,
            );
        } else if let Some(s) = node.as_singleton_class_node() {
            let body = self.body_info(s.body());
            self.check(
                Kind::Class,
                self.cfg.class_style,
                self.line_of(s.location().start_offset()),
                self.line_of(s.end_keyword_loc().start_offset()),
                None,
                &body,
            );
        } else if let Some(m) = node.as_module_node() {
            let body = self.body_info(m.body());
            self.check(
                Kind::Module,
                self.cfg.module_style,
                self.line_of(m.location().start_offset()),
                self.line_of(m.end_keyword_loc().start_offset()),
                None,
                &body,
            );
        } else if let Some(b) = node.as_begin_node() {
            if b.begin_keyword_loc().is_none() {
                return; // implicit begin: handled through its def/block owner
            }
            let Some(end_kw) = b.end_keyword_loc() else {
                return;
            };
            let first = self.line_of(b.location().start_offset());
            let end_line = self.line_of(end_kw.start_offset());
            // `on_kwbegin`: `check(node, nil)` — the body is always nil, so
            // with the fixed `no_empty_lines` style the begin/end lines are
            // checked regardless of rescue/ensure sections in between.
            self.check(Kind::Begin, NO_EMPTY_LINES, first, end_line, None, &None);
            // EHK `on_kwbegin`: `check_body(node.children.first, node.loc.line)`.
            self.check_exception_keywords(&b.as_node(), first, Some(end_line));
        } else if let Some(c) = node.as_call_node() {
            let Some(block) = c.block() else { return };
            let Some(bn) = block.as_block_node() else {
                return;
            };
            self.on_block(node, &bn);
        } else if let Some(s) = node.as_super_node() {
            let Some(block) = s.block() else { return };
            let Some(bn) = block.as_block_node() else {
                return;
            };
            self.on_block(node, &bn);
        } else if let Some(s) = node.as_forwarding_super_node() {
            let Some(bn) = s.block() else { return };
            self.on_block(node, &bn);
        } else if let Some(l) = node.as_lambda_node() {
            // parser: `-> { }` is `(block (lambda) ...)`.
            let first = self.line_of(l.location().start_offset());
            let open_line = self.line_of(l.opening_loc().start_offset());
            let close_line = self.line_of(l.closing_loc().start_offset());
            let body_lines = l.body().map(|b| self.parser_lines(&b));
            let start = self.body_start_line(open_line, &l.parameters());
            self.check_block_body(open_line, close_line, start, body_lines);
            self.ehk_block = Some((start, close_line));
            self.check_ehk_body(l.body(), first, Some(close_line));
            self.ehk_block = None;
        }
    }

    fn on_def(&mut self, d: &ruby_prism::DefNode<'_>) {
        if let Some(eq) = d.equal_loc() {
            // Endless method: `offending_endless_method?` and its dedicated
            // whole-line offense (`line_range(line).resize(1)` — same shape as
            // `source_range(line, 0)` on the empty line).
            if let Some(body) = d.body() {
                let eq_line = self.line_of(eq.start_offset());
                let body_first = self.line_of(body.location().start_offset());
                if body_first > eq_line + 1 && self.line_empty0(eq_line) {
                    self.push(
                        Kind::Method,
                        eq_line + 1,
                        false,
                        extra_msg("method", "beginning"),
                    );
                }
            }
            // EHK: an endless def's body is a plain expression (a trailing
            // modifier `rescue` binds to the whole `def`), so there is nothing
            // keyword-shaped to check.
            return;
        }
        let Some(end_kw) = d.end_keyword_loc() else {
            return;
        };
        let first = self.line_of(d.location().start_offset());
        let end_line = self.line_of(end_kw.start_offset());
        // `node.arguments.source_range&.last_line`: parser's args node covers
        // the parens when present, the bare parameter list otherwise, and has
        // no source range at all for a paren-less empty list.
        let adjusted = if let Some(rp) = d.rparen_loc() {
            Some(self.line_of(rp.start_offset()))
        } else {
            d.parameters()
                .map(|p| self.line_of(p.location().end_offset()))
        };
        // Method body style is fixed `no_empty_lines`, so the nil-body guard
        // never fires and no namespace info is needed.
        self.check_simple(
            Kind::Method,
            NO_EMPTY_LINES,
            first,
            end_line,
            adjusted,
            d.body().map(|b| self.parser_lines(&b)),
        );
        self.check_ehk_body(d.body(), first, Some(end_line));
    }

    /// Shared `on_block` for call/super blocks: `EmptyLinesAroundBlockBody`
    /// plus EHK (`it` blocks included since 1.92.3, rubocop#16325).
    fn on_block(&mut self, node: &Node<'_>, bn: &ruby_prism::BlockNode<'_>) {
        let first = self.line_of(node.location().start_offset());
        let open_line = self.line_of(bn.opening_loc().start_offset());
        let close_line = self.line_of(bn.closing_loc().start_offset());
        let start = self.body_start_line(open_line, &bn.parameters());
        let body_lines = bn.body().map(|b| self.parser_lines(&b));
        self.check_block_body(open_line, close_line, start, body_lines);
        self.ehk_block = Some((start, close_line));
        self.check_ehk_body(bn.body(), first, Some(close_line));
        self.ehk_block = None;
    }

    /// `body_start_line` (1.92, rubocop#15953 / #16059): the line of `{` /
    /// `do`, or the last line of the block arguments when they end later.
    /// `arguments?` is false for numbered / `it` parameters and for empty
    /// pipes (`||`), which parser models as an empty `args` node.
    fn body_start_line(&self, open_line: usize, params: &Option<Node<'_>>) -> usize {
        let Some(bp) = params.as_ref().and_then(|p| p.as_block_parameters_node()) else {
            return open_line;
        };
        if bp.parameters().is_none() && bp.locals().iter().next().is_none() {
            return open_line;
        }
        let end = bp.location().end_offset();
        open_line.max(self.line_of(end.saturating_sub(1)))
    }

    /// `rubocop-ast` overrides `BlockNode#single_line?` to compare the
    /// *delimiter* lines (`loc.begin.line == loc.end.line`), so a single-line
    /// `{ ... }` hanging off a multiline call chain is skipped; `open_line`
    /// stands in for the node's first line (the adjusted first line always
    /// overrides it for the actual checks).
    fn check_block_body(
        &mut self,
        open_line: usize,
        close_line: usize,
        body_start: usize,
        body_lines: Option<(usize, usize)>,
    ) {
        self.check_simple(
            Kind::Block,
            self.cfg.block_style,
            open_line,
            close_line,
            Some(body_start),
            body_lines,
        );
    }

    /// `check(node, body, adjusted_first_line:)` for the styles that never
    /// inspect the body's children (method/block/begin: `no_empty_lines` or
    /// `empty_lines` only).
    fn check_simple(
        &mut self,
        kind: Kind,
        style: u8,
        node_first: usize,
        node_last: usize,
        adjusted: Option<usize>,
        body_lines: Option<(usize, usize)>,
    ) {
        if body_lines.is_none() && style != NO_EMPTY_LINES {
            return; // valid_body_style?
        }
        if node_first == node_last {
            return; // single_line?
        }
        let first = adjusted.unwrap_or(node_first);
        if first >= node_last {
            return; // 1.92: `first_line >= last_line`
        }
        self.check_both(kind, style, body_lines, first, node_last);
    }

    /// `check` for class/module (full style set; `body` is the
    /// parser-equivalent body, `None` for nil).
    fn check(
        &mut self,
        kind: Kind,
        style: u8,
        node_first: usize,
        node_last: usize,
        adjusted: Option<usize>,
        body: &Option<BodyInfo>,
    ) {
        if body.is_none() && style != NO_EMPTY_LINES {
            return; // valid_body_style?
        }
        if node_first == node_last {
            return; // single_line?
        }
        let first = adjusted.unwrap_or(node_first);
        let last = node_last;
        if first >= last {
            return; // 1.92: `first_line >= last_line`
        }
        match style {
            EMPTY_LINES_EXCEPT_NAMESPACE => {
                // body is Some here (nil bodies returned above).
                let ns = body.as_ref().is_some_and(namespace_with_one_child);
                let inner = if ns { NO_EMPTY_LINES } else { EMPTY_LINES };
                let lines = body.as_ref().map(|b| b.lines);
                self.check_both(kind, inner, lines, first, last);
            }
            EMPTY_LINES_SPECIAL => {
                let Some(body) = body else { return };
                let lines = Some(body.lines);
                if namespace_with_one_child(body) {
                    self.check_both(kind, NO_EMPTY_LINES, lines, first, last);
                } else {
                    if body.children.first().is_some_and(|c| c.required) {
                        self.check_beginning(kind, EMPTY_LINES, lines, first);
                    } else {
                        self.check_beginning(kind, NO_EMPTY_LINES, lines, first);
                        self.check_deferred(kind, body);
                    }
                    self.check_ending(kind, EMPTY_LINES, lines, last);
                }
            }
            _ => {
                let lines = body.as_ref().map(|b| b.lines);
                self.check_both(kind, style, lines, first, last);
            }
        }
    }

    fn check_both(
        &mut self,
        kind: Kind,
        style: u8,
        body: Option<(usize, usize)>,
        first: usize,
        last: usize,
    ) {
        match style {
            BEGINNING_ONLY => {
                self.check_beginning(kind, EMPTY_LINES, body, first);
                self.check_ending(kind, NO_EMPTY_LINES, body, last);
            }
            ENDING_ONLY => {
                self.check_beginning(kind, NO_EMPTY_LINES, body, first);
                self.check_ending(kind, EMPTY_LINES, body, last);
            }
            _ => {
                self.check_beginning(kind, style, body, first);
                self.check_ending(kind, style, body, last);
            }
        }
    }

    /// 1.92.3 (rubocop#16305): no blank line is required at the beginning
    /// when the body starts on the (adjusted) first line.
    fn check_beginning(&mut self, kind: Kind, style: u8, body: Option<(usize, usize)>, first: usize) {
        if style == EMPTY_LINES && body.is_some_and(|(f, _)| f == first) {
            return;
        }
        self.check_source(kind, style, first, "beginning");
    }

    /// 1.92.3 (rubocop#16305): no blank line is required at the end when
    /// the body ends on the closing line.
    fn check_ending(&mut self, kind: Kind, style: u8, body: Option<(usize, usize)>, last: usize) {
        if style == EMPTY_LINES && body.is_some_and(|(_, l)| l == last) {
            return;
        }
        // `last - 2` can underflow only for a node "ending" on line 1, which
        // `single_line?` already excluded.
        self.check_source(kind, style, last - 2, "end");
    }

    /// `check_source` + `check_line`. `line_idx0` is the 0-based index into
    /// `lines` (stock passes the same value).
    fn check_source(&mut self, kind: Kind, style: u8, line_idx0: usize, desc: &str) {
        let (offending, msg) = match style {
            NO_EMPTY_LINES => (self.line_empty0(line_idx0), extra_msg(kind.label(), desc)),
            EMPTY_LINES => (
                !self.line_empty0(line_idx0),
                missing_msg(kind.label(), desc),
            ),
            _ => return,
        };
        if !offending {
            return;
        }
        let offset = if style == EMPTY_LINES && msg.contains("end.") {
            2
        } else {
            1
        };
        self.push(kind, line_idx0 + offset, style == EMPTY_LINES, msg);
    }

    /// `check_deferred_empty_line(body)`.
    fn check_deferred(&mut self, kind: Kind, body: &BodyInfo) {
        let Some(idx) = body.children.iter().position(|c| c.required) else {
            return;
        };
        let child = &body.children[idx];
        // `same_line_as_previous_statement?` (1.92.3, rubocop#16372). The
        // deferred path only runs when the first child is not required, so
        // `idx > 0` and the left sibling exists.
        if idx > 0 && body.children[idx - 1].last_line == child.first_line {
            return;
        }
        // `previous_line_ignoring_comments(node.first_line)`:
        // decrement-then-read, skipping comment lines, clamped at 0.
        let mut line0 = child.first_line.saturating_sub(2);
        loop {
            if !self.line_comment0(line0) || line0 == 0 {
                break;
            }
            line0 -= 1;
        }
        if self.line_empty0(line0) {
            return;
        }
        let msg = format!(
            "Empty line missing before first {} definition",
            child.type_name
        );
        self.push(kind, line0 + 2, true, msg);
    }

    // --- EmptyLinesAroundExceptionHandlingKeywords. ---

    /// `check_body(body, line_of_def_or_kwbegin)` for a def/block body. The
    /// parser body is a `rescue`/`ensure` node exactly when prism wraps the
    /// body in an implicit `BeginNode` (or, for `on_kwbegin`, when the
    /// explicit begin carries the clauses — the caller passes the `BeginNode`
    /// itself and `children.first` is that same wrapper).
    fn check_ehk_body(
        &mut self,
        body: Option<Node<'_>>,
        node_line: usize,
        end_kw_line: Option<usize>,
    ) {
        let Some(body) = body else { return };
        if let Some(bn) = body.as_begin_node() {
            if bn.begin_keyword_loc().is_some() {
                // Explicit begin as the sole body statement: parser sees a
                // `kwbegin` child, whose own visit handles it.
                return;
            }
            self.check_exception_keywords(&bn.as_node(), node_line, end_kw_line);
        } else if let Some(stmts) = body.as_statements_node() {
            // parser unwraps a sole statement: a lone `x rescue y` IS the
            // body, and a modifier rescue is a parser `rescue` node with one
            // resbody (probe: stock flags a blank line above it).
            let nodes = stmts.body();
            if nodes.len() == 1
                && let Some(rm) = nodes.iter().next().and_then(|n| {
                    n.as_rescue_modifier_node()
                        .map(|r| r.keyword_loc().start_offset())
                })
            {
                self.check_modifier_rescue(rm, node_line, end_kw_line);
            }
        }
    }

    /// A parser `rescue` node built from a modifier `rescue`: one resbody
    /// whose expression starts at the keyword, no else. `last_body_line` is
    /// therefore the keyword line itself.
    fn check_modifier_rescue(
        &mut self,
        keyword_start: usize,
        node_line: usize,
        end_kw_line: Option<usize>,
    ) {
        let Some(end_kw_line) = end_kw_line else {
            return;
        };
        let kw_line = self.line_of(keyword_start);
        if kw_line == node_line || kw_line == end_kw_line {
            return;
        }
        self.check_keyword_lines("rescue", kw_line);
    }

    /// The keyword loop shared by the def/block path and the kwbegin path.
    /// `begin_node` carries the rescue/else/ensure clauses.
    fn check_exception_keywords(
        &mut self,
        begin_node: &Node<'_>,
        node_line: usize,
        end_kw_line: Option<usize>,
    ) {
        let Some(bn) = begin_node.as_begin_node() else {
            return;
        };
        let has_ensure = bn.ensure_clause().is_some();
        if !has_ensure && bn.rescue_clause().is_none() {
            // `on_kwbegin` passes `node.children.first`: with no clauses that
            // is the first *statement*, which can itself be a modifier
            // rescue (a parser `rescue` node).
            if let Some(rm) = bn
                .statements()
                .and_then(|st| st.body().iter().next())
                .and_then(|n| {
                    n.as_rescue_modifier_node()
                        .map(|r| r.keyword_loc().start_offset())
                })
            {
                self.check_modifier_rescue(rm, node_line, end_kw_line);
            }
            return;
        }
        let Some(end_kw_line) = end_kw_line else {
            return;
        };

        // keyword_locations: `[ensure?, else?, rescue, rescue, ...]` —
        // `keyword_locations_in_ensure` prepends `ensure`, and
        // `keyword_locations_in_rescue` puts `loc.else` before the resbodies.
        let mut keywords: Vec<(&'static str, usize)> = Vec::new();
        if let Some(ens) = bn.ensure_clause() {
            keywords.push((
                "ensure",
                self.line_of(ens.ensure_keyword_loc().start_offset()),
            ));
        }
        let mut resbody_lines: Vec<usize> = Vec::new();
        if let Some(rescue) = bn.rescue_clause() {
            if let Some(els) = bn.else_clause() {
                keywords.push(("else", self.line_of(els.else_keyword_loc().start_offset())));
            }
            let mut clause = Some(rescue);
            while let Some(r) = clause {
                resbody_lines.push(self.line_of(r.keyword_loc().start_offset()));
                clause = r.subsequent();
            }
            for &line in &resbody_lines {
                keywords.push(("rescue", line));
            }
        }

        // `last_body_and_end_on_same_line?(body)` — loop-invariant.
        let body_last_line = if has_ensure {
            // parser `ensure` node `last_line`: the end of the ensure body,
            // or the `ensure` keyword itself when the body is empty.
            let ens = bn.ensure_clause().expect("checked above");
            ens.statements()
                .map(|st| self.line_of(st.location().end_offset()))
                .unwrap_or_else(|| self.line_of(ens.ensure_keyword_loc().start_offset()))
        } else if let Some(els) = bn.else_clause() {
            // rescue node with else: `body.loc.else.line`.
            self.line_of(els.else_keyword_loc().start_offset())
        } else {
            // `resbody_branches.last.loc.line`: a resbody's expression starts
            // at its `rescue` keyword.
            *resbody_lines.last().expect("rescue clause present")
        };
        let skip_all = body_last_line == end_kw_line;

        for (kw, line) in keywords {
            if line == node_line || skip_all {
                continue;
            }
            self.check_keyword_lines(kw, line);
        }
    }

    /// One keyword's pair of `check_line` calls: below the keyword, then
    /// above it (stock's order).
    fn check_keyword_lines(&mut self, kw: &str, line: usize) {
        // `lines[line]`: the line below the 1-based keyword line.
        if self.line_empty0(line) {
            let deferred = self.only_empty_lines_until_block_end(line);
            self.push_ehk(line + 1, ehk_msg("after", kw), deferred);
        }
        // `lines[line - 2]`: the line above the keyword.
        if line >= 2 && self.line_empty0(line - 2) {
            let deferred = self.only_empty_lines_since_block_start(line);
            self.push_ehk(line - 1, ehk_msg("before", kw), deferred);
        }
    }

    /// `beginning_left_to_block_body_cop?` minus the config check (1.92.3,
    /// rubocop#16262): the body's parent is a block and
    /// `only_empty_lines_since_beginning?` — walking up from the line above
    /// the keyword, the first line whose `strip` is non-empty is the block
    /// body start line (`block_body_start_line`, the same as the block body
    /// cop's `body_start_line`).
    fn only_empty_lines_since_block_start(&self, keyword_line: usize) -> bool {
        let Some((start_line, _)) = self.ehk_block else {
            return false;
        };
        let prev = (start_line..keyword_line)
            .rev()
            .find(|&l| !self.line_blank0(l - 1));
        prev == Some(start_line)
    }

    /// `lines[idx0].strip.empty?` (a missing line counts as non-blank).
    fn line_blank0(&self, idx0: usize) -> bool {
        self.line_content0(idx0).is_some_and(|c| {
            c.iter()
                .all(|&b| matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r' | 0))
        })
    }

    /// `left_to_block_body_cop?` minus the config check (done per offense in
    /// the wrapper): the body's parent is a block and
    /// `only_empty_lines_until_end?` — the first line after the keyword whose
    /// `strip` is non-empty is the block's closing line.
    fn only_empty_lines_until_block_end(&self, keyword_line: usize) -> bool {
        let Some((_, end_line)) = self.ehk_block else {
            return false;
        };
        let next = (keyword_line + 1..=end_line).find(|&l| !self.line_blank0(l - 1));
        next == Some(end_line)
    }

    fn push_ehk(&mut self, line1: usize, message: String, block_body_deferred: bool) {
        let Some((start, end)) = self.line_head_range(line1) else {
            return;
        };
        self.out.exception_keywords.push(EmptyLineOffense {
            start_offset: start,
            end_offset: end,
            insert: false,
            message,
            block_body_deferred,
        });
    }

    /// Parser-equivalent body info for a class/module body (always a
    /// `StatementsNode` or absent in prism).
    fn body_info(&self, body: Option<Node<'_>>) -> Option<BodyInfo> {
        let body = body?;
        let lines = self.parser_lines(&body);
        let Some(stmts) = body.as_statements_node() else {
            // A body with `rescue` / `ensure` is an implicit `BeginNode`;
            // parser sees one `rescue` / `ensure` child, which is never
            // `empty_line_required?`.
            return Some(BodyInfo {
                is_begin: false,
                children: vec![ChildInfo {
                    is_constant_def: false,
                    required: false,
                    first_line: lines.0,
                    last_line: lines.1,
                    type_name: "",
                }],
                lines,
            });
        };
        let mut iter = stmts.body().iter();
        if let (Some(sole), None) = (iter.next(), iter.next())
            && let Some(paren) = sole.as_parentheses_node()
        {
            // A sole `( ... )` statement is the body itself in parser: a
            // `begin` node (`begin_type?`) whose children are the statements
            // inside the parentheses.
            let children: Vec<ChildInfo> = match paren.body() {
                None => Vec::new(),
                Some(inner) => match inner.as_statements_node() {
                    Some(st) => st.body().iter().map(|n| self.child_info(&n)).collect(),
                    None => vec![self.child_info(&inner)],
                },
            };
            return Some(BodyInfo {
                is_begin: true,
                children,
                lines,
            });
        }
        let children: Vec<ChildInfo> = stmts.body().iter().map(|n| self.child_info(&n)).collect();
        if children.is_empty() {
            return None;
        }
        Some(BodyInfo {
            is_begin: children.len() > 1,
            children,
            lines,
        })
    }

    /// Parser `first_line` / `last_line` of a body. A `StatementsNode` spans
    /// its statements like parser's `begin` (or the sole child). An
    /// implicit `BeginNode` (a body with `rescue` / `ensure` clauses) spans
    /// up to the `end` keyword in prism, while parser's `rescue` / `ensure`
    /// node ends at the last clause: the `ensure` body (or keyword), the
    /// `else` body (or keyword), or the last resbody (body, `then`,
    /// variable, exception list, or keyword, in that order).
    fn parser_lines(&self, body: &Node<'_>) -> (usize, usize) {
        let Some(bn) = body.as_begin_node().filter(|b| b.begin_keyword_loc().is_none()) else {
            let loc = body.location();
            return (
                self.line_of(loc.start_offset()),
                self.line_of(loc.end_offset().saturating_sub(1).max(loc.start_offset())),
            );
        };
        let end_line = |loc: ruby_prism::Location<'_>| {
            self.line_of(loc.end_offset().saturating_sub(1).max(loc.start_offset()))
        };
        let first = if let Some(st) = bn.statements() {
            self.line_of(st.location().start_offset())
        } else if let Some(r) = bn.rescue_clause() {
            self.line_of(r.keyword_loc().start_offset())
        } else if let Some(e) = bn.ensure_clause() {
            self.line_of(e.ensure_keyword_loc().start_offset())
        } else {
            self.line_of(bn.location().start_offset())
        };
        let last = if let Some(e) = bn.ensure_clause() {
            match e.statements() {
                Some(st) => end_line(st.location()),
                None => end_line(e.ensure_keyword_loc()),
            }
        } else if let Some(e) = bn.else_clause() {
            match e.statements() {
                Some(st) => end_line(st.location()),
                None => end_line(e.else_keyword_loc()),
            }
        } else if let Some(mut r) = bn.rescue_clause() {
            while let Some(next) = r.subsequent() {
                r = next;
            }
            if let Some(st) = r.statements() {
                end_line(st.location())
            } else if let Some(t) = r.then_keyword_loc() {
                end_line(t)
            } else if let Some(v) = r.reference() {
                end_line(v.location())
            } else if let Some(x) = r.exceptions().iter().last() {
                end_line(x.location())
            } else {
                end_line(r.keyword_loc())
            }
        } else {
            end_line(bn.location())
        };
        (first, last)
    }

    fn child_info(&self, node: &Node<'_>) -> ChildInfo {
        let first_line = self.line_of(node.location().start_offset());
        let last_line = self.parser_lines(node).1;
        if let Some(d) = node.as_def_node() {
            return ChildInfo {
                is_constant_def: false,
                required: true, // any_def
                first_line,
                last_line,
                type_name: if d.receiver().is_some() {
                    "defs"
                } else {
                    "def"
                },
            };
        }
        if node.as_class_node().is_some() {
            return ChildInfo {
                is_constant_def: true,
                required: true,
                first_line,
                last_line,
                type_name: "class",
            };
        }
        if node.as_module_node().is_some() {
            return ChildInfo {
                is_constant_def: true,
                required: true,
                first_line,
                last_line,
                type_name: "module",
            };
        }
        if is_access_modifier_send(node) {
            return ChildInfo {
                is_constant_def: false,
                required: true,
                first_line,
                last_line,
                type_name: "send",
            };
        }
        if is_modifier_access_modifier(node) {
            return ChildInfo {
                is_constant_def: false,
                required: true,
                first_line,
                last_line,
                type_name: "if",
            };
        }
        ChildInfo {
            is_constant_def: false,
            required: false,
            first_line,
            last_line,
            type_name: "",
        }
    }
}

/// `access_modifier_send?`: `(send nil? {:private :protected :public})`, a
/// bare modifier — no receiver, no arguments, no block (a block makes the
/// parser node a `block`, not a `send`).
fn is_access_modifier_send(node: &Node<'_>) -> bool {
    node.as_call_node().is_some_and(|c| {
        c.receiver().is_none()
            && c.arguments().is_none()
            && c.block().is_none()
            && matches!(c.name().as_slice(), b"private" | b"protected" | b"public")
    })
}

/// The 1.92.3 arm of `empty_line_required?` (rubocop#16309):
/// `[{(if _ #access_modifier_send? nil?) (if _ nil? #access_modifier_send?)}
/// modifier_form?]`, i.e. `private if cond` / `private unless cond`.
fn is_modifier_access_modifier(node: &Node<'_>) -> bool {
    let sole_modifier = |stmts: Option<ruby_prism::StatementsNode<'_>>, kw_start: usize| {
        stmts.is_some_and(|st| {
            let mut it = st.body().iter();
            match (it.next(), it.next()) {
                (Some(n), None) => {
                    n.location().start_offset() < kw_start && is_access_modifier_send(&n)
                }
                _ => false,
            }
        })
    };
    if let Some(i) = node.as_if_node() {
        // A modifier `if` has an `if` keyword after its body and no `end`.
        return i.end_keyword_loc().is_none()
            && i.subsequent().is_none()
            && i.if_keyword_loc()
                .is_some_and(|kw| sole_modifier(i.statements(), kw.start_offset()));
    }
    if let Some(u) = node.as_unless_node() {
        return u.end_keyword_loc().is_none()
            && u.else_clause().is_none()
            && sole_modifier(u.statements(), u.keyword_loc().start_offset());
    }
    false
}

/// `namespace?(body, with_one_child: true)`: a single class/module child.
fn namespace_with_one_child(body: &BodyInfo) -> bool {
    !body.is_begin && body.children.first().is_some_and(|c| c.is_constant_def)
}


fn extra_msg(kind: &str, loc: &str) -> String {
    format!("Extra empty line detected at {kind} body {loc}.")
}

fn missing_msg(kind: &str, loc: &str) -> String {
    format!("Empty line missing at {kind} body {loc}.")
}

fn ehk_msg(position: &str, keyword: &str) -> String {
    format!("Extra empty line detected {position} the `{keyword}`.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(class_style: u8, module_style: u8, block_style: u8) -> Config {
        Config {
            class_style,
            module_style,
            block_style,
        }
    }

    fn default_cfg() -> Config {
        cfg(NO_EMPTY_LINES, NO_EMPTY_LINES, NO_EMPTY_LINES)
    }

    fn run(source: &str, c: Config) -> FamilyOffenses {
        check_empty_lines_around_body(source.as_bytes(), c)
    }

    // Stock 1.92.1 (rubocop#16059): with multiline block arguments the body
    // starts after the closing `|`, so the blank line after it is the
    // "beginning" line (offense on line 4).
    #[test]
    fn block_body_starts_after_multiline_arguments() {
        let src = "foo(a,\n    b) { |x,\n          y|\n\n  x\n}\n";
        let got = run(src, default_cfg());
        assert_eq!(got.block_body.len(), 1);
        assert_eq!(got.block_body[0].start_offset, src.find("\n\n").unwrap() + 1);
        assert!(got.block_body[0].message.contains("beginning"));
    }

    // Stock 1.92.1 (rubocop#15814): the blank line after a block body's last
    // keyword, followed only by blank lines up to the block end, is flagged as a
    // candidate for the block body cop; a def body never is.
    #[test]
    fn ehk_after_keyword_deferred_only_in_block() {
        let block = run("foo do\n  bar\nrescue\n\nend\n", default_cfg());
        assert_eq!(block.exception_keywords.len(), 1);
        assert!(block.exception_keywords[0].block_body_deferred);
        let def = run("def m\n  bar\nrescue\n\nend\n", default_cfg());
        assert_eq!(def.exception_keywords.len(), 1);
        assert!(!def.exception_keywords[0].block_body_deferred);
    }

    fn shape(v: &[EmptyLineOffense]) -> Vec<(usize, usize, bool, String)> {
        v.iter()
            .map(|o| (o.start_offset, o.end_offset, o.insert, o.message.clone()))
            .collect()
    }

    // Typical: a method body starting with a blank line.
    #[test]
    fn method_body_blank_at_beginning() {
        let got = run("def m\n\n  x\nend\n", default_cfg());
        assert_eq!(
            shape(&got.method_body),
            vec![(
                6,
                7,
                false,
                "Extra empty line detected at method body beginning.".to_string()
            )]
        );
    }

    // Typical: a method body ending with a blank line.
    #[test]
    fn method_body_blank_at_end() {
        let got = run("def m\n  x\n\nend\n", default_cfg());
        assert_eq!(
            shape(&got.method_body),
            vec![(
                10,
                11,
                false,
                "Extra empty line detected at method body end.".to_string()
            )]
        );
    }

    // Whitespace-only lines are not "empty".
    #[test]
    fn method_body_spaces_line_is_not_empty() {
        let got = run("def m\n  \n  x\nend\n", default_cfg());
        assert!(got.method_body.is_empty());
    }

    // Multi-line argument list: the body check starts after the `)` line.
    #[test]
    fn method_body_multiline_arguments() {
        let src = "def m(\n  a\n)\n\n  x\nend\n";
        let got = run(src, default_cfg());
        assert_eq!(got.method_body.len(), 1);
        assert_eq!(got.method_body[0].start_offset, 13);
    }

    // Endless method with an extra line after `=`.
    #[test]
    fn endless_method_extra_line() {
        let src = "def foo(a) =\n\n  quux\n";
        let got = run(src, default_cfg());
        assert_eq!(
            shape(&got.method_body),
            vec![(
                13,
                14,
                false,
                "Extra empty line detected at method body beginning.".to_string()
            )]
        );
    }

    // Endless method whose next line is a comment: no offense.
    #[test]
    fn endless_method_comment_line() {
        let got = run("def foo(a) =\n  # c\n  quux\n", default_cfg());
        assert!(got.method_body.is_empty());
    }

    // Class body: both blanks flagged (the wrapper-side dedup is range-based;
    // here both records are emitted in stock order).
    #[test]
    fn class_body_blanks() {
        let got = run("class C\n\n  x\n\nend\n", default_cfg());
        let msgs: Vec<&str> = got.class_body.iter().map(|o| o.message.as_str()).collect();
        assert_eq!(
            msgs,
            vec![
                "Extra empty line detected at class body beginning.",
                "Extra empty line detected at class body end."
            ]
        );
    }

    // Empty class body with one blank line: beginning first, then end (same
    // range — stock's add_offense dedup keeps only the first).
    #[test]
    fn class_empty_body_blank() {
        let got = run("class C\n\nend\n", default_cfg());
        assert_eq!(got.class_body.len(), 2);
        assert_eq!(
            got.class_body[0].start_offset,
            got.class_body[1].start_offset
        );
        assert!(got.class_body[0].message.contains("beginning"));
    }

    // Superclass spanning multiple lines adjusts the first line.
    #[test]
    fn class_multiline_superclass() {
        let src = "class C < Struct.new(\n  :a\n)\n\n  x\nend\n";
        let got = run(src, default_cfg());
        assert_eq!(got.class_body.len(), 1);
        assert!(got.class_body[0].message.contains("beginning"));
    }

    // empty_lines style: missing blanks at both ends.
    #[test]
    fn class_empty_lines_style_missing() {
        let got = run(
            "class C\n  x\nend\n",
            cfg(EMPTY_LINES, NO_EMPTY_LINES, NO_EMPTY_LINES),
        );
        assert_eq!(
            shape(&got.class_body),
            vec![
                (
                    8,
                    9,
                    true,
                    "Empty line missing at class body beginning.".to_string()
                ),
                (
                    12,
                    13,
                    true,
                    "Empty line missing at class body end.".to_string()
                ),
            ]
        );
    }

    // empty_lines style ignores classes with an empty body.
    #[test]
    fn class_empty_lines_style_nil_body() {
        let got = run(
            "class C\nend\n",
            cfg(EMPTY_LINES, NO_EMPTY_LINES, NO_EMPTY_LINES),
        );
        assert!(got.class_body.is_empty());
    }

    // except_namespace: a single class child makes the outer a namespace.
    #[test]
    fn class_except_namespace() {
        let src = "class P\n\n  class C\n\n    x\n\n  end\nend\n";
        let got = run(src, cfg(EMPTY_LINES_EXCEPT_NAMESPACE, 0, 0));
        // P is a namespace (no empty lines wanted): blank after `class P`.
        let msgs: Vec<&str> = got.class_body.iter().map(|o| o.message.as_str()).collect();
        assert_eq!(
            msgs,
            vec!["Extra empty line detected at class body beginning."]
        );
    }

    // empty_lines_special: first child a method requires a leading blank.
    #[test]
    fn class_special_method_first() {
        let src = "class C\n  def m; end\nend\n";
        let got = run(src, cfg(EMPTY_LINES_SPECIAL, 0, 0));
        let msgs: Vec<&str> = got.class_body.iter().map(|o| o.message.as_str()).collect();
        assert_eq!(
            msgs,
            vec![
                "Empty line missing at class body beginning.",
                "Empty line missing at class body end."
            ]
        );
        // missing-at-end uses offset 2: the range is on the `end` line.
        assert_eq!(got.class_body[1].start_offset, 21);
    }

    // empty_lines_special: non-method first child defers the blank to the
    // first definition, skipping comments.
    #[test]
    fn class_special_deferred_with_comment() {
        let src = "class C\n  include X\n  # c\n  def m; end\n\nend\n";
        let got = run(src, cfg(EMPTY_LINES_SPECIAL, 0, 0));
        let msgs: Vec<&str> = got.class_body.iter().map(|o| o.message.as_str()).collect();
        assert_eq!(msgs, vec!["Empty line missing before first def definition"]);
        // Inserted before the comment line (line 3, byte 20): the comment
        // stays attached to the def.
        assert_eq!(got.class_body[0].start_offset, 20);
        assert!(got.class_body[0].insert);
    }

    // empty_lines_special: `def self.m` reports "defs", a bare access
    // modifier reports "send".
    #[test]
    fn class_special_deferred_type_names() {
        let got = run(
            "class C\n  include X\n  def self.m; end\n\nend\n",
            cfg(EMPTY_LINES_SPECIAL, 0, 0),
        );
        assert!(got.class_body[0].message.contains("first defs definition"));
        let got = run(
            "class C\n  include X\n  private\n\nend\n",
            cfg(EMPTY_LINES_SPECIAL, 0, 0),
        );
        assert!(got.class_body[0].message.contains("first send definition"));
    }

    // `private :foo` is not a bare modifier: deferral targets the def.
    #[test]
    fn class_special_modifier_with_argument_not_required() {
        let got = run(
            "class C\n  private :foo\n  def m; end\n\nend\n",
            cfg(EMPTY_LINES_SPECIAL, 0, 0),
        );
        assert!(got.class_body[0].message.contains("first def definition"));
    }

    // beginning_only / ending_only.
    #[test]
    fn class_beginning_only_flags_end_blank() {
        let got = run("class C\n\n  x\n\nend\n", cfg(BEGINNING_ONLY, 0, 0));
        let msgs: Vec<&str> = got.class_body.iter().map(|o| o.message.as_str()).collect();
        assert_eq!(msgs, vec!["Extra empty line detected at class body end."]);
    }

    #[test]
    fn class_ending_only_flags_beginning_blank() {
        let got = run("class C\n\n  x\n\nend\n", cfg(ENDING_ONLY, 0, 0));
        let msgs: Vec<&str> = got.class_body.iter().map(|o| o.message.as_str()).collect();
        assert_eq!(
            msgs,
            vec!["Extra empty line detected at class body beginning."]
        );
    }

    // Singleton class uses the class style and kind.
    #[test]
    fn sclass_body() {
        let got = run("class << self\n\n  x\nend\n", default_cfg());
        assert_eq!(got.class_body.len(), 1);
        assert!(got.class_body[0].message.contains("class body beginning"));
    }

    // Module body.
    #[test]
    fn module_body_blank() {
        let got = run("module M\n\n  x\nend\n", default_cfg());
        assert_eq!(got.module_body.len(), 1);
        assert!(got.module_body[0].message.contains("module body beginning"));
    }

    // Block body: do..end, braces, numbered, super and lambda forms.
    #[test]
    fn block_body_blank() {
        let got = run("foo do\n\n  x\nend\n", default_cfg());
        assert_eq!(got.block_body.len(), 1);
        let got = run("foo {\n\n  x\n}\n", default_cfg());
        assert_eq!(got.block_body.len(), 1);
        let got = run("foo {\n  _1\n\n}\n", default_cfg());
        assert!(got.block_body[0].message.contains("block body end"));
        let got = run("def m\n  super do\n\n    x\n  end\nend\n", default_cfg());
        assert_eq!(got.block_body.len(), 1);
        let got = run("-> do\n\n  x\nend\n", default_cfg());
        assert_eq!(got.block_body.len(), 1);
    }

    // BlockNode#single_line? compares the DELIMITER lines: a one-line
    // `{ ... }` hanging off a multiline chain is skipped even though the
    // node spans several lines (mastodon corpus regression).
    #[test]
    fn block_single_line_braces_on_multiline_chain() {
        let src = "allow(foo)\n  .to receive(:call)\n    .with(x) { fab }\n\nother\n";
        let got = run(src, default_cfg());
        assert!(got.block_body.is_empty());
    }

    // Block body check starts after the call part of a multi-line call.
    #[test]
    fn block_body_multiline_send() {
        let src = "foo a,\n  b do\n\n  x\nend\n";
        let got = run(src, default_cfg());
        assert_eq!(got.block_body.len(), 1);
        assert!(got.block_body[0].message.contains("beginning"));
    }

    // Block empty_lines style.
    #[test]
    fn block_empty_lines_style() {
        let got = run("foo do\n  x\nend\n", cfg(0, 0, EMPTY_LINES));
        assert_eq!(got.block_body.len(), 2);
        assert!(got.block_body.iter().all(|o| o.insert));
        // Empty block bodies are ignored under empty_lines.
        let got = run("foo do\nend\n", cfg(0, 0, EMPTY_LINES));
        assert!(got.block_body.is_empty());
    }

    // Begin body: blanks after `begin` and before `end`, regardless of
    // rescue/ensure sections.
    #[test]
    fn begin_body_blanks() {
        let got = run("begin\n\n  x\nrescue\n  y\n\nend\n", default_cfg());
        let msgs: Vec<&str> = got.begin_body.iter().map(|o| o.message.as_str()).collect();
        assert_eq!(
            msgs,
            vec![
                "Extra empty line detected at `begin` body beginning.",
                "Extra empty line detected at `begin` body end."
            ]
        );
    }

    // EHK: blanks around rescue/else/ensure keywords, in stock's
    // [ensure, else, rescue...] emission order.
    #[test]
    fn ehk_full_set() {
        let src = "begin\n  a\n\nrescue\n\n  b\n\nelse\n\n  c\n\nensure\n\n  d\nend\n";
        let got = run(src, default_cfg());
        let msgs: Vec<&str> = got
            .exception_keywords
            .iter()
            .map(|o| o.message.as_str())
            .collect();
        assert_eq!(
            msgs,
            vec![
                "Extra empty line detected after the `ensure`.",
                "Extra empty line detected before the `ensure`.",
                "Extra empty line detected after the `else`.",
                "Extra empty line detected before the `else`.",
                "Extra empty line detected after the `rescue`.",
                "Extra empty line detected before the `rescue`.",
            ]
        );
    }

    // EHK skip: last body line and `end` on the same line.
    #[test]
    fn ehk_skips_when_end_shares_last_body_line() {
        let got = run("def m\n  x\n\nensure end\n", default_cfg());
        assert!(got.exception_keywords.is_empty());
        let got = run(
            "begin\n  foo\n\nrescue => x\nrescue => y; end\n",
            default_cfg(),
        );
        assert!(got.exception_keywords.is_empty());
    }

    // EHK: ensure with empty body but `end` on its own line is checked.
    #[test]
    fn ehk_ensure_empty_body_separate_end() {
        let got = run("def m\n  x\n\nensure\nend\n", default_cfg());
        let msgs: Vec<&str> = got
            .exception_keywords
            .iter()
            .map(|o| o.message.as_str())
            .collect();
        assert_eq!(msgs, vec!["Extra empty line detected before the `ensure`."]);
    }

    // EHK skip: keyword on the def/begin line itself.
    #[test]
    fn ehk_keyword_on_node_line() {
        let got = run("begin; x\nrescue; y\nend\n", default_cfg());
        assert!(got.exception_keywords.is_empty());
    }

    // EHK in blocks (incl. numbered) but not it-blocks.
    #[test]
    fn ehk_blocks_numbered_and_it() {
        let got = run("foo do\n  f(_1)\n\nrescue\n  y\nend\n", default_cfg());
        assert_eq!(got.exception_keywords.len(), 1);
        let got = run("foo do\n  f(it)\n\nrescue\n  y\nend\n", default_cfg());
        assert_eq!(got.exception_keywords.len(), 1);
        let got = run("-> do\n  x\n\nrescue\n  y\nend\n", default_cfg());
        assert_eq!(got.exception_keywords.len(), 1);
    }

    // EHK: a def body that is a modifier rescue still has a resbody keyword.
    #[test]
    fn ehk_modifier_rescue_in_def() {
        let got = run("def foo\n\n  x rescue y\nend\n", default_cfg());
        let msgs: Vec<&str> = got
            .exception_keywords
            .iter()
            .map(|o| o.message.as_str())
            .collect();
        assert_eq!(msgs, vec!["Extra empty line detected before the `rescue`."]);
    }

    // EHK: a kwbegin checks a modifier rescue as its *first statement* (even
    // among several), while def/block bodies only when it is the sole one.
    #[test]
    fn ehk_modifier_rescue_first_statement() {
        let got = run("begin\n  x rescue y\n\n  z\nend\n", default_cfg());
        assert_eq!(got.exception_keywords.len(), 1);
        assert!(got.exception_keywords[0].message.contains("after"));
        let got = run("begin\n  a\n\n  x rescue y\nend\n", default_cfg());
        assert!(got.exception_keywords.is_empty());
        let got = run("def m\n\n  x rescue y\n  z\nend\n", default_cfg());
        assert!(got.exception_keywords.is_empty());
        let got = run("foo do\n\n  x rescue y\nend\n", default_cfg());
        assert_eq!(got.exception_keywords.len(), 1);
    }

    // EHK: blank line sandwiched between two rescue keywords emits both
    // (after first, before second) at the same range; the wrapper-side
    // dedup keeps the first, like stock.
    #[test]
    fn ehk_blank_between_rescue_keywords() {
        let got = run(
            "begin\n  x\nrescue A\n\nrescue B\n  y\nend\n",
            default_cfg(),
        );
        assert_eq!(got.exception_keywords.len(), 2);
        assert_eq!(
            got.exception_keywords[0].start_offset,
            got.exception_keywords[1].start_offset
        );
        assert!(got.exception_keywords[0].message.contains("after"));
    }

    // Single-line nodes are ignored everywhere.
    #[test]
    fn single_line_nodes() {
        let got = run(
            "def m; x; end\nclass C; end\nfoo { x }\nbegin; x; end\n",
            default_cfg(),
        );
        assert!(got.method_body.is_empty());
        assert!(got.class_body.is_empty());
        assert!(got.block_body.is_empty());
        assert!(got.begin_body.is_empty());
    }

    // Multibyte first character: the offense range ends on the next char
    // boundary (1 character, not 1 byte).
    #[test]
    fn multibyte_first_char_range() {
        let src = "class C\n  x\nend\n";
        let got = run(src, cfg(EMPTY_LINES, 0, 0));
        // sanity on the ASCII case first
        assert_eq!(
            got.class_body[0].end_offset - got.class_body[0].start_offset,
            1
        );
        let src = "class C\n„x\nend\n";
        let got = run(src, cfg(EMPTY_LINES, 0, 0));
        assert_eq!(
            got.class_body[0].end_offset - got.class_body[0].start_offset,
            3
        );
    }
}
