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
/// line. Node and string claims are write-once (the first wins, matching
/// upstream's `return if breakable_range_by_line_index[...]` guards); block
/// claims assign unconditionally and overwrite earlier claims, matching
/// upstream's `check_for_breakable_block`.
#[derive(Debug)]
pub struct Breakable {
    pub line_index: usize,
    pub insert_offset: usize,
    /// End byte offset of the stock breakable RANGE. Stock stores the chosen
    /// element node's full `source_range` for node claims (the corrector's
    /// `insert_before` then spans the whole element, which matters for
    /// `TreeRewriter` merge conflicts against other cops in the same
    /// iteration); block / string / semicolon claims store a 1-byte range.
    pub end_offset: usize,
    /// Empty = newline break; `'`/`"` = string-split continuation delimiter.
    pub delimiter: String,
    /// Further 1-byte break points on the same line (since 1.92 a string
    /// that needs several pieces is split in one pass).
    pub extra_breaks: Vec<usize>,
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
    compute_breakables_with(source, max, split_strings, false, candidate_lines)
}

/// Like [`compute_breakables_filtered`], with `compact_array_brackets` set
/// when `Layout/SpaceInsideArrayLiteralBrackets` is enabled with the
/// `compact` style (1.92's `compact_array_brackets_enforced?`).
pub fn compute_breakables_with(
    source: &[u8],
    max: usize,
    split_strings: bool,
    compact_array_brackets: bool,
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
            compact_array_brackets,
            candidate_lines,
            stack: Vec::new(),
            ranges: std::collections::BTreeMap::new(),
            delimiters: std::collections::BTreeMap::new(),
            literals,
            comments,
            comment_lines,
            extra_breaks: std::collections::BTreeMap::new(),
        };
        // Semicolons are claimed first (upstream does this in
        // `on_new_investigation`, before the node walk), so a NODE claim never
        // overrides a semicolon break on the same line. BLOCK claims do — they
        // assign unconditionally, exactly like upstream.
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
    compact_array_brackets: bool,
    /// When `Some`, only lines whose 0-based index is in the set may produce a
    /// breakable; other lines are skipped before the expensive extraction.
    candidate_lines: Option<&'a HashSet<usize>>,
    stack: Vec<Frame>,
    ranges: std::collections::BTreeMap<usize, (usize, usize)>,
    delimiters: std::collections::BTreeMap<usize, String>,
    literals: Vec<(usize, usize)>,
    comments: Vec<(usize, usize)>,
    comment_lines: std::collections::BTreeSet<usize>,
    /// Extra break offsets after the first one, for a string split into
    /// several pieces in one pass.
    extra_breaks: std::collections::BTreeMap<usize, Vec<usize>>,
}

