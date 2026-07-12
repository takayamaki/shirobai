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

use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

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
    prior_ranges: &[(usize, usize)],
) -> Vec<IndentationOffense> {
    let mut rule = build_rule(source, config, allowed_lines, prior_ranges);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule<'a>(
    source: &'a [u8],
    config: Config,
    allowed_lines: &'a [usize],
    prior_ranges: &[(usize, usize)],
) -> Visitor<'a> {
    let bom = source.starts_with(&[0xef, 0xbb, 0xbf]);
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        config,
        allowed_lines,
        bom,
        // Seed with correction ranges from earlier autocorrect iterations so a
        // correction nested in an already-corrected range stays suppressed
        // (`other_offense_in_same_range?` is cop-instance state, not per-pass).
        offense_ranges: prior_ranges.to_vec(),
        offenses: Vec::new(),
        ignored: Vec::new(),
        bare_send_stack: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    config: Config,
    allowed_lines: &'a [usize],
    bom: bool,
    /// Registered correction ranges, for `other_offense_in_same_range?`.
    offense_ranges: Vec<(usize, usize)>,
    pub(crate) offenses: Vec<IndentationOffense>,
    /// Node ranges that have been `ignore_node`'d (assignment rhs, def under modifier).
    ignored: Vec<(usize, usize)>,
    /// Ancestor stack used to resolve `leftmost_modifier_of` for `foo def` /
    /// `public foo def` chains. `ArgumentsNode` is transparent (parser-gem has
    /// no such wrapper), so the climb skips it.
    bare_send_stack: Vec<SendFrame>,
}

#[derive(Clone, Copy, PartialEq)]
enum SendFrame {
    /// A receiver-less `send` ancestor, with its start offset.
    BareSend(usize),
    /// A `send` with a receiver, carrying the chain info needed to resolve a
    /// `relative_to_receiver` block base (dot / selector offsets and the
    /// receiver's last line).
    Call(CallInfo),
    /// An `arguments` wrapper, transparent to the modifier climb.
    Arguments,
    /// Any other node; breaks the modifier climb.
    Other,
}

#[derive(Clone, Copy, PartialEq)]
struct CallInfo {
    /// Begin offset of the `.`/`&.` operator, if present.
    dot: Option<usize>,
    /// Begin offset of the selector (message), if present.
    selector: Option<usize>,
    /// 1-based last line of the receiver, if present.
    receiver_last_line: Option<usize>,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

impl<'a> Visitor<'a> {
    fn line_start(&self, off: usize) -> usize {
        self.line_index.line_start(off)
    }

    /// 1-based line number of `off`.
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    /// `range.column`: number of characters from the line start to `off`,
    /// adjusted for a byte-order mark on line 1 (`effective_column`).
    fn column(&self, off: usize) -> isize {
        let chars = self.line_index.column(self.source, off);
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
                return self.visual_column(body_off) - self.visual_column(base_off);
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
        // Maintain the ancestor stack used by `leftmost_modifier_of`. Push for
        // every branch node so `leave` pops symmetrically.
        let frame = if node.as_arguments_node().is_some() {
            SendFrame::Arguments
        } else if let Some(c) = node.as_call_node() {
            if c.receiver().is_none() {
                SendFrame::BareSend(node.location().start_offset())
            } else {
                SendFrame::Call(CallInfo {
                    dot: c.call_operator_loc().map(|l| l.start_offset()),
                    selector: c.message_loc().map(|l| l.start_offset()),
                    receiver_last_line: c
                        .receiver()
                        .map(|r| self.line_of(r.location().end_offset().saturating_sub(1))),
                })
            }
        } else {
            SendFrame::Other
        };
        self.bare_send_stack.push(frame);
    }

