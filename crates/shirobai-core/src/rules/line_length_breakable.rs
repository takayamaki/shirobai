//! Autocorrection support for `Layout/LineLength`.
//!
//! The detection side ([`super::line_length`]) only finds over-long lines. The
//! upstream cop can also *break* certain long lines by inserting a newline (and,
//! for `SplitStrings`, a string-continuation). This module ports that part: it
//! walks the AST once and, for every source line, computes where (if anywhere) a
//! break may be inserted.
//!
//! The result is a per-line `(line_index, insert_offset, delimiter)`:
//! - `insert_offset` is the byte offset *before which* the corrector inserts.
//! - `delimiter` is empty for an ordinary newline break, or the string quote
//!   (`'` / `"`) for a `SplitStrings` break (the Ruby side then inserts
//!   `<delim> \<newline><delim>` instead of a bare newline).
//!
//! Mirrors `RuboCop::Cop::Layout::LineLength`'s
//! `check_for_breakable_node` / `_block` / `_semicolons` / `_str` / `_dstr`
//! and `RuboCop::Cop::CheckLineBreakable`.

use super::line_index::LineIndex;
use ruby_prism::{Node, Visit};
use std::collections::HashSet;

/// A break that may be inserted on a particular source line. At most one per
/// line (the first builder to claim a line wins, matching upstream's
/// `breakable_range_by_line_index` write-once-per-line behaviour).
pub struct Breakable {
    pub line_index: usize,
    pub insert_offset: usize,
    /// Empty = newline break; `'`/`"` = string-split continuation delimiter.
    pub delimiter: String,
}

/// Compute every per-line breakable insertion point for `source`.
///
/// `max` is the configured `Max`; `split_strings` is the `SplitStrings` option
/// (when false, string/dstr splitting is disabled, matching upstream).
pub fn compute_breakables(source: &[u8], max: usize, split_strings: bool) -> Vec<Breakable> {
    compute_breakables_filtered(source, max, split_strings, None)
}

/// Like [`compute_breakables`], but when `candidate_lines` is `Some`, only the
/// breakable on a line whose 0-based index is in the set is computed.
///
/// The AST is still walked in full (the walk is cheap), but the expensive
/// break-point extraction is skipped for any node whose claim line is not a
/// candidate. Because a line that is not a `LineLength` candidate (length ≤
/// `Max`) can never become an offense, it can never consume a breakable range,
/// so the output for candidate lines is identical to the unfiltered output.
pub fn compute_breakables_filtered(
    source: &[u8],
    max: usize,
    split_strings: bool,
    candidate_lines: Option<&HashSet<usize>>,
) -> Vec<Breakable> {
    // No candidate line can become an offense, so no breakable is ever
    // consumed. Skip the parse + AST walk + byte scans entirely.
    if candidate_lines.is_some_and(|c| c.is_empty()) {
        return Vec::new();
    }
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    super::parse_cache::with_parsed(source, |source, node| {
        // Collect literal/comment ranges from the already-parsed AST (we must
        // not re-enter the parse cache while it is borrowed).
        let literals = collect_literal_ranges(node);
        let comments = comment_byte_ranges_with(source, &literals);
        let comment_lines: std::collections::BTreeSet<usize> = comments
            .iter()
            .map(|&(s, _)| line_index.line_of(s))
            .collect();

        let mut v = BreakableVisitor {
            source,
            line_index: &line_index,
            max,
            split_strings,
            candidate_lines,
            stack: Vec::new(),
            ranges: std::collections::BTreeMap::new(),
            delimiters: std::collections::BTreeMap::new(),
            literals,
            comments,
            comment_lines,
            string_parent_start: None,
        };
        // Semicolons are claimed first (upstream does this in
        // `on_new_investigation`, before the node walk), so a node never
        // overrides a semicolon break on the same line.
        v.collect_semicolons(node);
        v.visit(node);
        v.into_breakables()
    })
}

fn char_count(bytes: &[u8]) -> usize {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.chars().count(),
        Err(_) => bytes.len(),
    }
}

/// Last column (0-based char column of the end offset) on the node's *last*
/// line. Mirrors `source_range.last_column`.
fn last_column(li: &LineIndex, source: &[u8], end_off: usize) -> usize {
    li.column(source, end_off)
}

/// Lightweight ancestor frame. Prism has no parent pointers, so we track the
/// chain of breakable-relevant ancestors ourselves.
#[derive(Clone)]
struct Frame {
    kind: FrameKind,
    first_line: usize,
    /// What this node looks like *as a parent* to its direct children.
    parent_info: ParentInfo,
}

#[derive(Clone)]
enum FrameKind {
    /// A hash/array/call collection, with its (process_args-applied) element
    /// line/column data needed by the "contained by breakable collection"
    /// predicates.
    Collection {
        elements: Vec<ElemLoc>,
        breakable: bool,
    },
    Other,
}

/// Information a child needs about its immediate (effective) parent for the
/// string-splitting predicates.
#[derive(Clone, Copy, Default)]
struct ParentInfo {
    /// Parent is a `pair` / `kwoptarg` / `array` — strings inside are not split
    /// (the collection is broken instead).
    forbids_string_split: bool,
    /// Parent is a `dstr` (interpolated string); `(begin_offset, end_offset)` of
    /// its opening quote, used to derive a string part's delimiter.
    dstr_open: Option<(usize, usize)>,
    /// Byte offset of the parent node's start (for `column_offset_between`).
    start_offset: usize,
}

#[derive(Clone, Copy)]
struct ElemLoc {
    first_line: usize,
    last_line: usize,
}

struct BreakableVisitor<'a> {
    source: &'a [u8],
    line_index: &'a LineIndex,
    max: usize,
    split_strings: bool,
    /// When `Some`, only lines whose 0-based index is in the set may produce a
    /// breakable; other lines are skipped before the expensive extraction.
    candidate_lines: Option<&'a HashSet<usize>>,
    stack: Vec<Frame>,
    ranges: std::collections::BTreeMap<usize, usize>,
    delimiters: std::collections::BTreeMap<usize, String>,
    literals: Vec<(usize, usize)>,
    comments: Vec<(usize, usize)>,
    comment_lines: std::collections::BTreeSet<usize>,
    /// Explicit parent start offset used while processing dstr string parts.
    string_parent_start: Option<usize>,
}

