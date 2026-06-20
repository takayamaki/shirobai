//! `Style/BlockDelimiters`.
//!
//! The walk emits an ordered event stream mirroring stock's traversal: an
//! `Ignore` for every block registered by `on_send` (a block inside the
//! unparenthesized arguments of a call), and a `Candidate` for every block
//! whose delimiters violate the configured style (allowed methods and the
//! style decision are folded in; `AllowedPatterns` are not — regex matching
//! stays on the Ruby side, which takes the raw-event path when patterns are
//! configured).
//!
//! [`resolve`] then replays stock's `ignore_node` / `part_of_ignored_node?`
//! bookkeeping over the stream assuming every offense is enabled: a candidate
//! contained (inclusive, like `IgnoredNode`) in a prior-pass range or a
//! send-ignore is dropped; one contained only in an earlier *offense* range
//! is dropped but flags `has_conditional`, because its real fate depends on
//! `enabled_line?` of the suppressing offense (a `rubocop:disable` on the
//! outer block keeps stock's `ignore_node` from running). The wrapper falls
//! back to a full Ruby-side replay of the raw events in that rare case, where
//! `add_offense` natively reproduces the disable semantics.
//!
//! Corrections are emitted as the exact `Corrector` call sequence stock makes
//! (`replace` / `remove` / `insert_before` / `insert_after` / `wrap`), so the
//! merged rewrite is byte-identical, including the comment relocation of
//! `move_comment_before_block` and the `begin`/`end` wrapping of multiline
//! rescue/ensure bodies.

use ruby_prism::{CallNode, Node, StatementsNode};

type Range = (usize, usize);

/// One `corrector` call: `kind` 0 = `replace`, 1 = `remove`,
/// 2 = `insert_before`, 3 = `insert_after`, 4 = `wrap(range, "begin\n",
/// "\nend")` (text unused).
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionOp {
    pub kind: u8,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// One walk event, in stock callback order.
#[derive(Debug)]
pub enum Event {
    /// `ignore_node(block)` from `on_send`: the block node's full range.
    Ignore(Range),
    /// An improper block (before `part_of_ignored_node?` filtering).
    Candidate(Candidate),
}

#[derive(Debug)]
pub struct Candidate {
    /// The block node's full range (what `ignore_node` would record).
    pub block: Range,
    /// The opening delimiter token (`{` or `do`) — the offense range.
    pub token: Range,
    /// `node.method_name.to_s` (`"lambda"` / `"super"` for literals), for the
    /// Ruby-side `AllowedPatterns` check.
    pub method_name: String,
    pub message: String,
    pub ops: Vec<CorrectionOp>,
}

/// The resolved (all-offenses-enabled) outcome for one source.
pub struct BlockDelimitersResult {
    pub offenses: Vec<Candidate>,
    /// Blocks ignored by `on_send`, for the wrapper's cross-pass bookkeeping.
    pub send_ignores: Vec<Range>,
    /// A candidate was suppressed solely by offense ranges: the exact outcome
    /// depends on disable directives, so the wrapper must replay the raw
    /// events through `add_offense`.
    pub has_conditional: bool,
}

#[derive(Clone)]
pub struct Config {
    /// 0 = line_count_based, 1 = semantic, 2 = braces_for_chaining,
    /// 3 = always_braces.
    pub style: u8,
    pub allow_braces_on_procedural_oneliners: bool,
    pub procedural_methods: Vec<String>,
    pub functional_methods: Vec<String>,
    pub allowed_methods: Vec<String>,
    pub braces_required_methods: Vec<String>,
}

pub fn check_block_delimiters(
    source: &[u8],
    cfg: &Config,
    prior_ignored: &[Range],
) -> BlockDelimitersResult {
    let mut rule = build_rule(source, cfg.clone());
    super::dispatch::run(source, &mut [&mut rule]);
    resolve(rule.events, prior_ignored)
}

pub fn check_block_delimiters_events(source: &[u8], cfg: &Config) -> Vec<Event> {
    let mut rule = build_rule(source, cfg.clone());
    super::dispatch::run(source, &mut [&mut rule]);
    rule.events
}

/// Build the rule for use standalone or in a shared-walk bundle. Collects the
/// comment ranges up front (the parse cache cannot be re-entered mid-walk).
pub(crate) fn build_rule(source: &[u8], cfg: Config) -> Visitor<'_> {
    let comments = super::parse_cache::comment_ranges(source);
    let semantic = cfg.style == 1;
    Visitor {
        source,
        cfg,
        semantic,
        comments,
        frames: Vec::new(),
        events: Vec::new(),
    }
}

fn range_contains(outer: Range, inner: Range) -> bool {
    outer.0 <= inner.0 && inner.1 <= outer.1
}

/// Replay stock's in-order ignore bookkeeping (all offenses assumed enabled).
pub fn resolve(events: Vec<Event>, prior_ignored: &[Range]) -> BlockDelimitersResult {
    let mut send_ignores: Vec<Range> = Vec::new();
    let mut offense_ranges: Vec<Range> = Vec::new();
    let mut offenses = Vec::new();
    let mut has_conditional = false;
    for event in events {
        match event {
            Event::Ignore(range) => send_ignores.push(range),
            Event::Candidate(c) => {
                let unconditional = prior_ignored
                    .iter()
                    .chain(send_ignores.iter())
                    .any(|r| range_contains(*r, c.block));
                if unconditional {
                    continue;
                }
                if offense_ranges.iter().any(|r| range_contains(*r, c.block)) {
                    has_conditional = true;
                    continue;
                }
                offense_ranges.push(c.block);
                offenses.push(c);
            }
        }
    }
    BlockDelimitersResult {
        offenses,
        send_ignores,
        has_conditional,
    }
}

/// `RuboCop::AST::MethodIdentifierPredicates::OPERATOR_METHODS`.
const OPERATOR_METHODS: &[&[u8]] = &[
    b"|", b"^", b"&", b"<=>", b"==", b"===", b"=~", b">", b">=", b"<", b"<=", b"<<", b">>", b"+",
    b"-", b"*", b"/", b"%", b"**", b"~", b"+@", b"-@", b"!@", b"~@", b"[]", b"[]=", b"!", b"!=",
    b"!~", b"`",
];

/// `comparison_method?` names ending in `=` (excluded from
/// `assignment_method?`).
const COMPARISONS_ENDING_IN_EQ: &[&[u8]] = &[b"==", b"===", b"!=", b"<=", b">="];

fn is_ws(byte: u8) -> bool {
    // Ruby `/\s/`: ASCII whitespace only.
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c)
}

fn node_range(node: &Node<'_>) -> Range {
    let loc = node.location();
    (loc.start_offset(), loc.end_offset())
}

fn loc_range(loc: &ruby_prism::Location<'_>) -> Range {
    (loc.start_offset(), loc.end_offset())
}

/// Statement-list shape needed for the parser-parent emulation: the covered
/// range, the count and the last statement's range.
#[derive(Clone, Copy, Default)]
struct ListInfo {
    range: Option<Range>,
    count: usize,
    last: Range,
}

impl ListInfo {
    fn from_stmts(stmts: &Option<StatementsNode<'_>>) -> ListInfo {
        let Some(stmts) = stmts else {
            return ListInfo::default();
        };
        let body = stmts.body();
        let count = body.len();
        if count == 0 {
            return ListInfo::default();
        }
        let first = node_range(&body.iter().next().expect("count checked"));
        let last = node_range(&body.iter().last().expect("count checked"));
        ListInfo {
            range: Some((first.0, last.1)),
            count,
            last,
        }
    }

    fn covers(&self, child: Range) -> bool {
        self.range.is_some_and(|r| range_contains(r, child))
    }

    /// `(used, scope)` for a statement of a body list: 2+ statements form a
    /// parser sequence `begin` (never "used"; `children.last` decides scope),
    /// a single statement attaches to the position's owner directly.
    fn body_stmt(&self, child: Range, single_used: bool, single_scope: bool) -> (bool, bool) {
        if self.count == 1 {
            (single_used, single_scope)
        } else {
            (false, self.last == child)
        }
    }
}