    fn leave(&mut self) {
        self.bare_send_stack.pop();
    }
}

impl<'a> Visitor<'a> {
    fn dispatch(&mut self, node: &Node<'_>) {
        if let Some(n) = node.as_def_node() {
            if !self.is_ignored(loc(&n.as_node().location())) {
                self.on_def(n.def_keyword_loc().start_offset(), n.body().as_ref());
            }
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
            if !self.is_ignored(loc(&n.as_node().location())) {
                // `on_while` bases on `node` (its full source range), not the
                // `while` keyword: for a post-condition `expr while c` the base
                // is the start of `expr` (so the body on the same line is
                // skipped); the loop body itself is checked by `on_kwbegin`.
                self.on_while_node(node.location().start_offset(), n.statements().as_ref());
            }
        } else if let Some(n) = node.as_until_node() {
            if !self.is_ignored(loc(&n.as_node().location())) {
                self.on_while_node(node.location().start_offset(), n.statements().as_ref());
            }
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
        } else if let Some(n) = node.as_parentheses_node() {
            self.on_parens_group(&n);
        } else if let Some(c) = node.as_call_node() {
            self.on_send(node, &c);
        } else if let Some((assign_start, value)) = assignment_value(node) {
            self.on_assignment(assign_start, &value);
        }
    }

    /// `on_send`: handle `adjacent_def_modifier?` (`foo def`) and otherwise the
    /// `CheckAssignment` attribute-write path.
    fn on_send(&mut self, node: &Node<'_>, c: &ruby_prism::CallNode<'_>) {
        // adjacent_def_modifier?: (send nil? _ (any_def ...)).
        if c.receiver().is_none()
            && let Some(args) = c.arguments()
        {
            let arg_list: Vec<_> = args.arguments().iter().collect();
            if arg_list.len() == 1
                && let Some(def) = arg_list[0].as_def_node()
            {
                self.on_adjacent_def_modifier(node, &def);
                return;
            }
        }
        // CheckAssignment#on_send: attribute/index write with an if/while rhs.
        if let Some((assign_start, value)) = assignment_value(node) {
            self.on_assignment(assign_start, &value);
        }
    }

    /// `foo def test` / `public foo def test`: check the def body against the
    /// leftmost modifier (start_of_line) or the def itself (DefEndAlignment def).
    fn on_adjacent_def_modifier(&mut self, send: &Node<'_>, def: &ruby_prism::DefNode<'_>) {
        let send_range = loc(&send.location());
        let base_off = if self.config.def_end_align_def {
            def.def_keyword_loc().start_offset()
        } else {
            self.leftmost_modifier_of(send_range.0)
        };
        if let Some(body) = def.body() {
            let bref = self.make_body(&body);
            self.check_indentation(base_off, Some(bref), false);
        }
        // ignore_node(def): the normal on_def must skip it.
        self.ignored.push(loc(&def.as_node().location()));
    }

    /// `leftmost_modifier_of(node)`: climb receiver-less send ancestors to the
    /// outermost one. The ancestor stack holds only ancestors of the current
    /// node (its own frame is pushed after dispatch).
    fn leftmost_modifier_of(&self, send_start: usize) -> usize {
        let mut leftmost = send_start;
        for &frame in self.bare_send_stack.iter().rev() {
            match frame {
                SendFrame::Arguments => continue,
                SendFrame::BareSend(start) => leftmost = start,
                SendFrame::Call(_) | SendFrame::Other => break,
            }
        }
        leftmost
    }