impl BreakableVisitor<'_> {
    fn into_breakables(self) -> Vec<Breakable> {
        self.ranges
            .into_iter()
            .map(|(line_index, insert_offset)| Breakable {
                line_index,
                insert_offset,
                delimiter: self
                    .delimiters
                    .get(&line_index)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect()
    }

    /// Whether the given 0-based line index may produce a breakable. Always
    /// true when no candidate filter is active.
    fn is_candidate(&self, line_index: usize) -> bool {
        match self.candidate_lines {
            Some(set) => set.contains(&line_index),
            None => true,
        }
    }

    fn claim(&mut self, line_index: usize, insert_offset: usize) {
        self.ranges.entry(line_index).or_insert(insert_offset);
    }

    fn claim_string(&mut self, line_index: usize, insert_offset: usize, delimiter: String) {
        if self.ranges.contains_key(&line_index) {
            return;
        }
        self.ranges.insert(line_index, insert_offset);
        self.delimiters.insert(line_index, delimiter);
    }

    /// `check_for_breakable_semicolons` — claim a break after each statement
    /// semicolon (a `;` that is a `tSEMI` token, i.e. not inside a string or
    /// comment), mirroring upstream's reverse-iteration overwrite.
    fn collect_semicolons(&mut self, _node: &Node<'_>) {
        if !self.source.contains(&b';') {
            return;
        }
        let len = self.source.len();

        // Semicolon offsets in source order.
        let mut semis: Vec<usize> = Vec::new();
        for (i, &b) in self.source.iter().enumerate() {
            if b == b';' && !in_ranges(&self.literals, i) && !in_ranges(&self.comments, i) {
                semis.push(i);
            }
        }

        // Reverse iteration with overwrite, matching `tokens.reverse_each`.
        for &semi in semis.iter().rev() {
            let end_pos = semi + 1;
            if end_pos >= len {
                continue;
            }
            // `same_line?(next_range, range)` — next char on the same line.
            let next = self.source[end_pos];
            if next == b'\r' || next == b'\n' {
                continue;
            }
            if next == b';' {
                continue;
            }
            let line_index = self.line_index.line_of(semi) - 1;
            if !self.is_candidate(line_index) {
                continue;
            }
            // Overwrite (semicolons win and overwrite earlier semicolons).
            self.ranges.insert(line_index, end_pos);
        }
    }

    fn line_length_chars(&self, line_index: usize) -> usize {
        // Character length of the source line (without trailing newline),
        // matching `processed_source.lines[i].length`.
        let mut start = 0usize;
        let mut idx = 0usize;
        let len = self.source.len();
        let mut pos = 0usize;
        while pos < len {
            if self.source[pos] == b'\n' {
                if idx == line_index {
                    return char_count(&self.source[start..pos]);
                }
                idx += 1;
                start = pos + 1;
            }
            pos += 1;
        }
        if idx == line_index {
            char_count(&self.source[start..len])
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------------
// extract_breakable_node and friends
// ---------------------------------------------------------------------------

/// process_args-applied element list of a node, as `(node, column, first_line,
/// last_line)`. Returns `None` when the node is not a breakable kind.
fn collection_elements<'pr>(
    source: &[u8],
    node: &Node<'pr>,
) -> Option<(Vec<Node<'pr>>, bool, bool)> {
    // (elements, is_call, is_def)
    if let Some(call) = node.as_call_node() {
        let args = call
            .arguments()
            .map(|a| process_args(a.arguments().iter().collect::<Vec<_>>()))
            .unwrap_or_default();
        Some((args, true, false))
    } else if let Some(def) = node.as_def_node() {
        let params = def.parameters().map(|p| def_params(&p)).unwrap_or_default();
        Some((params, false, true))
    } else if let Some(arr) = node.as_array_node() {
        Some((arr.elements().iter().collect(), false, false))
    } else if let Some(h) = node.as_hash_node() {
        Some((h.elements().iter().collect(), false, false))
    } else {
        let _ = source;
        None
    }
}

/// def's parameter list, flattened in source order (matching parser-gem
/// `def_node.arguments`).
fn def_params<'pr>(p: &ruby_prism::ParametersNode<'pr>) -> Vec<Node<'pr>> {
    let mut out: Vec<Node<'pr>> = Vec::new();
    out.extend(p.requireds().iter());
    out.extend(p.optionals().iter());
    if let Some(r) = p.rest() {
        out.push(r);
    }
    out.extend(p.posts().iter());
    out.extend(p.keywords().iter());
    if let Some(kr) = p.keyword_rest() {
        out.push(kr);
    }
    if let Some(b) = p.block() {
        out.push(b.as_node());
    }
    out
}

/// `process_args`: if the last argument is a braceless keyword hash, splice in
/// its pairs in place of the hash.
fn process_args<'pr>(mut args: Vec<Node<'pr>>) -> Vec<Node<'pr>> {
    if let Some(last) = args.last()
        && let Some(kw) = last.as_keyword_hash_node()
    {
        // A `KeywordHashNode` is a braceless trailing hash; flatten it.
        let pairs: Vec<Node<'pr>> = kw.elements().iter().collect();
        args.pop();
        args.extend(pairs);
    }
    args
}

/// Byte offset of the position `n_chars` characters after `begin` (clamped to
/// the source length).
fn offset_after_chars(source: &[u8], begin: usize, n_chars: usize) -> usize {
    let rest = &source[begin..];
    match std::str::from_utf8(rest) {
        Ok(s) => {
            begin
                + s.char_indices()
                    .nth(n_chars)
                    .map(|(i, _)| i)
                    .unwrap_or(s.len())
        }
        Err(_) => (begin + n_chars).min(source.len()),
    }
}

/// `rindex(/\\(u[\da-f]{0,4}|x[\da-f]{0,2})?\z/)` — the char index of a trailing
/// escape sequence (the backslash), if the substring ends in one.
fn trailing_escape_char_index(substr: &str) -> Option<usize> {
    let chars: Vec<char> = substr.chars().collect();
    // Find the rightmost backslash whose suffix (to end) matches the pattern.
    for i in (0..chars.len()).rev() {
        if chars[i] != '\\' {
            continue;
        }
        let suffix = &chars[i + 1..];
        if escape_suffix_matches(suffix) {
            return Some(i);
        }
    }
    None
}