/// One hooked ancestor. Carries exactly what the parser-parent emulation and
/// the chain climbing need; everything that cannot host a relevant role is
/// `Other`.
enum Frame {
    /// prism `CallNode` (parser `send`/`csend`). `range` includes an attached
    /// block, exactly like the parser block wrapper. `last_child` is the
    /// parser send's last child (block-pass argument, last argument, or the
    /// receiver when there are no arguments).
    Call {
        range: Range,
        receiver: Option<Range>,
        last_child: Option<Range>,
    },
    /// `super` with an explicit argument list (its `ArgumentsNode` is visited
    /// through a typed field, so arguments land directly here). Not
    /// `call_type?`.
    Super { last_child: Option<Range> },
    /// A parser block wrapper whose direct body is its last child: prism
    /// `BlockNode` / `LambdaNode` / `ForwardingSuperNode` (whose `BlockNode`
    /// is reached through a typed field and never hooks).
    BlockWrapper,
    /// prism `ParenthesesNode`: a parser `begin`. `used` is its own
    /// `return_value_used?`, resolved against the stack at enter. Its hooked
    /// `StatementsNode` collapses into the same single parser `begin`.
    Paren { used: bool },
    /// A hooked `StatementsNode` (block/def/class/module/sclass/lambda/paren
    /// bodies): `(used, scope)` for single/multi resolved against its owner
    /// at enter.
    Statements {
        info: ListInfo,
        single_used: bool,
        single_scope: bool,
        multi_used: bool,
    },
    /// prism `BeginNode`: `kwbegin` and/or the implicit rescue/ensure body
    /// (its statement lists are typed fields and never hook).
    Begin {
        has_rescue_or_ensure: bool,
        stmts: ListInfo,
        else_stmts: ListInfo,
        ensure_stmts: ListInfo,
    },
    /// if/unless/while/until/case/case_match (and an `ElseNode` hooked via
    /// `IfNode#subsequent`): `parent.conditional?` holds for every direct
    /// child (predicate or single-statement branch); multi-statement branches
    /// form sequences.
    Conditional { branches: [ListInfo; 2] },
    /// when/in: the parser node's last child is the body (`nil` when empty).
    When { stmts: ListInfo },
    AndOr,
    ArrayOrRange,
    /// parser `pair`: the value is the last child.
    Pair { value: Range },
    /// Any parser assignment node (`lvasgn` family, op/or/and-writes,
    /// `masgn`); the assigned value is also the last child.
    Assign,
    /// return/break/next/defined?: the parser node's last child is the (last)
    /// wrapped expression.
    LastChild { last: Option<Range> },
    /// def/defs/class/module/sclass: hooked `Statements` bodies resolve
    /// against this; a directly attached (endless def) body is the parser
    /// node's last child.
    DefLike { body: Option<Range> },
    For { body: ListInfo },
    /// A rescue clause (parser `resbody`), pushed by `enter_rescue`; the
    /// handler body is its last child.
    RescueClause { stmts: ListInfo },
    /// `RescueModifierNode`: the rescue value is its `resbody`'s last child;
    /// the rescued expression is the `rescue` node's first child.
    RescueMod { rescue_expr: Range },
    /// String interpolation: parser wraps the statements in a `begin`
    /// unconditionally (its parent is the dstr — never "used").
    Embedded { stmts: ListInfo },
    /// Top level: multi-statement programs get a root sequence `begin`; a
    /// single statement has no parser parent at all.
    Program { stmts: ListInfo },
    Other,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    cfg: Config,
    /// Style is `semantic`: the only style needing the parser-parent
    /// emulation, so the non-semantic walk pushes slim frames.
    semantic: bool,
    comments: Vec<Range>,
    frames: Vec<Frame>,
    pub(crate) events: Vec<Event>,
}

impl<'a> Visitor<'a> {
    fn list_in(&self, name: &str, list: &[String]) -> bool {
        list.iter().any(|m| m == name)
    }

    // --- on_block ----------------------------------------------------------

    /// `on_block` for a parser block wrapper: a prism call/super with a
    /// literal block, or a lambda literal. Runs before the node's own frame
    /// is pushed, so `frames.last()` is the parser parent context.
    #[allow(clippy::too_many_arguments)]
    fn on_block(
        &mut self,
        block_range: Range,
        opening: Range,
        closing: Range,
        method_name: &str,
        _node: &Node<'_>,
        body: Option<&Node<'_>>,
        send_arguments: bool,
        send_parenthesized: bool,
    ) {
        let braces = self.source[opening.0] == b'{';
        // `BlockNode#multiline?` compares the delimiter lines.
        let multiline = self.source[opening.0..closing.0].contains(&b'\n');
        if self.proper_block_style(braces, multiline, block_range, method_name, body) {
            return;
        }
        let message = self.message(braces, multiline, block_range, method_name);
        let ops = self.correction_ops(
            braces,
            multiline,
            block_range,
            opening,
            closing,
            body,
            send_arguments,
            send_parenthesized,
        );
        self.events.push(Event::Candidate(Candidate {
            block: block_range,
            token: opening,
            method_name: method_name.to_string(),
            message,
            ops,
        }));
    }

    /// `proper_block_style?` minus `matches_allowed_pattern?` (Ruby-side).
    fn proper_block_style(
        &self,
        braces: bool,
        multiline: bool,
        block_range: Range,
        method_name: &str,
        body: Option<&Node<'_>>,
    ) -> bool {
        if self.require_do_end(braces, multiline, body) {
            return true;
        }
        if self.list_in(method_name, &self.cfg.allowed_methods) {
            return true;
        }
        if self.list_in(method_name, &self.cfg.braces_required_methods) {
            return braces;
        }
        match self.cfg.style {
            0 => multiline ^ braces,
            1 => self.semantic_block_style(braces, multiline, block_range, method_name),
            2 => {
                if multiline {
                    if self.chained(block_range) { braces } else { !braces }
                } else {
                    braces
                }
            }
            _ => braces,
        }
    }

    /// `require_do_end?`: a single-line `do`..`end` block cannot use braces
    /// when it contains `ensure` or a block-level `rescue` (as opposed to a
    /// bare modifier rescue `expr rescue expr`).
    fn require_do_end(&self, braces: bool, multiline: bool, body: Option<&Node<'_>>) -> bool {
        if braces || multiline {
            return false;
        }
        let Some(body) = body else { return false };
        // Prism wraps block-level rescue/ensure in a BeginNode (implicit
        // begin). Modifier rescue stays as a RescueModifierNode inside a
        // StatementsNode.
        if let Some(begin) = body.as_begin_node() {
            if begin.ensure_clause().is_some() {
                return true;
            }
            if let Some(rescue) = begin.rescue_clause() {
                let has_protected_body = begin.statements().is_some();
                return !is_modifier_rescue(&rescue, has_protected_body);
            }
        }
        // Walk into StatementsNode: the body of a block is often a
        // StatementsNode wrapping the actual content.
        if let Some(stmts) = body.as_statements_node() {
            for stmt in stmts.body().iter() {
                if let Some(begin) = stmt.as_begin_node() {
                    if begin.ensure_clause().is_some() {
                        return true;
                    }
                    if let Some(rescue) = begin.rescue_clause() {
                        let has_protected_body = begin.statements().is_some();
                        return !is_modifier_rescue(&rescue, has_protected_body);
                    }
                }
            }
        }
        false
    }

    fn semantic_block_style(
        &self,
        braces: bool,
        multiline: bool,
        block_range: Range,
        method_name: &str,
    ) -> bool {
        if braces {
            if self.list_in(method_name, &self.cfg.functional_methods) {
                return true;
            }
            let (used, scope) = self.resolve_parent(block_range);
            used || scope || (self.cfg.allow_braces_on_procedural_oneliners && !multiline)
        } else {
            self.list_in(method_name, &self.cfg.procedural_methods)
                || !self.resolve_parent(block_range).0
        }
    }

