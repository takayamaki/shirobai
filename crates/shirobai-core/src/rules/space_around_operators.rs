//! `Layout/SpaceAroundOperators`.
//!
//! Checks that binary operators are surrounded by exactly one space, with two
//! configurable exceptions (`**` and the rational-literal `/`) and an
//! `AllowForAlignment` option (default `true`) that permits extra spacing used
//! for vertical alignment.
//!
//! ## Hybrid structure
//!
//! Core *detection* is AST-driven, reproducing stock's per-node callbacks. Each
//! callback yields one operator to check (`check_operator(type, operator,
//! right_operand)`):
//!
//! - `on_send`: an operator-method `CallNode` (`+ - * / % ** << >> & | ^ < > <=
//!   >= <=> == === != =~ !~`). Setter calls (`x.y = 2`, `x[3] = 0`) route
//!   through the setter arm and check the `=`; otherwise a *regular* operator
//!   (not unary, no `.`/`::`, not `[] ! []=`) checks its message.
//! - the assignment write nodes (`loc.operator` = `=` / `+=` / `||=` / ...),
//!   `MultiWriteNode`, `And`/`Or` (`&&`/`||`), `AssocNode` (`=>`), `IfNode`
//!   ternary (`?` / `:`), `ClassNode` superclass (`<`), `SingletonClassNode`
//!   (`<<`), `RescueNode` (`=>`), and the Ruby 2.7+/3.0 pattern-matching
//!   operators (`|` alternation, `=>` capture, one-line `in` / `=>`).
//!
//! For each operator, `with_space = range_with_surrounding_space(operator)`
//! (expand over `[ \t]`, line continuations `\\\n`, then `\n`, both sides). The
//! offense decision has three arms (`offense_message`):
//!
//! 1. **"Space around operator detected"** — only `**` (when exponent style is
//!    `no_space`) and the rational `/` (rational style `no_space`): an offense
//!    iff there *is* surrounding space.
//! 2. **"Surrounding space missing"** — `with_space.source` is not `/^\s.*\s$/`
//!    (at least one side lacks a space). Alignment-independent.
//! 3. **"should be surrounded by a single space"** — there is excess leading or
//!    trailing space. **This is the only arm that consults `AllowForAlignment`.**
//!
//! ## Two-phase wiring (`AllowForAlignment`)
//!
//! Arms 1 and 2 are fully decided in the AST walk. Arm 3 needs the parser-gem
//! token list (`tokens_within(line)` / `assignment_tokens`), which shirobai
//! lexes in the walk-*outer* phase (the token cache shares a `RefCell` with the
//! AST parse, so it cannot be touched mid-walk). So the AST walk emits, for each
//! excess-space candidate, the data the alignment predicates need, and
//! [`resolve`] filters them in the walk-outer phase against the token list. This
//! is the same shape as `Layout/LineLength` collecting heredocs in the walk and
//! computing line lengths outside it.
//!
//! Offsets are **byte** offsets; the Ruby wrapper maps them through
//! `Shirobai::SourceOffsets`.

use ruby_prism::{
    AssocNode, CallNode, ClassNode, IfNode, Location, Node, RescueNode, SingletonClassNode, Visit,
};

use super::aligner::{Aligner, Tri};
use super::line_index::LineIndex;
use super::tokens::Token;

/// `EnforcedStyleForExponentOperator` / `EnforcedStyleForRationalLiterals`:
/// 0 = `no_space` (default), 1 = `space`.
pub const STYLE_NO_SPACE: u8 = 0;
pub const STYLE_SPACE: u8 = 1;

/// `Layout/SpaceAroundOperators` configuration.
#[derive(Clone, Copy)]
pub struct Config {
    /// `EnforcedStyleForExponentOperator`: `space` allows `a ** b`.
    pub exponent_style: u8,
    /// `EnforcedStyleForRationalLiterals`: `space` allows `1 / 48r`.
    pub rational_style: u8,
    /// `AllowForAlignment` (default true): permit extra spaces used to align.
    pub allow_for_alignment: bool,
    /// `Layout/HashAlignment`'s `EnforcedHashRocketStyle` includes `table`.
    /// A `=>` in a multi-line hash whose pairs are not all on one line is then
    /// left to `Layout/HashAlignment` (stock's `hash_table_style?` guard).
    pub hash_table_style: bool,
    /// `Layout/ExtraSpacing`'s `ForceEqualSignAlignment`. Changes the
    /// excess-space autocorrect of an `=`-ending operator from "replace with
    /// ` op `" to "insert a trailing space" (avoids an infinite-loop collision
    /// with ExtraSpacing's leading-space insertion).
    pub force_equal_sign_alignment: bool,
}

/// The message arm an offense reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    /// "Space around operator `op` detected." (`**` / rational `/`).
    Detected,
    /// "Surrounding space missing for operator `op`."
    Missing,
    /// "Operator `op` should be surrounded by a single space."
    SingleSpace,
}

/// One reported offense. The offense highlight is the operator range
/// `[op_start, op_end)`; the autocorrect target is the `with_space` range
/// `[ws_start, ws_end)`, rewritten to `replacement`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceAroundOperatorsOffense {
    pub op_start: usize,
    pub op_end: usize,
    pub ws_start: usize,
    pub ws_end: usize,
    pub kind: MessageKind,
    /// The operator text (for the message, e.g. `+=`).
    pub operator: Vec<u8>,
    /// The replacement applied to `[ws_start, ws_end)`.
    pub replacement: Vec<u8>,
}

/// Whether an operator routes through the assignment alignment path (`:assignment`)
/// or the generic operator alignment path (everything else). Stock distinguishes
/// only these two for `excess_leading_space?`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AlignType {
    /// `on_assignment` for a plain assignment (`lvasgn` / `casgn` / ... / `masgn`),
    /// *not* an op-assign (`op_asgn_type?` is `:special_asgn`).
    Assignment,
    /// Everything else (binary, ternary, pair, class, setter, op-assign, ...).
    Other,
}

