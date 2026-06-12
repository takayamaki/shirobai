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
//! - `EmptyLinesAroundExceptionHandlingKeywords` aliases `on_block` /
//!   `on_numblock` but **not** `on_itblock` (stock quirk), so a block whose
//!   parameter is `it` is skipped — mirrored via `ItParametersNode`.

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
pub struct EmptyLineOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub insert: bool,
    pub message: String,
}

/// The per-cop `EnforcedStyle`s (the other three cops have a fixed style).
#[derive(Clone, Copy)]
pub struct Config {
    pub class_style: u8,
    pub module_style: u8,
    pub block_style: u8,
}

/// All six cops' offenses for one source.
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
    /// `empty_line_required?`: `{any_def class module (send nil? {:private
    /// :protected :public})}` (the send arm is the *bare* modifier: no
    /// receiver, no arguments, no block).
    required: bool,
    /// 1-based first line (for `check_deferred_empty_line`).
    first_line: usize,
    /// Parser node type interpolated into `MSG_DEFERRED`.
    type_name: &'static str,
}

/// Parser-equivalent body of a class/module: `None` is a nil body; `is_begin`
/// is `body.begin_type?` (more than one statement).
struct BodyInfo {
    is_begin: bool,
    children: Vec<ChildInfo>,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    /// Byte offset of the start of each line (`line_starts[0] == 0`).
    line_starts: Vec<usize>,
    cfg: Config,
    out: FamilyOffenses,
}