    /// `(return_value_used?, return_value_of_scope?)` via the parser-parent
    /// emulation against the nearest hooked ancestor.
    fn resolve_parent(&self, b: Range) -> (bool, bool) {
        let Some(frame) = self.frames.last() else {
            return (false, false);
        };
        match frame {
            Frame::Call { last_child, .. } => (true, *last_child == Some(b)),
            Frame::Super { last_child } => (false, *last_child == Some(b)),
            Frame::BlockWrapper => (false, true),
            Frame::Paren { used } => (*used, true),
            Frame::Statements {
                info,
                single_used,
                single_scope,
                multi_used,
            } => {
                if info.count == 1 {
                    (*single_used, *single_scope)
                } else {
                    (*multi_used, info.last == b)
                }
            }
            Frame::Begin {
                has_rescue_or_ensure,
                stmts,
                else_stmts,
                ensure_stmts,
            } => {
                if stmts.covers(b) {
                    if *has_rescue_or_ensure {
                        // Protected body: a single statement is the rescue/
                        // ensure node's FIRST child (never last); 2+ form a
                        // sequence.
                        (false, stmts.count > 1 && stmts.last == b)
                    } else {
                        // Plain kwbegin holds its statements directly; the
                        // last one is `children.last`.
                        (false, stmts.last == b)
                    }
                } else if else_stmts.covers(b) {
                    // The rescue-else is the rescue node's last child.
                    else_stmts.body_stmt(b, false, true)
                } else if ensure_stmts.covers(b) {
                    // The ensure branch is the ensure node's last child.
                    ensure_stmts.body_stmt(b, false, true)
                } else {
                    (false, false)
                }
            }
            Frame::Conditional { branches } => {
                for list in branches {
                    if list.covers(b) && list.count > 1 {
                        return (false, list.last == b);
                    }
                }
                // Predicate or single-statement branch: a direct child of a
                // `conditional?` parent.
                (false, true)
            }
            Frame::When { stmts } => {
                if stmts.covers(b) {
                    stmts.body_stmt(b, false, true)
                } else {
                    // Conditions/pattern: the body slot (even nil) is last.
                    (false, false)
                }
            }
            Frame::AndOr | Frame::ArrayOrRange => (false, true),
            Frame::Pair { value } => (false, *value == b),
            Frame::Assign => (true, true),
            Frame::LastChild { last } => (false, *last == Some(b)),
            Frame::DefLike { body } => (false, *body == Some(b)),
            Frame::For { body } => {
                if body.covers(b) {
                    body.body_stmt(b, false, true)
                } else {
                    (false, false)
                }
            }
            Frame::RescueClause { stmts } => {
                if stmts.covers(b) {
                    stmts.body_stmt(b, false, true)
                } else {
                    (false, false)
                }
            }
            Frame::RescueMod { rescue_expr } => (false, *rescue_expr == b),
            Frame::Embedded { stmts } => {
                // Always a parser begin, even with one statement.
                (false, stmts.count <= 1 || stmts.last == b)
            }
            Frame::Program { stmts } => {
                if stmts.count > 1 {
                    (false, stmts.last == b)
                } else {
                    // A single top-level statement has no parser parent.
                    (false, false)
                }
            }
            Frame::Other => (false, false),
        }
    }

    /// `BlockNode#chained?`: the parser parent is a send whose receiver is
    /// this block.
    fn chained(&self, block_range: Range) -> bool {
        matches!(
            self.frames.last(),
            Some(Frame::Call { receiver: Some(r), .. }) if *r == block_range
        )
    }

    // --- messages ----------------------------------------------------------

    fn message(
        &self,
        braces: bool,
        multiline: bool,
        block_range: Range,
        method_name: &str,
    ) -> String {
        if self.list_in(method_name, &self.cfg.braces_required_methods) {
            return format!("Brace delimiters `{{...}}` required for '{method_name}' method.");
        }
        match self.cfg.style {
            0 => {
                if multiline {
                    "Avoid using `{...}` for multi-line blocks.".to_string()
                } else {
                    "Prefer `{...}` over `do...end` for single-line blocks.".to_string()
                }
            }
            1 => {
                if braces {
                    "Prefer `do...end` over `{...}` for procedural blocks.".to_string()
                } else {
                    "Prefer `{...}` over `do...end` for functional blocks.".to_string()
                }
            }
            2 => {
                if multiline {
                    if self.chained(block_range) {
                        "Prefer `{...}` over `do...end` for multi-line chained blocks.".to_string()
                    } else {
                        "Prefer `do...end` for multi-line blocks without chaining.".to_string()
                    }
                } else {
                    "Prefer `{...}` over `do...end` for single-line blocks.".to_string()
                }
            }
            _ => "Prefer `{...}` over `do...end` for blocks.".to_string(),
        }
    }