/// An excess-space candidate deferred to the alignment phase. Arms 1 and 2 are
/// decided in the walk and emitted directly; this carries arm 3's inputs.
struct ExcessCandidate {
    op_start: usize,
    op_end: usize,
    ws_start: usize,
    ws_end: usize,
    operator: Vec<u8>,
    replacement: Vec<u8>,
    align_type: AlignType,
    /// `right_operand.source_range` for `excess_trailing_space?`.
    right_start: usize,
    right_end: usize,
}

/// The product of the AST walk: offenses already decided plus the excess-space
/// candidates that still need the token-based alignment check.
pub struct WalkResult {
    /// Arms 1 and 2 (and arm 3 once resolved): final offenses.
    decided: Vec<SpaceAroundOperatorsOffense>,
    /// Arm-3 candidates pending the alignment phase.
    excess: Vec<ExcessCandidate>,
    /// Byte positions of `=` operators that `remove_equals_in_def` excludes
    /// (optarg defaults and endless-def separators). Used by the alignment
    /// phase's `assignment_tokens`.
    def_equals: Vec<usize>,
}

/// Standalone entry point (per-cop ext fallback). Walks the AST, gets the
/// parser-gem token list from the same shared parse, and resolves alignment in
/// one shot.
pub fn check_space_around_operators(
    source: &[u8],
    config: Config,
) -> Vec<SpaceAroundOperatorsOffense> {
    // Collect the tokens first so this token consumer is the first toucher of
    // the shared parse cache (the entry is then built with tokens; `run_walk`'s
    // `with_parsed` reuses it with no re-parse). The tokens are the pm_lex
    // stream translated into the parser-gem shape the alignment phase reads.
    let tokens = super::parse_cache::with_parsed_and_tokens(source, |owner, _root, raw| {
        super::tokens::translate_tokens(owner, raw)
    });
    let walk = run_walk(source, config);
    resolve(source, config, walk, &tokens)
}

/// Run only the AST walk (phase 1). The bundle calls this through `build_rule`
/// on the shared walk and `resolve` separately in its walk-outer phase.
pub fn run_walk(source: &[u8], config: Config) -> WalkResult {
    let mut rule = build_rule(source, config);
    super::parse_cache::with_parsed(source, |_s, node| rule.visit(node));
    rule.finish()
}

/// Build the AST rule for the shared walk.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        config,
        decided: Vec::new(),
        excess: Vec::new(),
        def_equals: Vec::new(),
        hash_same_line: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: std::rc::Rc<LineIndex>,
    config: Config,
    decided: Vec<SpaceAroundOperatorsOffense>,
    excess: Vec<ExcessCandidate>,
    def_equals: Vec<usize>,
    /// Stack of the enclosing hash's `pairs_on_same_line?` (for the table-style
    /// `=>` guard). Top is the innermost hash/kwhash.
    hash_same_line: Vec<bool>,
}

impl<'a> Visitor<'a> {
    pub(crate) fn finish(self) -> WalkResult {
        WalkResult {
            decided: self.decided,
            excess: self.excess,
            def_equals: self.def_equals,
        }
    }