/// Whether the chars after a backslash match `(u[\da-f]{0,4}|x[\da-f]{0,2})?` to
/// the end.
fn escape_suffix_matches(suffix: &[char]) -> bool {
    if suffix.is_empty() {
        return true; // bare backslash at end
    }
    let is_hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
    match suffix[0] {
        'u' => suffix[1..].len() <= 4 && suffix[1..].iter().all(|&c| is_hex(c)),
        'x' => suffix[1..].len() <= 2 && suffix[1..].iter().all(|&c| is_hex(c)),
        _ => false,
    }
}

/// What `node` looks like to its direct children for string-splitting purposes.
fn parent_info_for(_source: &[u8], node: &Node<'_>) -> ParentInfo {
    let forbids_string_split = node.as_assoc_node().is_some()
        || node.as_keyword_hash_node().is_some()
        || node.as_array_node().is_some()
        || node.as_optional_keyword_parameter_node().is_some();
    let dstr_open = node
        .as_interpolated_string_node()
        .and_then(|s| s.opening_loc())
        .map(|o| (o.start_offset(), o.end_offset()));
    ParentInfo {
        forbids_string_split,
        dstr_open,
        start_offset: node.location().start_offset(),
    }
}

/// `breakable_collection?`: a real bracketed collection (or non-hash) with at
/// least two elements.
fn breakable_collection(node: &Node<'_>, elements: &[Node<'_>]) -> bool {
    let starts_with_bracket = match node.as_hash_node() {
        Some(_) => true, // a `HashNode` always has `{}`
        None => true,
    };
    starts_with_bracket && elements.len() >= 2
}

/// One-based first/last line of a node's location.
fn node_lines(li: &LineIndex, node: &Node<'_>) -> (usize, usize) {
    let loc = node.location();
    (li.line_of(loc.start_offset()), li.line_of(loc.end_offset()))
}

/// `node.multiline?`
fn multiline(li: &LineIndex, node: &Node<'_>) -> bool {
    let (f, l) = node_lines(li, node);
    f != l
}

/// `heredoc?` for a string/xstring node: opening starts with `<<`.
fn is_heredoc(source: &[u8], node: &Node<'_>) -> bool {
    let open = if let Some(s) = node.as_string_node() {
        s.opening_loc()
    } else if let Some(s) = node.as_interpolated_string_node() {
        s.opening_loc()
    } else if let Some(s) = node.as_x_string_node() {
        Some(s.opening_loc())
    } else if let Some(s) = node.as_interpolated_x_string_node() {
        Some(s.opening_loc())
    } else {
        return false;
    };
    open.is_some_and(|o| source[o.start_offset()..o.end_offset()].starts_with(b"<<"))
}

/// Whether `node`'s first argument is a heredoc (call only).
fn first_argument_is_heredoc(source: &[u8], call: &ruby_prism::CallNode<'_>) -> bool {
    call.arguments()
        .and_then(|a| a.arguments().iter().next())
        .is_some_and(|first| is_heredoc(source, &first))
}

impl<'pr> Visit<'pr> for BreakableVisitor<'_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        self.on_enter(&node);
        let (first_line, _) = node_lines(self.line_index, &node);
        let kind = self.frame_kind_for(&node);
        let parent_info = parent_info_for(self.source, &node);
        self.stack.push(Frame {
            kind,
            first_line,
            parent_info,
        });
    }

    fn visit_branch_node_leave(&mut self) {
        self.stack.pop();
    }

    // String nodes are leaves and the generic `visit_branch_node_enter` hook is
    // not reliably called for them, so the string-split checks live in the typed
    // visitors. These do not push an ancestor frame (strings are leaves), so the
    // stack still reflects the string's true ancestors here.
    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        if self.split_strings {
            let parent_is_dstr = self
                .stack
                .last()
                .is_some_and(|f| f.parent_info.dstr_open.is_some());
            if !parent_is_dstr {
                self.check_for_breakable_str(&node.as_node(), node, None);
            }
        }
    }

    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        if self.split_strings {
            self.check_for_breakable_dstr(&node.as_node(), node);
        }
        // Recurse into the interpolation contents so nested breakable nodes are
        // still detected (the static string parts are handled above).
        for part in node.parts().iter() {
            if part.as_string_node().is_none() {
                self.visit(&part);
            }
        }
    }
}

impl<'pr> BreakableVisitor<'_> {
    /// Build the ancestor frame this node contributes (for the
    /// "contained by breakable collection" predicates and string-parent check).
    fn frame_kind_for(&self, node: &Node<'pr>) -> FrameKind {
        if let Some((elements, _is_call, _is_def)) = collection_elements(self.source, node) {
            // Only hash/array/call frames matter for the containment predicates.
            if node.as_def_node().is_some() {
                return FrameKind::Other;
            }
            let breakable = breakable_collection(node, &elements);
            let elems = elements
                .iter()
                .map(|e| {
                    let (f, l) = node_lines(self.line_index, e);
                    ElemLoc {
                        first_line: f,
                        last_line: l,
                    }
                })
                .collect();
            return FrameKind::Collection {
                elements: elems,
                breakable,
            };
        }
        FrameKind::Other
    }

    fn on_enter(&mut self, node: &Node<'pr>) {
        // Blocks: in Prism a `BlockNode` is the call's child and the enclosing
        // call carries the receiver. Handle the pair at the call (or lambda).
        if let Some(call) = node.as_call_node()
            && let Some(block) = call.block().and_then(|b| b.as_block_node())
        {
            self.check_for_breakable_block(
                node.location().start_offset(),
                call.receiver().as_ref(),
                &block,
                false,
            );
        }
        if let Some(lambda) = node.as_lambda_node() {
            // `->(x) { }` — represented as a LambdaNode, not a call+block.
            self.check_for_breakable_lambda(node.location().start_offset(), &lambda);
        }
        if collection_elements(self.source, node).is_some() {
            self.check_for_breakable_node(node);
        }
    }

    // --- breakable block -------------------------------------------------