    // --- corrections -------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn correction_ops(
        &self,
        braces: bool,
        multiline: bool,
        block_range: Range,
        opening: Range,
        closing: Range,
        body: Option<&Node<'_>>,
        send_arguments: bool,
        send_parenthesized: bool,
    ) -> Vec<CorrectionOp> {
        // `correction_would_break_code?`.
        if !braces && send_arguments && !send_parenthesized {
            return Vec::new();
        }
        let mut ops = Vec::new();
        let op = |kind: u8, (s, e): Range, text: &str| CorrectionOp {
            kind,
            start: s,
            end: e,
            text: text.to_string(),
        };
        if braces {
            // replace_braces_with_do_end.
            if !self.ws_before(opening.0) {
                ops.push(op(2, opening, " "));
            }
            if !self.ws_before(closing.0) {
                ops.push(op(2, closing, " "));
            }
            if !self.ws_at(opening.0 + 1) {
                ops.push(op(3, opening, " "));
            }
            ops.push(op(0, opening, "do"));
            if let Some(comment) = self.comment_at_line_of(closing.0) {
                self.move_comment_ops(&mut ops, comment, block_range, closing);
            }
            ops.push(op(0, closing, "end"));
        } else {
            // replace_do_end_with_braces.
            if !self.ws_at(opening.0 + 2) {
                ops.push(op(3, opening, " "));
            }
            ops.push(op(0, opening, "{"));
            ops.push(op(0, closing, "}"));
            // `begin_required?`.
            if multiline
                && let Some(body) = body
                && let Some(begin) = body.as_begin_node()
                && (begin.rescue_clause().is_some() || begin.ensure_clause().is_some())
            {
                ops.push(op(4, parser_body_range(&begin), ""));
            }
        }
        ops
    }

    /// `move_comment_before_block`, as corrector ops.
    fn move_comment_ops(
        &self,
        ops: &mut Vec<CorrectionOp>,
        comment: Range,
        block_range: Range,
        closing: Range,
    ) {
        let target_end = if self.chained(block_range) {
            self.end_of_chain(block_range).1
        } else {
            closing.1
        };
        // `range.end.join(comment.source_range.begin)` (min/max of the two
        // points), then strip the trailing whitespace run (`/\s+\z/`).
        let pre_start = target_end.min(comment.0);
        let mut pre_end = target_end.max(comment.0);
        while pre_end > pre_start && is_ws(self.source[pre_end - 1]) {
            pre_end -= 1;
        }
        // remove(range_with_surrounding_space(comment, side: :right)):
        // spaces/tabs first, then newlines.
        let mut ext_end = comment.1;
        while ext_end < self.source.len() && matches!(self.source[ext_end], b' ' | b'\t') {
            ext_end += 1;
        }
        while ext_end < self.source.len() && self.source[ext_end] == b'\n' {
            ext_end += 1;
        }
        ops.push(CorrectionOp {
            kind: 1,
            start: comment.0,
            end: ext_end,
            text: String::new(),
        });
        // remove_trailing_whitespace: the gap between the code end and the
        // comment, if it is pure whitespace.
        let t_start = pre_end.min(comment.0);
        let t_end = pre_end.max(comment.0);
        if t_end > t_start && self.source[t_start..t_end].iter().all(|&b| is_ws(b)) {
            ops.push(CorrectionOp {
                kind: 1,
                start: t_start,
                end: t_end,
                text: String::new(),
            });
        }
        ops.push(CorrectionOp {
            kind: 3,
            start: pre_start,
            end: pre_end,
            text: "\n".to_string(),
        });
        let mut text = String::from_utf8_lossy(&self.source[comment.0..comment.1]).into_owned();
        text.push('\n');
        ops.push(CorrectionOp {
            kind: 2,
            start: block_range.0,
            end: block_range.1,
            text,
        });
    }

    /// `end_of_chain(node.parent).source_range`: climb the receiver chain
    /// through the hooked ancestors. A prism call's range already includes
    /// its block, exactly like the parser block wrapper `end_of_chain`
    /// switches to via `with_block?`.
    fn end_of_chain(&self, block_range: Range) -> Range {
        let mut current = block_range;
        for frame in self.frames.iter().rev() {
            match frame {
                Frame::Call {
                    range,
                    receiver: Some(r),
                    ..
                } if *r == current => current = *range,
                _ => break,
            }
        }
        current
    }

    // --- on_send (ignore registration) -------------------------------------

    /// `on_send` / `on_csend`: register every block inside the
    /// unparenthesized arguments as ignored.
    fn on_send(&mut self, call: &CallNode<'_>) {
        let args = call.arguments();
        let block_arg = call
            .block()
            .filter(|b| b.as_block_argument_node().is_some());
        // parser `arguments?` counts the block-pass argument.
        let args_empty = args.as_ref().is_none_or(|a| a.arguments().is_empty());
        if args_empty && block_arg.is_none() {
            return;
        }
        // `parenthesized?` is `loc_is?(:end, ')')`.
        if call
            .closing_loc()
            .is_some_and(|l| self.source[l.start_offset()] == b')')
        {
            return;
        }
        let name = call.name().as_slice();
        if name.ends_with(b"=") && !COMPARISONS_ENDING_IN_EQ.contains(&name) {
            return;
        }
        let arg_nodes: Vec<Node> = args.map(|a| a.arguments().iter().collect()).unwrap_or_default();
        if OPERATOR_METHODS.contains(&name) {
            let one_arg = arg_nodes.len() + usize::from(block_arg.is_some()) == 1;
            if one_arg && arg_nodes.first().is_some_and(is_parser_block_type) {
                return;
            }
        }
        for arg in &arg_nodes {
            self.get_blocks(arg);
        }
    }

    /// `get_blocks`: yield (= ignore) every block wrapper reachable without
    /// crossing a syntactic boundary that already disambiguates the binding.
    fn get_blocks(&mut self, node: &Node<'_>) {
        // Parser block wrappers (block / numblock / itblock), including
        // lambda literals and super.
        if node.as_lambda_node().is_some() {
            self.events.push(Event::Ignore(node_range(node)));
            return;
        }
        if let Some(call) = node.as_call_node() {
            if call.block().is_some_and(|b| b.as_block_node().is_some()) {
                self.events.push(Event::Ignore(node_range(node)));
                return;
            }
            // Plain send/csend: descend into the receiver and arguments (a
            // block-pass argument matches no branch in stock).
            if let Some(receiver) = call.receiver() {
                self.get_blocks(&receiver);
            }
            if let Some(args) = call.arguments() {
                for arg in &args.arguments() {
                    self.get_blocks(&arg);
                }
            }
            return;
        }
        if let Some(sup) = node.as_super_node() {
            if sup.block().is_some_and(|b| b.as_block_node().is_some()) {
                self.events.push(Event::Ignore(node_range(node)));
            }
            return;
        }
        if let Some(fsup) = node.as_forwarding_super_node() {
            if fsup.block().is_some() {
                self.events.push(Event::Ignore(node_range(node)));
            }
            return;
        }
        // A braceless hash argument: descend into its pairs (kwsplats match
        // no branch); a braced hash stops the descent.
        if let Some(kwhash) = node.as_keyword_hash_node() {
            for element in &kwhash.elements() {
                if let Some(assoc) = element.as_assoc_node() {
                    self.get_blocks(&assoc.key());
                    self.get_blocks(&assoc.value());
                }
            }
        }
    }

    // --- byte helpers -------------------------------------------------------

    fn ws_before(&self, pos: usize) -> bool {
        pos > 0 && is_ws(self.source[pos - 1])
    }

    fn ws_at(&self, pos: usize) -> bool {
        pos < self.source.len() && is_ws(self.source[pos])
    }

    /// `processed_source.comment_at_line(line)`: the first comment starting
    /// on the same line as `pos`.
    fn comment_at_line_of(&self, pos: usize) -> Option<Range> {
        let line_start = self.source[..pos]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        let line_end = self.source[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(self.source.len(), |i| pos + i);
        self.comments
            .iter()
            .find(|(s, _)| line_start <= *s && *s < line_end)
            .copied()
    }

    // --- frame construction -------------------------------------------------

    fn push_frame(&mut self, node: &Node<'_>) {
        let frame = if let Some(call) = node.as_call_node() {
            Frame::Call {
                range: node_range(node),
                receiver: call.receiver().map(|r| node_range(&r)),
                last_child: if self.semantic {
                    call_last_child(&call)
                } else {
                    None
                },
            }
        } else if self.semantic {
            self.semantic_frame(node)
        } else {
            Frame::Other
        };
        self.frames.push(frame);
    }

    fn semantic_frame(&self, node: &Node<'_>) -> Frame {
        if node.as_block_node().is_some()
            || node.as_lambda_node().is_some()
            || node.as_forwarding_super_node().is_some()
        {
            return Frame::BlockWrapper;
        }
        if let Some(sup) = node.as_super_node() {
            let last_child = sup
                .block()
                .filter(|b| b.as_block_argument_node().is_some())
                .map(|b| node_range(&b))
                .or_else(|| last_arg_range(&sup.arguments()));
            return Frame::Super { last_child };
        }
        if node.as_parentheses_node().is_some() {
            // The paren begin's own `return_value_used?` against the current
            // stack (`return_value_used?` recurses through begin parents).
            let used = self.resolve_parent(node_range(node)).0;
            return Frame::Paren { used };
        }
        if let Some(stmts) = node.as_statements_node() {
            let info = ListInfo::from_stmts(&Some(stmts));
            return match self.frames.last() {
                // Block/lambda body: the parser block's body is its last
                // child.
                Some(Frame::BlockWrapper) => Frame::Statements {
                    info,
                    single_used: false,
                    single_scope: true,
                    multi_used: false,
                },
                // def/defs/class/module/sclass body: also the last child.
                Some(Frame::DefLike { .. }) => Frame::Statements {
                    info,
                    single_used: false,
                    single_scope: true,
                    multi_used: false,
                },
                // Parens: the paren and the sequence are ONE parser begin —
                // `used` comes from the paren for any statement count, and
                // `children.last` is the last statement (the single
                // statement is trivially last).
                Some(Frame::Paren { used }) => Frame::Statements {
                    info,
                    single_used: *used,
                    single_scope: true,
                    multi_used: *used,
                },
                _ => Frame::Other,
            };
        }
        if let Some(begin) = node.as_begin_node() {
            return Frame::Begin {
                has_rescue_or_ensure: begin.rescue_clause().is_some()
                    || begin.ensure_clause().is_some(),
                stmts: ListInfo::from_stmts(&begin.statements()),
                else_stmts: ListInfo::from_stmts(&begin.else_clause().and_then(|e| e.statements())),
                ensure_stmts: ListInfo::from_stmts(
                    &begin.ensure_clause().and_then(|e| e.statements()),
                ),
            };
        }
        if let Some(if_node) = node.as_if_node() {
            return Frame::Conditional {
                branches: [
                    ListInfo::from_stmts(&if_node.statements()),
                    ListInfo::default(),
                ],
            };
        }
        if let Some(else_node) = node.as_else_node() {
            // Hooked only via `IfNode#subsequent`: the single-statement else
            // branch is a direct child of the `if`.
            return Frame::Conditional {
                branches: [
                    ListInfo::from_stmts(&else_node.statements()),
                    ListInfo::default(),
                ],
            };
        }
        if let Some(unless_node) = node.as_unless_node() {
            return Frame::Conditional {
                branches: [
                    ListInfo::from_stmts(&unless_node.statements()),
                    ListInfo::from_stmts(&unless_node.else_clause().and_then(|e| e.statements())),
                ],
            };
        }
        if let Some(while_node) = node.as_while_node() {
            return Frame::Conditional {
                branches: [
                    ListInfo::from_stmts(&while_node.statements()),
                    ListInfo::default(),
                ],
            };
        }
        if let Some(until_node) = node.as_until_node() {
            return Frame::Conditional {
                branches: [
                    ListInfo::from_stmts(&until_node.statements()),
                    ListInfo::default(),
                ],
            };
        }
        if let Some(case_node) = node.as_case_node() {
            return Frame::Conditional {
                branches: [
                    ListInfo::from_stmts(&case_node.else_clause().and_then(|e| e.statements())),
                    ListInfo::default(),
                ],
            };
        }
        if let Some(case_match) = node.as_case_match_node() {
            return Frame::Conditional {
                branches: [
                    ListInfo::from_stmts(&case_match.else_clause().and_then(|e| e.statements())),
                    ListInfo::default(),
                ],
            };
        }
        if let Some(when_node) = node.as_when_node() {
            return Frame::When {
                stmts: ListInfo::from_stmts(&when_node.statements()),
            };
        }
        if let Some(in_node) = node.as_in_node() {
            return Frame::When {
                stmts: ListInfo::from_stmts(&in_node.statements()),
            };
        }
        if node.as_and_node().is_some() || node.as_or_node().is_some() {
            return Frame::AndOr;
        }
        if node.as_array_node().is_some() || node.as_range_node().is_some() {
            return Frame::ArrayOrRange;
        }
        if let Some(assoc) = node.as_assoc_node() {
            return Frame::Pair {
                value: node_range(&assoc.value()),
            };
        }
        if is_assignment_node(node) {
            return Frame::Assign;
        }
        if let Some(ret) = node.as_return_node() {
            return Frame::LastChild {
                last: last_arg_range(&ret.arguments()),
            };
        }
        if let Some(brk) = node.as_break_node() {
            return Frame::LastChild {
                last: last_arg_range(&brk.arguments()),
            };
        }
        if let Some(nxt) = node.as_next_node() {
            return Frame::LastChild {
                last: last_arg_range(&nxt.arguments()),
            };
        }
        if let Some(defined) = node.as_defined_node() {
            return Frame::LastChild {
                last: Some(node_range(&defined.value())),
            };
        }
        if let Some(def) = node.as_def_node() {
            return Frame::DefLike {
                body: def
                    .body()
                    .filter(|b| b.as_statements_node().is_none() && b.as_begin_node().is_none())
                    .map(|b| node_range(&b)),
            };
        }
        if node.as_class_node().is_some()
            || node.as_module_node().is_some()
            || node.as_singleton_class_node().is_some()
        {
            return Frame::DefLike { body: None };
        }
        if let Some(for_node) = node.as_for_node() {
            return Frame::For {
                body: ListInfo::from_stmts(&for_node.statements()),
            };
        }
        if let Some(rescue_mod) = node.as_rescue_modifier_node() {
            return Frame::RescueMod {
                rescue_expr: node_range(&rescue_mod.rescue_expression()),
            };
        }
        if let Some(embedded) = node.as_embedded_statements_node() {
            return Frame::Embedded {
                stmts: ListInfo::from_stmts(&embedded.statements()),
            };
        }
        if let Some(program) = node.as_program_node() {
            return Frame::Program {
                stmts: ListInfo::from_stmts(&Some(program.statements())),
            };
        }
        Frame::Other
    }
}