    fn s(&self, start: usize, end: usize) -> &'a [u8] {
        &self.source[start..end]
    }

    /// `range_with_surrounding_space(operator)`: expand `[op_start, op_end)`
    /// left and right over `[ \t]`, then a `\\\n` continuation, then a single
    /// run of `\n`, on both sides. Returns the expanded byte range.
    fn with_surrounding_space(&self, op_start: usize, op_end: usize) -> (usize, usize) {
        let begin = self.final_pos_left(op_start);
        let end = self.final_pos_right(op_end);
        (begin, end)
    }

    /// `final_pos` going left: spaces/tabs, then `\n`s. Stock's `final_pos`
    /// passes `continuations: false`, so the `\\\n` line-continuation move is a
    /// no-op (and must be skipped — consuming it past the `\n` would defeat the
    /// `with_space.source.start_with?("\n")` guard for an operator at the start
    /// of a continued line).
    fn final_pos_left(&self, mut pos: usize) -> usize {
        // move over [ \t] (char immediately left is src[pos-1]).
        while pos > 0 && matches!(self.source.get(pos - 1), Some(b' ') | Some(b'\t')) {
            pos -= 1;
        }
        // move over \n (newlines: true).
        while pos > 0 && self.source.get(pos - 1) == Some(&b'\n') {
            pos -= 1;
        }
        pos
    }

    /// `final_pos` going right: spaces/tabs, then `\n`s (continuations off).
    fn final_pos_right(&self, mut pos: usize) -> usize {
        let n = self.source.len();
        while pos < n && matches!(self.source.get(pos), Some(b' ') | Some(b'\t')) {
            pos += 1;
        }
        while pos < n && self.source.get(pos) == Some(&b'\n') {
            pos += 1;
        }
        pos
    }

    /// `check_operator(type, operator, right_operand)`. `op` is the operator
    /// byte range; `right` is the right-operand byte range (its `source_range`).
    fn check_operator(
        &mut self,
        align_type: AlignType,
        op_start: usize,
        op_end: usize,
        right_start: usize,
        right_end: usize,
        right_is_rational: bool,
    ) {
        let (ws_start, ws_end) = self.with_surrounding_space(op_start, op_end);
        let with_space = self.s(ws_start, ws_end);
        // `return if with_space.source.start_with?("\n")`.
        if with_space.first() == Some(&b'\n') {
            return;
        }
        // The comment exclusion (`comment_at_line`) needs the token list, which
        // is only available in the walk-outer phase, so it is applied uniformly
        // to every emitted offense in `resolve`.

        let operator = self.s(op_start, op_end).to_vec();
        let Some((kind, replacement)) =
            self.offense(op_start, op_end, ws_start, ws_end, right_is_rational, align_type)
        else {
            return;
        };

        match kind {
            MessageKind::SingleSpace => {
                // Arm 3: defer to the alignment phase.
                self.excess.push(ExcessCandidate {
                    op_start,
                    op_end,
                    ws_start,
                    ws_end,
                    operator,
                    replacement,
                    align_type,
                    right_start,
                    right_end,
                });
            }
            _ => {
                self.decided.push(SpaceAroundOperatorsOffense {
                    op_start,
                    op_end,
                    ws_start,
                    ws_end,
                    kind,
                    operator,
                    replacement,
                });
            }
        }
    }

    /// `offense_message` + the autocorrect replacement. Returns `None` when no
    /// offense, else `(kind, replacement)`. For arm 3 the replacement is still
    /// computed here (it does not depend on alignment).
    fn offense(
        &self,
        op_start: usize,
        op_end: usize,
        ws_start: usize,
        ws_end: usize,
        right_is_rational: bool,
        _align_type: AlignType,
    ) -> Option<(MessageKind, Vec<u8>)> {
        let op = self.s(op_start, op_end);
        let with_space = self.s(ws_start, ws_end);

        if self.should_not_have_surrounding_space(op, right_is_rational) {
            // `return if with_space.is?(operator.source)` — no surrounding space.
            if with_space == op {
                return None;
            }
            return Some((MessageKind::Detected, self.replacement(ws_start, ws_end, right_is_rational)));
        }

        // `!/^\s.*\s$/.match?(with_space.source)` — missing space on a side.
        if !is_surrounded_by_space(with_space) {
            return Some((MessageKind::Missing, self.replacement(ws_start, ws_end, right_is_rational)));
        }

        // Arm 3 candidate: excess leading or trailing space. The
        // `with_space.source.start_with?(EXCESSIVE_SPACE)` / `end_with?` gate
        // (the `  ` two-space prefix/suffix) is checked here so non-excess
        // single-spaced operators do not even reach the alignment phase.
        let lead_excess = with_space.starts_with(b"  ");
        let trail_excess = with_space.ends_with(b"  ");
        if lead_excess || trail_excess {
            return Some((
                MessageKind::SingleSpace,
                self.replacement(ws_start, ws_end, right_is_rational),
            ));
        }
        None
    }

    /// `should_not_have_surrounding_space?`: `**` (exponent no_space) or `/`
    /// whose RHS is a rational literal (rational no_space).
    fn should_not_have_surrounding_space(&self, op: &[u8], right_is_rational: bool) -> bool {
        if op == b"**" {
            self.config.exponent_style != STYLE_SPACE
        } else if op == b"/" {
            // `!space_around_slash_operator?(right)`: slash should be no_space
            // only when RHS is rational and rational style is not space.
            right_is_rational && self.config.rational_style != STYLE_SPACE
        } else {
            false
        }
    }

    /// The autocorrect replacement for the `with_space` range, mirroring
    /// `autocorrect`. (`force_equal_sign_alignment` insert-after case is
    /// handled by returning the special marker the Ruby wrapper recognizes.)
    fn replacement(&self, ws_start: usize, ws_end: usize, right_is_rational: bool) -> Vec<u8> {
        let range_source = self.s(ws_start, ws_end);
        if range_source.windows(2).any(|w| w == b"**")
            && self.config.exponent_style != STYLE_SPACE
        {
            return b"**".to_vec();
        }
        if range_source.contains(&b'/') && right_is_rational && self.config.rational_style != STYLE_SPACE
        {
            return b"/".to_vec();
        }
        if range_source.last() == Some(&b'\n') {
            // `" #{range_source.strip}\n"`.
            let mut out = vec![b' '];
            out.extend_from_slice(strip(range_source));
            out.push(b'\n');
            return out;
        }
        // enclose_operator_with_space.
        let operator = strip(range_source);
        if self.config.force_equal_sign_alignment && range_source.last() != Some(&b' ') {
            // `corrector.insert_after(range, ' ')`: the marker is the original
            // source with one trailing space (so applying it byte-equals
            // insert_after).
            let mut out = range_source.to_vec();
            out.push(b' ');
            return out;
        }
        let mut out = vec![b' '];
        out.extend_from_slice(operator);
        out.push(b' ');
        out
    }

}

/// `/^\s.*\s$/.match?(s)`: starts and ends with a Ruby `\s`
/// (`[ \t\r\n\f\v]`), with at least one char (the regex needs `\s` at both
/// ends, so length >= 2 unless the single char satisfies both — but `^\s` and
/// `\s$` on a 1-char string both match the same char).
fn is_surrounded_by_space(s: &[u8]) -> bool {
    let is_ws = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0x0b);
    match (s.first(), s.last()) {
        (Some(&f), Some(&l)) => is_ws(f) && is_ws(l),
        _ => false,
    }
}

/// Ruby `String#strip`: trim leading/trailing whitespace (`\0` too) bytes.
fn strip(s: &[u8]) -> &[u8] {
    let is_strip = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0c | 0x0b | 0);
    let mut start = 0;
    let mut end = s.len();
    while start < end && is_strip(s[start]) {
        start += 1;
    }
    while end > start && is_strip(s[end - 1]) {
        end -= 1;
    }
    &s[start..end]
}

// ===== resolve (phase 2: alignment + comment exclusion) =====

