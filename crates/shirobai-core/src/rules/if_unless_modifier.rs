//! `Style/IfUnlessModifier` (Rust core).
//!
//! Stock checks two directions on every non-ternary, non-elsif, non-else
//! `if`/`unless`:
//!
//! 1. A multiline `if cond ... end` whose modifier rewrite fits within
//!    `Layout/LineLength` `Max` gets "Favor modifier ... usage" and is
//!    rewritten with `to_modifier_form` (StatementModifier mixin).
//! 2. A single-line modifier form whose line is longer than `Max` gets
//!    "Modifier form ... makes the line too long" and is rewritten to the
//!    block form (`to_normal_form`, with comment-move and heredoc-move
//!    variants).
//!
//! Everything byte/AST-shaped is decided here: shape eligibility, the
//! reconstructed modifier form and its length, the block-form rewrite ops.
//! Two decisions depend on Ruby-side regexes and are finished by the wrapper:
//!
//! - the first-line comment's `comment_disables_cop?` check (it decides
//!   whether the comment is carried into the modifier form, which changes
//!   the length; we return both variants and both fit-flags), and
//! - the `Layout/LineLength` exemptions on the too-long direction
//!   (`AllowedPatterns`, `AllowURI`, cop directives, per-line disables).
//!   We emit a superset of candidates; the wrapper prunes.
//!
//! Parser-vs-prism mapping notes (all probed against stock):
//!
//! - parser materializes a `begin` node for 2+ statements in a body and for
//!   parenthesized expressions, but NOT for the direct children of a plain
//!   `kwbegin`. `another_statement_on_same_line?` stops at the first parser
//!   `begin`, so the climb here reproduces those materialization rules.
//! - parser `or_asgn`/`and_asgn`/`op_asgn` on a local carry an inner zero
//!   value `lvasgn` as their first child; `defined_argument_is_undefined?`
//!   sees it as a left sibling. Prism has no such node, so it is
//!   reproduced virtually (same for the `rescue => e` reference and the
//!   `for` index, which parser emits as `lvasgn`).
//! - parser `match_with_lvasgn` (named-capture `=~`) is ONE node; prism
//!   wraps a `CallNode` in a `MatchWriteNode`. `parenthesize?` must not see
//!   that CallNode as a send parent.

use std::rc::Rc;

use ruby_prism::{Node, StatementsNode, Visit};

use super::dispatch::{Interest, Rule};
use super::line_index::LineIndex;

/// Packed config: `max_line_length` mirrors stock's
/// `AutocorrectLogic#max_line_length` (`nil` when `Layout/LineLength` is
/// disabled -> `None`); `tab_width` mirrors `LineLengthHelp#tab_indentation_width`
/// (always an integer after the config fallbacks).
#[derive(Clone, Copy)]
pub struct Config {
    pub max_line_length: Option<i64>,
    pub tab_width: i64,
}

/// One corrector operation the wrapper applies verbatim.
/// `kind`: 0 = replace `[start, end)` with `text`, 1 = remove `[start, end)`.
pub struct Op {
    pub kind: u8,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// One candidate offense, in document (walk) order.
///
/// `kind` 0 = "use modifier" (direction 1), 1 = "too long" (direction 2).
/// For direction 1 the wrapper picks the with/without-comment variant after
/// running `comment_disables_cop?`; for direction 2 the wrapper applies the
/// `Layout/LineLength` regex exemptions and, if the offense stands, plays
/// back `ops`.
pub struct IfUnlessModifierCandidate {
    pub kind: u8,
    pub keyword_start: usize,
    pub keyword_end: usize,
    pub node_start: usize,
    pub node_end: usize,
    pub is_unless: bool,
    /// `another_modifier_if_on_same_line?` (correction skip, both directions).
    pub another_modifier_same_line: bool,
    /// Direction 1: first-line comment (`first_line_comment` candidate).
    pub has_comment: bool,
    pub comment_start: usize,
    pub comment_end: usize,
    /// Direction 1: `code_after(node)` is non-empty (with a non-disabling
    /// first-line comment this rejects the node, stock `non_eligible_node?`).
    pub has_code_after_end: bool,
    /// Direction 1: `length_in_modifier_form <= max` without / with the
    /// first-line comment appended.
    pub fits_no_comment: bool,
    pub fits_with_comment: bool,
    /// Direction 1: `to_modifier_form` without / with the comment.
    pub replacement_no_comment: String,
    pub replacement_with_comment: String,
    /// 1-based first line of the node (direction 2 Ruby-side checks).
    pub line_number: usize,
    /// Direction 2: the corrector ops (removals first, then the replace).
    pub ops: Vec<Op>,
}

/// Standalone entry point: same rule the bundle drives.
pub fn check_if_unless_modifier(source: &[u8], cfg: Config) -> Vec<IfUnlessModifierCandidate> {
    let mut rule = build_rule(source, cfg);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.candidates
}

/// Build the shared-walk rule. Collects comments up front (the parse cache
/// cannot be re-entered mid-walk).
pub(crate) fn build_rule(source: &[u8], cfg: Config) -> Visitor<'_> {
    let comments = super::parse_cache::comment_ranges(source);
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    let comment_lines: Vec<usize> = comments.iter().map(|c| line_index.line_of(c.0)).collect();
    Visitor {
        source,
        li: line_index,
        cfg,
        comments,
        comment_lines,
        frames: Vec::new(),
        candidates: Vec::new(),
    }
}

/// Byte-copy a `Node` into a `Node<'static>` (same scheme as
/// `empty_line_after_guard_clause`): prism's `Node` is plain pointers behind
/// a non-Copy `PhantomData`; every stashed copy is popped within the same
/// `dispatch::run`, inside the parse's lifetime held by `parse_cache`.
#[allow(clippy::missing_safety_doc)]
unsafe fn copy_to_static<'a>(node: &Node<'a>) -> Node<'static> {
    unsafe { std::mem::transmute_copy::<Node<'a>, Node<'static>>(node) }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    li: Rc<LineIndex>,
    cfg: Config,
    /// Comment byte ranges in document order + their 1-based first lines.
    comments: Vec<(usize, usize)>,
    comment_lines: Vec<usize>,
    frames: Vec<Node<'static>>,
    pub(crate) candidates: Vec<IfUnlessModifierCandidate>,
}