/// The parser send's last child: the block-pass argument, the last argument,
/// or the receiver (an attached literal block is the wrapper's child, not the
/// send's).
fn call_last_child(call: &CallNode<'_>) -> Option<Range> {
    if let Some(block_arg) = call.block().filter(|b| b.as_block_argument_node().is_some()) {
        return Some(node_range(&block_arg));
    }
    last_arg_range(&call.arguments()).or_else(|| call.receiver().map(|r| node_range(&r)))
}

fn last_arg_range(args: &Option<ruby_prism::ArgumentsNode<'_>>) -> Option<Range> {
    args.as_ref()
        .and_then(|a| a.arguments().iter().last().map(|n| node_range(&n)))
}

/// parser `assignment?`: lvasgn/ivasgn/cvasgn/gvasgn/casgn/masgn plus the
/// op/or/and shorthand writes (a setter send is `call_type?`, handled by the
/// `Call` frame).
fn is_assignment_node(node: &Node<'_>) -> bool {
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
        || node.as_call_operator_write_node().is_some()
        || node.as_call_or_write_node().is_some()
        || node.as_call_and_write_node().is_some()
        || node.as_index_operator_write_node().is_some()
        || node.as_index_or_write_node().is_some()
        || node.as_index_and_write_node().is_some()
}

/// `first_argument.block_type?`: a parser `block` — a block wrapper whose
/// parameters are not numbered (`numblock`). `it` blocks are kept plain to
/// match the parser engine on default target rubies.
fn is_parser_block_type(node: &Node<'_>) -> bool {
    let block = if let Some(call) = node.as_call_node() {
        call.block()
    } else if let Some(sup) = node.as_super_node() {
        sup.block()
    } else if node.as_forwarding_super_node().is_some() {
        return true;
    } else if let Some(lambda) = node.as_lambda_node() {
        return lambda
            .parameters()
            .is_none_or(|p| p.as_numbered_parameters_node().is_none());
    } else {
        return false;
    };
    block.and_then(|b| b.as_block_node()).is_some_and(|b| {
        b.parameters()
            .is_none_or(|p| p.as_numbered_parameters_node().is_none())
    })
}

/// The parser range of a block's rescue/ensure body (the implicit-begin
/// translation), for `corrector.wrap`: from the first protected statement
/// (or the `rescue`/`ensure` keyword when the protected body is empty) to
/// the end of the outermost clause (ensure branch / rescue-else / the last
/// rescue clause).
fn parser_body_range(begin: &ruby_prism::BeginNode<'_>) -> Range {
    let stmts_first = begin
        .statements()
        .and_then(|s| s.body().iter().next().map(|n| node_range(&n).0));
    let start = stmts_first.unwrap_or_else(|| {
        if let Some(rescue) = begin.rescue_clause() {
            rescue.keyword_loc().start_offset()
        } else if let Some(ensure) = begin.ensure_clause() {
            ensure.ensure_keyword_loc().start_offset()
        } else {
            begin.location().start_offset()
        }
    });
    let end = if let Some(ensure) = begin.ensure_clause() {
        ensure
            .statements()
            .and_then(|s| s.body().iter().last().map(|n| node_range(&n).1))
            .unwrap_or_else(|| ensure.ensure_keyword_loc().end_offset())
    } else if let Some(else_end) = begin
        .else_clause()
        .and_then(|e| e.statements())
        .and_then(|s| s.body().iter().last().map(|n| node_range(&n).1))
    {
        else_end
    } else if let Some(rescue) = begin.rescue_clause() {
        // The last clause of the chain.
        let mut clause = rescue;
        while let Some(next) = clause.subsequent() {
            clause = next;
        }
        clause
            .statements()
            .and_then(|s| s.body().iter().last().map(|n| node_range(&n).1))
            .or_else(|| clause.reference().map(|r| node_range(&r).1))
            .or_else(|| clause.exceptions().iter().last().map(|e| node_range(&e).1))
            .unwrap_or_else(|| clause.keyword_loc().end_offset())
    } else {
        begin.location().end_offset()
    };
    (start, end)
}