/// Resolve the walk's excess-space candidates against the token list and emit
/// the final offense list (decided arms 1/2 plus surviving arm-3 offenses), in
/// the walk's emission order interleaved by operator start position to match
/// stock's `add_offense` order (offenses are sorted by RuboCop before output,
/// so emission order only needs to be deterministic).
pub fn resolve(
    source: &[u8],
    config: Config,
    walk: WalkResult,
    tokens: &[Token],
) -> Vec<SpaceAroundOperatorsOffense> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    let aligner = Aligner::new(source, &line_index, tokens, &walk.def_equals);

    // Apply the comment exclusion (`comment_at_line`) to the arms decided in the
    // walk; it needs the token list available only here.
    let mut out: Vec<SpaceAroundOperatorsOffense> = walk
        .decided
        .into_iter()
        .filter(|o| !aligner.comment_excludes(o.op_start, o.ws_end))
        .collect();
    for c in &walk.excess {
        if aligner.comment_excludes(c.op_start, c.ws_end) {
            continue;
        }
        let excess_lead = source[c.ws_start..c.ws_end].starts_with(b"  ");
        let excess_trail = source[c.ws_start..c.ws_end].ends_with(b"  ");
        let leading =
            excess_lead && excess_leading_space(&aligner, config, c.align_type, c.op_start, c.op_end);
        let trailing =
            excess_trail && excess_trailing_space(&aligner, config, c.right_start, c.right_end);
        if leading || trailing {
            out.push(SpaceAroundOperatorsOffense {
                op_start: c.op_start,
                op_end: c.op_end,
                ws_start: c.ws_start,
                ws_end: c.ws_end,
                kind: MessageKind::SingleSpace,
                operator: c.operator.clone(),
                replacement: c.replacement.clone(),
            });
        }
    }
    out.sort_by_key(|o| o.op_start);
    out
}

// ===== resolve phase: SAO-specific alignment arms =====

/// `excess_leading_space?(type, operator, with_space)`. The
/// `with_space.source.start_with?(EXCESSIVE_SPACE)` gate is applied by the
/// caller; here we only do the alignment decision.
fn excess_leading_space(
    aligner: &Aligner<'_>,
    config: Config,
    align_type: AlignType,
    op_start: usize,
    op_end: usize,
) -> bool {
    // `return false unless allow_for_alignment?` — when alignment is *not*
    // allowed, a leading excess is *not* flagged by this arm (only excess
    // trailing space is, via `excess_trailing_space?`). This asymmetry is
    // stock's actual behavior.
    if !config.allow_for_alignment {
        return false;
    }
    if align_type != AlignType::Assignment {
        // `!aligned_with_operator?(operator)`.
        return !aligner.aligned_with_operator(op_start, op_end);
    }
    // The `:assignment` path.
    let align_preceding = aligner.aligned_with_preceding_equals_operator(op_start, op_end);
    let align_subsequent = aligner.aligned_with_subsequent_equals_operator(op_start, op_end);
    if align_preceding == Tri::Yes || align_subsequent == Tri::None {
        return false;
    }
    align_subsequent != Tri::Yes
}

/// `excess_trailing_space?(right_operand, with_space)`. The `with_space`
/// suffix gate is applied by the caller.
fn excess_trailing_space(
    aligner: &Aligner<'_>,
    config: Config,
    right_start: usize,
    right_end: usize,
) -> bool {
    !config.allow_for_alignment || !aligner.aligned_with_something(right_start, right_end)
}

// ===== AST visitor (callback reproduction) =====