impl Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if node.as_if_node().is_some() || node.as_unless_node().is_some() {
            self.handle(node);
        }
        self.frames.push(unsafe { copy_to_static(node) });
    }

    fn leave(&mut self) {
        self.frames.pop();
    }

    fn enter_rescue(&mut self, node: &Node<'_>) {
        self.frames.push(unsafe { copy_to_static(node) });
    }

    fn leave_rescue(&mut self) {
        self.frames.pop();
    }

    fn interest(&self) -> Interest {
        // Needs the full ancestor stack (dstr ancestors, parser-parent
        // reconstruction) and the rescue frames; leaf nodes are never
        // ancestors of an `if`, so the leaf hook is dropped.
        Interest(Interest::LEAVE | Interest::RESCUE | Interest::ENTER_ALL)
    }
}

// ---------------------------------------------------------------------------
// Small text helpers (all byte-based; lengths are Ruby CHARACTER counts).

/// Ruby `String#length` for valid UTF-8: count non-continuation bytes.
fn char_count(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| (b & 0xC0) != 0x80).count()
}

/// Ruby `String#chomp`: strip one trailing `"\r\n"`, `"\n"` or `"\r"`.
fn chomp(bytes: &[u8]) -> &[u8] {
    if let Some(b) = bytes.strip_suffix(b"\r\n") {
        b
    } else if let Some(b) = bytes.strip_suffix(b"\n") {
        b
    } else if let Some(b) = bytes.strip_suffix(b"\r") {
        b
    } else {
        bytes
    }
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

impl<'a> Visitor<'a> {
    fn slice(&self, start: usize, end: usize) -> &'a [u8] {
        &self.source[start..end]
    }

    /// Byte range `[start, end)` of 1-based line `line` (without the `\n`).
    fn line_bounds(&self, line: usize) -> (usize, usize) {
        let starts = self.li.line_starts();
        let start = starts[line - 1];
        let end = if line < starts.len() {
            starts[line] - 1
        } else {
            self.source.len()
        };
        (start, end)
    }

    /// `LineLengthHelp#indentation_difference` (leading tabs only).
    fn indentation_difference(&self, line: &[u8]) -> i64 {
        // `tab_indentation_width` is always an integer after the config
        // fallbacks (stock's `|| 2` chain), so no `return 0 unless` arm.
        let index = if line.first() != Some(&b'\t') {
            0
        } else {
            line.iter().position(|&b| b != b'\t').unwrap_or(0)
        };
        index as i64 * (self.cfg.tab_width - 1)
    }

    /// `LineLengthHelp#line_length`: chars + tab adjustment.
    fn line_length(&self, line: &[u8]) -> i64 {
        char_count(line) as i64 + self.indentation_difference(line)
    }

    /// First comment whose (first) line is `line` (stock `comments.find`).
    fn comment_on_line(&self, line: usize) -> Option<(usize, usize)> {
        self.comment_lines
            .iter()
            .position(|&l| l == line)
            .map(|i| self.comments[i])
    }

    /// `processed_source.line_with_comment?(line)`.
    fn line_with_comment(&self, line: usize) -> bool {
        self.comment_lines.contains(&line)
    }

    /// `processed_source.contains_comment?(range)` — line-based: any comment
    /// STARTING on a line the range spans.
    fn contains_comment(&self, start: usize, end: usize) -> bool {
        let first = self.li.line_of(start);
        let last = self.li.line_of(end);
        self.comment_lines.iter().any(|&l| l >= first && l <= last)
    }

    /// `Node#nonempty_line_count`: lines of the node source with `/\S/`.
    fn nonempty_line_count(&self, start: usize, end: usize) -> usize {
        self.slice(start, end)
            .split(|&b| b == b'\n')
            .filter(|line| {
                line.iter()
                    .any(|&b| !matches!(b, b' ' | b'\t' | b'\r' | b'\x0B' | b'\x0C'))
            })
            .count()
    }

    /// Nearest ancestor frame that parser also sees as this node's parent.
    /// Prism's `ArgumentsNode` wrapper has no parser equivalent, and a
    /// `StatementsNode` reached through an `Option<Node>` body slot (parens,
    /// def/class/block bodies) DOES get a frame from the generic walk — both
    /// are transparent for parser-parent purposes (the statements list is
    /// found again through its owner's `stmts_lists_of`).
    fn effective_parent(&self) -> Option<&Node<'static>> {
        self.effective_ancestors().next()
    }

    /// Ancestor frames bottom-up with the transparent wrappers skipped
    /// (used for the parenthesize / pair -> hash / collection steps).
    fn effective_ancestors(&self) -> impl Iterator<Item = &Node<'static>> {
        self.frames
            .iter()
            .rev()
            .filter(|f| f.as_arguments_node().is_none() && f.as_statements_node().is_none())
    }

    // -----------------------------------------------------------------------
    // Main handler

    fn handle(&mut self, node: &Node<'_>) {
        let (kw_start, kw_end, end_kw, predicate, statements, is_unless) =
            if let Some(i) = node.as_if_node() {
                let Some(kw) = i.if_keyword_loc() else {
                    return; // ternary
                };
                if kw.as_slice() != b"if" {
                    return; // elsif
                }
                if i.subsequent().is_some() {
                    return; // else? (also true for elsif chains)
                }
                (
                    kw.start_offset(),
                    kw.end_offset(),
                    i.end_keyword_loc(),
                    i.predicate(),
                    i.statements(),
                    false,
                )
            } else if let Some(u) = node.as_unless_node() {
                if u.else_clause().is_some() {
                    return;
                }
                let kw = u.keyword_loc();
                (
                    kw.start_offset(),
                    kw.end_offset(),
                    u.end_keyword_loc(),
                    u.predicate(),
                    u.statements(),
                    true,
                )
            } else {
                return;
            };

        let loc = node.location();
        let (ns, ne) = (loc.start_offset(), loc.end_offset());
        let body_nodes: Vec<Node<'_>> = statements
            .as_ref()
            .map(|s| s.body().iter().collect())
            .unwrap_or_default();

        // Every check below is a pure rejection, so the order is free; the
        // cheap byte-shaped gates go first, the condition/subtree scans and
        // the parser-parent reconstructions go last.
        match end_kw {
            None => self.direction_too_long(
                kw_start, kw_end, ns, ne, &predicate, &body_nodes, is_unless,
            ),
            Some(end_kw) => self.direction_use_modifier(
                kw_start,
                kw_end,
                (end_kw.start_offset(), end_kw.end_offset()),
                ns,
                ne,
                &predicate,
                &body_nodes,
                is_unless,
            ),
        }
    }

    /// The `on_if`-top rejections shared by both directions: endless-def
    /// body, dstr ancestors, `defined?` with an undefined argument,
    /// pattern-matching in the condition.
    fn common_rejection(&self, ns: usize, body_nodes: &[Node<'_>], scan: &CondScan) -> bool {
        if body_nodes.len() == 1
            && let Some(d) = body_nodes[0].as_def_node()
            && d.equal_loc().is_some()
        {
            return true; // `endless_method?(node.body)`
        }
        if self
            .frames
            .iter()
            .any(|f| f.as_interpolated_string_node().is_some())
        {
            return true; // `node.ancestors.any?(&:dstr_type?)`
        }
        if scan.has_match_pattern {
            return true;
        }
        for arg in &scan.defined_args {
            match arg {
                DefinedArg::Send => return true,
                DefinedArg::Lvar(name) => {
                    if !self.left_sibling_lvasgn(ns, name) {
                        return true;
                    }
                }
                DefinedArg::Other => {}
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // Direction 1: multiline if/unless -> modifier form

    #[allow(clippy::too_many_arguments)]
    fn direction_use_modifier(
        &mut self,
        kw_start: usize,
        kw_end: usize,
        end_kw: (usize, usize),
        ns: usize,
        ne: usize,
        predicate: &Node<'_>,
        body_nodes: &[Node<'_>],
        is_unless: bool,
    ) {
        // Cheap byte-shaped gates first (`non_eligible_node?` /
        // `non_eligible_body?` pieces).
        if self.nonempty_line_count(ns, ne) > 3 {
            return;
        }
        if body_nodes.is_empty() || body_nodes.len() >= 2 {
            return; // nil body / parser begin
        }
        let body = &body_nodes[0];
        if body.as_parentheses_node().is_some() {
            return; // parser wraps parenthesized sole statements in `begin`
        }
        let body_loc = body.location();
        if body_loc.start_offset() == body_loc.end_offset() {
            return; // empty_source?
        }
        let last_line = self.li.line_of(ne);
        if self.line_with_comment(last_line) {
            return;
        }
        if self.contains_comment(body_loc.start_offset(), body_loc.end_offset()) {
            return;
        }
        // Condition / ancestor scans.
        let scan = scan_condition(predicate);
        if scan.has_lvasgn {
            return; // `non_eligible_condition?`
        }
        // `named_capture_in_condition?` suppresses the offense entirely for
        // block forms (the elsif arm needs `modifier_form?`).
        if predicate.as_match_write_node().is_some() {
            return;
        }
        if self.common_rejection(ns, body_nodes, &scan) {
            return;
        }
        if self.chained(ns, ne) {
            return;
        }
        if body_nodes.iter().any(subtree_has_non_elsif_if) {
            return; // nested_conditional? (candidates have no else branch)
        }
        if self.multiline_inside_collection(ns, ne) {
            return;
        }

        // `to_modifier_form` (without the first-line comment).
        let keyword: &[u8] = if is_unless { b"unless" } else { b"if" };
        let body_src = self.if_body_source(body);
        let cond_loc = predicate.location();
        let cond_src = self.slice(cond_loc.start_offset(), cond_loc.end_offset());
        let mut expr: Vec<u8> = Vec::new();
        expr.extend_from_slice(&body_src);
        expr.push(b' ');
        expr.extend_from_slice(keyword);
        expr.push(b' ');
        expr.extend_from_slice(cond_src);
        if self.parenthesize() {
            let mut wrapped = Vec::with_capacity(expr.len() + 2);
            wrapped.push(b'(');
            wrapped.extend_from_slice(&expr);
            wrapped.push(b')');
            expr = wrapped;
        }

        let first_line = self.li.line_of(ns);
        let comment = self.comment_on_line(first_line);
        let mut expr_with_comment = expr.clone();
        if let Some((cs, ce)) = comment {
            expr_with_comment.push(b' ');
            expr_with_comment.extend_from_slice(self.slice(cs, ce));
        }

        // `length_in_modifier_form`: code before the keyword on its line +
        // expression + code after `end` on its line.
        let line_start = self.li.line_starts()[first_line - 1];
        let code_before = self.slice(line_start, ns);
        let (_, end_line_end) = self.line_bounds(last_line);
        let code_after = self.slice(end_kw.1, end_line_end);
        let has_code_after_end = !code_after.is_empty();

        let assemble = |e: &[u8]| -> Vec<u8> {
            let mut s = Vec::with_capacity(code_before.len() + e.len() + code_after.len());
            s.extend_from_slice(code_before);
            s.extend_from_slice(e);
            s.extend_from_slice(code_after);
            s
        };
        let len_no = self.line_length(&assemble(&expr));
        let len_with = self.line_length(&assemble(&expr_with_comment));
        let fits_no_comment = self.cfg.max_line_length.is_none_or(|m| len_no <= m);
        if !fits_no_comment {
            // The with-comment form is never shorter, so no variant fits.
            return;
        }
        let fits_with_comment = self.cfg.max_line_length.is_none_or(|m| len_with <= m);

        let another = self.another_modifier_if_on_same_line(ns, first_line);
        self.candidates.push(IfUnlessModifierCandidate {
            kind: 0,
            keyword_start: kw_start,
            keyword_end: kw_end,
            node_start: ns,
            node_end: ne,
            is_unless,
            another_modifier_same_line: another,
            has_comment: comment.is_some(),
            comment_start: comment.map_or(0, |c| c.0),
            comment_end: comment.map_or(0, |c| c.1),
            has_code_after_end,
            fits_no_comment,
            fits_with_comment,
            replacement_no_comment: lossy(&expr),
            replacement_with_comment: lossy(&expr_with_comment),
            line_number: first_line,
            ops: Vec::new(),
        });
    }

    /// `StatementModifier#if_body_source`: a call whose last hash argument
    /// omits a value must be re-rendered with parentheses.
    fn if_body_source(&self, body: &Node<'_>) -> Vec<u8> {
        let loc = body.location();
        let plain = self.slice(loc.start_offset(), loc.end_offset()).to_vec();
        let Some(call) = body.as_call_node() else {
            return plain;
        };
        if call.name().as_slice() == b"[]=" {
            return plain;
        }
        // A block/block-pass makes the parser-side `last_argument` a
        // non-hash (block_pass) or turns the body into a `block` node —
        // both take the plain path.
        if call.block().is_some() {
            return plain;
        }
        let Some(args) = call.arguments() else {
            return plain;
        };
        let arg_nodes: Vec<Node<'_>> = args.arguments().iter().collect();
        let Some(last) = arg_nodes.last() else {
            return plain;
        };
        let assocs: Vec<ruby_prism::AssocNode<'_>> = if let Some(h) = last.as_hash_node() {
            h.elements().iter().filter_map(|e| e.as_assoc_node()).collect()
        } else if let Some(h) = last.as_keyword_hash_node() {
            h.elements().iter().filter_map(|e| e.as_assoc_node()).collect()
        } else {
            return plain;
        };
        let omission = assocs
            .last()
            .is_some_and(|a| a.value().as_implicit_node().is_some());
        if !omission {
            return plain;
        }
        // `method_source(if_body)(args.join(', '))`.
        let sel_end = match call.message_loc() {
            Some(m) => m.end_offset(),
            None => match call.call_operator_loc() {
                Some(d) => d.end_offset(), // implicit `.()` call
                None => return plain,
            },
        };
        let mut out = self.slice(loc.start_offset(), sel_end).to_vec();
        out.push(b'(');
        for (i, a) in arg_nodes.iter().enumerate() {
            if i > 0 {
                out.extend_from_slice(b", ");
            }
            let al = a.location();
            out.extend_from_slice(self.slice(al.start_offset(), al.end_offset()));
        }
        out.push(b')');
        out
    }

    /// `StatementModifier#parenthesize?` on the parser parent.
    fn parenthesize(&self) -> bool {
        let mut anc = self.effective_ancestors();
        let Some(parent) = anc.next() else {
            return false;
        };
        // parser `assignment?` / `operator_keyword?` / array / pair / send.
        if parent.as_and_node().is_some()
            || parent.as_or_node().is_some()
            || parent.as_array_node().is_some()
            || parent.as_assoc_node().is_some()
            || parent.as_multi_write_node().is_some()
            || parent.as_local_variable_write_node().is_some()
            || parent.as_local_variable_or_write_node().is_some()
            || parent.as_local_variable_and_write_node().is_some()
            || parent.as_local_variable_operator_write_node().is_some()
            || parent.as_instance_variable_write_node().is_some()
            || parent.as_instance_variable_or_write_node().is_some()
            || parent.as_instance_variable_and_write_node().is_some()
            || parent.as_instance_variable_operator_write_node().is_some()
            || parent.as_class_variable_write_node().is_some()
            || parent.as_class_variable_or_write_node().is_some()
            || parent.as_class_variable_and_write_node().is_some()
            || parent.as_class_variable_operator_write_node().is_some()
            || parent.as_global_variable_write_node().is_some()
            || parent.as_global_variable_or_write_node().is_some()
            || parent.as_global_variable_and_write_node().is_some()
            || parent.as_global_variable_operator_write_node().is_some()
            || parent.as_constant_write_node().is_some()
            || parent.as_constant_or_write_node().is_some()
            || parent.as_constant_and_write_node().is_some()
            || parent.as_constant_operator_write_node().is_some()
            || parent.as_constant_path_write_node().is_some()
            || parent.as_constant_path_or_write_node().is_some()
            || parent.as_constant_path_and_write_node().is_some()
            || parent.as_constant_path_operator_write_node().is_some()
            || parent.as_call_or_write_node().is_some()
            || parent.as_call_and_write_node().is_some()
            || parent.as_call_operator_write_node().is_some()
            || parent.as_index_or_write_node().is_some()
            || parent.as_index_and_write_node().is_some()
            || parent.as_index_operator_write_node().is_some()
        {
            return true;
        }
        if let Some(call) = parent.as_call_node() {
            // parser `csend` is not `send_type?`.
            if call
                .call_operator_loc()
                .is_some_and(|op| op.as_slice() == b"&.")
            {
                return false;
            }
            // parser folds `MatchWriteNode`'s inner call into ONE
            // `match_with_lvasgn` node, which is not a send.
            if let Some(grand) = anc.next()
                && let Some(mw) = grand.as_match_write_node()
            {
                let c = mw.call();
                if c.location().start_offset() == call.location().start_offset() {
                    return false;
                }
            }
            return true;
        }
        false
    }

    /// `Node#chained?`: parent is a call and we are its receiver.
    fn chained(&self, ns: usize, ne: usize) -> bool {
        let Some(parent) = self.effective_parent() else {
            return false;
        };
        let Some(call) = parent.as_call_node() else {
            return false;
        };
        call.receiver().is_some_and(|r| {
            let l = r.location();
            l.start_offset() == ns && l.end_offset() == ne
        })
    }

    // -----------------------------------------------------------------------
    // Direction 2: overlong single-line modifier -> block form

    #[allow(clippy::too_many_arguments)]
    fn direction_too_long(
        &mut self,
        kw_start: usize,
        kw_end: usize,
        ns: usize,
        ne: usize,
        predicate: &Node<'_>,
        body_nodes: &[Node<'_>],
        is_unless: bool,
    ) {
        let Some(max) = self.cfg.max_line_length else {
            return; // no max: `too_long_single_line?` is false
        };
        if self.slice(ns, ne).contains(&b'\n') {
            return; // not single-line
        }
        let line_no = self.li.line_of(ns);
        let (ls, le) = self.line_bounds(line_no);
        let line = self.slice(ls, le);
        if self.line_length(line) <= max {
            return;
        }
        let scan = scan_condition(predicate);
        if self.common_rejection(ns, body_nodes, &scan) {
            return;
        }
        if self.another_statement_on_same_line(ns, ne) {
            return;
        }
        let [body] = body_nodes else {
            return; // modifier form always has exactly one body statement
        };

        // `replacement_for_modifier_form` — pick the rewrite and its ops.
        let keyword: &[u8] = if is_unless { b"unless" } else { b"if" };
        let body_loc = body.location();
        let body_src = self.slice(body_loc.start_offset(), body_loc.end_offset());
        let cond_loc = predicate.location();
        let cond_src = self.slice(cond_loc.start_offset(), cond_loc.end_offset());
        let indent = " ".repeat(self.li.column(self.source, ns));

        let mut ops: Vec<Op> = Vec::new();
        let comment = self.comment_on_line(line_no);
        let moved_comment = comment.is_some_and(|(cs, ce)| {
            // `too_long_due_to_comment_after_modifier?` — plain char lengths
            // (NO tab adjustment here, unlike `line_length`).
            let source_length = char_count(line) as i64;
            let comment_length = char_count(self.slice(cs, ce)) as i64;
            source_length - comment_length <= max && max <= source_length
        });
        if moved_comment {
            let (cs, ce) = comment.unwrap();
            // `range_with_surrounding_space(side: :left)`: horizontal
            // whitespace, then newlines (defaults).
            let mut b = cs;
            while b > 0 && matches!(self.source[b - 1], b' ' | b'\t') {
                b -= 1;
            }
            while b > 0 && self.source[b - 1] == b'\n' {
                b -= 1;
            }
            ops.push(Op { kind: 1, start: b, end: ce, text: String::new() });
            let mut text: Vec<u8> = Vec::new();
            text.extend_from_slice(self.slice(cs, ce));
            text.push(b'\n');
            text.extend_from_slice(indent.as_bytes());
            text.extend_from_slice(body_src);
            text.push(b' ');
            text.extend_from_slice(keyword);
            text.push(b' ');
            text.extend_from_slice(cond_src);
            ops.push(Op { kind: 0, start: ns, end: ne, text: lossy(&text) });
        } else if let Some((hb, he)) = self.heredoc_in_if_branch(body) {
            ops.push(self.remove_whole_lines(hb));
            ops.push(self.remove_whole_lines(he));
            let mut text: Vec<u8> = Vec::new();
            text.extend_from_slice(keyword);
            text.push(b' ');
            text.extend_from_slice(cond_src);
            text.push(b'\n');
            text.extend_from_slice(indent.as_bytes());
            text.extend_from_slice(b"  ");
            text.extend_from_slice(body_src);
            text.push(b'\n');
            text.extend_from_slice(indent.as_bytes());
            text.extend_from_slice(b"  ");
            text.extend_from_slice(chomp(self.slice(hb.0, hb.1)));
            text.push(b'\n');
            text.extend_from_slice(indent.as_bytes());
            text.extend_from_slice(b"  ");
            text.extend_from_slice(chomp(self.slice(he.0, he.1)));
            text.push(b'\n');
            text.extend_from_slice(indent.as_bytes());
            text.extend_from_slice(b"end");
            ops.push(Op { kind: 0, start: ns, end: ne, text: lossy(&text) });
        } else {
            let mut text: Vec<u8> = Vec::new();
            text.extend_from_slice(keyword);
            text.push(b' ');
            text.extend_from_slice(cond_src);
            text.push(b'\n');
            text.extend_from_slice(indent.as_bytes());
            text.extend_from_slice(b"  ");
            text.extend_from_slice(body_src);
            text.push(b'\n');
            text.extend_from_slice(indent.as_bytes());
            text.extend_from_slice(b"end");
            ops.push(Op { kind: 0, start: ns, end: ne, text: lossy(&text) });
        }

        let another = self.another_modifier_if_on_same_line(ns, line_no);
        self.candidates.push(IfUnlessModifierCandidate {
            kind: 1,
            keyword_start: kw_start,
            keyword_end: kw_end,
            node_start: ns,
            node_end: ne,
            is_unless,
            another_modifier_same_line: another,
            has_comment: false,
            comment_start: 0,
            comment_end: 0,
            has_code_after_end: false,
            fits_no_comment: false,
            fits_with_comment: false,
            replacement_no_comment: String::new(),
            replacement_with_comment: String::new(),
            line_number: line_no,
            ops,
        });
    }

    /// Stock: `node.if_branch.last_argument if node.if_branch.send_type?`
    /// then `last_argument.heredoc?`. Returns the parser `heredoc_body` and
    /// `heredoc_end` byte ranges.
    fn heredoc_in_if_branch(&self, body: &Node<'_>) -> Option<((usize, usize), (usize, usize))> {
        let call = body.as_call_node()?;
        // parser `send_type?` excludes csend.
        if call
            .call_operator_loc()
            .is_some_and(|op| op.as_slice() == b"&.")
        {
            return None;
        }
        // A block-pass is parser's last argument (not a heredoc); a block
        // makes the parser body a `block` node (not `send_type?`).
        if call.block().is_some() {
            return None;
        }
        let args = call.arguments()?;
        let last = args.arguments().iter().last()?;
        heredoc_ranges(&last)
    }

    /// `range_by_whole_lines(range, include_final_newline: true)` as a
    /// remove op.
    fn remove_whole_lines(&self, range: (usize, usize)) -> Op {
        let start_line = self.li.line_of(range.0);
        let begin = self.li.line_starts()[start_line - 1];
        let end_line = self.li.line_of(range.1);
        let (_, le) = self.line_bounds(end_line);
        let end = (le + 1).min(self.source.len());
        Op { kind: 1, start: begin, end, text: String::new() }
    }

    // -----------------------------------------------------------------------
    // Parser-parent reconstruction

    /// `another_statement_on_same_line?`: climb to the first parser `begin`
    /// and look at the next sibling statement.
    fn another_statement_on_same_line(&self, ns: usize, ne: usize) -> bool {
        let line_no = self.li.line_of(ne);
        let mut child_start = ns;
        for anc in self.frames.iter().rev() {
            if anc.as_arguments_node().is_some() || anc.as_statements_node().is_some() {
                continue; // no parser equivalent (see `effective_parent`)
            }
            if let Some((list, idx)) = self.stmts_position(anc, child_start) {
                let materializes = anc.as_parentheses_node().is_some()
                    || (list.len() >= 2 && !kwbegin_direct_children(anc));
                if materializes {
                    return match list.get(idx + 1) {
                        Some(sib) => self.li.line_of(sib.location().start_offset()) == line_no,
                        None => false,
                    };
                }
            }
            child_start = anc.location().start_offset();
        }
        false
    }

    /// If `child_start` is one of `anc`'s statement-list elements, return
    /// the list and the index.
    fn stmts_position(
        &self,
        anc: &Node<'static>,
        child_start: usize,
    ) -> Option<(Vec<Node<'static>>, usize)> {
        for stmts in stmts_lists_of(anc) {
            let list: Vec<Node<'static>> = stmts
                .body()
                .iter()
                .map(|n| unsafe { copy_to_static(&n) })
                .collect();
            if let Some(idx) = list
                .iter()
                .position(|n| n.location().start_offset() == child_start)
            {
                return Some((list, idx));
            }
        }
        None
    }

    /// `find_containing_collection`: the parser parent (or, through a parser
    /// `begin`, the grandparent) when it is an array / call / hash-of-pair.
    fn find_containing_collection(&self, ns: usize) -> Option<Node<'static>> {
        let mut anc = self.effective_ancestors();
        let parent = anc.next()?;
        // Statements-level parents: only a parenthesized `begin` can put a
        // collection one level up; every other statements owner is not a
        // collection (and neither is its parent through a materialized
        // `begin`).
        if self.stmts_position(parent, ns).is_some() {
            parent.as_parentheses_node()?;
            let grand = anc.next()?;
            return self.collection_from(grand, &mut anc);
        }
        let parent = unsafe { copy_to_static(parent) };
        self.collection_from(&parent, &mut anc)
    }

    fn collection_from<'x>(
        &self,
        node: &Node<'static>,
        anc: &mut impl Iterator<Item = &'x Node<'static>>,
    ) -> Option<Node<'static>> {
        if node.as_array_node().is_some() || node.as_call_node().is_some() {
            return Some(unsafe { copy_to_static(node) });
        }
        if node.as_assoc_node().is_some() {
            // pair -> its parent hash.
            let hash = anc.next()?;
            if hash.as_hash_node().is_some() || hash.as_keyword_hash_node().is_some() {
                return Some(unsafe { copy_to_static(hash) });
            }
        }
        None
    }

    /// `multiline_inside_collection?` (direction 1 only).
    fn multiline_inside_collection(&self, ns: usize, ne: usize) -> bool {
        let Some(coll) = self.find_containing_collection(ns) else {
            return false;
        };
        let node_first = self.li.line_of(ns);
        let node_end_kw_line = self.li.line_of(ne);
        for child in collection_children(&coll) {
            let Some(inner) = unwrap_begin(&child) else {
                continue;
            };
            let Some((first, end_kw)) = if_lines(&inner) else {
                continue;
            };
            let inner_first = self.li.line_of(first);
            if inner_first == node_end_kw_line {
                return true;
            }
            if let Some(e) = end_kw
                && self.li.line_of(e) == node_first
            {
                return true;
            }
        }
        false
    }

    /// `another_modifier_if_on_same_line?` (correction skip).
    fn another_modifier_if_on_same_line(&self, ns: usize, line: usize) -> bool {
        let Some(coll) = self.find_containing_collection(ns) else {
            return false;
        };
        let mut scan = ModifierIfScan {
            self_start: ns,
            line,
            li: &self.li,
            found: false,
        };
        scan.visit(&coll);
        scan.found
    }

    // -----------------------------------------------------------------------
    // defined? left-sibling reconstruction

    /// `defined_argument_is_undefined?`'s sibling search: is any parser left
    /// sibling of the if an `lvasgn` of `name`? (Returning false makes the
    /// cop skip the node.)
    fn left_sibling_lvasgn(&self, ns: usize, name: &[u8]) -> bool {
        let Some(parent) = self.effective_ancestors().next() else {
            return false;
        };
        if let Some((list, idx)) = self.stmts_position(parent, ns) {
            // Multi-statement lists and parenthesized sole statements give
            // the preceding statements as parser left siblings; a sole
            // statement's parser parent is the owner itself.
            if list.len() >= 2 || parent.as_parentheses_node().is_some() {
                return list[..idx].iter().any(|n| is_lvasgn_named(n, name));
            }
            return self.positional_left_siblings(parent, ns, name, true);
        }
        self.positional_left_siblings(parent, ns, name, false)
    }

    /// Parser left siblings when the if is a positional child (or the sole
    /// statement, `via_stmts`) of `parent`.
    fn positional_left_siblings(
        &self,
        parent: &Node<'static>,
        ns: usize,
        name: &[u8],
        via_stmts: bool,
    ) -> bool {
        if let Some(i) = parent.as_if_node() {
            if i.predicate().location().start_offset() == ns {
                return false; // condition position: no left siblings
            }
            // then-branch: left sibling is the predicate; else-branch adds
            // the then-branch parser node (the sole statement, or a `begin`
            // that is never an lvasgn).
            let mut lefts: Vec<Node<'_>> = vec![i.predicate()];
            let in_then = i
                .statements()
                .is_some_and(|s| s.body().iter().any(|n| n.location().start_offset() == ns));
            if !in_then
                && let Some(stmts) = i.statements()
            {
                let body: Vec<Node<'_>> = stmts.body().iter().collect();
                if body.len() == 1 {
                    lefts.push(unsafe { copy_to_static(&body[0]) });
                }
            }
            return lefts.iter().any(|n| is_lvasgn_named(n, name));
        }
        if let Some(u) = parent.as_unless_node() {
            return is_lvasgn_named(&u.predicate(), name);
        }
        if let Some(w) = parent.as_while_node() {
            return is_lvasgn_named(&w.predicate(), name);
        }
        if let Some(w) = parent.as_until_node() {
            return is_lvasgn_named(&w.predicate(), name);
        }
        if let Some(w) = parent.as_when_node() {
            return w.conditions().iter().any(|n| is_lvasgn_named(&n, name));
        }
        if let Some(r) = parent.as_rescue_node() {
            // parser resbody: [exception array, reference (lvasgn!), body] —
            // the exceptions sit inside an array node, so only the reference
            // can be a direct lvasgn sibling.
            return r.reference().is_some_and(|n| is_lvasgn_named(&n, name));
        }
        if let Some(f) = parent.as_for_node() {
            // parser for: [index (lvasgn!), collection, body].
            if is_lvasgn_named(&f.index(), name) {
                return true;
            }
            return is_lvasgn_named(&f.collection(), name);
        }
        if via_stmts {
            // Sole statement of def/block/class/kwbegin/... — no lvasgn-able
            // parser left siblings.
            return false;
        }
        if let Some(a) = parent.as_array_node() {
            for n in a.elements().iter() {
                if n.location().start_offset() == ns {
                    break;
                }
                if is_lvasgn_named(&n, name) {
                    return true;
                }
            }
            return false;
        }
        if let Some(c) = parent.as_call_node() {
            if let Some(r) = c.receiver()
                && is_lvasgn_named(&r, name)
            {
                return true;
            }
            if let Some(args) = c.arguments() {
                for n in args.arguments().iter() {
                    if n.location().start_offset() == ns {
                        break;
                    }
                    if is_lvasgn_named(&n, name) {
                        return true;
                    }
                }
            }
            return false;
        }
        if let Some(a) = parent.as_and_node() {
            return is_lvasgn_named(&a.left(), name);
        }
        if let Some(o) = parent.as_or_node() {
            return is_lvasgn_named(&o.left(), name);
        }
        // parser or_asgn/and_asgn/op_asgn on a local: first child is a
        // zero-value `lvasgn` of the target.
        if let Some(w) = parent.as_local_variable_or_write_node() {
            return w.name().as_slice() == name;
        }
        if let Some(w) = parent.as_local_variable_and_write_node() {
            return w.name().as_slice() == name;
        }
        if let Some(w) = parent.as_local_variable_operator_write_node() {
            return w.name().as_slice() == name;
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Node-shape helpers (free functions)

/// The statements lists parser flattens into this node's children/bodies.
fn stmts_lists_of<'pr>(node: &Node<'pr>) -> Vec<StatementsNode<'pr>> {
    let mut out = Vec::new();
    if let Some(p) = node.as_program_node() {
        out.push(p.statements());
    } else if let Some(d) = node.as_def_node() {
        if let Some(s) = d.body().and_then(|b| b.as_statements_node()) {
            out.push(s);
        }
    } else if let Some(c) = node.as_class_node() {
        if let Some(s) = c.body().and_then(|b| b.as_statements_node()) {
            out.push(s);
        }
    } else if let Some(m) = node.as_module_node() {
        if let Some(s) = m.body().and_then(|b| b.as_statements_node()) {
            out.push(s);
        }
    } else if let Some(sc) = node.as_singleton_class_node() {
        if let Some(s) = sc.body().and_then(|b| b.as_statements_node()) {
            out.push(s);
        }
    } else if let Some(b) = node.as_block_node() {
        if let Some(s) = b.body().and_then(|b| b.as_statements_node()) {
            out.push(s);
        }
    } else if let Some(l) = node.as_lambda_node() {
        if let Some(s) = l.body().and_then(|b| b.as_statements_node()) {
            out.push(s);
        }
    } else if let Some(b) = node.as_begin_node() {
        if let Some(s) = b.statements() {
            out.push(s);
        }
        if let Some(s) = b.else_clause().and_then(|e| e.statements()) {
            out.push(s);
        }
        if let Some(s) = b.ensure_clause().and_then(|e| e.statements()) {
            out.push(s);
        }
    } else if let Some(r) = node.as_rescue_node() {
        if let Some(s) = r.statements() {
            out.push(s);
        }
    } else if let Some(i) = node.as_if_node() {
        if let Some(s) = i.statements() {
            out.push(s);
        }
        if let Some(s) = i
            .subsequent()
            .and_then(|sub| sub.as_else_node())
            .and_then(|e| e.statements())
        {
            out.push(s);
        }
    } else if let Some(u) = node.as_unless_node() {
        if let Some(s) = u.statements() {
            out.push(s);
        }
        if let Some(s) = u.else_clause().and_then(|e| e.statements()) {
            out.push(s);
        }
    } else if let Some(e) = node.as_else_node() {
        if let Some(s) = e.statements() {
            out.push(s);
        }
    } else if let Some(e) = node.as_ensure_node() {
        if let Some(s) = e.statements() {
            out.push(s);
        }
    } else if let Some(w) = node.as_while_node() {
        if let Some(s) = w.statements() {
            out.push(s);
        }
    } else if let Some(w) = node.as_until_node() {
        if let Some(s) = w.statements() {
            out.push(s);
        }
    } else if let Some(f) = node.as_for_node() {
        if let Some(s) = f.statements() {
            out.push(s);
        }
    } else if let Some(w) = node.as_when_node() {
        if let Some(s) = w.statements() {
            out.push(s);
        }
    } else if let Some(w) = node.as_in_node() {
        if let Some(s) = w.statements() {
            out.push(s);
        }
    } else if let Some(c) = node.as_case_node() {
        if let Some(s) = c.else_clause().and_then(|e| e.statements()) {
            out.push(s);
        }
    } else if let Some(c) = node.as_case_match_node() {
        if let Some(s) = c.else_clause().and_then(|e| e.statements()) {
            out.push(s);
        }
    } else if let Some(p) = node.as_parentheses_node()
        && let Some(s) = p.body().and_then(|b| b.as_statements_node())
    {
        out.push(s);
    }
    out
}

/// A plain `begin; ...; end` keeps its statements as direct parser children
/// (no `begin` node); rescue/ensure/else re-introduce it.
fn kwbegin_direct_children(node: &Node<'_>) -> bool {
    let Some(b) = node.as_begin_node() else {
        return false;
    };
    b.begin_keyword_loc().is_some()
        && b.rescue_clause().is_none()
        && b.ensure_clause().is_none()
        && b.else_clause().is_none()
}

/// parser children of a collection node (`array` / `send`|`csend` / hash),
/// in document order, skipping non-node children (the selector symbol).
fn collection_children<'pr>(coll: &Node<'pr>) -> Vec<Node<'pr>> {
    if let Some(a) = coll.as_array_node() {
        return a.elements().iter().collect();
    }
    if let Some(c) = coll.as_call_node() {
        let mut out: Vec<Node<'pr>> = Vec::new();
        if let Some(r) = c.receiver() {
            out.push(r);
        }
        if let Some(args) = c.arguments() {
            out.extend(args.arguments().iter());
        }
        return out;
    }
    if let Some(h) = coll.as_hash_node() {
        return h.elements().iter().collect();
    }
    if let Some(h) = coll.as_keyword_hash_node() {
        return h.elements().iter().collect();
    }
    Vec::new()
}