    fn check_for_breakable_block(
        &mut self,
        block_expr_start: usize,
        receiver: Option<&Node<'pr>>,
        block: &ruby_prism::BlockNode<'pr>,
        is_lambda: bool,
    ) {
        // `block_node.single_line?`
        let (bf, bl) = node_lines(self.line_index, &block.as_node());
        // single_line? is over the whole block expression, whose first line is
        // the receiver/call line.
        let expr_first_line = self.line_index.line_of(block_expr_start);
        if expr_first_line != bl || bf != expr_first_line {
            return;
        }
        let line_index = expr_first_line - 1;
        if !self.is_candidate(line_index) {
            return;
        }
        if let Some(r) = receiver
            && receiver_contains_heredoc(self.source, r)
        {
            return;
        }

        let begin_pos = self.breakable_block_range_begin(block, is_lambda);
        self.claim(line_index, begin_pos + 1);
    }

    fn check_for_breakable_lambda(
        &mut self,
        block_expr_start: usize,
        lambda: &ruby_prism::LambdaNode<'pr>,
    ) {
        let (lf, ll) = node_lines(self.line_index, &lambda.as_node());
        let expr_first_line = self.line_index.line_of(block_expr_start);
        if lf != ll || lf != expr_first_line {
            return;
        }
        if !self.is_candidate(expr_first_line - 1) {
            return;
        }
        // Lambdas always use `{ }` / `do end`; `breakable_block_range` for a
        // lambda always takes the `loc.begin` branch (the `!lambda?` guard fails).
        let open = lambda.opening_loc();
        let open_src = &self.source[open.start_offset()..open.end_offset()];
        let begin_pos = if open_src == b"do" {
            open.start_offset() + 1
        } else {
            open.start_offset()
        };
        self.claim(expr_first_line - 1, begin_pos + 1);
    }

    /// `breakable_block_range(block_node).begin_pos`.
    fn breakable_block_range_begin(
        &self,
        block: &ruby_prism::BlockNode<'pr>,
        is_lambda: bool,
    ) -> usize {
        let has_pipe_args = block
            .parameters()
            .as_ref()
            .and_then(|p| p.as_block_parameters_node())
            .is_some();

        if has_pipe_args && !is_lambda {
            // `block_node.arguments.loc.end` — the closing `|`.
            let bp = block
                .parameters()
                .unwrap()
                .as_block_parameters_node()
                .unwrap();
            if let Some(close) = bp.closing_loc() {
                return close.start_offset();
            }
        }
        let open = block.opening_loc();
        let open_src = &self.source[open.start_offset()..open.end_offset()];
        if open_src == b"{" {
            open.start_offset()
        } else {
            // `do` — `loc.begin.adjust(begin_pos: 1)`.
            open.start_offset() + 1
        }
    }

    // --- breakable string / dstr ----------------------------------------

    /// `check_for_breakable_str`. `parent_quote` is `Some` when `node` is a part
    /// of a dstr (then the delimiter comes from the dstr's opening quote).
    fn check_for_breakable_str(
        &mut self,
        node: &Node<'pr>,
        s: &ruby_prism::StringNode<'pr>,
        parent_quote: Option<u8>,
    ) {
        let line_index = self.line_index.line_of(node.location().start_offset()) - 1;
        if self.ranges.contains_key(&line_index) {
            return;
        }
        if !self.is_candidate(line_index) {
            return;
        }
        if !self.breakable_string(node, s) {
            return;
        }
        let Some(delimiter) = self.string_delimiter(node, s, parent_quote) else {
            return;
        };
        let Some(pos) = self.breakable_string_position(node, parent_quote) else {
            return;
        };
        self.claim_string(line_index, pos, (delimiter as char).to_string());
    }

    /// `breakable_string?` — minus `allow_string_split?` (the caller already
    /// gates on `self.split_strings`).
    fn breakable_string(&self, node: &Node<'pr>, _s: &ruby_prism::StringNode<'pr>) -> bool {
        // single_line?
        if multiline(self.line_index, node) {
            return false;
        }
        // !heredoc?
        if is_heredoc(self.source, node) {
            return false;
        }
        // !node.parent&.type?(:pair, :kwoptarg, :array)
        if self
            .stack
            .last()
            .is_some_and(|f| f.parent_info.forbids_string_split)
        {
            return false;
        }
        true
    }

    /// `string_delimiter` — the opening quote (`'`/`"`), or `None`.
    fn string_delimiter(
        &self,
        node: &Node<'pr>,
        s: &ruby_prism::StringNode<'pr>,
        parent_quote: Option<u8>,
    ) -> Option<u8> {
        let quote = if let Some(open) = s.opening_loc() {
            self.source.get(open.start_offset()).copied()
        } else {
            parent_quote
        };
        match quote {
            Some(q @ (b'\'' | b'"')) => Some(q),
            _ => {
                let _ = node;
                None
            }
        }
    }

    /// `breakable_string_position` — returns the byte offset before which the
    /// break is inserted, or `None`.
    fn breakable_string_position(
        &self,
        node: &Node<'pr>,
        parent_quote: Option<u8>,
    ) -> Option<usize> {
        let (range_begin, range_end) = self.string_source_range(node, parent_quote);
        // return if source_range.last_column < max
        if last_column(self.line_index, self.source, range_end) < self.max {
            return None;
        }
        let break_end = self.breakable_string_range(node, parent_quote, range_begin, range_end)?;
        if break_end == range_begin {
            None
        } else {
            Some(break_end)
        }
    }

    /// `source_range` of the string node (without quotes for a dstr part).
    fn string_source_range(&self, node: &Node<'pr>, parent_quote: Option<u8>) -> (usize, usize) {
        let loc = node.location();
        if parent_quote.is_some() {
            // A dstr part: its source range is the part content (no quotes).
            (loc.start_offset(), loc.end_offset())
        } else {
            (loc.start_offset(), loc.end_offset())
        }
    }