impl<'pr> Visit<'pr> for Visitor<'_> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        self.on_send(node);
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_assoc_node(&mut self, node: &AssocNode<'pr>) {
        self.on_pair(node);
        ruby_prism::visit_assoc_node(self, node);
    }

    fn visit_hash_node(&mut self, node: &ruby_prism::HashNode<'pr>) {
        let same_line = self.pairs_on_same_line(node.elements().iter());
        self.hash_same_line.push(same_line);
        ruby_prism::visit_hash_node(self, node);
        self.hash_same_line.pop();
    }

    fn visit_keyword_hash_node(&mut self, node: &ruby_prism::KeywordHashNode<'pr>) {
        let same_line = self.pairs_on_same_line(node.elements().iter());
        self.hash_same_line.push(same_line);
        ruby_prism::visit_keyword_hash_node(self, node);
        self.hash_same_line.pop();
    }

    fn visit_if_node(&mut self, node: &IfNode<'pr>) {
        self.on_if(node);
        ruby_prism::visit_if_node(self, node);
    }

    fn visit_class_node(&mut self, node: &ClassNode<'pr>) {
        self.on_class(node);
        ruby_prism::visit_class_node(self, node);
    }

    fn visit_singleton_class_node(&mut self, node: &SingletonClassNode<'pr>) {
        self.on_sclass(node);
        ruby_prism::visit_singleton_class_node(self, node);
    }

    fn visit_rescue_node(&mut self, node: &RescueNode<'pr>) {
        self.on_resbody(node);
        ruby_prism::visit_rescue_node(self, node);
    }

    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        self.on_logical(node.operator_loc(), node.right().location());
        ruby_prism::visit_and_node(self, node);
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        self.on_logical(node.operator_loc(), node.right().location());
        ruby_prism::visit_or_node(self, node);
    }

    fn visit_alternation_pattern_node(&mut self, node: &ruby_prism::AlternationPatternNode<'pr>) {
        // `on_match_alt`: check the `|` operator against the whole node.
        let n = node.as_node();
        self.check_node_operator(AlignType::Other, node.operator_loc(), &n);
        ruby_prism::visit_alternation_pattern_node(self, node);
    }

    fn visit_capture_pattern_node(&mut self, node: &ruby_prism::CapturePatternNode<'pr>) {
        // `on_match_as`: check `=>` against the whole node.
        let n = node.as_node();
        self.check_node_operator(AlignType::Other, node.operator_loc(), &n);
        ruby_prism::visit_capture_pattern_node(self, node);
    }

    fn visit_match_predicate_node(&mut self, node: &ruby_prism::MatchPredicateNode<'pr>) {
        // one-line `in`: not an operator this cop checks (SpaceAroundKeyword's
        // job); stock `on_match_pattern` only fires for `MatchRequiredNode`
        // (`=>`), reached below. The `in` predicate's pattern still recurses.
        ruby_prism::visit_match_predicate_node(self, node);
    }

    fn visit_match_required_node(&mut self, node: &ruby_prism::MatchRequiredNode<'pr>) {
        // `on_match_pattern` (`=>`), Ruby >= 3.0 only. The whole node is the
        // right operand.
        let n = node.as_node();
        self.check_node_operator(AlignType::Other, node.operator_loc(), &n);
        ruby_prism::visit_match_required_node(self, node);
    }

    // Collect optarg / endless-def `=` positions for `remove_equals_in_def`.
    fn visit_optional_parameter_node(&mut self, node: &ruby_prism::OptionalParameterNode<'pr>) {
        self.def_equals.push(node.operator_loc().start_offset());
        ruby_prism::visit_optional_parameter_node(self, node);
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        // Endless def: `def foo = body`; `loc.assignment` is the `=`.
        if let Some(eq) = node.equal_loc() {
            self.def_equals.push(eq.start_offset());
        }
        ruby_prism::visit_def_node(self, node);
    }

    // Assignment write nodes: loc.operator = the assignment operator.
    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        self.on_assignment_plain(node.operator_loc(), node.value().location());
        ruby_prism::visit_local_variable_write_node(self, node);
    }
    fn visit_instance_variable_write_node(&mut self, node: &ruby_prism::InstanceVariableWriteNode<'pr>) {
        self.on_assignment_plain(node.operator_loc(), node.value().location());
        ruby_prism::visit_instance_variable_write_node(self, node);
    }
    fn visit_class_variable_write_node(&mut self, node: &ruby_prism::ClassVariableWriteNode<'pr>) {
        self.on_assignment_plain(node.operator_loc(), node.value().location());
        ruby_prism::visit_class_variable_write_node(self, node);
    }
    fn visit_global_variable_write_node(&mut self, node: &ruby_prism::GlobalVariableWriteNode<'pr>) {
        self.on_assignment_plain(node.operator_loc(), node.value().location());
        ruby_prism::visit_global_variable_write_node(self, node);
    }
    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'pr>) {
        self.on_assignment_plain(node.operator_loc(), node.value().location());
        ruby_prism::visit_constant_write_node(self, node);
    }
    fn visit_constant_path_write_node(&mut self, node: &ruby_prism::ConstantPathWriteNode<'pr>) {
        self.on_assignment_plain(node.operator_loc(), node.value().location());
        ruby_prism::visit_constant_path_write_node(self, node);
    }
    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        self.on_assignment_plain(node.operator_loc(), node.value().location());
        ruby_prism::visit_multi_write_node(self, node);
    }

    // Op-assign write nodes (`+=` / `||=` / `&&=`): :special_asgn (Other).
    fn visit_local_variable_operator_write_node(&mut self, node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>) {
        self.check_op_assign(node.binary_operator_loc(), node.value().location());
        ruby_prism::visit_local_variable_operator_write_node(self, node);
    }
    fn visit_instance_variable_operator_write_node(&mut self, node: &ruby_prism::InstanceVariableOperatorWriteNode<'pr>) {
        self.check_op_assign(node.binary_operator_loc(), node.value().location());
        ruby_prism::visit_instance_variable_operator_write_node(self, node);
    }
    fn visit_class_variable_operator_write_node(&mut self, node: &ruby_prism::ClassVariableOperatorWriteNode<'pr>) {
        self.check_op_assign(node.binary_operator_loc(), node.value().location());
        ruby_prism::visit_class_variable_operator_write_node(self, node);
    }
    fn visit_global_variable_operator_write_node(&mut self, node: &ruby_prism::GlobalVariableOperatorWriteNode<'pr>) {
        self.check_op_assign(node.binary_operator_loc(), node.value().location());
        ruby_prism::visit_global_variable_operator_write_node(self, node);
    }
    fn visit_constant_operator_write_node(&mut self, node: &ruby_prism::ConstantOperatorWriteNode<'pr>) {
        self.check_op_assign(node.binary_operator_loc(), node.value().location());
        ruby_prism::visit_constant_operator_write_node(self, node);
    }
    fn visit_constant_path_operator_write_node(&mut self, node: &ruby_prism::ConstantPathOperatorWriteNode<'pr>) {
        self.check_op_assign(node.binary_operator_loc(), node.value().location());
        ruby_prism::visit_constant_path_operator_write_node(self, node);
    }
    fn visit_local_variable_or_write_node(&mut self, node: &ruby_prism::LocalVariableOrWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_local_variable_or_write_node(self, node);
    }
    fn visit_instance_variable_or_write_node(&mut self, node: &ruby_prism::InstanceVariableOrWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_instance_variable_or_write_node(self, node);
    }
    fn visit_class_variable_or_write_node(&mut self, node: &ruby_prism::ClassVariableOrWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_class_variable_or_write_node(self, node);
    }
    fn visit_global_variable_or_write_node(&mut self, node: &ruby_prism::GlobalVariableOrWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_global_variable_or_write_node(self, node);
    }
    fn visit_constant_or_write_node(&mut self, node: &ruby_prism::ConstantOrWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_constant_or_write_node(self, node);
    }
    fn visit_constant_path_or_write_node(&mut self, node: &ruby_prism::ConstantPathOrWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_constant_path_or_write_node(self, node);
    }
    fn visit_local_variable_and_write_node(&mut self, node: &ruby_prism::LocalVariableAndWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_local_variable_and_write_node(self, node);
    }
    fn visit_instance_variable_and_write_node(&mut self, node: &ruby_prism::InstanceVariableAndWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_instance_variable_and_write_node(self, node);
    }
    fn visit_class_variable_and_write_node(&mut self, node: &ruby_prism::ClassVariableAndWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_class_variable_and_write_node(self, node);
    }
    fn visit_global_variable_and_write_node(&mut self, node: &ruby_prism::GlobalVariableAndWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_global_variable_and_write_node(self, node);
    }
    fn visit_constant_and_write_node(&mut self, node: &ruby_prism::ConstantAndWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_constant_and_write_node(self, node);
    }
    fn visit_constant_path_and_write_node(&mut self, node: &ruby_prism::ConstantPathAndWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_constant_path_and_write_node(self, node);
    }

    // Attribute op-assign / and/or-assign: `obj.attr += 1` (`self.foo ||= 1`).
    // Prism splits these into Call{Operator,Or,And}WriteNode; parser-gem makes
    // them an `op_asgn` (`on_op_asgn` -> `on_assignment`, :special_asgn).
    fn visit_call_operator_write_node(&mut self, node: &ruby_prism::CallOperatorWriteNode<'pr>) {
        self.check_op_assign(node.binary_operator_loc(), node.value().location());
        ruby_prism::visit_call_operator_write_node(self, node);
    }
    fn visit_call_or_write_node(&mut self, node: &ruby_prism::CallOrWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_call_or_write_node(self, node);
    }
    fn visit_call_and_write_node(&mut self, node: &ruby_prism::CallAndWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_call_and_write_node(self, node);
    }

    // Index op-assign / and/or-assign: `x[3] += 1`. Prism: IndexOperatorWriteNode etc.
    fn visit_index_operator_write_node(&mut self, node: &ruby_prism::IndexOperatorWriteNode<'pr>) {
        self.check_op_assign(node.binary_operator_loc(), node.value().location());
        ruby_prism::visit_index_operator_write_node(self, node);
    }
    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_index_or_write_node(self, node);
    }
    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        self.check_op_assign(node.operator_loc(), node.value().location());
        ruby_prism::visit_index_and_write_node(self, node);
    }
}

