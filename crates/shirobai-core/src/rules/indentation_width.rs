//! `Layout/IndentationWidth`.
//!
//! Checks that indentation uses the configured number of spaces (default 2).
//! Rust walks the AST, decides the base location for every indentable body
//! (def/class/module/if/case/while/for/block/rescue/ensure/begin/assignment),
//! computes `column_offset_between(body, base)` and the resulting `column_delta`,
//! produces the offense range plus the message; Ruby applies the realignment via
//! `AlignmentCorrector` (same division of labour as the other indentation cops).
//!
//! The cop logic upstream is written against the parser-gem AST; here it is
//! reconstructed over Prism. Block structure differs between the two parsers, so
//! block / method-chain handling is done explicitly rather than by 1:1 node
//! translation.

use ruby_prism::{Location, Node};

/// One indentation offense. `[start_offset, end_offset)` is the offense range
/// (`offending_range`). `[correct_start, correct_end)` is the node range that
/// Ruby realigns by `column_delta`. `autocorrect` is false when the correction
/// would overlap an already-registered correction range (`other_offense_in_same_range?`).
pub struct IndentationOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub message: String,
    pub autocorrect: bool,
    pub correct_start: usize,
    pub correct_end: usize,
}

#[derive(Clone, Copy)]
pub struct Config {
    pub width: usize,
    /// `EnforcedStyleAlignWith`: false = start_of_line, true = relative_to_receiver.
    pub relative_to_receiver: bool,
    /// `Layout/AccessModifierIndentation` EnforcedStyle == 'outdent'.
    pub access_modifier_outdent: bool,
    /// `Layout/IndentationConsistency` EnforcedStyle == 'indented_internal_methods'.
    pub indented_internal_methods: bool,
    /// `Layout/EndAlignment` EnforcedStyleAlignWith: 0 keyword, 1 variable, 2 start_of_line.
    pub end_align: u8,
    /// `Layout/DefEndAlignment` EnforcedStyleAlignWith: 0 start_of_line, 1 def.
    pub def_end_align_def: bool,
    /// `Layout/IndentationStyle` EnforcedStyle == 'tabs'.
    pub use_tabs: bool,
}

const SPECIAL_MODIFIERS: [&[u8]; 2] = [b"private", b"protected"];