/// `unwrap_begin`: pair -> value, then parenthesized -> first statement.
fn unwrap_begin<'pr>(child: &Node<'pr>) -> Option<Node<'pr>> {
    let mut cur = if let Some(a) = child.as_assoc_node() {
        a.value()
    } else {
        unsafe { std::mem::transmute_copy::<Node<'pr>, Node<'pr>>(child) }
    };
    if let Some(p) = cur.as_parentheses_node() {
        cur = p.body().and_then(|b| {
            b.as_statements_node()
                .and_then(|s| s.body().iter().next())
        })?;
    }
    Some(cur)
}

/// For an if/unless node (excluding ternaries): its start offset and the
/// `end` keyword offset (None for modifier forms).
fn if_lines(node: &Node<'_>) -> Option<(usize, Option<usize>)> {
    if let Some(i) = node.as_if_node() {
        i.if_keyword_loc()?; // ternary has no keyword
        return Some((
            node.location().start_offset(),
            i.end_keyword_loc().map(|l| l.start_offset()),
        ));
    }
    if let Some(u) = node.as_unless_node() {
        return Some((
            node.location().start_offset(),
            u.end_keyword_loc().map(|l| l.start_offset()),
        ));
    }
    None
}

/// parser `lvasgn` equivalents: a local write, or the target forms parser
/// materializes as `lvasgn` (rescue reference, for index, mlhs entries).
fn is_lvasgn_named(node: &Node<'_>, name: &[u8]) -> bool {
    if let Some(w) = node.as_local_variable_write_node() {
        return w.name().as_slice() == name;
    }
    if let Some(t) = node.as_local_variable_target_node() {
        return t.name().as_slice() == name;
    }
    false
}