impl Visitor<'_> {
    /// `on_send`.
    fn on_send<'pr>(&mut self, node: &CallNode<'pr>) {
        // setter method? -> on_setter_method.
        if let Some(eq) = node.equal_loc() {
            // check_operator(:special_asgn, =, first_argument).
            if let Some(arg) = self.first_argument(node) {
                let (rs, re) = (arg.start_offset(), arg.end_offset());
                self.check_operator(
                    AlignType::Other,
                    eq.start_offset(),
                    eq.end_offset(),
                    rs,
                    re,
                    false,
                );
            }
            return;
        }
        // regular_operator? -> check the message operator.
        if !self.regular_operator(node) {
            return;
        }
        let Some(msg) = node.message_loc() else {
            return;
        };
        let right = self.first_argument(node);
        let (rs, re) = match &right {
            Some(r) => (r.start_offset(), r.end_offset()),
            None => (msg.end_offset(), msg.end_offset()),
        };
        let right_is_rational = self.location_is_rational(node, 0);
        self.check_operator(
            AlignType::Other,
            msg.start_offset(),
            msg.end_offset(),
            rs,
            re,
            right_is_rational,
        );
    }

    /// First argument location of a call (the `node.first_argument`).
    fn first_argument<'pr>(&self, node: &CallNode<'pr>) -> Option<Location<'pr>> {
        node.arguments()
            .and_then(|args| args.arguments().iter().next().map(|a| a.location()))
    }

    /// Whether the call's first argument is a rational literal.
    fn location_is_rational<'pr>(&self, node: &CallNode<'pr>, _start: usize) -> bool {
        node.arguments()
            .and_then(|args| args.arguments().iter().next())
            .map(|a| a.as_rational_node().is_some())
            .unwrap_or(false)
    }

    /// `regular_operator?`: not unary, no dot/double-colon, an operator method
    /// that is not in IRREGULAR_METHODS (`[] ! []=`).
    fn regular_operator<'pr>(&self, node: &CallNode<'pr>) -> bool {
        // unary_operation?: operator_method? && message starts at expression start.
        let Some(msg) = node.message_loc() else {
            return false;
        };
        if node.call_operator_loc().is_some() {
            // dot? or double_colon? -> excluded.
            return false;
        }
        let name = node.name();
        let name_bytes = name.as_slice();
        if !is_operator_method(name_bytes) {
            return false;
        }
        if is_irregular_method(name_bytes) {
            return false;
        }
        // unary: message at expression start.
        if msg.start_offset() == node.location().start_offset() {
            return false;
        }
        true
    }

    /// `on_pair`.
    fn on_pair<'pr>(&mut self, node: &AssocNode<'pr>) {
        let Some(op) = node.operator_loc() else {
            return; // not a hash_rocket? (a `key: value` pair has no operator).
        };
        // `return unless node.hash_rocket?` — only `=>` (not `:`).
        if self.s(op.start_offset(), op.end_offset()) != b"=>" {
            return;
        }
        // `return if hash_table_style? && !node.parent.pairs_on_same_line?`.
        // The enclosing hash's `pairs_on_same_line?` is on the stack top.
        if self.config.hash_table_style && !self.hash_same_line.last().copied().unwrap_or(false) {
            return;
        }
        // Stock passes the whole pair as `right_operand`
        // (`check_operator(:pair, node.loc.operator, node)`), so
        // `excess_trailing_space?` measures `aligned_with_something?` from the
        // pair's own `source_range` — i.e. the KEY column, where a column of
        // aligned `:key =>` pairs lines up. Using the value's range instead
        // would test the value column and miss that alignment.
        let pair = node.location();
        self.check_operator(
            AlignType::Other,
            op.start_offset(),
            op.end_offset(),
            pair.start_offset(),
            pair.end_offset(),
            false,
        );
    }

    /// `pairs_on_same_line?`: any two consecutive `pair` (AssocNode, kwsplat
    /// excluded) share a source line (`first.same_line?(second)` =
    /// `loc.last_line == other.loc.line || loc.line == other.loc.last_line`).
    fn pairs_on_same_line<'pr, I>(&self, elements: I) -> bool
    where
        I: Iterator<Item = Node<'pr>>,
    {
        let pairs: Vec<(usize, usize)> = elements
            .filter_map(|e| e.as_assoc_node().map(|a| a.as_node().location()))
            .map(|loc| {
                (
                    self.line_index.line_of(loc.start_offset()),
                    self.line_index.line_of(loc.end_offset().saturating_sub(1).max(loc.start_offset())),
                )
            })
            .collect();
        pairs.windows(2).any(|w| {
            let (first_line, first_last) = w[0];
            let (second_line, second_last) = w[1];
            first_last == second_line || first_line == second_last
        })
    }

    /// `on_if` (ternary only).
    fn on_if<'pr>(&mut self, node: &IfNode<'pr>) {
        // ternary?: then_keyword is `?` and there is an else `:`.
        let Some(q) = node.then_keyword_loc() else {
            return;
        };
        if self.s(q.start_offset(), q.end_offset()) != b"?" {
            return;
        }
        let Some(else_node) = node.subsequent() else {
            return;
        };
        let Some(else_node) = else_node.as_else_node() else {
            return;
        };
        let colon = else_node.else_keyword_loc();
        if self.s(colon.start_offset(), colon.end_offset()) != b":" {
            return;
        }
        // check_operator(:if, question, if_branch); check_operator(:if, colon, else_branch).
        let if_branch = node.statements().map(|s| s.location());
        let (qs, qe) = (q.start_offset(), q.end_offset());
        if let Some(b) = if_branch {
            self.check_operator(AlignType::Other, qs, qe, b.start_offset(), b.end_offset(), false);
        } else {
            self.check_operator(AlignType::Other, qs, qe, qe, qe, false);
        }
        let else_branch = else_node.statements().map(|s| s.location());
        let (cs, ce) = (colon.start_offset(), colon.end_offset());
        if let Some(b) = else_branch {
            self.check_operator(AlignType::Other, cs, ce, b.start_offset(), b.end_offset(), false);
        } else {
            self.check_operator(AlignType::Other, cs, ce, ce, ce, false);
        }
    }

    /// `on_class` (superclass `<`).
    fn on_class<'pr>(&mut self, node: &ClassNode<'pr>) {
        let Some(op) = node.inheritance_operator_loc() else {
            return;
        };
        let Some(superclass) = node.superclass() else {
            return;
        };
        let sc = superclass.location();
        self.check_operator(
            AlignType::Other,
            op.start_offset(),
            op.end_offset(),
            sc.start_offset(),
            sc.end_offset(),
            false,
        );
    }

    /// `on_sclass` (`<<`).
    fn on_sclass<'pr>(&mut self, node: &SingletonClassNode<'pr>) {
        let op = node.operator_loc();
        let body = node.expression().location();
        self.check_operator(
            AlignType::Other,
            op.start_offset(),
            op.end_offset(),
            body.start_offset(),
            body.end_offset(),
            false,
        );
    }

    /// `on_resbody` (rescue `=>`).
    fn on_resbody<'pr>(&mut self, node: &RescueNode<'pr>) {
        let Some(op) = node.operator_loc() else {
            return;
        };
        let Some(var) = node.reference() else {
            return;
        };
        let v = var.location();
        self.check_operator(
            AlignType::Other,
            op.start_offset(),
            op.end_offset(),
            v.start_offset(),
            v.end_offset(),
            false,
        );
    }

    /// `on_or` / `on_and` (`||` / `&&`).
    fn on_logical<'pr>(&mut self, op: Location<'pr>, right: Location<'pr>) {
        self.check_operator(
            AlignType::Other,
            op.start_offset(),
            op.end_offset(),
            right.start_offset(),
            right.end_offset(),
            false,
        );
    }

    /// Plain `on_assignment` (not op-assign): :assignment align type. The right
    /// operand is the value (`node.rhs`).
    fn on_assignment_plain<'pr>(&mut self, op: Location<'pr>, right: Location<'pr>) {
        self.check_operator(
            AlignType::Assignment,
            op.start_offset(),
            op.end_offset(),
            right.start_offset(),
            right.end_offset(),
            false,
        );
    }

    /// `on_assignment` for an op-assign (`op_asgn_type?`): :special_asgn (Other).
    fn check_op_assign<'pr>(&mut self, op: Location<'pr>, right: Location<'pr>) {
        self.check_operator(
            AlignType::Other,
            op.start_offset(),
            op.end_offset(),
            right.start_offset(),
            right.end_offset(),
            false,
        );
    }

    /// Pattern-matching operator check against a whole node as the right operand
    /// (stock passes the node itself).
    fn check_node_operator<'pr>(&mut self, align_type: AlignType, op: Location<'pr>, node: &Node<'pr>) {
        let loc = node.location();
        self.check_operator(
            align_type,
            op.start_offset(),
            op.end_offset(),
            loc.start_offset(),
            loc.end_offset(),
            false,
        );
    }
}