pub fn check_indentation_width(
    source: &[u8],
    config: Config,
    allowed_lines: &[usize],
) -> Vec<IndentationOffense> {
    let bom = source.starts_with(&[0xef, 0xbb, 0xbf]);
    let mut rule = Visitor {
        source,
        config,
        allowed_lines,
        bom,
        comment_lines: comment_lines(source),
        offense_ranges: Vec::new(),
        offenses: Vec::new(),
        ignored: Vec::new(),
    };
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

struct Visitor<'a> {
    source: &'a [u8],
    config: Config,
    allowed_lines: &'a [usize],
    bom: bool,
    #[allow(dead_code)] // used by the relative_to_receiver stage
    comment_lines: Vec<usize>,
    /// Registered correction ranges, for `other_offense_in_same_range?`.
    offense_ranges: Vec<(usize, usize)>,
    offenses: Vec<IndentationOffense>,
    /// Node ranges that have been `ignore_node`'d (assignment rhs, def under modifier).
    #[allow(dead_code)] // used by the assignment / def-modifier stages
    ignored: Vec<(usize, usize)>,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

/// 1-based line numbers of comments that begin their line.
fn comment_lines(source: &[u8]) -> Vec<usize> {
    let result = ruby_prism::parse(source);
    let mut lines = Vec::new();
    for c in result.comments() {
        let l = c.location();
        let start = l.start_offset();
        let line_start = match source[..start].iter().rposition(|&b| b == b'\n') {
            Some(i) => i + 1,
            None => 0,
        };
        if source[line_start..start]
            .iter()
            .all(|&b| b == b' ' || b == b'\t')
        {
            let line = source[..start].iter().filter(|&&b| b == b'\n').count() + 1;
            lines.push(line);
        }
    }
    lines
}

impl<'a> Visitor<'a> {
    fn line_start(&self, off: usize) -> usize {
        match self.source[..off].iter().rposition(|&b| b == b'\n') {
            Some(i) => i + 1,
            None => 0,
        }
    }

    /// 1-based line number of `off`.
    fn line_of(&self, off: usize) -> usize {
        self.source[..off].iter().filter(|&&b| b == b'\n').count() + 1
    }

    /// `range.column`: number of characters from the line start to `off`,
    /// adjusted for a byte-order mark on line 1 (`effective_column`).
    fn column(&self, off: usize) -> isize {
        let ls = self.line_start(off);
        let chars = std::str::from_utf8(&self.source[ls..off])
            .map(|s| s.chars().count())
            .unwrap_or(off - ls);
        let line = self.line_of(off);
        if line == 1 && self.bom {
            // BOM is one codepoint at the very start; effective_column subtracts 1.
            chars as isize - 1
        } else {
            chars as isize
        }
    }

    /// `line_indentation(range)`: the leading whitespace string of `off`'s line.
    fn line_indentation(&self, off: usize) -> &'a [u8] {
        let ls = self.line_start(off);
        let mut end = ls;
        while end < self.source.len()
            && (self.source[end] == b' ' || self.source[end] == b'\t')
            && end < off
        {
            end += 1;
        }
        &self.source[ls..end]
    }

    fn line_uses_tabs(&self, off: usize) -> bool {
        self.line_indentation(off).contains(&b'\t')
    }

    /// `visual_column(range)`: tab_count*width + space_count of the line indent.
    fn visual_column(&self, off: usize) -> isize {
        let ind = self.line_indentation(off);
        let tabs = ind.iter().filter(|&&b| b == b'\t').count();
        let spaces = ind.iter().filter(|&&b| b == b' ').count();
        (tabs * self.config.width + spaces) as isize
    }

    /// `column_offset_between(body, base)` with the cop's tab override.
    fn column_offset_between(&self, body_off: usize, base_off: usize) -> isize {
        if self.config.use_tabs {
            let base_tabs = self.line_uses_tabs(base_off);
            let body_tabs = self.line_uses_tabs(body_off);
            if base_tabs || body_tabs {
                return self.visual_column(base_off) - self.visual_column(body_off);
            }
        }
        self.column(body_off) - self.column(base_off)
    }

    /// `begins_its_line?(loc)`: only whitespace precedes `off` on its line.
    fn begins_its_line(&self, off: usize) -> bool {
        let ls = self.line_start(off);
        self.source[ls..off]
            .iter()
            .all(|&b| b == b' ' || b == b'\t')
    }

    fn same_line(&self, a: usize, b: usize) -> bool {
        self.line_of(a) == self.line_of(b)
    }

    #[allow(dead_code)] // used by the relative_to_receiver / assignment stages
    fn text(&self, s: usize, e: usize) -> &'a str {
        std::str::from_utf8(&self.source[s..e]).unwrap_or("")
    }

    /// The source line text (without trailing newline) for the line containing `off`.
    fn source_line(&self, off: usize) -> &'a [u8] {
        let ls = self.line_start(off);
        let end = self.source[ls..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| ls + p)
            .unwrap_or(self.source.len());
        &self.source[ls..end]
    }

    /// First non-whitespace character offset on the line containing `off`,
    /// as a column (char index). `nil` -> None (blank line).
    fn first_non_ws_column(&self, off: usize) -> Option<usize> {
        let line = self.source_line(off);
        line.iter().position(|&b| b != b' ' && b != b'\t')
    }

    #[allow(dead_code)] // used by the assignment / def-modifier stages
    fn is_ignored(&self, range: (usize, usize)) -> bool {
        self.ignored.contains(&range)
    }

    /// `allowed_line?(base_loc)`.
    fn allowed_line(&self, base_off: usize) -> bool {
        self.allowed_lines.contains(&self.line_of(base_off))
    }

    /// The core `check_indentation(base_loc, body_node, style)`.
    /// `body` is `(start, end)`; `body_kind` describes special body handling.
    fn check_indentation(&mut self, base_off: usize, body: Option<BodyRef>, style_internal: bool) {
        let Some(body) = body else { return };
        if !self.indentation_to_check(base_off, &body) {
            return;
        }
        let body_start = body.start;
        let indentation = self.column_offset_between(body_start, base_off);
        let column_delta = self.config.width as isize - indentation;
        if column_delta == 0 {
            return;
        }
        self.offense(body, indentation, column_delta, style_internal);
    }

    /// `indentation_to_check?` + `skip_check?`.
    fn indentation_to_check(&self, base_off: usize, body: &BodyRef) -> bool {
        if self.skip_check(base_off, body) {
            return false;
        }
        // rescue/ensure body emptiness is handled by the caller producing the
        // right BodyRef; here body is always a real statement.
        true
    }

    fn skip_check(&self, base_off: usize, body: &BodyRef) -> bool {
        if self.allowed_line(base_off) {
            return true;
        }
        // same_line?(body, base): body begins on the same line as base.
        if self.same_line(body.start, base_off) {
            return true;
        }
        if body.starts_with_access_modifier {
            return true;
        }
        // line doesn't start with the body.
        match self.first_non_ws_column(body.start) {
            Some(c) => self.column(body.start) as usize != c,
            None => true,
        }
    }

    fn offense(
        &mut self,
        body: BodyRef,
        indentation: isize,
        column_delta: isize,
        style_internal: bool,
    ) {
        // offense corrects the first statement in a begin body.
        let correct = body.correct_range;

        let within_prior = self
            .offense_ranges
            .iter()
            .any(|&(s, e)| correct.0 >= s && correct.1 <= e);
        let autocorrect = !within_prior;
        if !within_prior {
            self.offense_ranges.push(correct);
        }

        let name = if style_internal {
            " indented_internal_methods"
        } else {
            ""
        };
        let message = self.message(indentation, name);

        let off = self.offending_range(body.start, indentation);
        self.offenses.push(IndentationOffense {
            start_offset: off.0,
            end_offset: off.1,
            column_delta,
            message,
            autocorrect,
            correct_start: correct.0,
            correct_end: correct.1,
        });
    }

    fn message(&self, indentation: isize, name: &str) -> String {
        if self.config.use_tabs {
            let configured = 1;
            let actual = indentation / self.config.width as isize;
            format!("Use {configured} (not {actual}) tabs for{name} indentation.")
        } else {
            format!(
                "Use {} (not {indentation}) spaces for{name} indentation.",
                self.config.width
            )
        }
    }

    /// `offending_range(body_node, indentation)`.
    fn offending_range(&self, body_start: usize, indentation: isize) -> (usize, usize) {
        let begin_pos = body_start;
        let ind = if self.config.use_tabs {
            begin_pos - self.line_indentation(body_start).len()
        } else {
            (begin_pos as isize - indentation) as usize
        };
        if indentation >= 0 {
            (ind, begin_pos)
        } else {
            (begin_pos, ind)
        }
    }
}