impl BreakableVisitor<'_> {
    fn into_breakables(mut self) -> Vec<Breakable> {
        self.ranges
            .into_iter()
            .map(|(line_index, (insert_offset, end_offset))| Breakable {
                line_index,
                insert_offset,
                end_offset,
                delimiter: self
                    .delimiters
                    .get(&line_index)
                    .cloned()
                    .unwrap_or_default(),
                extra_breaks: self.extra_breaks.remove(&line_index).unwrap_or_default(),
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

    fn claim(&mut self, line_index: usize, insert_offset: usize, end_offset: usize) {
        self.ranges
            .entry(line_index)
            .or_insert((insert_offset, end_offset));
    }

    /// Block claims ASSIGN instead of or-inserting: stock's
    /// `check_for_breakable_block` writes `breakable_range_by_line_index[i]`
    /// unconditionally, so a block beats an earlier semicolon / node / string
    /// claim on the same line. A string's stored delimiter is NOT cleared —
    /// stock never removes entries from `breakable_string_delimiters` either,
    /// so a line whose string claim was overwritten by a block still inserts
    /// the string-continuation form at the block position.
    fn claim_block(&mut self, line_index: usize, insert_offset: usize) {
        // 1.92: `return if breakable_string_delimiters[line_index]`.
        if self.delimiters.contains_key(&line_index) {
            return;
        }
        self.ranges
            .insert(line_index, (insert_offset, insert_offset + 1));
    }

    /// A string claim: the first offset is the stored breakable range; the
    /// rest are the extra breaks stock's `string_break_ranges` adds.
    fn claim_string_multi(&mut self, line_index: usize, offsets: Vec<usize>, delimiter: String) {
        if self.ranges.contains_key(&line_index) {
            return;
        }
        let first = offsets[0];
        self.ranges.insert(line_index, (first, first + 1));
        self.delimiters.insert(line_index, delimiter);
        if offsets.len() > 1 {
            self.extra_breaks.insert(line_index, offsets[1..].to_vec());
        }
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
            self.ranges.insert(line_index, (end_pos, end_pos + 1));
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

/// The chars of `bytes` (bytes as chars on invalid UTF-8, like the other
/// char-count helpers).
fn chars_of(bytes: &[u8]) -> Vec<char> {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.chars().collect(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

/// Ruby `\s`.
fn is_ruby_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

/// `text.strip.empty?` (Ruby's `strip` also drops NUL).
fn is_blank(text: &[char]) -> bool {
    text.iter().all(|&c| is_ruby_space(c) || c == '\0')
}

/// `text.match(ESCAPE_SEQUENCE_TAIL)&.begin(1)` — the char index where a
/// trailing, possibly incomplete escape sequence starts (stock 1.92's
/// `/(?<!\\)(?:\\\\)*(\\(?:(?:[MC]-|c)\\)*(?:u\{[\da-f\x20]*|u[\da-f]{0,4}|
/// x[\da-f]{0,2}|[0-7]{0,3}|[MC]-?|c)?)\z/i`). The leftmost start wins, and
/// the greedy `(?:\\\\)*` takes as many backslash pairs as it can.
fn escape_sequence_start(text: &[char]) -> Option<usize> {
    for s in 0..text.len() {
        if text[s] != '\\' || (s > 0 && text[s - 1] == '\\') {
            continue;
        }
        let run = text[s..].iter().take_while(|&&c| c == '\\').count();
        for k in (0..=run / 2).rev() {
            let g = s + 2 * k;
            if g < text.len() && text[g] == '\\' && escape_chain_then_tail(&text[g + 1..]) {
                return Some(g);
            }
        }
    }
    None
}

/// `(?:(?:[MC]-|c)\\)*(?:tail)?\z` after the escape's backslash.
fn escape_chain_then_tail(u: &[char]) -> bool {
    if escape_tail(u) {
        return true;
    }
    let prefix = match u.first().map(|c| c.to_ascii_lowercase()) {
        Some('m' | 'c') if u.get(1) == Some(&'-') => 2,
        Some('c') => 1,
        _ => return false,
    };
    // `c-\` also parses as the `c` prefix only when the next char is `\`.
    let try_prefix = |n: usize| u.get(n) == Some(&'\\') && escape_chain_then_tail(&u[n + 1..]);
    try_prefix(prefix) || (prefix == 2 && u[0].eq_ignore_ascii_case(&'c') && try_prefix(1))
}

/// `(?:u\{[\da-f\x20]*|u[\da-f]{0,4}|x[\da-f]{0,2}|[0-7]{0,3}|[MC]-?|c)?\z`.
fn escape_tail(u: &[char]) -> bool {
    let hex = |c: &char| c.is_ascii_hexdigit();
    let Some(first) = u.first() else {
        return true;
    };
    match first.to_ascii_lowercase() {
        'u' => {
            (u.get(1) == Some(&'{') && u[2..].iter().all(|c| hex(c) || *c == ' '))
                || (u.len() <= 5 && u[1..].iter().all(hex))
        }
        'x' => u.len() <= 3 && u[1..].iter().all(hex),
        'm' | 'c' => u.len() == 1 || (u.len() == 2 && u[1] == '-'),
        _ => u.len() <= 3 && u.iter().all(|c| ('0'..='7').contains(c)),
    }
}

/// A single-line string node as stock's string-splitting helpers see it.
/// Every index is a char index into `chars` (the node's source).
struct StrView<'a> {
    chars: &'a [char],
    /// `node.loc.column`.
    column: usize,
    /// Chars before the content (the opening quote's length; 0 without one).
    content_begin: usize,
    /// `string_content_length(node)`.
    content_len: usize,
    /// `starts_after_quote?(node)`.
    starts_after_quote: bool,
}

impl StrView<'_> {
    fn last_column(&self) -> usize {
        self.column + self.chars.len()
    }

    /// `breakable_string_position(node)` — the char index before which the
    /// break is inserted.
    fn breakable_position(&self, max: usize) -> Option<usize> {
        if self.last_column() < max {
            return None;
        }
        let mut pos = self.breakable_range(max)?;
        if self.blank_left_part(pos) {
            pos = self.range_at_limit(max)?;
        }
        self.leaves_two_parts(pos).then_some(pos)
    }

    fn leaves_two_parts(&self, pos: usize) -> bool {
        // `!blank_rest?`: `node.source[offset...string_content_length]`.
        let rest_end = self.content_len.max(pos).min(self.chars.len());
        pos > self.content_begin && !is_blank(&self.chars[pos.min(rest_end)..rest_end])
    }

    fn blank_left_part(&self, pos: usize) -> bool {
        pos > self.content_begin
            && self.starts_after_quote
            && is_blank(&self.chars[self.content_begin..pos.min(self.chars.len())])
    }

    /// `breakable_string_range(node)` — the end of the kept part.
    fn breakable_range(&self, max: usize) -> Option<usize> {
        // `largest_possible_string`: `Max` - quote - ` \`, minus the column.
        let max_length = (max as isize - 3 - self.column as isize).max(0) as usize;
        let substr = &self.chars[..max_length.min(self.chars.len())];
        if let Some(space_pos) = self.breakable_space_position(substr.len()) {
            // A space can separate codepoints inside a braced unicode escape.
            let head = &substr[..(space_pos + 1).min(substr.len())];
            return Some(escape_sequence_start(head).unwrap_or(space_pos + 1));
        }
        if let Some(escape_pos) = escape_sequence_start(substr) {
            return Some(escape_pos);
        }
        self.range_at_limit(max)
    }

    fn breakable_space_position(&self, substr_len: usize) -> Option<usize> {
        let limit = self.content_len as isize - 1;
        let space_pos = self.space_before_word(substr_len as isize - 1)?;
        if space_pos as isize + 1 > limit {
            return self.space_before_word(limit - 1);
        }
        Some(space_pos)
    }

    /// `node.source.rindex(/\s(?=\S)/, max_pos)`.
    fn space_before_word(&self, max_pos: isize) -> Option<usize> {
        if max_pos < 0 {
            return None;
        }
        let chars = self.chars;
        let top = (max_pos as usize).min(chars.len().saturating_sub(1));
        (0..=top).rev().find(|&i| {
            i < chars.len()
                && is_ruby_space(chars[i])
                && chars.get(i + 1).is_some_and(|&c| !is_ruby_space(c))
        })
    }

    /// `breakable_string_range_at_limit(source_range)`.
    fn range_at_limit(&self, max: usize) -> Option<usize> {
        let size = self.chars.len() as isize;
        let adjustment = max as isize - self.last_column() as isize - 3;
        if adjustment.abs() > size {
            return None;
        }
        let mut cut = (size + adjustment) as usize;
        // `size_before_whitespace_run`
        if cut != 0
            && self.chars.get(cut).is_some_and(|&c| is_ruby_space(c))
            && let Some(i) = (0..cut).rev().find(|&i| !is_ruby_space(self.chars[i]))
        {
            cut = i;
        }
        let cut_text = &self.chars[..cut.min(self.chars.len())];
        Some(escape_sequence_start(cut_text).unwrap_or(cut))
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

    // parser wraps a call-with-block in a `(block (send) args body)` node:
    // from inside the block body, the SEND is NOT an ancestor — the wrapper
    // block node is (kind-wise a non-collection, first line = the whole
    // expression's). prism hangs the block off the CallNode, so the naive
    // stack would show the call's collection frame from inside the block and
    // `contained_by_multiline_collection_that_could_be_broken_up` would
    // wrongly suppress claims inside the block (mastodon accounts_index).
    // Swap the call frame for the parser block wrapper while descending into
    // a real block child (a `BlockArgumentNode` stays an argument: parser
    // keeps `&blk` under the send).
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if let Some(r) = node.receiver() {
            self.visit(&r);
        }
        if let Some(a) = node.arguments() {
            self.visit_arguments_node(&a);
        }
        let Some(block) = node.block() else { return };
        if block.as_block_node().is_none() {
            self.visit(&block);
            return;
        }
        let call_frame = self.stack.pop();
        let first_line = call_frame.as_ref().map_or_else(
            || self.line_index.line_of(node.location().start_offset()),
            |f| f.first_line,
        );
        self.stack.push(Frame {
            kind: FrameKind::Other,
            first_line,
            parent_info: ParentInfo {
                forbids_string_split: false,
                dstr_open: None,
            },
        });
        self.visit(&block);
        self.stack.pop();
        if let Some(f) = call_frame {
            self.stack.push(f);
        }
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
        // Then the children in source order, as parser's pre-order walk
        // does: a string part's claim and a claim from inside an
        // interpolation (e.g. a block) race for the same line.
        let parent_open = node
            .opening_loc()
            .map(|o| (o.start_offset(), o.end_offset()));
        for part in node.parts().iter() {
            match part.as_string_node() {
                Some(s) if self.split_strings => self.check_for_breakable_str(&part, &s, parent_open),
                Some(_) => {}
                None => self.visit(&part),
            }
        }
    }

    // `BeginNode.rescue_clause` is a concretely-typed field: the generated
    // walker calls `visit_rescue_node` directly, so a `RescueNode` never
    // reaches the generic branch hooks (see the trap table). parser wraps the
    // exception list of `rescue A, B => e` in an implicit ARRAY node that
    // `on_array` treats as a breakable collection; synthesize it here. The
    // collection frame is pushed over the exception subtrees only — parser's
    // implicit array is the parent of the exceptions, not of the body.
    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        self.check_for_breakable_rescue(node);
        let exceptions: Vec<Node<'pr>> = node.exceptions().iter().collect();
        if !exceptions.is_empty() {
            let elems: Vec<ElemLoc> = exceptions
                .iter()
                .map(|e| {
                    let (f, l) = node_lines(self.line_index, e);
                    ElemLoc {
                        first_line: f,
                        last_line: l,
                    }
                })
                .collect();
            let first_line = elems[0].first_line;
            self.stack.push(Frame {
                kind: FrameKind::Collection {
                    elements: elems,
                    breakable: exceptions.len() >= 2,
                },
                first_line,
                // parser: a string exception's parent is the implicit array,
                // which forbids string splitting.
                parent_info: ParentInfo {
                    forbids_string_split: true,
                    dstr_open: None,
                },
            });
            for e in &exceptions {
                self.visit(e);
            }
            self.stack.pop();
        }
        if let Some(r) = node.reference() {
            self.visit(&r);
        }
        if let Some(s) = node.statements() {
            self.visit_statements_node(&s);
        }
        if let Some(sub) = node.subsequent() {
            self.visit_rescue_node(&sub);
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
        // `super(...) { }` / bare `super { }` — parser wraps them in a block
        // node too, so stock's `on_block` fires for them (no receiver).
        if let Some(sup) = node.as_super_node()
            && let Some(block) = sup.block().and_then(|b| b.as_block_node())
        {
            self.check_for_breakable_block(node.location().start_offset(), None, &block, false);
        }
        if let Some(sup) = node.as_forwarding_super_node()
            && let Some(block) = sup.block()
        {
            self.check_for_breakable_block(node.location().start_offset(), None, &block, false);
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
        // `block_node.single_line?` — BlockNode OVERRIDES the generic
        // predicate: it compares only the `{`/`do` line with the `}`/`end`
        // line. The receiver part may span any number of lines.
        let open_line = self
            .line_index
            .line_of(block.opening_loc().start_offset());
        let close_line = self
            .line_index
            .line_of(block.closing_loc().start_offset());
        if open_line != close_line {
            return;
        }
        // The claim goes to the block EXPRESSION's first line (the receiver
        // line) even when the insertion point sits on a later line.
        let line_index = self.line_index.line_of(block_expr_start) - 1;
        if !self.is_candidate(line_index) {
            return;
        }
        if let Some(r) = receiver
            && receiver_contains_heredoc(self.source, r)
        {
            return;
        }

        let begin_pos = self.breakable_block_range_begin(block, is_lambda);
        self.claim_block(line_index, begin_pos + 1);
    }

    fn check_for_breakable_lambda(
        &mut self,
        block_expr_start: usize,
        lambda: &ruby_prism::LambdaNode<'pr>,
    ) {
        // Same `single_line?` as blocks: only the `{`/`do` line vs the
        // `}`/`end` line.
        let open = lambda.opening_loc();
        let open_line = self.line_index.line_of(open.start_offset());
        let close_line = self
            .line_index
            .line_of(lambda.closing_loc().start_offset());
        if open_line != close_line {
            return;
        }
        let line_index = self.line_index.line_of(block_expr_start) - 1;
        if !self.is_candidate(line_index) {
            return;
        }
        // Lambdas always use `{ }` / `do end`; `breakable_block_range` for a
        // lambda always takes the `loc.begin` branch (the `!lambda?` guard fails).
        let open_src = &self.source[open.start_offset()..open.end_offset()];
        let begin_pos = if open_src == b"do" {
            open.start_offset() + 1
        } else {
            open.start_offset()
        };
        self.claim_block(line_index, begin_pos + 1);
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

    /// `check_for_breakable_str`. `parent_open` is the opening-quote range of
    /// the enclosing dstr when `node` is one of its parts (then the delimiter
    /// may come from the dstr's opening quote).
    fn check_for_breakable_str(
        &mut self,
        node: &Node<'pr>,
        s: &ruby_prism::StringNode<'pr>,
        parent_open: Option<(usize, usize)>,
    ) {
        let line_index = self.line_index.line_of(node.location().start_offset()) - 1;
        if self.ranges.contains_key(&line_index) {
            return;
        }
        if !self.is_candidate(line_index) {
            return;
        }
        if !self.breakable_string(node, parent_open.is_some()) {
            return;
        }
        let parent_quote = parent_open.and_then(|(b, _)| self.source.get(b).copied());
        let Some(delimiter) = self.string_delimiter(node, s, parent_quote) else {
            return;
        };
        let loc = node.location();
        let (begin, end) = (loc.start_offset(), loc.end_offset());
        let chars = chars_of(&self.source[begin..end]);
        let content_begin = s
            .opening_loc()
            .map(|o| char_count(&self.source[begin..o.end_offset()]))
            .unwrap_or(0);
        let content_len = s
            .closing_loc()
            .map(|c| char_count(&self.source[begin..c.start_offset()]))
            .unwrap_or(chars.len());
        // `starts_after_quote?`
        let starts_after_quote = s.opening_loc().is_some()
            || parent_open.is_some_and(|(_, quote_end)| quote_end == begin);
        let view = StrView {
            chars: &chars,
            column: self.line_index.column(self.source, begin),
            content_begin,
            content_len,
            starts_after_quote,
        };
        let Some(pos) = view.breakable_position(self.max) else {
            return;
        };
        let mut offsets = vec![offset_after_chars(self.source, begin, pos)];
        // `break_ranges` / `string_break_ranges` (1.92): a string with a
        // closing delimiter is broken into as many pieces as needed in one
        // pass. Each further break point comes from re-running the position
        // logic on `<delimiter><rest of the string>` as a fresh string at
        // column 0 (stock re-parses that text).
        if s.closing_loc().is_some() {
            let mut cur = pos;
            loop {
                let mut rest: Vec<char> = Vec::with_capacity(chars.len() - cur + 1);
                rest.push(delimiter as char);
                rest.extend_from_slice(&chars[cur..]);
                let rest_view = StrView {
                    chars: &rest,
                    column: 0,
                    content_begin: 1,
                    content_len: rest.len() - 1,
                    starts_after_quote: true,
                };
                let Some(rest_pos) = rest_view.breakable_position(self.max) else {
                    break;
                };
                cur = cur + rest_pos - 1;
                offsets.push(offset_after_chars(self.source, begin, cur));
            }
        }
        self.claim_string_multi(line_index, offsets, (delimiter as char).to_string());
    }

    /// `breakable_string?` — minus `allow_string_split?` (the caller already
    /// gates on `self.split_strings`).
    fn breakable_string(&self, node: &Node<'pr>, is_dstr_part: bool) -> bool {
        // single_line?
        if multiline(self.line_index, node) {
            return false;
        }
        // !heredoc?
        if is_heredoc(self.source, node) {
            return false;
        }
        // !node.parent&.type?(:pair, :kwoptarg, :array) — a dstr part's
        // parent is the dstr itself.
        if !is_dstr_part
            && self
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
        _node: &Node<'pr>,
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
            _ => None,
        }
    }

    fn check_for_breakable_dstr(
        &mut self,
        node: &Node<'pr>,
        istr: &ruby_prism::InterpolatedStringNode<'pr>,
    ) {
        // breakable_dstr? = breakable_string? && !child_nodes.one?
        let parts: Vec<Node<'pr>> = istr.parts().iter().collect();

        // parser fires `on_dstr` before the parts' `on_str`; the caller visits
        // the parts afterwards.
        let parent_open = istr
            .opening_loc()
            .map(|o| (o.start_offset(), o.end_offset()));
        let parent_quote = parent_open.and_then(|(b, _)| self.source.get(b).copied());

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
                if !(col < self.max && last_col >= self.max) {
                    continue;
                }
                // `opens_string?` (1.92): an interpolation preceded only by
                // blanks after the opening quote is no break point (the split
                // would leave a blank string).
                if let Some((_, quote_end)) = parent_open
                    && is_blank(&chars_of(&self.source[quote_end..begin]))
                {
                    continue;
                }
                self.claim_string_multi(line_index, vec![begin], (delimiter as char).to_string());
                break;
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

    // --- breakable rescue (implicit exception array) ----------------------

    /// parser wraps the exception classes of `rescue A, B, C => e` in an
    /// implicit `array` node and `extract_breakable_node` treats it like any
    /// other collection. Prism keeps the exceptions as a bare NodeList on
    /// `RescueNode`, so the array is synthesized: its range runs from the
    /// first exception's start to the last exception's end.
    fn check_for_breakable_rescue(&mut self, rescue: &ruby_prism::RescueNode<'pr>) {
        let exceptions: Vec<Node<'pr>> = rescue.exceptions().iter().collect();
        // `breakable_collection?`: an implicit array is never a hash, so only
        // the two-element rule applies.
        if exceptions.len() < 2 {
            return;
        }
        let first_line = self
            .line_index
            .line_of(exceptions.first().unwrap().location().start_offset());
        // Cheap pre-filter, as in `check_for_breakable_node`.
        if !self.is_candidate(first_line - 1) {
            return;
        }
        // `safe_to_ignore?`: `already_on_multiple_lines?` measures the
        // implicit array's own span, then the two containment predicates.
        let last_line = self
            .line_index
            .line_of(exceptions.last().unwrap().location().end_offset());
        if first_line != last_line {
            return;
        }
        if self.contained_by_breakable_collection_on_same_line_at(first_line) {
            return;
        }
        if self.contained_by_multiline_collection_that_could_be_broken_up() {
            return;
        }
        if self.line_with_comment(first_line) {
            return;
        }
        if self.line_length_chars(first_line - 1) <= self.max {
            return;
        }
        // `extract_first_element_over_column_limit`: not a call, so no
        // drop-first; the implicit list IS a parser `array`, so the heredoc
        // shift applies.
        let mut i = 0usize;
        while self.within_column_limit(exceptions.get(i), first_line) {
            i += 1;
        }
        let heredoc_pos = heredoc_begin_position(self.source, &exceptions);
        let Some(i) = heredoc_pos.map_or(Some(i), |pos| shift_index_for_heredoc(&exceptions, i, pos)) else {
            return;
        };
        let bn = if i == 0 {
            exceptions.into_iter().next()
        } else {
            exceptions.into_iter().nth(i - 1)
        };
        let Some(bn) = bn else { return };
        let loc = bn.location();
        let start = loc.start_offset();
        let line_index = self.line_index.line_of(start) - 1;
        if !self.is_candidate(line_index) {
            return;
        }
        self.claim(line_index, start, loc.end_offset());
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
            if self.compact_nested_array_break(node, &bn) {
                return;
            }
            let loc = bn.location();
            let start = loc.start_offset();
            let line_index = self.line_index.line_of(start) - 1;
            if !self.is_candidate(line_index) {
                return;
            }
            self.claim(line_index, start, loc.end_offset());
        }
    }

    /// `compact_nested_array_break?` (1.92): with
    /// `Layout/SpaceInsideArrayLiteralBrackets` enforcing `compact`, do not
    /// break before a nested array that opens a `[` array (the two cops
    /// would undo each other forever).
    fn compact_nested_array_break(&self, node: &Node<'pr>, breakable: &Node<'pr>) -> bool {
        if !self.compact_array_brackets {
            return false;
        }
        let Some(array) = node.as_array_node() else {
            return false;
        };
        let square = array
            .opening_loc()
            .is_some_and(|o| &self.source[o.start_offset()..o.end_offset()] == b"[");
        if !square {
            return false;
        }
        let first_is_breakable = array
            .elements()
            .iter()
            .next()
            .is_some_and(|f| f.location().start_offset() == breakable.location().start_offset()
                && f.location().end_offset() == breakable.location().end_offset());
        first_is_breakable && self.source.get(breakable.location().start_offset()) == Some(&b'[')
    }

    fn extract_breakable_node(&self, node: &Node<'pr>) -> Option<Node<'pr>> {
        if let Some(call) = node.as_call_node() {
            if chained_to_heredoc(self.source, &call) {
                return None;
            }
            // parser-gem's `send.arguments` includes the block-pass argument
            // (`&blk`, bare `&`) as the LAST argument; prism keeps it in
            // `block()`. Append it BEFORE `process_args`: with a block-pass
            // last, a preceding braceless keyword hash is not the last
            // argument and is NOT flattened into its pairs, and the
            // block-pass itself is a pickable breakable element.
            let mut raw: Vec<Node<'pr>> = call
                .arguments()
                .map(|a| a.arguments().iter().collect())
                .unwrap_or_default();
            if let Some(block) = call.block()
                && block.as_block_argument_node().is_some()
            {
                raw.push(block);
            }
            let args = process_args(raw);
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
        // `heredoc_begin_position` looks at every child, the first one too.
        let applies = is_call || node.as_array_node().is_some() || node.as_hash_node().is_some();
        let heredoc_pos = if applies {
            heredoc_begin_position(self.source, &elements)
        } else {
            None
        };

        // `first_element_movable?` (1.92): an unparenthesized call or method
        // definition keeps its first element on the line.
        let drop_first = if is_call {
            let call = node.as_call_node().unwrap();
            call.opening_loc().is_none() && !first_argument_is_heredoc(self.source, &call)
        } else if let Some(def) = node.as_def_node() {
            def.lparen_loc().is_none()
        } else {
            false
        };
        if drop_first {
            if elements.is_empty() {
                return None;
            }
            elements.remove(0);
        }

        let mut i = 0usize;
        while self.within_column_limit(elements.get(i), line) {
            i += 1;
        }

        let i = match heredoc_pos {
            Some(pos) => shift_index_for_heredoc(&elements, i, pos)?,
            None => i,
        };
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
        if let Some(call) = node.as_call_node() {
            // Stock asks `multiline?` on the parser-gem SEND node, which stops
            // before an attached block; prism's `CallNode` spans through the
            // block's `end`. Measure the send part only, so a single-line call
            // that carries a multi-line block stays breakable.
            let (first_line, _) = node_lines(self.line_index, node);
            let send_last_line = self
                .line_index
                .line_of(super::send_range::send_node_end_offset(&call));
            return first_line != send_last_line;
        }
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
        self.contained_by_breakable_collection_on_same_line_at(node_first_line)
    }

    fn contained_by_breakable_collection_on_same_line_at(&self, node_first_line: usize) -> bool {
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
/// `heredoc_begin_position` — the start of the first heredoc among `children`
/// and their descendants (1.92 also finds a heredoc nested in an element).
fn heredoc_begin_position(source: &[u8], children: &[Node<'_>]) -> Option<usize> {
    let mut finder = HeredocPositionFinder { source, first: None };
    for child in children {
        finder.visit(child);
        if finder.first.is_some() {
            break;
        }
    }
    finder.first
}

struct HeredocPositionFinder<'a> {
    source: &'a [u8],
    first: Option<usize>,
}

impl HeredocPositionFinder<'_> {
    fn note(&mut self, node: &Node<'_>) {
        if is_heredoc(self.source, node) {
            let start = node.location().start_offset();
            self.first = Some(self.first.map_or(start, |f| f.min(start)));
        }
    }
}

impl<'pr> Visit<'pr> for HeredocPositionFinder<'_> {
    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        self.note(&node.as_node());
    }
    fn visit_x_string_node(&mut self, node: &ruby_prism::XStringNode<'pr>) {
        self.note(&node.as_node());
    }
    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        self.note(&node.as_node());
        ruby_prism::visit_interpolated_string_node(self, node);
    }
    fn visit_interpolated_x_string_node(
        &mut self,
        node: &ruby_prism::InterpolatedXStringNode<'pr>,
    ) {
        self.note(&node.as_node());
        ruby_prism::visit_interpolated_x_string_node(self, node);
    }
}

/// The heredoc branch of `shift_elements_for_heredoc_arg`: never break after
/// the element holding the heredoc. `None` mirrors the Ruby `nil` return.
fn shift_index_for_heredoc(elements: &[Node<'_>], index: usize, heredoc_pos: usize) -> Option<usize> {
    let last_safe = elements
        .iter()
        .rposition(|e| e.location().start_offset() <= heredoc_pos)?;
    if last_safe == 0 && elements[0].location().end_offset() > heredoc_pos {
        return None;
    }
    Some(index.min(last_safe + 1))
}

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

    // A rescue clause with several exception classes is broken like the
    // implicit array parser builds for it: before the class crossing `max`.
    #[test]
    fn breakable_rescue_exception_list() {
        let src = "begin\n  foo\nrescue Aaa::BbbError, Ccc::DddError, Eee::FffError => e\n  bar\nend\n";
        let r = run(src, 40, false);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, 2);
        assert_eq!(r[0].1, src.find("Eee").unwrap());
    }

    // A multi-line exception list is already broken; no claim.
    #[test]
    fn multiline_rescue_exception_list_ignored() {
        let src = "begin\n  foo\nrescue Aaa::BbbError,\n  Ccc::DddError, Eee::FffError, Ggg::HhhError => e\n  bar\nend\n";
        assert!(run(src, 40, false).is_empty());
    }

    // A single-line `{ }` block hanging off a multi-line receiver still
    // claims: BlockNode#single_line? compares only the `{` and `}` lines,
    // and the claim goes to the expression's FIRST line while the insertion
    // point sits after the `|args|` on a later line.
    #[test]
    fn block_on_multiline_receiver_claims_first_line() {
        let src = "((abc[:key][1] || []) + [xyz_padding_pad])\n  .any? { |target_id| abc[:blocking][target_id] }\n";
        let r = run(src, 40, false);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, 0, "claim goes to the receiver line");
        // The byte before the insertion point is the closing `|`.
        assert_eq!(src.as_bytes()[r[0].1 - 1], b'|');
    }

    // A block claim overwrites an earlier semicolon claim on the same line.
    #[test]
    fn block_overwrites_semicolon_claim() {
        let src = "aaa = 1; bbb.ccc_ddd(eee).fff { |g| g * 27 }\n";
        let r = run(src, 40, false);
        assert_eq!(r.len(), 1);
        assert_eq!(src.as_bytes()[r[0].1 - 1], b'|', "block wins over the semicolon");
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

    // 1.91 `breakable_space_position`: a space that is the last character of
    // the string content is no break point (the split would change nothing
    // and loop); the previous space is used instead.
    #[test]
    fn split_string_skips_a_trailing_space_break_point() {
        let src = "foo(\"aaaaaaaaaaaaaaaaaaaaaaaaa#{b} cc dd \" \\\n    \"ee\")\n";
        let got = run(src, 40, true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, 0);
        // 1.92 moved the break before `cc` (vendor spec).
        assert_eq!(got[0].1, src.find("cc dd").unwrap());
        assert_eq!(got[0].2, "\"");
    }

    // 1.92 `string_break_ranges`: a long string is split into every piece it
    // needs in one pass. Expected offsets from the stock CLI (`-A`, Max 40):
    // `x = 'a*32' \` / `'a*36' \` / `'a*32'`.
    #[test]
    fn split_string_into_several_pieces() {
        let src = format!("x = '{}'\n", "a".repeat(100));
        let got = compute_breakables(src.as_bytes(), 40, true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].insert_offset, 37);
        assert_eq!(got[0].extra_breaks, vec![73]);
    }

    // 1.92 `ESCAPE_SEQUENCE_TAIL`: an escaped backslash is complete, so the
    // cut may land right after it. Stock CLI (`-A`, Max 40) gives
    // `"\\x41 * 6\\" \` / `"x41...\\x" \` / `"41..."`.
    #[test]
    fn split_string_after_escaped_backslash() {
        let src = format!("x = \"{}\"\n", "\\\\x41".repeat(20));
        let got = compute_breakables(src.as_bytes(), 40, true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].insert_offset, 37);
        assert_eq!(got[0].extra_breaks, vec![73]);
    }

    #[test]
    fn escape_sequence_tail() {
        let start = |s: &str| escape_sequence_start(&s.chars().collect::<Vec<_>>());
        assert_eq!(start("ab\\"), Some(2));
        assert_eq!(start("ab\\\\"), None);
        assert_eq!(start("ab\\\\\\0"), Some(4));
        assert_eq!(start("ab\\M-\\C-"), Some(2));
        assert_eq!(start("ab\\u{1F600 1F6"), Some(2));
        assert_eq!(start("ab\\x4"), Some(2));
        assert_eq!(start("ab\\x41"), Some(2));
        assert_eq!(start("ab\\0001"), None);
    }
}