    /// `check_assignment(node, rhs)`: the rhs `if`/`while`/`until` is checked
    /// against a base that is either the assignment (variable alignment) or the
    /// rhs keyword, then `ignore_node`'d so the normal walk skips it.
    fn on_assignment(&mut self, assign_start: usize, value: &Node<'_>) {
        // rhs = first_part_of_call_chain(rhs)
        let rhs = first_part_of_call_chain(value);
        let Some(rhs) = rhs else { return };

        let rhs_range = loc(&rhs.location());
        // variable_alignment?(node.loc, rhs, style):
        //   style == keyword -> false; else !line_break_before_keyword.
        let variable_alignment = if self.config.end_align == 0 {
            false
        } else {
            // line_break_before_keyword? = rhs.first_line > whole_expression.line
            self.line_of(rhs_range.0) <= self.line_of(assign_start)
        };

        if let Some(n) = rhs.as_if_node() {
            if n.end_keyword_loc().is_none() {
                return;
            }
            let base_off = if variable_alignment {
                assign_start
            } else {
                n.if_keyword_loc()
                    .map(|l| l.start_offset())
                    .unwrap_or(rhs_range.0)
            };
            self.check_if(
                base_off,
                n.statements().as_ref().map(|s| s.as_node()),
                n.subsequent().as_ref(),
            );
            self.ignored.push(rhs_range);
        } else if let Some(n) = rhs.as_while_node() {
            let base_off = if variable_alignment {
                assign_start
            } else {
                n.keyword_loc().start_offset()
            };
            self.on_while_with_base(base_off, n.statements().as_ref());
            self.ignored.push(rhs_range);
        } else if let Some(n) = rhs.as_until_node() {
            let base_off = if variable_alignment {
                assign_start
            } else {
                n.keyword_loc().start_offset()
            };
            self.on_while_with_base(base_off, n.statements().as_ref());
            self.ignored.push(rhs_range);
        }
    }

    fn on_while_with_base(
        &mut self,
        base_off: usize,
        body: Option<&ruby_prism::StatementsNode<'_>>,
    ) {
        let Some(body) = body else { return };
        let bref = self.make_body(&body.as_node());
        self.check_indentation(base_off, Some(bref), false);
    }