/// A body to check: its first-statement start, the correction range, and flags.
struct BodyRef {
    /// `body_node.source_range.begin_pos` (the body's first line start position).
    start: usize,
    /// The node range Ruby realigns (`offense`'s `body_node`, the first stmt of a begin).
    correct_range: (usize, usize),
    starts_with_access_modifier: bool,
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        self.dispatch(node);
    }

    fn leave(&mut self) {}
}

impl<'a> Visitor<'a> {
    fn dispatch(&mut self, node: &Node<'_>) {
        if let Some(n) = node.as_def_node() {
            self.on_def(n.def_keyword_loc().start_offset(), n.body().as_ref());
        } else if let Some(n) = node.as_class_node() {
            self.on_class(n.class_keyword_loc().start_offset(), n.body().as_ref());
        } else if let Some(n) = node.as_module_node() {
            self.on_class(n.module_keyword_loc().start_offset(), n.body().as_ref());
        } else if let Some(n) = node.as_singleton_class_node() {
            self.on_class(n.class_keyword_loc().start_offset(), n.body().as_ref());
        } else if let Some(n) = node.as_if_node() {
            self.on_if_node(&n);
        } else if let Some(n) = node.as_unless_node() {
            self.on_unless_node(&n);
        } else if let Some(n) = node.as_while_node() {
            self.on_while_node(n.keyword_loc().start_offset(), n.statements().as_ref());
        } else if let Some(n) = node.as_until_node() {
            self.on_while_node(n.keyword_loc().start_offset(), n.statements().as_ref());
        } else if let Some(n) = node.as_for_node() {
            self.on_for_node(&n);
        } else if let Some(n) = node.as_case_node() {
            self.on_case_node(&n);
        } else if let Some(n) = node.as_case_match_node() {
            self.on_case_match_node(&n);
        } else if let Some(n) = node.as_block_node() {
            self.on_block_node(node, &n);
        } else if let Some(n) = node.as_begin_node() {
            self.on_begin_node(&n);
        }
    }