/// `operator_method?`: the method name is in OPERATOR_METHODS.
fn is_operator_method(name: &[u8]) -> bool {
    matches!(
        name,
        b"|" | b"^" | b"&" | b"<=>" | b"==" | b"===" | b"=~" | b">" | b">=" | b"<" | b"<="
            | b"<<" | b">>" | b"+" | b"-" | b"*" | b"/" | b"%" | b"**" | b"~" | b"+@" | b"-@"
            | b"!@" | b"~@" | b"[]" | b"[]=" | b"!" | b"!=" | b"!~" | b"`"
    )
}

/// `IRREGULAR_METHODS = %i[[] ! []=]`.
fn is_irregular_method(name: &[u8]) -> bool {
    matches!(name, b"[]" | b"!" | b"[]=")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default config: no_space exponent/rational, AllowForAlignment on.
    fn default_cfg() -> Config {
        Config {
            exponent_style: STYLE_NO_SPACE,
            rational_style: STYLE_NO_SPACE,
            allow_for_alignment: true,
            hash_table_style: false,
            force_equal_sign_alignment: false,
        }
    }

    fn run(src: &str, cfg: Config) -> Vec<(usize, usize, MessageKind)> {
        check_space_around_operators(src.as_bytes(), cfg)
            .into_iter()
            .map(|o| (o.op_start, o.op_end, o.kind))
            .collect()
    }

    /// A binary operator without spaces is a missing-space offense.
    #[test]
    fn missing_space_binary() {
        assert_eq!(
            run("a+b\n", default_cfg()),
            vec![(1, 2, MessageKind::Missing)]
        );
    }

    /// A clean single-spaced operator is not an offense.
    #[test]
    fn clean_binary() {
        assert!(run("a + b\n", default_cfg()).is_empty());
    }

    /// An assignment without spaces flags the `=`.
    #[test]
    fn missing_space_assignment() {
        assert_eq!(run("x=0\n", default_cfg()), vec![(1, 2, MessageKind::Missing)]);
    }

    /// A setter call `x.y=2` flags the `=` (the setter arm), not the `.`.
    #[test]
    fn setter_call() {
        assert_eq!(run("x.y=2\n", default_cfg()), vec![(3, 4, MessageKind::Missing)]);
    }

    /// `**` with spaces is a "space detected" offense under the default no_space
    /// exponent style; without spaces it is clean.
    #[test]
    fn exponent_no_space_style() {
        assert_eq!(
            run("a ** b\n", default_cfg()),
            vec![(2, 4, MessageKind::Detected)]
        );
        assert!(run("a**b\n", default_cfg()).is_empty());
    }

    /// Under `space` exponent style, `a**b` is missing-space and `a ** b` clean.
    #[test]
    fn exponent_space_style() {
        let cfg = Config {
            exponent_style: STYLE_SPACE,
            ..default_cfg()
        };
        assert_eq!(run("a**b\n", cfg), vec![(1, 3, MessageKind::Missing)]);
        assert!(run("a ** b\n", cfg).is_empty());
    }

    /// A rational-literal `/` with spaces is "space detected" (no_space default);
    /// a non-rational `/` with spaces is clean.
    #[test]
    fn rational_slash() {
        assert_eq!(
            run("a / 42r\n", default_cfg()),
            vec![(2, 3, MessageKind::Detected)]
        );
        assert!(run("a / 42\n", default_cfg()).is_empty());
    }

    /// Unary `-foo` / `!a` / a dotted operator call / a scope operator are not
    /// flagged (excluded from the `on_send` operator path).
    #[test]
    fn unary_and_excluded_operators() {
        assert!(run("-foo\n", default_cfg()).is_empty());
        assert!(run("!a\n", default_cfg()).is_empty());
        assert!(run("Date.today.+(1)\n", default_cfg()).is_empty());
        assert!(run("Zlib::GzipWriter\n", default_cfg()).is_empty());
        assert!(run("files[2]\n", default_cfg()).is_empty());
    }

    /// An aligned hash rocket with extra space is accepted under AllowForAlignment
    /// (the excess-trailing arm finds the alignment), but flagged when off.
    #[test]
    fn allow_for_alignment_hash() {
        let aligned = "{\n  1 =>  2,\n  11 => 3\n}\n";
        assert!(run(aligned, default_cfg()).is_empty());
        let cfg_off = Config {
            allow_for_alignment: false,
            ..default_cfg()
        };
        // With alignment off the extra trailing space is an offense.
        let off = run(aligned, cfg_off);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].2, MessageKind::SingleSpace);
    }

    /// Aligned assignments (`:assignment` path, preceding/subsequent `=`) are
    /// accepted; the alignment uses the assignment-token line tracking.
    #[test]
    fn allow_for_alignment_assignments() {
        let aligned = "@integer_message = 12345\n@output  = StringIO.new\n@logger  = Logger.new(x)\n";
        assert!(run(aligned, default_cfg()).is_empty());
    }

    /// With AllowForAlignment off, a *leading* excess space is NOT flagged (only
    /// a trailing excess is) — stock's `excess_leading_space?` asymmetry.
    #[test]
    fn no_alignment_leading_excess_not_flagged() {
        let cfg = Config {
            allow_for_alignment: false,
            ..default_cfg()
        };
        // `x   ||= 1`: leading excess, single trailing space -> no offense.
        assert!(run("x   ||= 1\n", cfg).is_empty());
        // `x ||=   1`: single leading, trailing excess -> flagged.
        let off = run("x ||=   1\n", cfg);
        assert_eq!(off, vec![(2, 5, MessageKind::SingleSpace)]);
    }

    /// An operator at the start of a continued line (`\` continuation) is not
    /// flagged (the `with_space` starts with a newline).
    #[test]
    fn continuation_line_operator() {
        assert!(run("a = b \\\n    && c\n", default_cfg()).is_empty());
    }

    /// A trailing-comment-aligned operator is accepted (the comment exclusion).
    #[test]
    fn trailing_comment_excluded() {
        assert!(run("foo +  # comment\n  bar\n", default_cfg()).is_empty());
    }

    /// A ternary without spaces flags both `?` and `:`.
    #[test]
    fn ternary() {
        let off = run("x == 0?1:2\n", default_cfg());
        // `?` at byte 6, `:` at byte 8.
        assert!(off.iter().any(|o| o.0 == 6 && o.2 == MessageKind::Missing));
        assert!(off.iter().any(|o| o.0 == 8 && o.2 == MessageKind::Missing));
    }

    /// The singleton-class `<<` is flagged when unspaced.
    #[test]
    fn sclass_operator() {
        assert_eq!(
            run("class<<self\nend\n", default_cfg()),
            vec![(5, 7, MessageKind::Missing)]
        );
    }

    /// A rescue `=>` is flagged when unspaced.
    #[test]
    fn rescue_assoc() {
        let off = run("begin\nrescue E=>e\nend\n", default_cfg());
        assert!(off.iter().any(|o| o.2 == MessageKind::Missing));
    }

    /// An attribute op-assign `uri.path+= x` flags the `+=` (CallOperatorWriteNode).
    #[test]
    fn attribute_op_assign() {
        let off = run("uri.path+= '/'\n", default_cfg());
        assert!(off.iter().any(|o| o.2 == MessageKind::Missing
            && &"uri.path+= '/'\n".as_bytes()[o.0..o.1] == b"+="));
    }
}