/// Heredoc body/end parser ranges for a (possibly interpolated) string or
/// xstring node, or None when the node is not a heredoc.
fn heredoc_ranges(node: &Node<'_>) -> Option<((usize, usize), (usize, usize))> {
    let (opening, content, closing) = if let Some(s) = node.as_string_node() {
        (
            s.opening_loc()?,
            Some((s.content_loc().start_offset(), s.content_loc().end_offset())),
            s.closing_loc()?,
        )
    } else if let Some(x) = node.as_x_string_node() {
        (
            x.opening_loc(),
            Some((x.content_loc().start_offset(), x.content_loc().end_offset())),
            x.closing_loc(),
        )
    } else if let Some(s) = node.as_interpolated_string_node() {
        let parts: Vec<Node<'_>> = s.parts().iter().collect();
        let content = match (parts.first(), parts.last()) {
            (Some(f), Some(l)) => Some((
                f.location().start_offset(),
                l.location().end_offset(),
            )),
            _ => None,
        };
        (s.opening_loc()?, content, s.closing_loc()?)
    } else if let Some(s) = node.as_interpolated_x_string_node() {
        let parts: Vec<Node<'_>> = s.parts().iter().collect();
        let content = match (parts.first(), parts.last()) {
            (Some(f), Some(l)) => Some((
                f.location().start_offset(),
                l.location().end_offset(),
            )),
            _ => None,
        };
        (s.opening_loc(), content, s.closing_loc())
    } else {
        return None;
    };
    if !opening.as_slice().starts_with(b"<<") {
        return None;
    }
    // parser heredoc_end excludes the trailing newline prism includes.
    let mut ce = closing.end_offset();
    let cs = closing.start_offset();
    if ce > cs && node_slice_ends_newline(closing.as_slice()) {
        ce -= 1;
    }
    // Empty (interpolated) heredoc: zero-width body at the delimiter start.
    let body = content.unwrap_or((cs, cs));
    Some((body, (cs, ce)))
}