    /// First statement of a Prism body node (statements / begin), or the node
    /// itself. Returns `(begin_pos, full_range, is_begin_with_multiple)`.
    fn body_first_stmt(&self, body: &Node<'_>) -> (usize, (usize, usize)) {
        if let Some(st) = body.as_statements_node() {
            let stmts: Vec<_> = st.body().iter().collect();
            if let Some(first) = stmts.first() {
                let fl = first.location();
                return (fl.start_offset(), loc(&fl));
            }
            let l = st.as_node().location();
            return (l.start_offset(), loc(&l));
        }
        let l = body.location();
        (l.start_offset(), loc(&l))
    }

    /// Build a `BodyRef` for a body node. `correct_first_stmt` mirrors
    /// `offense`: a `begin`-type body corrects only its first statement.
    fn make_body(&self, body: &Node<'_>) -> BodyRef {
        let (start, _full) = self.body_first_stmt(body);
        // correction range: first statement of the body (begin -> first child).
        let correct_range = if let Some(st) = body.as_statements_node() {
            let stmts: Vec<_> = st.body().iter().collect();
            if let Some(first) = stmts.first() {
                loc(&first.location())
            } else {
                loc(&st.as_node().location())
            }
        } else {
            loc(&body.location())
        };
        let starts_with_access_modifier = self.body_starts_with_access_modifier(body);
        BodyRef {
            start,
            correct_range,
            starts_with_access_modifier,
        }
    }

    fn body_starts_with_access_modifier(&self, body: &Node<'_>) -> bool {
        if let Some(st) = body.as_statements_node()
            && let Some(first) = st.body().iter().next()
            && let Some(c) = first.as_call_node()
            && c.receiver().is_none()
            && c.arguments().is_none()
            && c.block().is_none()
        {
            return is_bare_access_modifier(c.name().as_slice());
        }
        false
    }

    fn on_def(&mut self, keyword_off: usize, body: Option<&Node<'_>>) {
        // `return if ignored_node?(node)` is handled at the modifier-send level.
        let Some(body) = body else { return };
        let bref = self.make_body(body);
        self.check_indentation(keyword_off, Some(bref), false);
    }

    fn on_class(&mut self, keyword_off: usize, body: Option<&Node<'_>>) {
        let Some(body) = body else { return };
        // return if same_line?(base, body)
        let (body_start, _) = self.body_first_stmt(body);
        if self.same_line(keyword_off, body_start) {
            return;
        }
        self.check_members(keyword_off, body);
    }

    fn on_if_node(&mut self, n: &ruby_prism::IfNode<'_>) {
        // ternary / modifier-form: an `if` without an `end` keyword.
        if n.end_keyword_loc().is_none() {
            return;
        }
        let Some(if_kw) = n.if_keyword_loc() else {
            return;
        };
        let base_off = if_kw.start_offset();
        self.check_if(
            base_off,
            n.statements().as_ref().map(|s| s.as_node()),
            n.subsequent().as_ref(),
        );
    }

    fn on_unless_node(&mut self, n: &ruby_prism::UnlessNode<'_>) {
        if n.end_keyword_loc().is_none() {
            return;
        }
        let base_off = n.keyword_loc().start_offset();
        let else_node = n.else_clause().map(|e| e.as_node());
        self.check_if(
            base_off,
            n.statements().as_ref().map(|s| s.as_node()),
            else_node.as_ref(),
        );
    }