    /// `breakable_string_range` — returns the *end offset* of the kept portion.
    fn breakable_string_range(
        &self,
        node: &Node<'pr>,
        parent_quote: Option<u8>,
        range_begin: usize,
        range_end: usize,
    ) -> Option<usize> {
        let substr = self.largest_possible_string(node, parent_quote, range_begin, range_end);
        // rindex(/\s/)
        if let Some(space_char_idx) = substr.char_indices().rev().find_map(|(i, c)| {
            if c.is_whitespace() {
                Some(substr[..i].chars().count())
            } else {
                None
            }
        }) {
            // resize(space_pos + 1)
            return Some(offset_after_chars(
                self.source,
                range_begin,
                space_char_idx + 1,
            ));
        }
        // rindex(/\\(u[\da-f]{0,4}|x[\da-f]{0,2})?\z/)
        if let Some(escape_char_idx) = trailing_escape_char_index(&substr) {
            // resize(escape_pos)
            return Some(offset_after_chars(
                self.source,
                range_begin,
                escape_char_idx,
            ));
        }
        // adjustment = max - source_range.last_column - 3
        let last_col = last_column(self.line_index, self.source, range_end) as isize;
        let adjustment = self.max as isize - last_col - 3;
        let size = char_count(&self.source[range_begin..range_end]) as isize;
        if adjustment.abs() > size {
            return None;
        }
        // adjust(end_pos: adjustment) — end_pos += adjustment (in characters).
        let end_chars = char_count(&self.source[range_begin..range_end]);
        let new_chars = (end_chars as isize + adjustment) as usize;
        Some(offset_after_chars(self.source, range_begin, new_chars))
    }

    /// `largest_possible_string` — the leading substring (as a `&str`) considered
    /// when choosing a break point.
    fn largest_possible_string(
        &self,
        node: &Node<'pr>,
        parent_quote: Option<u8>,
        range_begin: usize,
        range_end: usize,
    ) -> String {
        let mut max_length = self.max as isize - 3;
        // Offset by the string's starting column so the broken line actually
        // fits within `Max` (rubocop#15402). On the same line as its parent use
        // the column difference; otherwise the string is indented on its own
        // line, so subtract that indentation — without this an indented string
        // under a multi-line parent never shortens below `Max` and the
        // autocorrect loops, inserting empty `"" \` fragments.
        let parent_start = self.parent_start_offset();
        let node_col = self
            .line_index
            .column(self.source, node.location().start_offset()) as isize;
        max_length -= match parent_start {
            Some(p) if self.line_index.line_of(range_begin) == self.line_index.line_of(p) => {
                let parent_col = self.line_index.column(self.source, p) as isize;
                node_col - parent_col
            }
            _ => node_col,
        };
        let _ = parent_quote;
        let full = String::from_utf8_lossy(&self.source[range_begin..range_end]);
        if max_length <= 0 {
            return String::new();
        }
        full.chars().take(max_length as usize).collect()
    }

    fn parent_start_offset(&self) -> Option<usize> {
        // For a dstr part the parent (the dstr) is not yet on the stack, so the
        // explicit override is used; otherwise the immediate ancestor frame.
        self.string_parent_start
            .or_else(|| self.stack.last().map(|f| f.parent_info.start_offset))
    }

    fn check_for_breakable_dstr(
        &mut self,
        node: &Node<'pr>,
        istr: &ruby_prism::InterpolatedStringNode<'pr>,
    ) {
        // breakable_dstr? = breakable_string? && !child_nodes.one?
        let parts: Vec<Node<'pr>> = istr.parts().iter().collect();

        // First, run the str check on every string part (parser fires `on_str`
        // for them). The parent quote is the dstr's opening quote.
        let parent_quote = istr
            .opening_loc()
            .and_then(|o| self.source.get(o.start_offset()).copied());
        if self.split_strings {
            let saved = self.string_parent_start;
            self.string_parent_start = Some(node.location().start_offset());
            for part in &parts {
                if let Some(s) = part.as_string_node() {
                    self.check_for_breakable_str(part, &s, parent_quote);
                }
            }
            self.string_parent_start = saved;
        }

        // Then the dstr-level break (before an interpolation crossing the max).
        if !self.dstr_breakable(node, &parts) {
            return;
        }
        let Some(delimiter) = parent_quote.filter(|q| *q == b'\'' || *q == b'"') else {
            return;
        };
        let line_index = self.line_index.line_of(node.location().start_offset()) - 1;
        if self.ranges.contains_key(&line_index) {
            return;
        }
        if !self.is_candidate(line_index) {
            return;
        }
        for part in &parts {
            if let Some(embed) = part.as_embedded_statements_node() {
                let loc = embed.as_node().location();
                let begin = loc.start_offset();
                let col = self.line_index.column(self.source, begin);
                let last_col = last_column(self.line_index, self.source, loc.end_offset());
                if col < self.max && last_col >= self.max {
                    self.claim_string(line_index, begin, (delimiter as char).to_string());
                    break;
                }
            }
        }
    }