fn node_slice_ends_newline(slice: &[u8]) -> bool {
    slice.last() == Some(&b'\n')
}

// ---------------------------------------------------------------------------
// Condition subtree scan

enum DefinedArg {
    Lvar(Vec<u8>),
    Send,
    Other,
}

struct CondScan {
    has_lvasgn: bool,
    has_match_pattern: bool,
    defined_args: Vec<DefinedArg>,
}

fn scan_condition(cond: &Node<'_>) -> CondScan {
    let mut scan = CondScanVisitor {
        out: CondScan {
            has_lvasgn: false,
            has_match_pattern: false,
            defined_args: Vec::new(),
        },
        match_write_targets: Vec::new(),
    };
    scan.visit(cond);
    scan.out
}

struct CondScanVisitor {
    out: CondScan,
    /// Start offsets of `MatchWriteNode` targets: parser's
    /// `match_with_lvasgn` has NO `lvasgn` children, so these prism
    /// `LocalVariableTargetNode`s must not count for `non_eligible_condition?`.
    match_write_targets: Vec<usize>,
}

impl CondScanVisitor {
    fn check(&mut self, node: &Node<'_>) {
        if let Some(mw) = node.as_match_write_node() {
            self.match_write_targets
                .extend(mw.targets().iter().map(|t| t.location().start_offset()));
        }
        // parser materializes an inner zero-value `lvasgn` child for
        // `x op= ...` on a local, so the shorthand writes count too.
        if node.as_local_variable_write_node().is_some()
            || node.as_local_variable_or_write_node().is_some()
            || node.as_local_variable_and_write_node().is_some()
            || node.as_local_variable_operator_write_node().is_some()
        {
            self.out.has_lvasgn = true;
        }
        if let Some(t) = node.as_local_variable_target_node()
            && !self
                .match_write_targets
                .contains(&t.as_node().location().start_offset())
        {
            self.out.has_lvasgn = true;
        }
        if node.as_match_predicate_node().is_some() || node.as_match_required_node().is_some() {
            self.out.has_match_pattern = true;
        }
        if let Some(d) = node.as_defined_node() {
            let value = d.value();
            let arg = if let Some(l) = value.as_local_variable_read_node() {
                DefinedArg::Lvar(l.name().as_slice().to_vec())
            } else if let Some(c) = value.as_call_node() {
                // parser `send` excludes csend.
                if c.call_operator_loc()
                    .is_some_and(|op| op.as_slice() == b"&.")
                {
                    DefinedArg::Other
                } else {
                    DefinedArg::Send
                }
            } else {
                DefinedArg::Other
            };
            self.out.defined_args.push(arg);
        }
    }
}