    /// `check_if(node, body, else_clause, base_loc)`. `subsequent` is the
    /// Prism `else`/`elsif` node (an ElseNode or another IfNode).
    fn check_if(&mut self, base_off: usize, body: Option<Node<'_>>, subsequent: Option<&Node<'_>>) {
        if let Some(b) = &body {
            let bref = self.make_body(b);
            self.check_indentation(base_off, Some(bref), false);
        }
        let Some(subsequent) = subsequent else {
            return;
        };
        // If the subsequent is an elsif, it gets its own on_if call; skip.
        if subsequent.as_if_node().is_some() {
            return;
        }
        // Otherwise it is an else clause.
        let Some(els) = subsequent.as_else_node() else {
            return;
        };
        let else_kw = els.else_keyword_loc().start_offset();
        let Some(stmts) = els.statements() else {
            return;
        };
        let bref = self.make_body(&stmts.as_node());
        self.check_indentation(else_kw, Some(bref), false);
    }

    fn on_while_node(&mut self, keyword_off: usize, body: Option<&ruby_prism::StatementsNode<'_>>) {
        let Some(body) = body else { return };
        let bref = self.make_body(&body.as_node());
        self.check_indentation(keyword_off, Some(bref), false);
    }

    fn on_for_node(&mut self, n: &ruby_prism::ForNode<'_>) {
        let Some(body) = n.statements() else { return };
        let bref = self.make_body(&body.as_node());
        self.check_indentation(n.for_keyword_loc().start_offset(), Some(bref), false);
    }

    fn on_case_node(&mut self, n: &ruby_prism::CaseNode<'_>) {
        let conditions: Vec<_> = n.conditions().iter().collect();
        let mut last_when_kw = None;
        for cond in &conditions {
            if let Some(w) = cond.as_when_node() {
                let kw = w.keyword_loc().start_offset();
                last_when_kw = Some(kw);
                if let Some(body) = w.statements() {
                    let bref = self.make_body(&body.as_node());
                    self.check_indentation(kw, Some(bref), false);
                }
            }
        }
        if let (Some(kw), Some(els)) = (last_when_kw, n.else_clause())
            && let Some(body) = els.statements()
        {
            let bref = self.make_body(&body.as_node());
            self.check_indentation(kw, Some(bref), false);
        }
    }

    fn on_case_match_node(&mut self, n: &ruby_prism::CaseMatchNode<'_>) {
        let conditions: Vec<_> = n.conditions().iter().collect();
        let mut last_in_kw = None;
        for cond in &conditions {
            if let Some(inp) = cond.as_in_node() {
                let kw = inp.in_loc().start_offset();
                last_in_kw = Some(kw);
                if let Some(body) = inp.statements() {
                    let bref = self.make_body(&body.as_node());
                    self.check_indentation(kw, Some(bref), false);
                }
            }
        }
        if let (Some(kw), Some(els)) = (last_in_kw, n.else_clause())
            && let Some(body) = els.statements()
        {
            let bref = self.make_body(&body.as_node());
            self.check_indentation(kw, Some(bref), false);
        }
    }

    fn on_block_node(&mut self, _block: &Node<'_>, n: &ruby_prism::BlockNode<'_>) {
        let end_start = n.closing_loc().start_offset();
        if !self.begins_its_line(end_start) {
            return;
        }
        let Some(body) = n.body() else { return };
        let bref = self.make_body(&body);
        self.check_indentation(end_start, Some(bref), false);
    }

    fn on_begin_node(&mut self, n: &ruby_prism::BeginNode<'_>) {
        // rescue / ensure handling within a begin.
        if let Some(rescue) = n.rescue_clause() {
            self.on_rescue_chain(&rescue);
        }
        if let Some(ensure) = n.ensure_clause() {
            self.on_ensure(&ensure);
        }
    }

    fn on_rescue_chain(&mut self, rescue: &ruby_prism::RescueNode<'_>) {
        // on_resbody: check body against keyword.
        let kw = rescue.keyword_loc().start_offset();
        if let Some(body) = rescue.statements() {
            let bref = self.make_body(&body.as_node());
            self.check_indentation(kw, Some(bref), false);
        }
        if let Some(next) = rescue.subsequent() {
            self.on_rescue_chain(&next);
        }
    }