    fn dstr_breakable(&self, node: &Node<'pr>, parts: &[Node<'pr>]) -> bool {
        if multiline(self.line_index, node) {
            return false;
        }
        if is_heredoc(self.source, node) {
            return false;
        }
        if self
            .stack
            .last()
            .is_some_and(|f| f.parent_info.forbids_string_split)
        {
            return false;
        }
        // !child_nodes.one?
        parts.len() != 1
    }

    // --- breakable node -------------------------------------------------

    fn check_for_breakable_node(&mut self, node: &Node<'pr>) {
        // Cheap pre-filter: a breakable node is only extracted when the
        // container's first line exceeds `max` (see
        // `extract_breakable_node_from_elements`), which means that line is a
        // `LineLength` candidate. If it is not, no claim can happen, so skip the
        // expensive extraction entirely.
        if self.candidate_lines.is_some() {
            let (first_line, _) = node_lines(self.line_index, node);
            if !self.is_candidate(first_line - 1) {
                return;
            }
        }
        if let Some(bn) = self.extract_breakable_node(node) {
            let start = bn.location().start_offset();
            let line_index = self.line_index.line_of(start) - 1;
            if !self.is_candidate(line_index) {
                return;
            }
            self.claim(line_index, start);
        }
    }

    fn extract_breakable_node(&self, node: &Node<'pr>) -> Option<Node<'pr>> {
        if let Some(call) = node.as_call_node() {
            if chained_to_heredoc(self.source, &call) {
                return None;
            }
            let args = call
                .arguments()
                .map(|a| process_args(a.arguments().iter().collect()))
                .unwrap_or_default();
            return self.extract_breakable_node_from_elements(node, args, true, false);
        }
        if let Some(def) = node.as_def_node() {
            let params = def.parameters().map(|p| def_params(&p)).unwrap_or_default();
            return self.extract_breakable_node_from_elements(node, params, false, true);
        }
        let (elements, _, _) = collection_elements(self.source, node)?;
        if node.as_array_node().is_some() || node.as_hash_node().is_some() {
            return self.extract_breakable_node_from_elements(node, elements, false, false);
        }
        None
    }

    fn extract_breakable_node_from_elements(
        &self,
        node: &Node<'pr>,
        elements: Vec<Node<'pr>>,
        is_call: bool,
        is_def: bool,
    ) -> Option<Node<'pr>> {
        if !breakable_collection(node, &elements) {
            return None;
        }
        if self.safe_to_ignore(node, is_def) {
            return None;
        }
        let (first_line, _) = node_lines(self.line_index, node);
        if self.line_with_comment(first_line) {
            return None;
        }
        if self.line_length_chars(first_line - 1) <= self.max {
            return None;
        }
        self.extract_first_element_over_column_limit(node, elements, is_call)
    }

    fn extract_first_element_over_column_limit(
        &self,
        node: &Node<'pr>,
        mut elements: Vec<Node<'pr>>,
        is_call: bool,
    ) -> Option<Node<'pr>> {
        let (line, _) = node_lines(self.line_index, node);

        if is_call {
            let call = node.as_call_node().unwrap();
            let parenthesized = call.opening_loc().is_some();
            if !parenthesized && !first_argument_is_heredoc(self.source, &call) {
                if elements.is_empty() {
                    return None;
                }
                elements.remove(0);
            }
        }

        let mut i = 0usize;
        while self.within_column_limit(elements.get(i), line) {
            i += 1;
        }

        let i = self.shift_elements_for_heredoc_arg(node, &elements, i)?;
        if i == 0 {
            return elements.into_iter().next();
        }
        elements.into_iter().nth(i - 1)
    }

    fn within_column_limit(&self, element: Option<&Node<'pr>>, line: usize) -> bool {
        match element {
            Some(e) => {
                let col = self
                    .line_index
                    .column(self.source, e.location().start_offset());
                let (f, _) = node_lines(self.line_index, e);
                col <= self.max && f == line
            }
            None => false,
        }
    }

    /// `shift_elements_for_heredoc_arg` — `None` mirrors the Ruby `nil` return
    /// (no breakable node). The `Some(usize)` is the (possibly shifted) index.
    fn shift_elements_for_heredoc_arg(
        &self,
        node: &Node<'pr>,
        elements: &[Node<'pr>],
        index: usize,
    ) -> Option<usize> {
        let applies = node.as_call_node().is_some() || node.as_array_node().is_some();
        if !applies {
            return Some(index);
        }
        let heredoc_index = elements.iter().position(|e| is_heredoc(self.source, e));
        let Some(hi) = heredoc_index else {
            return Some(index);
        };
        if hi == 0 {
            return None;
        }
        if hi >= index {
            Some(index)
        } else {
            Some(hi + 1)
        }
    }

    fn safe_to_ignore(&self, node: &Node<'pr>, is_def: bool) -> bool {
        if self.already_on_multiple_lines(node, is_def) {
            return true;
        }
        if self.contained_by_breakable_collection_on_same_line(node) {
            return true;
        }
        if self.contained_by_multiline_collection_that_could_be_broken_up() {
            return true;
        }
        false
    }

    fn already_on_multiple_lines(&self, node: &Node<'pr>, is_def: bool) -> bool {
        if is_def {
            let def = node.as_def_node().unwrap();
            let (first_line, _) = node_lines(self.line_index, node);
            let last_param_last_line = def
                .parameters()
                .map(|p| def_params(&p))
                .and_then(|ps| ps.last().map(|p| node_lines(self.line_index, p).1));
            return match last_param_last_line {
                Some(ll) => first_line != ll,
                // No parameters: parser raises; treat as not multiline.
                None => false,
            };
        }
        multiline(self.line_index, node)
    }

    fn contained_by_breakable_collection_on_same_line(&self, node: &Node<'pr>) -> bool {
        let (node_first_line, _) = node_lines(self.line_index, node);
        for frame in self.stack.iter().rev() {
            if frame.first_line != node_first_line {
                break;
            }
            if let FrameKind::Collection { breakable, .. } = &frame.kind
                && *breakable
            {
                return true;
            }
        }
        false
    }

    fn contained_by_multiline_collection_that_could_be_broken_up(&self) -> bool {
        for frame in self.stack.iter().rev() {
            if let FrameKind::Collection {
                elements,
                breakable,
            } = &frame.kind
                && *breakable
            {
                return children_could_be_broken_up(elements);
            }
        }
        false
    }

    /// `processed_source.line_with_comment?(line)` — does the given (one-based)
    /// line contain a comment token?
    fn line_with_comment(&self, line: usize) -> bool {
        self.comment_lines.contains(&line)
    }
}

/// `children_could_be_broken_up?`
fn children_could_be_broken_up(children: &[ElemLoc]) -> bool {
    if children.is_empty() {
        return false;
    }
    // all_on_same_line?
    if children.first().unwrap().first_line == children.last().unwrap().last_line {
        return false;
    }
    let mut last_seen_line: i64 = -1;
    for child in children {
        if last_seen_line >= child.first_line as i64 {
            return true;
        }
        last_seen_line = child.last_line as i64;
    }
    false
}

/// `chained_to_heredoc?`
fn chained_to_heredoc(source: &[u8], call: &ruby_prism::CallNode<'_>) -> bool {
    let mut recv = call.receiver();
    while let Some(node) = recv {
        if is_heredoc(source, &node) {
            return true;
        }
        recv = node.as_call_node().and_then(|c| c.receiver());
    }
    false
}