/// A bare modifier rescue: `expr rescue expr` — single branch, no
/// exceptions list, no exception variable, no else. This form CAN be
/// written with braces, unlike a block-level rescue.
///
/// In Prism, block-level rescue creates a `BeginNode` containing a
/// `RescueNode` chain. The `RescueNode` here is the first (and possibly
/// only) rescue clause. Stock checks on the parser-gem `:rescue` wrapper:
///   - `body.nil?` → no protected expression before the rescue keyword
///   - `else_branch` → has an else
///   - `resbody_branches.one?` → exactly one rescue clause
///   - `resbody.exceptions.empty?` → no exception class list
///   - `resbody.exception_variable.nil?` → no `=> e`
///
/// A block-level `rescue` (even bare) always has the protected body at the
/// `BeginNode` level (in `begin.statements()`), and the `RescueNode` itself
/// has no `statements()` (only `exceptions`, `exception`, and `subsequent`).
/// So a modifier rescue in this context is one with no exceptions, no
/// exception variable, and no subsequent clause.
fn is_modifier_rescue(rescue: &ruby_prism::RescueNode<'_>, has_protected_body: bool) -> bool {
    if !has_protected_body {
        return false;
    }
    if rescue.subsequent().is_some() {
        return false;
    }
    let exceptions: Vec<_> = rescue.exceptions().iter().collect();
    if !exceptions.is_empty() {
        return false;
    }
    if rescue.reference().is_some() {
        return false;
    }
    true
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        // on_block fires before on_send, like parser visits the block wrapper
        // before its inner send; both run before the node's frame is pushed.
        if let Some(call) = node.as_call_node() {
            if let Some(block) = call.block().and_then(|b| b.as_block_node()) {
                let name = String::from_utf8_lossy(call.name().as_slice()).into_owned();
                let body = block.body();
                self.on_block(
                    node_range(node),
                    loc_range(&block.opening_loc()),
                    loc_range(&block.closing_loc()),
                    &name,
                    node,
                    body.as_ref(),
                    call.arguments().is_some_and(|a| !a.arguments().is_empty()),
                    // `parenthesized?` is `loc_is?(:end, ')')` (an index
                    // call's delimiters are brackets).
                    call.closing_loc()
                        .is_some_and(|l| self.source[l.start_offset()] == b')'),
                );
            }
            self.on_send(&call);
            self.push_frame(node);
            return;
        }
        if let Some(sup) = node.as_super_node() {
            if let Some(block) = sup.block().and_then(|b| b.as_block_node()) {
                let body = block.body();
                self.on_block(
                    node_range(node),
                    loc_range(&block.opening_loc()),
                    loc_range(&block.closing_loc()),
                    "super",
                    node,
                    body.as_ref(),
                    sup.arguments().is_some_and(|a| !a.arguments().is_empty()),
                    sup.lparen_loc().is_some(),
                );
            }
            self.push_frame(node);
            return;
        }
        if let Some(fsup) = node.as_forwarding_super_node() {
            if let Some(block) = fsup.block() {
                let body = block.body();
                self.on_block(
                    node_range(node),
                    loc_range(&block.opening_loc()),
                    loc_range(&block.closing_loc()),
                    "super",
                    node,
                    body.as_ref(),
                    false,
                    false,
                );
            }
            self.push_frame(node);
            return;
        }
        if let Some(lambda) = node.as_lambda_node() {
            let body = lambda.body();
            self.on_block(
                node_range(node),
                loc_range(&lambda.opening_loc()),
                loc_range(&lambda.closing_loc()),
                "lambda",
                node,
                body.as_ref(),
                false,
                false,
            );
            self.push_frame(node);
            return;
        }
        self.push_frame(node);
    }

    fn leave(&mut self) {
        self.frames.pop();
    }

    fn enter_rescue(&mut self, node: &Node<'_>) {
        let stmts = if self.semantic {
            node.as_rescue_node()
                .map(|r| ListInfo::from_stmts(&r.statements()))
                .unwrap_or_default()
        } else {
            ListInfo::default()
        };
        self.frames.push(Frame::RescueClause { stmts });
    }

    fn leave_rescue(&mut self) {
        self.frames.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(style: u8) -> Config {
        Config {
            style,
            allow_braces_on_procedural_oneliners: false,
            procedural_methods: Vec::new(),
            functional_methods: Vec::new(),
            allowed_methods: ["lambda", "proc", "it"].map(String::from).to_vec(),
            braces_required_methods: Vec::new(),
        }
    }

    /// The vendor-spec `semantic` context config.
    fn semantic_cfg() -> Config {
        Config {
            style: 1,
            allow_braces_on_procedural_oneliners: false,
            procedural_methods: vec!["tap".to_string()],
            functional_methods: vec!["let".to_string()],
            allowed_methods: vec!["lambda".to_string()],
            braces_required_methods: Vec::new(),
        }
    }

    fn run(source: &str, cfg: &Config) -> BlockDelimitersResult {
        check_block_delimiters(source.as_bytes(), cfg, &[])
    }

    /// Apply the corrector ops the way `Parser::Source::TreeRewriter` merges
    /// them: point insertions before same-start replacements.
    fn apply(source: &str, offenses: &[Candidate]) -> String {
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        for offense in offenses {
            for op in &offense.ops {
                match op.kind {
                    0 => edits.push((op.start, op.end, op.text.clone())),
                    1 => edits.push((op.start, op.end, String::new())),
                    2 => edits.push((op.start, op.start, op.text.clone())),
                    3 => edits.push((op.end, op.end, op.text.clone())),
                    4 => {
                        edits.push((op.start, op.start, "begin\n".to_string()));
                        edits.push((op.end, op.end, "\nend".to_string()));
                    }
                    _ => unreachable!(),
                }
            }
        }
        edits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        let mut out = source.as_bytes().to_vec();
        for (start, end, text) in edits {
            out.splice(start..end, text.into_bytes());
        }
        String::from_utf8(out).unwrap()
    }

    fn corrected(source: &str, cfg: &Config) -> String {
        apply(source, &run(source, cfg).offenses)
    }

    #[test]
    fn line_count_based_basics() {
        let c = cfg(0);
        // Single-line do-end: offense on the `do` token, corrected to braces.
        let r = run("each do |x| end\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert_eq!((r.offenses[0].token), (5, 7));
        assert_eq!(
            r.offenses[0].message,
            "Prefer `{...}` over `do...end` for single-line blocks."
        );
        assert_eq!(corrected("each do |x| end\n", &c), "each { |x| }\n");
        assert_eq!(corrected("each do|x| x end\n", &c), "each { |x| x }\n");
        // Proper styles.
        assert!(run("each { |x| }\n", &c).offenses.is_empty());
        assert!(run("each do |x|\nend\n", &c).offenses.is_empty());
        // Multi-line braces.
        let r = run("each { |x|\n}\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert_eq!(
            r.offenses[0].message,
            "Avoid using `{...}` for multi-line blocks."
        );
        assert_eq!(corrected("each { |x|\n}\n", &c), "each do |x|\nend\n");
        assert_eq!(
            corrected("each{ |x|\n  some_method\n  other_method\n}\n", &c),
            "each do |x|\n  some_method\n  other_method\nend\n"
        );
    }

    #[test]
    fn delimiter_lines_decide_multiline() {
        // The receiver spans lines but the delimiters share one.
        assert!(
            run("foo(\n  bar\n).each { |x| x }\n", &cfg(0))
                .offenses
                .is_empty()
        );
    }

    #[test]
    fn on_send_ignores_blocks_in_unparenthesized_args() {
        let c = cfg(0);
        let r = run("puts [1, 2, 3].map { |n|\n  n * n\n}, 1\n", &c);
        assert!(r.offenses.is_empty());
        assert_eq!(r.send_ignores.len(), 1);
        // Nested send: the block is reached through the receiver.
        assert!(
            run("puts [0] + [1,2,3].map { |n|\n  n * n\n}, 1\n", &c)
                .offenses
                .is_empty()
        );
        // Parenthesized: not ignored, so the offense fires.
        assert_eq!(
            run("puts([1, 2, 3].map { |n|\n  n * n\n})\n", &c).offenses.len(),
            1
        );
        // Assignment methods don't ignore.
        assert_eq!(
            run("h2[k2] = Hash.new { |h3,k3|\n  h3[k3] = 0\n}\n", &c)
                .offenses
                .len(),
            1
        );
        // Operator method with a single block argument doesn't ignore (and
        // the chained block arg goes through the receiver descent).
        assert!(
            run("'%s' %\n  %w[foo].map { |v|\n    v\n  }.join(', ')\n", &c)
                .offenses
                .is_empty()
        );
        // Braceless hash values are reachable…
        assert!(
            run("my_method :arg1, arg2: proc {\n  something\n}, arg3: :v\n", &c)
                .offenses
                .is_empty()
        );
        // …braced hashes stop the descent (no ignore, no offense either way
        // here because `lambda` is allowed; use a non-allowed method).
        assert_eq!(
            run("my_method({ arg2: foo {\n  something\n} })\n", &c)
                .offenses
                .len(),
            1
        );
    }

    #[test]
    fn nested_offense_suppression_is_conditional() {
        let c = cfg(0);
        let r = run("foo {\n  bar do |x| x end\n}\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert_eq!(r.offenses[0].token, (4, 5));
        assert!(r.has_conditional);
        // An allowed outer block doesn't suppress (and isn't conditional).
        let r = run("proc {\n  bar do |x| x end\n}\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert_eq!(
            r.offenses[0].message,
            "Prefer `{...}` over `do...end` for single-line blocks."
        );
        assert!(!r.has_conditional);
    }

    #[test]
    fn prior_ignored_ranges_suppress() {
        let c = cfg(0);
        // Pass-2 style: the whole file range was ignored in a prior pass.
        let r = check_block_delimiters("bar do |x| x end\n".as_bytes(), &c, &[(0, 30)]);
        assert!(r.offenses.is_empty());
        assert!(!r.has_conditional);
    }

    #[test]
    fn require_do_end_quirk() {
        let c = cfg(0);
        // A real rescue clause with a class list requires do-end.
        assert!(
            run("foo do next unless bar; rescue StandardError; end\n", &c)
                .offenses
                .is_empty()
        );
        // A modifier rescue has no class list: offense.
        let src = "foo do next unless bar rescue StandardError; end\n";
        assert_eq!(run(src, &c).offenses.len(), 1);
        assert_eq!(
            corrected(src, &c),
            "foo { next unless bar rescue StandardError; }\n"
        );
        // Inline modifier rescue, corrected.
        assert_eq!(
            corrected("map do |x| x.y? rescue z end\n", &c),
            "map { |x| x.y? rescue z }\n"
        );
    }

    #[test]
    fn correction_would_break_code() {
        let c = cfg(0);
        let r = run("s.subspec 'Subspec' do |sp| end\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert!(r.offenses[0].ops.is_empty());
    }

    #[test]
    fn super_blocks() {
        let c = cfg(0);
        assert_eq!(corrected("super do x end\n", &c), "super { x }\n");
        assert_eq!(corrected("super(1) do x end\n", &c), "super(1) { x }\n");
        // Unparenthesized args: offense without correction.
        let r = run("super 1 do x end\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert!(r.offenses[0].ops.is_empty());
    }

    #[test]
    fn lambda_blocks() {
        // `lambda` is in AllowedMethods by default.
        assert!(run("foo = -> do x end\n", &cfg(0)).offenses.is_empty());
        let mut c = cfg(0);
        c.allowed_methods.clear();
        assert_eq!(corrected("foo = -> do x end\n", &c), "foo = -> { x }\n");
        assert_eq!(
            corrected("foo = ->(x) {\n  x\n}\n", &c),
            "foo = ->(x) do\n  x\nend\n"
        );
    }

    #[test]
    fn csend_blocks() {
        assert_eq!(
            corrected("x&.each do |i| i end\n", &cfg(0)),
            "x&.each { |i| i }\n"
        );
    }

    #[test]
    fn comment_moves() {
        let c = cfg(0);
        assert_eq!(
            corrected("baz.map { |x|\nfoo(x) } # comment\n", &c),
            "# comment\nbaz.map do |x|\nfoo(x) end\n"
        );
        assert_eq!(
            corrected("[foo {\n}] # comment\n", &c),
            "[# comment\nfoo do\nend]\n"
        );
        assert_eq!(
            corrected("baz.map { |x|\nfoo(x) }.map { |x| x.quux } # comment\n", &c),
            "# comment\nbaz.map do |x|\nfoo(x) end.map { |x| x.quux }\n"
        );
        assert_eq!(
            corrected("baz.map { |x|\nfoo(x) }.qux.quux # comment\n", &c),
            "# comment\nbaz.map do |x|\nfoo(x) end.qux.quux\n"
        );
        assert_eq!(
            corrected("baz.map { |x|\n} # comment\n\n", &c),
            "# comment\nbaz.map do |x|\nend\n"
        );
        assert_eq!(
            corrected("my_method { |x|\n  x.foo } unless bar   # comment\n", &c),
            "# comment\nmy_method do |x|\n  x.foo end unless bar\n"
        );
        // Chain continuing past the comment line (stock probe).
        assert_eq!(
            corrected("baz.map { |x|\nfoo(x) }.qux # comment\n  .quux\n", &c),
            "# comment\nbaz.map do |x|\nfoo(x) end.qux   .quux\n\n"
        );
        // Comment on the BEGIN line is not moved.
        assert_eq!(
            corrected("each { |x| # c\n  x\n}\n", &c),
            "each do |x| # c\n  x\nend\n"
        );
    }

    #[test]
    fn adjacent_braces() {
        assert_eq!(
            corrected("(0..3).each { |a| a.times {\n  puts a\n}}\n", &cfg(0)),
            "(0..3).each do |a| a.times {\n  puts a\n} end\n"
        );
    }

    #[test]
    fn semantic_style() {
        let c = semantic_cfg();
        // Procedural: return value unused.
        let r = run("each { |x|\n  x\n}\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert_eq!(
            r.offenses[0].message,
            "Prefer `do...end` over `{...}` for procedural blocks."
        );
        assert_eq!(corrected("each { |x|\n  x\n}\n", &c), "each do |x|\n  x\nend\n");
        // Functional: assigned / passed / chained / scope value.
        assert!(run("foo = map { |x|\n  x\n}\n", &c).offenses.is_empty());
        assert!(run("puts map { |x|\n  x\n}\n", &c).offenses.is_empty());
        assert!(run("map { |x|\n  x\n}.inspect\n", &c).offenses.is_empty());
        assert!(run("block do\n  map { |x|\n    x\n  }\nend\n", &c).offenses.is_empty());
        assert!(run("let(:foo) {\n  x\n}\n", &c).offenses.is_empty());
        // do-end with a used return value.
        let r = run("foo = map do |x|\n  x\nend\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert_eq!(
            r.offenses[0].message,
            "Prefer `{...}` over `do...end` for functional blocks."
        );
        assert_eq!(
            corrected("foo = map do |x|\n  x\nend\n", &c),
            "foo = map { |x|\n  x\n}\n"
        );
        assert_eq!(
            corrected("puts (map do |x|\n  x\nend)\n", &c),
            "puts (map { |x|\n  x\n})\n"
        );
        assert_eq!(
            corrected("foo.bar = map do |x|\n  x\nend\n", &c),
            "foo.bar = map { |x|\n  x\n}\n"
        );
        // Scope value through do-end is fine.
        assert!(run("block do\n  map do |x|\n    x\n  end\nend\n", &c).offenses.is_empty());
        // Conditions / logical operators / arrays / ranges.
        assert!(run("return if any? { |x| x }\n", &c).offenses.is_empty());
        assert!(run("if any? { |x| x }\n  return\nend\n", &c).offenses.is_empty());
        assert!(run("while foo { |x| x }\nend\n", &c).offenses.is_empty());
        assert!(run("case foo { |x| x }\nwhen bar\nend\n", &c).offenses.is_empty());
        assert!(run("any? { |c| c } || foo\n", &c).offenses.is_empty());
        assert!(run("[detect { true }, other]\n", &c).offenses.is_empty());
        assert!(run("detect { true }..other\n", &c).offenses.is_empty());
        assert!(run("ary.map { |e| foo(e) }&.bar\n", &c).offenses.is_empty());
        // Known procedural / allowed methods with do-end.
        assert!(run("foo = bar.tap do |x|\n  x.age = 3\nend\n", &c).offenses.is_empty());
        assert!(run("foo = lambda do\n  puts 42\nend\n", &c).offenses.is_empty());
        // Procedural one-liners.
        let r = run("each { |x| puts x }\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert_eq!(corrected("each { |x| puts x }\n", &c), "each do |x| puts x end\n");
        let mut allow = c.clone();
        allow.allow_braces_on_procedural_oneliners = true;
        assert!(run("each { |x| puts x }\n", &allow).offenses.is_empty());
        assert!(run("each do |x| puts x; end\n", &allow).offenses.is_empty());
    }

    #[test]
    fn semantic_parent_quirks() {
        let c = semantic_cfg();
        // kwbegin: the last statement is the scope value.
        assert!(run("begin\n  map { |x|\n    x\n  }\nend\n", &c).offenses.is_empty());
        assert_eq!(
            run("begin\n  map { |x|\n    x\n  }\n  foo\nend\n", &c).offenses.len(),
            1
        );
        // Top level: last statement of a multi-statement program.
        assert!(run("foo\nmap { |x|\n  x\n}\n", &c).offenses.is_empty());
        assert_eq!(run("map { |x|\n  x\n}\nfoo\n", &c).offenses.len(), 1);
        assert_eq!(run("map { |x|\n  x\n}\n", &c).offenses.len(), 1);
        // Hash pair value.
        assert!(run("foo(a: map { |x|\n  x\n})\n", &c).offenses.is_empty());
        // return.
        assert!(run("def f\n  return map { |x|\n    x\n  }\nend\n", &c).offenses.is_empty());
        // def body: single statement is the scope value; non-last is not.
        assert!(run("def f\n  map { |x|\n    x\n  }\nend\n", &c).offenses.is_empty());
        assert_eq!(
            run("def f\n  map { |x|\n    x\n  }\n  foo\nend\n", &c).offenses.len(),
            1
        );
        // String interpolation: always begin-wrapped.
        assert!(run("s = \"a#{map do |x|\n  x\nend}b\"\n", &c).offenses.is_empty());
        // if branches: single statement is a conditional child; multi-last
        // is the sequence value.
        assert!(run("if cond\n  map do |x|\n    x\n  end\nend\n", &c).offenses.is_empty());
        assert!(run("if cond\n  foo\n  map do |x|\n    x\n  end\nend\n", &c).offenses.is_empty());
        // else branch via IfNode#subsequent.
        assert!(
            run("if cond\n  x\nelse\n  map do |x|\n    x\n  end\nend\n", &c)
                .offenses
                .is_empty()
        );
        // rescue handler body.
        assert!(
            run("begin\n  foo\nrescue A\n  map do |x|\n    x\n  end\nend\n", &c)
                .offenses
                .is_empty()
        );
        // Protected body: a single statement is NOT the scope value…
        assert_eq!(
            run("begin\n  map { |x|\n    x\n  }\nrescue A\n  b\nend\n", &c)
                .offenses
                .len(),
            1
        );
        // …but the last of a multi-statement sequence is.
        assert!(
            run("begin\n  foo\n  map { |x|\n    x\n  }\nrescue A\n  b\nend\n", &c)
                .offenses
                .is_empty()
        );
    }

    #[test]
    fn semantic_rescue_wrap() {
        let c = semantic_cfg();
        assert_eq!(
            corrected(
                "x = map do |a|\n  do_something\nrescue StandardError => e\n  puts 'oh no'\nend\n",
                &c
            ),
            "x = map { |a|\n  begin\ndo_something\nrescue StandardError => e\n  puts 'oh no'\nend\n}\n"
        );
        assert_eq!(
            corrected(
                "x = map do |a|\n  do_something\nensure\n  puts 'oh no'\nend\n",
                &c
            ),
            "x = map { |a|\n  begin\ndo_something\nensure\n  puts 'oh no'\nend\n}\n"
        );
        // Empty protected body: the wrap starts at the rescue keyword.
        assert_eq!(
            corrected("x = map do |a|\nrescue A => e\n  puts 1\nend\n", &c),
            "x = map { |a|\nbegin\nrescue A => e\n  puts 1\nend\n}\n"
        );
        // Else clause extends the wrapped range.
        assert_eq!(
            corrected("x = map do |a|\n  c\nrescue A\n  b\nelse\n  d\nend\n", &c),
            "x = map { |a|\n  begin\nc\nrescue A\n  b\nelse\n  d\nend\n}\n"
        );
        // Empty ensure branch ends at the keyword.
        assert_eq!(
            corrected("x = map do |a|\n  c\nensure\nend\n", &c),
            "x = map { |a|\n  begin\nc\nensure\nend\n}\n"
        );
    }

    #[test]
    fn braces_for_chaining() {
        let c = cfg(2);
        let r = run("each do |x|\nend.map(&:to_s)\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert_eq!(
            r.offenses[0].message,
            "Prefer `{...}` over `do...end` for multi-line chained blocks."
        );
        assert_eq!(
            corrected("each do |x|\nend.map(&:to_s)\n", &c),
            "each { |x|\n}.map(&:to_s)\n"
        );
        assert_eq!(
            corrected("arr&.each do |x|\nend&.map(&:to_s)\n", &c),
            "arr&.each { |x|\n}&.map(&:to_s)\n"
        );
        let r = run("each { |x|\n}\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert_eq!(
            r.offenses[0].message,
            "Prefer `do...end` for multi-line blocks without chaining."
        );
        assert!(run("each { |x|\n}.map(&:to_sym)\n", &c).offenses.is_empty());
        // Chained via []: allowed.
        assert!(
            run("foo = [{foo: :bar}].find { |h|\n  h.key?(:foo)\n}[:foo]\n", &c)
                .offenses
                .is_empty()
        );
        // Single line: braces preferred.
        assert_eq!(run("each do |x| end\n", &c).offenses.len(), 1);
        assert!(run("each { |x| }\n", &c).offenses.is_empty());
        // Multi-line do-end without chaining is proper.
        assert!(run("each do |x|\nend\n", &c).offenses.is_empty());
        // do-end with rescue, chained: wrapped in begin..end.
        assert_eq!(
            corrected(
                "map do |a|\n  do_something\nrescue StandardError => e\n  puts 'oh no'\nend.join('-')\n",
                &c
            ),
            "map { |a|\n  begin\ndo_something\nrescue StandardError => e\n  puts 'oh no'\nend\n}.join('-')\n"
        );
    }

    #[test]
    fn always_braces() {
        let c = cfg(3);
        let r = run("each do |x|\nend\n", &c);
        assert_eq!(r.offenses.len(), 1);
        assert_eq!(r.offenses[0].message, "Prefer `{...}` over `do...end` for blocks.");
        assert_eq!(corrected("each do |x|\nend\n", &c), "each { |x|\n}\n");
        assert!(run("each { |x|\n}\n", &c).offenses.is_empty());
        assert!(run("each { |x|\n}.map(&:to_sym)\n", &c).offenses.is_empty());
    }

    #[test]
    fn braces_required_methods() {
        let mut c = cfg(0);
        c.braces_required_methods = vec!["sig".to_string()];
        assert!(
            run("sig {\n  params(\n    foo: string,\n  ).void\n}\ndef consume(foo)\n  foo\nend\n", &c)
                .offenses
                .is_empty()
        );
        let src = "sig do\n  params(\n    foo: string,\n  ).void\nend\ndef consume(foo)\n  foo\nend\n";
        let r = run(src, &c);
        assert_eq!(r.offenses.len(), 1);
        assert_eq!(
            r.offenses[0].message,
            "Brace delimiters `{...}` required for 'sig' method."
        );
        assert_eq!(
            corrected(src, &c),
            "sig {\n  params(\n    foo: string,\n  ).void\n}\ndef consume(foo)\n  foo\nend\n"
        );
    }

    #[test]
    fn raw_events_carry_method_names() {
        let events = check_block_delimiters_events("each do |x| end\n".as_bytes(), &cfg(0));
        let candidates: Vec<&Candidate> = events
            .iter()
            .filter_map(|e| match e {
                Event::Candidate(c) => Some(c),
                Event::Ignore(_) => None,
            })
            .collect();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].method_name, "each");
    }

    #[test]
    fn numblock_and_itblock() {
        let c = cfg(0);
        assert_eq!(corrected("each do _1 end\n", &c), "each { _1 }\n");
        assert!(run("each { _1 }\n", &c).offenses.is_empty());
        assert!(run("puts [1, 2, 3].map {\n  _1 * _1\n}, 1\n", &c).offenses.is_empty());
        assert_eq!(corrected("each do it end\n", &c), "each { it }\n");
    }
}