impl super::dispatch::Rule for Visitor<'_> {
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
            // parser `send_node.last_line`: the call part ends at the argument
            // list's closing paren, the last argument, or the message.
            let send_last = if let Some(close) = c.closing_loc() {
                self.line_of(close.start_offset())
            } else if let Some(args) = c.arguments() {
                self.line_of(args.location().end_offset())
            } else if let Some(msg) = c.message_loc() {
                self.line_of(msg.end_offset())
            } else {
                self.line_of(c.location().start_offset())
            };
            self.on_block(node, &bn, send_last);
        } else if let Some(s) = node.as_super_node() {
            let Some(block) = s.block() else { return };
            let Some(bn) = block.as_block_node() else {
                return;
            };
            let send_last = if let Some(rp) = s.rparen_loc() {
                self.line_of(rp.start_offset())
            } else if let Some(args) = s.arguments() {
                self.line_of(args.location().end_offset())
            } else {
                self.line_of(s.keyword_loc().end_offset())
            };
            self.on_block(node, &bn, send_last);
        } else if let Some(s) = node.as_forwarding_super_node() {
            let Some(bn) = s.block() else { return };
            // Bare `super` is a single keyword: the call part ends where the
            // node starts.
            let send_last = self.line_of(s.location().start_offset());
            self.on_block(node, &bn, send_last);
        } else if let Some(l) = node.as_lambda_node() {
            // parser: `-> { }` is `(block (lambda) ...)`; `send_node` is the
            // bare `->` operator.
            let send_last = self.line_of(l.operator_loc().start_offset());
            let first = self.line_of(l.location().start_offset());
            let open_line = self.line_of(l.opening_loc().start_offset());
            let close_line = self.line_of(l.closing_loc().start_offset());
            let body_nil = l.body().is_none();
            self.check_block_body(open_line, close_line, send_last, body_nil);
            if !is_it_parameters(&l.parameters()) {
                self.check_ehk_body(l.body(), first, Some(close_line));
            }
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
            d.body().is_none(),
        );
        self.check_ehk_body(d.body(), first, Some(end_line));
    }

    /// Shared `on_block` for call/super blocks: `EmptyLinesAroundBlockBody`
    /// plus EHK (which skips `it`-parameter blocks; stock has no
    /// `on_itblock` alias).
    fn on_block(&mut self, node: &Node<'_>, bn: &ruby_prism::BlockNode<'_>, send_last: usize) {
        let first = self.line_of(node.location().start_offset());
        let open_line = self.line_of(bn.opening_loc().start_offset());
        let close_line = self.line_of(bn.closing_loc().start_offset());
        self.check_block_body(open_line, close_line, send_last, bn.body().is_none());
        if !is_it_parameters(&bn.parameters()) {
            self.check_ehk_body(bn.body(), first, Some(close_line));
        }
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
        send_last: usize,
        body_nil: bool,
    ) {
        self.check_simple(
            Kind::Block,
            self.cfg.block_style,
            open_line,
            close_line,
            Some(send_last),
            body_nil,
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
        body_nil: bool,
    ) {
        if body_nil && style != NO_EMPTY_LINES {
            return; // valid_body_style?
        }
        if node_first == node_last {
            return; // single_line?
        }
        let first = adjusted.unwrap_or(node_first);
        self.check_both(kind, style, first, node_last);
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
        match style {
            EMPTY_LINES_EXCEPT_NAMESPACE => {
                // body is Some here (nil bodies returned above).
                let ns = body.as_ref().is_some_and(namespace_with_one_child);
                let inner = if ns { NO_EMPTY_LINES } else { EMPTY_LINES };
                self.check_both(kind, inner, first, last);
            }
            EMPTY_LINES_SPECIAL => {
                let Some(body) = body else { return };
                if namespace_with_one_child(body) {
                    self.check_both(kind, NO_EMPTY_LINES, first, last);
                } else {
                    if body.children.first().is_some_and(|c| c.required) {
                        self.check_beginning(kind, EMPTY_LINES, first);
                    } else {
                        self.check_beginning(kind, NO_EMPTY_LINES, first);
                        self.check_deferred(kind, body);
                    }
                    self.check_ending(kind, EMPTY_LINES, last);
                }
            }
            _ => self.check_both(kind, style, first, last),
        }
    }

    fn check_both(&mut self, kind: Kind, style: u8, first: usize, last: usize) {
        match style {
            BEGINNING_ONLY => {
                self.check_beginning(kind, EMPTY_LINES, first);
                self.check_ending(kind, NO_EMPTY_LINES, last);
            }
            ENDING_ONLY => {
                self.check_beginning(kind, NO_EMPTY_LINES, first);
                self.check_ending(kind, EMPTY_LINES, last);
            }
            _ => {
                self.check_beginning(kind, style, first);
                self.check_ending(kind, style, last);
            }
        }
    }

    fn check_beginning(&mut self, kind: Kind, style: u8, first: usize) {
        self.check_source(kind, style, first, "beginning");
    }

    fn check_ending(&mut self, kind: Kind, style: u8, last: usize) {
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
        let Some(child) = body.children.iter().find(|c| c.required) else {
            return;
        };
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
            self.push_ehk(line + 1, ehk_msg("after", kw));
        }
        // `lines[line - 2]`: the line above the keyword.
        if line >= 2 && self.line_empty0(line - 2) {
            self.push_ehk(line - 1, ehk_msg("before", kw));
        }
    }

    fn push_ehk(&mut self, line1: usize, message: String) {
        let Some((start, end)) = self.line_head_range(line1) else {
            return;
        };
        self.out.exception_keywords.push(EmptyLineOffense {
            start_offset: start,
            end_offset: end,
            insert: false,
            message,
        });
    }

    /// Parser-equivalent body info for a class/module body (always a
    /// `StatementsNode` or absent in prism).
    fn body_info(&self, body: Option<Node<'_>>) -> Option<BodyInfo> {
        let body = body?;
        let Some(stmts) = body.as_statements_node() else {
            // Defensive: class/module bodies are StatementsNodes; treat
            // anything else as a single opaque child.
            return Some(BodyInfo {
                is_begin: false,
                children: vec![self.child_info(&body)],
            });
        };
        let children: Vec<ChildInfo> = stmts.body().iter().map(|n| self.child_info(&n)).collect();
        if children.is_empty() {
            return None;
        }
        Some(BodyInfo {
            is_begin: children.len() > 1,
            children,
        })
    }

    fn child_info(&self, node: &Node<'_>) -> ChildInfo {
        let first_line = self.line_of(node.location().start_offset());
        if let Some(d) = node.as_def_node() {
            return ChildInfo {
                is_constant_def: false,
                required: true, // any_def
                first_line,
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
                type_name: "class",
            };
        }
        if node.as_module_node().is_some() {
            return ChildInfo {
                is_constant_def: true,
                required: true,
                first_line,
                type_name: "module",
            };
        }
        // `(send nil? {:private :protected :public})`: bare modifier — no
        // receiver, no arguments, no block (a block makes the parser node a
        // `block`, not a `send`).
        if let Some(c) = node.as_call_node() {
            let bare = c.receiver().is_none()
                && c.arguments().is_none()
                && c.block().is_none()
                && matches!(c.name().as_slice(), b"private" | b"protected" | b"public");
            if bare {
                return ChildInfo {
                    is_constant_def: false,
                    required: true,
                    first_line,
                    type_name: "send",
                };
            }
        }
        ChildInfo {
            is_constant_def: false,
            required: false,
            first_line,
            type_name: "",
        }
    }
}

/// `namespace?(body, with_one_child: true)`: a single class/module child.
fn namespace_with_one_child(body: &BodyInfo) -> bool {
    !body.is_begin && body.children.first().is_some_and(|c| c.is_constant_def)
}

/// EHK's missing `on_itblock` alias: skip blocks whose parameter is `it`.
fn is_it_parameters(parameters: &Option<Node<'_>>) -> bool {
    parameters
        .as_ref()
        .is_some_and(|p| p.as_it_parameters_node().is_some())
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
    fn ehk_blocks_and_itblock_quirk() {
        let got = run("foo do\n  f(_1)\n\nrescue\n  y\nend\n", default_cfg());
        assert_eq!(got.exception_keywords.len(), 1);
        let got = run("foo do\n  f(it)\n\nrescue\n  y\nend\n", default_cfg());
        assert!(got.exception_keywords.is_empty());
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