/// `receiver_contains_heredoc?`
fn receiver_contains_heredoc(source: &[u8], receiver: &Node<'_>) -> bool {
    if is_heredoc(source, receiver) {
        return true;
    }
    let mut found = false;
    let mut v = HeredocFinder {
        source,
        found: &mut found,
    };
    v.visit(receiver);
    found
}

struct HeredocFinder<'a, 'b> {
    source: &'a [u8],
    found: &'b mut bool,
}

impl<'pr> Visit<'pr> for HeredocFinder<'_, '_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        if is_heredoc(self.source, &node) {
            *self.found = true;
        }
    }

    // Heredoc string nodes are leaves and are not reliably surfaced through the
    // generic `visit_branch_node_enter` hook when reached via a container's
    // recursion, so check them explicitly too.
    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        if node
            .opening_loc()
            .is_some_and(|o| self.source[o.start_offset()..o.end_offset()].starts_with(b"<<"))
        {
            *self.found = true;
        }
    }

    fn visit_x_string_node(&mut self, node: &ruby_prism::XStringNode<'pr>) {
        let o = node.opening_loc();
        if self.source[o.start_offset()..o.end_offset()].starts_with(b"<<") {
            *self.found = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Comment / semicolon scanning
// ---------------------------------------------------------------------------

/// Byte ranges that should be treated as "string-ish" — a `#` or `;` inside one
/// of these is not a comment / semicolon token. Collected from the AST so that
/// only true literal contents (not interpolation) are protected.
fn collect_literal_ranges(node: &Node<'_>) -> Vec<(usize, usize)> {
    let mut v = LiteralRangeVisitor { ranges: Vec::new() };
    v.visit(node);
    v.ranges.sort_by_key(|r| r.0);
    v.ranges
}

struct LiteralRangeVisitor {
    ranges: Vec<(usize, usize)>,
}

impl<'pr> Visit<'pr> for LiteralRangeVisitor {
    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        let c = node.content_loc();
        self.ranges.push((c.start_offset(), c.end_offset()));
    }
    fn visit_x_string_node(&mut self, node: &ruby_prism::XStringNode<'pr>) {
        let c = node.content_loc();
        self.ranges.push((c.start_offset(), c.end_offset()));
    }
    fn visit_symbol_node(&mut self, node: &ruby_prism::SymbolNode<'pr>) {
        let c = node.value_loc();
        if let Some(c) = c {
            self.ranges.push((c.start_offset(), c.end_offset()));
        }
    }
    fn visit_regular_expression_node(&mut self, node: &ruby_prism::RegularExpressionNode<'pr>) {
        let c = node.content_loc();
        self.ranges.push((c.start_offset(), c.end_offset()));
    }
    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        // Protect the static string parts; recurse for interpolation.
        for part in node.parts().iter() {
            if let Some(s) = part.as_string_node() {
                let c = s.content_loc();
                self.ranges.push((c.start_offset(), c.end_offset()));
            } else {
                self.visit(&part);
            }
        }
    }
    fn visit_embedded_statements_node(&mut self, node: &ruby_prism::EmbeddedStatementsNode<'pr>) {
        // Protect the `#{` opener: its `#` is interpolation, not a comment.
        // Only the opener — the statements inside may span lines and contain
        // real comments, so they are recursed, not blanket-protected.
        let o = node.opening_loc();
        self.ranges.push((o.start_offset(), o.end_offset()));
        if let Some(stmts) = node.statements() {
            for s in stmts.body().iter() {
                self.visit(&s);
            }
        }
    }
    fn visit_embedded_variable_node(&mut self, node: &ruby_prism::EmbeddedVariableNode<'pr>) {
        // Protect the `#` of `#@ivar` / `#$gvar` interpolation.
        let o = node.operator_loc();
        self.ranges.push((o.start_offset(), o.end_offset()));
    }
}

fn in_ranges(ranges: &[(usize, usize)], pos: usize) -> bool {
    ranges.iter().any(|&(s, e)| pos >= s && pos < e)
}