    fn on_ensure(&mut self, ensure: &ruby_prism::EnsureNode<'_>) {
        let kw = ensure.ensure_keyword_loc().start_offset();
        if let Some(body) = ensure.statements() {
            let bref = self.make_body(&body.as_node());
            self.check_indentation(kw, Some(bref), false);
        }
    }

    /// `check_members(base, [body])` for class/module/sclass.
    fn check_members(&mut self, base_off: usize, body: &Node<'_>) {
        // select_check_member: if body starts with an access modifier, check
        // that modifier (unless outdent style).
        let bref = self.make_body(body);
        if bref.starts_with_access_modifier {
            if self.config.access_modifier_outdent {
                // select_check_member returns nil -> check_indentation(base, nil)
            } else {
                // check the modifier itself (not skipped by access-modifier guard).
                let modifier = self.first_stmt_node_range(body);
                if let Some(mr) = modifier {
                    let mref = BodyRef {
                        start: mr.0,
                        correct_range: mr,
                        starts_with_access_modifier: false,
                    };
                    self.check_indentation(base_off, Some(mref), false);
                }
            }
        } else {
            self.check_indentation(base_off, Some(bref), false);
        }

        // normal style: check each member against base (skipping access-modifier sends).
        if !self.config.indented_internal_methods {
            self.check_members_normal(base_off, body);
        } else {
            self.check_members_indented_internal(body);
        }
    }

    fn check_members_normal(&mut self, base_off: usize, body: &Node<'_>) {
        let Some(st) = body.as_statements_node() else {
            return;
        };
        for member in st.body().iter() {
            if let Some(c) = member.as_call_node()
                && c.receiver().is_none()
                && c.arguments().is_none()
                && c.block().is_none()
                && is_bare_access_modifier(c.name().as_slice())
            {
                continue;
            }
            let mr = loc(&member.location());
            let mref = BodyRef {
                start: mr.0,
                correct_range: mr,
                starts_with_access_modifier: false,
            };
            self.check_indentation(base_off, Some(mref), false);
        }
    }

    fn check_members_indented_internal(&mut self, body: &Node<'_>) {
        let Some(st) = body.as_statements_node() else {
            return;
        };
        let mut previous_modifier: Option<usize> = None;
        for member in st.body().iter() {
            let is_special = member
                .as_call_node()
                .map(|c| {
                    c.receiver().is_none()
                        && c.arguments().is_none()
                        && c.block().is_none()
                        && is_special_modifier(c.name().as_slice())
                })
                .unwrap_or(false);
            if is_special {
                previous_modifier = Some(member.location().start_offset());
            } else if let Some(pm) = previous_modifier {
                let mr = loc(&member.location());
                let mref = BodyRef {
                    start: mr.0,
                    correct_range: mr,
                    starts_with_access_modifier: false,
                };
                self.check_indentation(pm, Some(mref), true);
                previous_modifier = None;
            }
        }
    }

    /// Range of the first statement node in a body.
    fn first_stmt_node_range(&self, body: &Node<'_>) -> Option<(usize, usize)> {
        let st = body.as_statements_node()?;
        let first = st.body().iter().next()?;
        Some(loc(&first.location()))
    }
}

/// Whether a Prism statements node's first child is a bare access modifier
/// send (`private` / `protected` / `public` with no args).
fn is_bare_access_modifier(name: &[u8]) -> bool {
    matches!(
        name,
        b"private" | b"protected" | b"public" | b"module_function"
    )
}

fn is_special_modifier(name: &[u8]) -> bool {
    SPECIAL_MODIFIERS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> Config {
        Config {
            width: 2,
            relative_to_receiver: false,
            access_modifier_outdent: false,
            indented_internal_methods: false,
            end_align: 1,
            def_end_align_def: false,
            use_tabs: false,
        }
    }

    fn run(source: &str) -> Vec<IndentationOffense> {
        check_indentation_width(source.as_bytes(), default_config(), &[])
    }

    #[test]
    fn if_body_under_indented() {
        // `func` at col 1 should be at col 2.
        let got = run("if cond\n func\nend\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].column_delta, 1);
        assert!(got[0].message.contains("Use 2 (not 1) spaces"));
    }

    #[test]
    fn correctly_indented_def_no_offense() {
        assert!(run("def test\n  func\nend\n").is_empty());
    }
}