impl<'pr> Visit<'pr> for CondScanVisitor {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        self.check(&node);
    }

    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        self.check(&node);
    }
}

/// `nested_conditional?`: any non-elsif if/unless/ternary in the subtree
/// (including the root).
fn subtree_has_non_elsif_if(node: &Node<'_>) -> bool {
    let mut scan = NestedIfScan { found: false };
    scan.visit(node);
    scan.found
}

struct NestedIfScan {
    found: bool,
}

impl NestedIfScan {
    fn check(&mut self, node: &Node<'_>) {
        if let Some(i) = node.as_if_node() {
            // elsif keyword nodes are allowed; ternaries (no keyword) count.
            match i.if_keyword_loc() {
                Some(kw) if kw.as_slice() == b"elsif" => {}
                _ => self.found = true,
            }
        } else if node.as_unless_node().is_some() {
            self.found = true;
        }
    }
}

impl<'pr> Visit<'pr> for NestedIfScan {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        self.check(&node);
    }
}

/// `another_modifier_if_on_same_line?`'s descendant scan: a DIFFERENT
/// modifier-form if/unless starting on `line`.
struct ModifierIfScan<'s> {
    self_start: usize,
    line: usize,
    li: &'s LineIndex,
    found: bool,
}

impl ModifierIfScan<'_> {
    fn check(&mut self, node: &Node<'_>) {
        let modifier = if let Some(i) = node.as_if_node() {
            i.if_keyword_loc()
                .is_some_and(|kw| kw.as_slice() == b"if")
                && i.end_keyword_loc().is_none()
        } else if let Some(u) = node.as_unless_node() {
            u.end_keyword_loc().is_none()
        } else {
            false
        };
        if !modifier {
            return;
        }
        let start = node.location().start_offset();
        if start != self.self_start && self.li.line_of(start) == self.line {
            self.found = true;
        }
    }
}

impl<'pr> Visit<'pr> for ModifierIfScan<'_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        self.check(&node);
    }
}