/// Comment byte ranges `(start, end)` — from each `#` that starts a comment to
/// the end of its line.
fn comment_byte_ranges_with(source: &[u8], literals: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let len = source.len();
    while i < len {
        let b = source[i];
        if b == b'#' && !in_ranges(literals, i) {
            let mut j = i;
            while j < len && source[j] != b'\n' {
                j += 1;
            }
            out.push((i, j));
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, max: usize, split: bool) -> Vec<(usize, usize, String)> {
        compute_breakables(src.as_bytes(), max, split)
            .into_iter()
            .map(|b| (b.line_index, b.insert_offset, b.delimiter))
            .collect()
    }

    fn run_filtered(
        src: &str,
        max: usize,
        split: bool,
        candidates: &HashSet<usize>,
    ) -> Vec<(usize, usize, String)> {
        compute_breakables_filtered(src.as_bytes(), max, split, Some(candidates))
            .into_iter()
            .map(|b| (b.line_index, b.insert_offset, b.delimiter))
            .collect()
    }

    /// The real candidate set: lines whose visible length exceeds `max`.
    fn candidate_lines(src: &str, max: usize) -> HashSet<usize> {
        super::super::line_length::check_line_length(src.as_bytes(), max, 0)
            .into_iter()
            .map(|c| c.line_index)
            .collect()
    }

    /// For an arbitrary source, the filtered output (using the real candidate
    /// set) must equal the unfiltered output restricted to candidate lines.
    fn assert_filter_parity(src: &str, max: usize, split: bool) {
        let candidates = candidate_lines(src, max);
        let unfiltered: Vec<_> = run(src, max, split)
            .into_iter()
            .filter(|(li, _, _)| candidates.contains(li))
            .collect();
        let filtered = run_filtered(src, max, split, &candidates);
        assert_eq!(filtered, unfiltered, "filter parity failed for:\n{src}");
    }

    // Filtering to the real candidate set is identical to the unfiltered output
    // restricted to candidate lines, across a variety of breakable shapes.
    #[test]
    fn filter_parity_mixed() {
        let long_hash = "{abc: \"100000\", def: \"100000\", ghi: \"100000\", jkl: \"100000\", mno: \"100000\"}\n";
        let short = "x = 1\n";
        let call = format!("method_call {}, abc\n", "x".repeat(28));
        let block = "foo.select { |bar| 4444000039123123129993912312312999199291203123123 }\n";
        let semi = "{foo: 1, bar: \"2\"}; a = 400000000000 + 500000000000000\n";
        let split_str = "'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbb'\n";

        // A file mixing candidate and non-candidate lines.
        let combined = format!("{short}{long_hash}{short}{call}{block}{semi}{short}");
        assert_filter_parity(&combined, 40, false);
        assert_filter_parity(&combined, 40, true);

        assert_filter_parity(long_hash, 40, false);
        assert_filter_parity(&call, 40, false);
        assert_filter_parity(block, 40, false);
        assert_filter_parity(semi, 40, false);
        assert_filter_parity(split_str, 40, true);
    }

    // A multi-line / heredoc breakable: the claim line must still match between
    // filtered and unfiltered.
    #[test]
    fn filter_parity_heredoc() {
        let src = "foo(<<~SQL, aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa)\n  SELECT 1\nSQL\n";
        assert_filter_parity(src, 40, false);

        let dstr = "x = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa#{bbbbbbbbbbbbbbbbbbbbbbbbbb}cccc\"\n";
        assert_filter_parity(dstr, 40, true);
    }

    // The `#` of a string interpolation (`#{`, `#@ivar`) is NOT a comment:
    // a long line whose argument is an interpolated string still yields its
    // breakable (upstream consults the real comment list, which has none
    // here). Regression: the comment scan used to treat `#{` as a comment
    // start, dropping every breakable on interpolated-string lines (seen on
    // stdlib fileutils.rb / `raise ArgumentError, "...#{...}"` lines).
    #[test]
    fn interpolation_hash_is_not_a_comment() {
        let pad = "a".repeat(30);
        // Two-argument unparenthesized call: the breakable is the dstr itself.
        let src = format!("foo bar, \"{pad}xx #{{baz}} yy\"\n");
        let breakables = run(&src, 40, false);
        assert_eq!(breakables.len(), 1);
        assert_eq!(breakables[0].0, 0);
        assert_eq!(breakables[0].1, src.find('"').unwrap());

        // `#@ivar` interpolation, same shape.
        let src = format!("foo bar, \"{pad}xx #@baz yy\"\n");
        assert_eq!(run(&src, 40, false).len(), 1);

        // A collection nested INSIDE an interpolation is still breakable
        // (fileutils.rb `fu_output_message "...#{[src,dest].flatten.join ' '}..."`).
        // `defg` starts past the limit, so the break lands before `abc`
        // (`elements[i - 1]` of `extract_first_element_over_column_limit`).
        let src = format!("foo \"{pad}#{{[abc, defg].flatten.join ' '}} tail tail\"\n");
        let breakables = run(&src, 40, false);
        assert_eq!(breakables.len(), 1);
        assert_eq!(breakables[0].1, src.find("abc").unwrap());

        // A REAL comment on the line still suppresses the breakable.
        let src = format!("foo bar, \"{pad}xx yy\" # note\n");
        assert!(run(&src, 40, false).is_empty());
    }

    // When the candidate set is empty, no breakables are produced even though
    // the unfiltered run would find some.
    #[test]
    fn empty_candidate_set_yields_nothing() {
        let src = "{abc: \"100000\", def: \"100000\", ghi: \"100000\", jkl: \"100000\", mno: \"100000\"}\n";
        let empty = HashSet::new();
        assert!(run_filtered(src, 40, false, &empty).is_empty());
        // Sanity: unfiltered does find one.
        assert_eq!(run(src, 40, false).len(), 1);
    }

    // Typical: a hash over the limit is broken before the element that crosses
    // the column limit.
    #[test]
    fn breakable_hash() {
        let src = "{abc: \"100000\", def: \"100000\", ghi: \"100000\", jkl: \"100000\", mno: \"100000\"}\n";
        let r = run(src, 40, false);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, 0);
        assert_eq!(r[0].2, "");
    }

    // A method call broken after the first argument when unparenthesized.
    #[test]
    fn unparenthesized_call_drops_first() {
        let args = "x".repeat(28);
        let src = format!("method_call {args}, abc\n");
        let r = run(&src, 40, false);
        assert_eq!(r.len(), 1);
        // Break is before `abc` (the second element).
        assert_eq!(&src.as_bytes()[r[0].1..r[0].1 + 3], b"abc");
    }

    // A single-line `{}` block is broken right after the `|args|`.
    #[test]
    fn breakable_block_braces() {
        let src = "foo.select { |bar| 4444000039123123129993912312312999199291203123123 }\n";
        let r = run(src, 40, false);
        assert_eq!(r.len(), 1);
        // The byte before the insertion point is `|` (closing pipe).
        assert_eq!(src.as_bytes()[r[0].1 - 1], b'|');
    }

    // A statement semicolon yields a break right after the `;`.
    #[test]
    fn breakable_semicolon() {
        let src = "{foo: 1, bar: \"2\"}; a = 400000000000 + 500000000000000\n";
        let r = run(src, 40, false);
        assert_eq!(r.len(), 1);
        assert_eq!(src.as_bytes()[r[0].1 - 1], b';');
    }

    // A semicolon inside a string literal is not a statement separator.
    #[test]
    fn semicolon_in_string_is_ignored() {
        let src = "x = 'a;b'\n";
        assert!(run(src, 40, false).is_empty());
    }

    // With SplitStrings, a long string with no spaces breaks at the limit and
    // carries its quote as the continuation delimiter.
    #[test]
    fn split_string_at_limit() {
        let src = "'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbb'\n";
        let r = run(src, 40, true);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].2, "'");
    }

    // SplitStrings off: no string break.
    #[test]
    fn no_split_when_disabled() {
        let src = "'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbb'\n";
        assert!(run(src, 40, false).is_empty());
    }

    // A string inside a hash value is not split; the hash is broken instead.
    #[test]
    fn string_in_hash_not_split() {
        let src = "{ x: 'aaaa', y: 'bbbbbbbbbbbbbbbbbbbbbbbbbbb', z: 'cccccccccccccccccccccccccccccccccccccccccc' }\n";
        let r = run(src, 40, true);
        assert_eq!(r.len(), 1);
        // Broken as a hash (no string delimiter).
        assert_eq!(r[0].2, "");
    }
}