    /// First statement of a Prism body node (statements / begin), or the node
    /// itself. Returns `(begin_pos, full_range)`.
    fn body_first_stmt(&self, body: &Node<'_>) -> (usize, (usize, usize)) {
        // An implicit `begin` (def/block body with rescue/ensure but no `begin`
        // keyword) is transparent: parser-gem reports `node.body` as the rescue
        // node whose range starts at the protected body, so descend to the
        // protected statements' first node.
        if let Some(bn) = body.as_begin_node()
            && bn.begin_keyword_loc().is_none()
            && let Some(st) = bn.statements()
        {
            return self.body_first_stmt(&st.as_node());
        }
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
        // An implicit `begin` (no `begin` keyword) is, in parser-gem, either a
        // transparent statement group (descend) or — when it carries rescue/
        // ensure — an `:ensure`/`:rescue` node that is NOT `begin_type`. For the
        // latter, `offense` does not reduce to the first child: the offense
        // range is the first protected statement but the correction spans the
        // whole node, realigning the rescue/ensure handler lines too.
        if let Some(bn) = body.as_begin_node()
            && bn.begin_keyword_loc().is_none()
            && let Some(st) = bn.statements()
        {
            if bn.rescue_clause().is_some() || bn.ensure_clause().is_some() {
                let (start, _) = self.body_first_stmt(&st.as_node());
                // parser-gem's `:ensure`/`:rescue` node range starts at the
                // protected body, not at the implicit begin's opening, and
                // ends at the last handler expression — NOT at the enclosing
                // `end` keyword that prism's implicit `BeginNode` includes.
                let end = parser_handlers_end(&bn);
                return BodyRef {
                    start,
                    correct_range: (start, end),
                    starts_with_access_modifier: self
                        .body_starts_with_access_modifier(&st.as_node()),
                };
            }
            return self.make_body(&st.as_node());
        }
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
        if self.is_ignored(loc(&n.as_node().location())) {
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
        let base_off = self.block_body_indentation_base(end_start);
        let Some(body) = n.body() else { return };
        let bref = self.make_body(&body);
        self.check_indentation(base_off, Some(bref), false);

        // indented_internal_methods style: when the block body contains an
        // access modifier, also check members the way `check_members` does.
        if self.config.indented_internal_methods
            && let Some(st) = body.as_statements_node()
            && self.contains_access_modifier(&st.as_node())
        {
            self.check_members_indented_internal(&st.as_node());
        }
    }

    /// `contains_access_modifier?(body)`: any bare access-modifier send among
    /// the body's statements.
    fn contains_access_modifier(&self, body: &Node<'_>) -> bool {
        let Some(st) = body.as_statements_node() else {
            return false;
        };
        st.body().iter().any(|m| {
            m.as_call_node()
                .map(|c| {
                    c.receiver().is_none()
                        && c.arguments().is_none()
                        && c.block().is_none()
                        && is_bare_access_modifier(c.name().as_slice())
                })
                .unwrap_or(false)
        })
    }

    /// `block_body_indentation_base(node, end_loc)`. For `relative_to_receiver`,
    /// when the chain's dot (or selector) is on a line after the receiver, the
    /// base is the dot (or selector) instead of the block's `end`.
    fn block_body_indentation_base(&self, end_start: usize) -> usize {
        if !self.config.relative_to_receiver {
            return end_start;
        }
        // The owning call frame is the immediate parent (top of stack, since the
        // block's own frame is not yet pushed during dispatch).
        let Some(SendFrame::Call(info)) = self.bare_send_stack.last().copied() else {
            return end_start;
        };
        // dot_on_new_line?: receiver.last_line < dot.line.
        if let (Some(dot), Some(recv_last)) = (info.dot, info.receiver_last_line)
            && recv_last < self.line_of(dot)
        {
            return dot;
        }
        // selector_on_new_line?: receiver.last_line < selector.line.
        if let (Some(sel), Some(recv_last)) = (info.selector, info.receiver_last_line)
            && info.dot.is_some()
            && recv_last < self.line_of(sel)
        {
            return sel;
        }
        end_start
    }

    /// `on_begin` for a parenthesized grouping expression `(...)` (rubocop#15311).
    /// prism models it as a `ParenthesesNode` (parser `:begin` with an explicit
    /// `(`). When the closing `)` begins its own line, the body is indented one
    /// step from the first non-space column of the line the `(` sits on.
    fn on_parens_group(&mut self, n: &ruby_prism::ParenthesesNode<'_>) {
        let close_start = n.closing_loc().start_offset();
        if !self.begins_its_line(close_start) {
            return;
        }
        let Some(body) = n.body() else { return };
        // `opening_line_start`: the first non-space offset of the `(`-line.
        let open_start = n.opening_loc().start_offset();
        let ls = self.line_start(open_start);
        // First non-space of the whole `(`-line; the `(` itself when nothing
        // precedes it on the line.
        let base_off = (ls..open_start)
            .find(|&i| !matches!(self.source[i], b' ' | b'\t'))
            .unwrap_or(open_start);
        let bref = self.make_body(&body);
        self.check_indentation(base_off, Some(bref), false);
    }

    fn on_begin_node(&mut self, n: &ruby_prism::BeginNode<'_>) {
        // on_kwbegin: an explicit `begin ... end` checks its protected body
        // against the `end` keyword, but only when `end` begins its line.
        if let (Some(_begin_kw), Some(end_loc)) = (n.begin_keyword_loc(), n.end_keyword_loc()) {
            let end_start = end_loc.start_offset();
            if self.begins_its_line(end_start)
                && let Some(body) = n.statements()
            {
                let mut bref = self.make_body(&body.as_node());
                // `node.children.first` is the whole rescue node when the
                // begin has handlers: realigning it shifts the rescue/else/
                // ensure keyword lines too. Extend the correction range from
                // the protected body to the last handler expression (the
                // parser-gem `:rescue`/`:ensure` node end); comment or blank
                // lines between it and `end` stay outside the realignment.
                if n.rescue_clause().is_some() || n.ensure_clause().is_some() {
                    bref.correct_range.1 = parser_handlers_end(n);
                }
                self.check_indentation(end_start, Some(bref), false);
            }
        }
        // on_resbody: each rescue clause body against its keyword.
        if let Some(rescue) = n.rescue_clause() {
            self.on_rescue_chain(&rescue);
        }
        // on_rescue: the rescue's else branch against the `else` keyword.
        if n.rescue_clause().is_some()
            && let Some(els) = n.else_clause()
        {
            let else_kw = els.else_keyword_loc().start_offset();
            if let Some(body) = els.statements() {
                let bref = self.make_body(&body.as_node());
                self.check_indentation(else_kw, Some(bref), false);
            }
        }
        // on_ensure: the ensure body against the `ensure` keyword.
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

/// End offset of the parser-gem `:rescue` / `:ensure` node wrapping a begin
/// body with handlers. parser-gem joins the protected body with the handler
/// clauses only — the enclosing `end` keyword (of the block / def / `begin`)
/// is NOT part of the node, unlike prism's `BeginNode` location:
///
///   - `:ensure` ends at its body's last statement, or at the `ensure`
///     keyword when the ensure body is empty.
///   - `:rescue` with an `else` ends at the else body's last statement, or
///     at the `else` keyword when the else body is empty.
///   - otherwise `:rescue` ends at the LAST resbody: its body's last
///     statement, or `then` / the `=> ref` target / the last exception
///     class / the `rescue` keyword as the clause shrinks.
fn parser_handlers_end(bn: &ruby_prism::BeginNode<'_>) -> usize {
    if let Some(ens) = bn.ensure_clause() {
        if let Some(st) = ens.statements()
            && let Some(last) = st.body().iter().last()
        {
            return last.location().end_offset();
        }
        return ens.ensure_keyword_loc().end_offset();
    }
    if bn.rescue_clause().is_some()
        && let Some(els) = bn.else_clause()
    {
        if let Some(st) = els.statements()
            && let Some(last) = st.body().iter().last()
        {
            return last.location().end_offset();
        }
        return els.else_keyword_loc().end_offset();
    }
    if let Some(mut clause) = bn.rescue_clause() {
        while let Some(next) = clause.subsequent() {
            clause = next;
        }
        if let Some(st) = clause.statements()
            && let Some(last) = st.body().iter().last()
        {
            return last.location().end_offset();
        }
        if let Some(then_loc) = clause.then_keyword_loc() {
            return then_loc.end_offset();
        }
        if let Some(reference) = clause.reference() {
            return reference.location().end_offset();
        }
        if let Some(last_exc) = clause.exceptions().iter().last() {
            return last_exc.location().end_offset();
        }
        return clause.keyword_loc().end_offset();
    }
    bn.as_node().location().end_offset()
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

/// `extract_rhs(node)` for assignment-like nodes: returns the assignment's
/// `(whole_expression_start, value)` when `node` is a write/op-asgn node or an
/// attribute/index-write `send`. Mirrors `CheckAssignment#extract_rhs`.
fn assignment_value<'pr>(node: &Node<'pr>) -> Option<(usize, Node<'pr>)> {
    let start = node.location().start_offset();

    macro_rules! v {
        ($e:expr) => {
            return Some((start, $e.value()))
        };
    }
    if let Some(n) = node.as_local_variable_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_instance_variable_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_class_variable_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_global_variable_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_constant_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_constant_path_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_local_variable_and_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_local_variable_or_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_local_variable_operator_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_instance_variable_and_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_instance_variable_or_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_instance_variable_operator_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_class_variable_and_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_class_variable_or_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_class_variable_operator_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_global_variable_and_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_global_variable_or_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_global_variable_operator_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_constant_and_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_constant_or_write_node() {
        v!(n)
    }
    if let Some(n) = node.as_constant_operator_write_node() {
        v!(n)
    }
    // `foo.bar = if ...` / `foo&.bar = if ...`: an attribute-write CallNode whose
    // last argument is the rhs.
    if let Some(c) = node.as_call_node()
        && c.is_attribute_write()
    {
        let args = c.arguments()?;
        let last = args.arguments().iter().last()?;
        return Some((start, last));
    }
    None
}

/// `first_part_of_call_chain(node)`: descend through call receivers to the
/// innermost non-call node (the receiver chain root). Returns `None` only when
/// `node` is itself a call with no receiver (a bare method send root, which the
/// caller treats as "not an if/while rhs").
fn first_part_of_call_chain<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    // First hop from the borrowed node.
    let mut current = match node.as_call_node() {
        Some(c) => c.receiver()?,
        None => return as_if_or_loop(node),
    };
    // Subsequent hops on owned receivers.
    loop {
        let next = current.as_call_node().and_then(|c| c.receiver());
        match next {
            Some(r) => current = r,
            None => return Some(current),
        }
    }
}

/// Returns an owned copy of `node` if it is an if/while/until (the only rhs
/// kinds the assignment path cares about), reconstructed via its typed
/// accessor; otherwise `None`.
fn as_if_or_loop<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    if let Some(n) = node.as_if_node() {
        return Some(n.as_node());
    }
    if let Some(n) = node.as_while_node() {
        return Some(n.as_node());
    }
    if let Some(n) = node.as_until_node() {
        return Some(n.as_node());
    }
    None
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
        check_indentation_width(source.as_bytes(), default_config(), &[], &[])
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

    fn run_cfg(source: &str, mutate: impl FnOnce(&mut Config)) -> Vec<IndentationOffense> {
        let mut cfg = default_config();
        mutate(&mut cfg);
        check_indentation_width(source.as_bytes(), cfg, &[], &[])
    }

    #[test]
    fn tabs_excessive_detected_not_corrected() {
        let got = run_cfg("if cond\n\t\tfunc\nend\n", |c| {
            c.use_tabs = true;
            c.width = 4;
        });
        assert_eq!(got.len(), 1);
        assert!(got[0].message.contains("Use 1 (not 2) tabs"));
    }

    #[test]
    fn post_condition_begin_end_while() {
        // `x = begin\n func1\n   func2\nend while cond`: only `func1` (col 1)
        // offends against the begin's `end` (col 0), expected col 2 -> delta +1.
        let got = run("x = begin\n func1\n   func2\nend while cond\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].column_delta, 1);
    }

    #[test]
    fn assignment_if_variable_aligned_offense() {
        // `0` at col 8 with end-aligned-keyword body; variable style base is the
        // assignment (col 0), so expected col 2 -> delta -6.
        let got = run("var = if a\n        0\n      end\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].column_delta, -6);
    }

    #[test]
    fn assignment_if_aligned_with_variable_accepted() {
        assert!(run("var = if a\n  0\nend\n").is_empty());
    }

    #[test]
    fn adjacent_def_modifier_offense() {
        // `foo def test\n      something\n  end`: body should align to col 2
        // (start_of_line: leftmost modifier `foo` at col 0).
        let got = run("foo def test\n      something\n  end\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].column_delta, -4);
    }

    #[test]
    fn adjacent_def_modifier_accepted() {
        assert!(run("foo def test\n  something\nend\n").is_empty());
    }

    #[test]
    fn relative_to_receiver_chained_block_offense() {
        // `foo\n  .bar do |x|\nx\nend`: with relative_to_receiver the base is
        // the dot (col 2), so body `x` (col 0) should be col 4 -> delta +4.
        let got = run_cfg("foo\n  .bar do |x|\nx\nend\n", |c| {
            c.relative_to_receiver = true
        });
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].column_delta, 4);
    }

    #[test]
    fn relative_to_receiver_chained_block_accepted() {
        let got = run_cfg("foo\n  .bar do |x|\n    x\nend\n", |c| {
            c.relative_to_receiver = true;
        });
        assert!(got.is_empty());
    }

    #[test]
    fn tabs_correct_no_offense() {
        let got = run_cfg("if cond\n\tfunc\nend\n", |c| {
            c.use_tabs = true;
            c.width = 4;
        });
        assert!(got.is_empty());
    }
}
