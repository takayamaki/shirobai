//! Shared alignment helper for the `AllowForAlignment` family of cops.
//!
//! `Layout/SpaceAroundOperators` and `Layout/SpaceBeforeFirstArg` both honour an
//! `AllowForAlignment` option that permits extra spacing used to vertically
//! align a token with something on a preceding or following line. The logic is
//! the `PrecedingFollowingAlignment` mixin in stock rubocop
//! (`lib/rubocop/cop/mixin/preceding_following_alignment.rb`). This module hosts
//! the single implementation both cops drive (copy-divergence is forbidden;
//! equivalence is held by each cop's cargo tests).
//!
//! [`Aligner`] is bound to one source + its parser-gem token list. It needs the
//! token list, so callers build it in the walk-outer phase (the token cache
//! shares a `RefCell` with the AST parse). All positions are **byte** offsets;
//! columns are character columns within a line.

use super::line_index::LineIndex;
use super::tokens::Token;

/// A `:none` / `:yes` / `:no` tri-state for `aligned_with_equals_sign`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tri {
    None,
    Yes,
    No,
}

/// A two-valued predicate selector for `aligned_with_adjacent_line?`.
#[derive(Clone, Copy)]
enum Predicate {
    /// `aligned_token?` = aligned_words || aligned_equals_operator.
    Token,
    /// `aligned_operator?` = aligned_identical || aligned_equals_operator.
    Operator,
}

/// Alignment helper bound to one source + its token list.
pub(crate) struct Aligner<'a> {
    source: &'a [u8],
    line_index: &'a LineIndex,
    tokens: &'a [Token],
    /// `=` byte positions excluded by `remove_equals_in_def`.
    def_equals: &'a [usize],
}

impl<'a> Aligner<'a> {
    pub(crate) fn new(
        source: &'a [u8],
        line_index: &'a LineIndex,
        tokens: &'a [Token],
        def_equals: &'a [usize],
    ) -> Self {
        Aligner {
            source,
            line_index,
            tokens,
            def_equals,
        }
    }

    /// 1-based line of `off`.
    fn line(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    /// Char column of `off` within its line.
    fn column(&self, off: usize) -> usize {
        self.line_index.column(self.source, off)
    }

    /// The number of lines in the source (stock `processed_source.lines.length`,
    /// which excludes a trailing empty line after the final `\n`).
    fn line_count(&self) -> usize {
        // `source.lines` splits on `\n` and drops a trailing empty field
        // (Ruby `String#lines` keeps the newline with each line and a trailing
        // `\n` does not create an empty final line).
        let mut count = 0usize;
        let mut had_content = false;
        for &b in self.source {
            had_content = true;
            if b == b'\n' {
                count += 1;
                had_content = false;
            }
        }
        if had_content {
            count += 1;
        }
        count
    }

    /// The text of 1-based `line` without its trailing `\n` (stock
    /// `processed_source.lines[line - 1]`).
    fn line_text(&self, line: usize) -> Option<&'a [u8]> {
        if line == 0 || line > self.line_count() {
            return None;
        }
        let start = self.line_index.line_starts().get(line - 1).copied()?;
        let end = self
            .line_index
            .line_starts()
            .get(line)
            .copied()
            .map(|s| {
                // strip the trailing \n that line_starts[line] sits just past.
                if s > start && self.source.get(s - 1) == Some(&b'\n') {
                    s - 1
                } else {
                    s
                }
            })
            .unwrap_or(self.source.len());
        Some(&self.source[start..end])
    }

    /// `comment = comment_at_line(line); comment && with_space.last_column ==
    /// comment.loc.column`. Comment columns come from the token list.
    pub(crate) fn comment_excludes(&self, op_start: usize, ws_end: usize) -> bool {
        let op_line = self.line(op_start);
        let Some(comment_col) = self.comment_column_on_line(op_line) else {
            return false;
        };
        self.column(ws_end) == comment_col
    }

    /// Char column of the comment token that begins on 1-based `line`, if any.
    fn comment_column_on_line(&self, line: usize) -> Option<usize> {
        self.tokens
            .iter()
            .find(|t| t.comment() && self.line(t.begin_pos) == line)
            .map(|t| self.column(t.begin_pos))
    }

    // ----- aligned_with_* predicates -----

    /// `aligned_with_something?(range)` = `aligned_with_adjacent_line?(range,
    /// aligned_token?)`.
    pub(crate) fn aligned_with_something(&self, start: usize, end: usize) -> bool {
        self.aligned_with_adjacent_line(start, end, Predicate::Token)
    }

    /// `aligned_with_operator?(range)` = `aligned_with_adjacent_line?(range,
    /// aligned_operator?)`.
    pub(crate) fn aligned_with_operator(&self, start: usize, end: usize) -> bool {
        self.aligned_with_adjacent_line(start, end, Predicate::Operator)
    }

    /// `aligned_with_adjacent_line?`: search the preceding lines (downward) and
    /// the subsequent lines (upward) for an aligned token, first across all
    /// lines, then restricted to the nearest same-indent line.
    fn aligned_with_adjacent_line(&self, start: usize, end: usize, pred: Predicate) -> bool {
        let range_line = self.line(start); // 1-based
        // pre: 0-based indices from (range_line - 2) downto 0.
        let pre: Vec<usize> = (0..range_line.saturating_sub(1)).rev().collect();
        // post: 0-based from range_line upto line_count - 1.
        let post: Vec<usize> = (range_line..self.line_count()).collect();

        if self.aligned_with_any_line(&pre, &post, start, end, None, pred) {
            return true;
        }
        // base_indentation = lines[range.line - 1] =~ /\S/ (0-based line index
        // range_line - 1, i.e. the range's own line). Char index of first
        // non-space.
        let base_indentation = self.line_text(range_line).and_then(first_non_space_index);
        self.aligned_with_any_line(&pre, &post, start, end, base_indentation, pred)
    }

    fn aligned_with_any_line(
        &self,
        pre: &[usize],
        post: &[usize],
        start: usize,
        end: usize,
        indent: Option<usize>,
        pred: Predicate,
    ) -> bool {
        self.aligned_with_line(pre, start, end, indent, pred)
            || self.aligned_with_line(post, start, end, indent, pred)
    }

    /// `aligned_with_line?(line_nos, range, indent)`. `line_nos` are 0-based
    /// line indices.
    fn aligned_with_line(
        &self,
        line_nos: &[usize],
        start: usize,
        end: usize,
        indent: Option<usize>,
        pred: Predicate,
    ) -> bool {
        for &lineno0 in line_nos {
            // next if aligned_comment_lines.include?(lineno + 1)
            if self.aligned_comment_line(lineno0 + 1) {
                continue;
            }
            let Some(line) = self.line_text(lineno0 + 1) else {
                continue;
            };
            let Some(index) = first_non_space_index(line) else {
                continue;
            };
            if matches!(indent, Some(ind) if ind != index) {
                continue;
            }
            // The first line with a non-space (and matching indent, if given)
            // decides the result; stock returns the predicate value here.
            // return yield(range, line, lineno + 1)
            return match pred {
                Predicate::Token => {
                    self.aligned_words(start, end, line)
                        || self.aligned_equals_operator(start, end, lineno0 + 1)
                }
                Predicate::Operator => {
                    self.aligned_identical(start, end, line)
                        || self.aligned_equals_operator(start, end, lineno0 + 1)
                }
            };
        }
        false
    }

    /// `aligned_comment_lines`: lines of full-line comments (a comment whose
    /// expression begins its line).
    fn aligned_comment_line(&self, line: usize) -> bool {
        self.tokens.iter().any(|t| {
            t.comment() && self.line(t.begin_pos) == line && self.begins_its_line(t.begin_pos)
        })
    }

    /// `begins_its_line?(range)`: the range's column equals the line's first
    /// non-space column.
    fn begins_its_line(&self, off: usize) -> bool {
        let line = self.line(off);
        let col = self.column(off);
        match self.line_text(line).and_then(first_non_space_index) {
            Some(i) => i == col,
            None => false,
        }
    }

    /// `aligned_words?(range, line)`: a non-space char two before the range's
    /// left edge is a space-then-nonspace, or the line has the same token text
    /// at the range's column.
    fn aligned_words(&self, start: usize, end: usize, line: &[u8]) -> bool {
        let left_edge = self.column(start);
        let chars = line_chars(line);
        // `/\s\S/.match?(line[left_edge - 1, 2])`: a whitespace char at
        // `left_edge - 1` immediately followed by a non-whitespace one.
        if let Some(prev) = left_edge.checked_sub(1) {
            let a = chars.get(prev).copied();
            let b = chars.get(left_edge).copied();
            if matches!((a, b), (Some(a), Some(b)) if is_ws_char(a) && !is_ws_char(b)) {
                return true;
            }
        }
        // `token == line[left_edge, token.length]`.
        let token = self.source_chars(start, end);
        if left_edge + token.len() <= chars.len() {
            return chars[left_edge..left_edge + token.len()] == token[..];
        }
        false
    }

    /// `aligned_identical?(range, line)`: `range.source == line[range.column,
    /// range.size]` (range.size is char length).
    fn aligned_identical(&self, start: usize, end: usize, line: &[u8]) -> bool {
        let col = self.column(start);
        let token = self.source_chars(start, end);
        let chars = line_chars(line);
        if col + token.len() <= chars.len() {
            chars[col..col + token.len()] == token[..]
        } else {
            false
        }
    }

    /// `aligned_equals_operator?(range, lineno)`: find the first
    /// assignment-or-comparison token on `lineno` (1-based) and test whether
    /// `range` aligns its `=`-end or `<<`-append with that token's last column.
    fn aligned_equals_operator(&self, start: usize, end: usize, lineno: usize) -> bool {
        let Some(line_range) = self.line_byte_range(lineno) else {
            return false;
        };
        // The first ASSIGNMENT_OR_COMPARISON token whose begin is within the line.
        let operator_token = self.tokens.iter().find(|t| {
            t.assignment_or_comparison() && t.begin_pos >= line_range.0 && t.begin_pos < line_range.1
        });

        self.aligned_with_preceding_equals(start, end, operator_token)
            || self.aligned_with_append_operator(start, end, operator_token)
    }

    /// `aligned_with_preceding_equals?(range, token)`: `range.source[-1] == '='
    /// && range.last_column == token.pos.last_column`.
    fn aligned_with_preceding_equals(
        &self,
        _start: usize,
        end: usize,
        token: Option<&Token>,
    ) -> bool {
        let Some(token) = token else { return false };
        if self.source.get(end - 1) != Some(&b'=') {
            return false;
        }
        self.column(end) == self.column(token.end_pos)
    }

    /// `aligned_with_append_operator?(range, token)`.
    fn aligned_with_append_operator(&self, start: usize, end: usize, token: Option<&Token>) -> bool {
        let Some(token) = token else { return false };
        let src = self.source_bytes(start, end);
        let token_is_lshift = self.token_is_lshift(token);
        let cond = (src == b"<<" && token.equal_sign())
            || (self.source.get(end - 1) == Some(&b'=') && token_is_lshift);
        cond && self.column(end) == self.column(token.end_pos)
    }

    /// Whether `token.type == :tLSHFT` (a `<<` whose text is `<<`). The token
    /// classifier folds `<<` into the `Comparison` group; disambiguate by text.
    fn token_is_lshift(&self, token: &Token) -> bool {
        self.source_bytes(token.begin_pos, token.end_pos) == b"<<"
    }

    // ----- :assignment path (preceding/subsequent equals alignment) -----

    pub(crate) fn aligned_with_preceding_equals_operator(
        &self,
        op_start: usize,
        op_end: usize,
    ) -> Tri {
        let line = self.line(op_start);
        // preceding_line_range = token.line.downto(1)  (1-based, inclusive)
        let range: Vec<usize> = (1..=line).rev().collect();
        self.aligned_with_equals_sign(op_start, op_end, &range)
    }

    pub(crate) fn aligned_with_subsequent_equals_operator(
        &self,
        op_start: usize,
        op_end: usize,
    ) -> Tri {
        let line = self.line(op_start);
        // subsequent_line_range = token.line.upto(lines.length)
        let range: Vec<usize> = (line..=self.line_count()).collect();
        self.aligned_with_equals_sign(op_start, op_end, &range)
    }

    /// `aligned_with_equals_sign(token, line_range)`.
    fn aligned_with_equals_sign(&self, op_start: usize, op_end: usize, line_range: &[usize]) -> Tri {
        let token_line = self.line(op_start);
        let token_line_indent = self.line_indentation(token_line);
        let assignment_lines = self.relevant_assignment_lines(line_range);
        let Some(&relevant_line_number) = assignment_lines.get(1) else {
            return Tri::None;
        };
        let relevant_indent = self.line_indentation(relevant_line_number);
        if relevant_indent < token_line_indent {
            return Tri::None;
        }
        if self.line_text(relevant_line_number).is_none() {
            return Tri::None;
        }
        if self.aligned_equals_operator(op_start, op_end, relevant_line_number) {
            Tri::Yes
        } else {
            Tri::No
        }
    }

    /// `relevant_assignment_lines(line_range)`. `line_range` is a sequence of
    /// 1-based line numbers (already in iteration order).
    fn relevant_assignment_lines(&self, line_range: &[usize]) -> Vec<usize> {
        let mut result = Vec::new();
        let Some(&first) = line_range.first() else {
            return result;
        };
        let original_line_indent = self.line_indentation(first);
        let mut relevant_line_indent_at_level = true;
        let assignment_lines = self.assignment_lines();

        for &line_number in line_range {
            let current_line_indent = self.line_indentation(line_number);
            let blank_line = self
                .line_text(line_number)
                .map(is_blank_line)
                .unwrap_or(true);
            if (current_line_indent < original_line_indent && !blank_line)
                || (relevant_line_indent_at_level && blank_line)
            {
                break;
            }
            if assignment_lines.contains(&line_number)
                && current_line_indent == original_line_indent
            {
                result.push(line_number);
            }
            if !blank_line {
                relevant_line_indent_at_level = current_line_indent == original_line_indent;
            }
        }
        result
    }

    /// `assignment_lines` = lines of `assignment_tokens`.
    fn assignment_lines(&self) -> Vec<usize> {
        self.assignment_tokens()
            .iter()
            .map(|&pos| self.line(pos))
            .collect()
    }

    /// `assignment_tokens`: all `equal_sign?` tokens minus the def/optarg `=`
    /// positions, deduplicated to the first per line.
    fn assignment_tokens(&self) -> Vec<usize> {
        let mut seen_lines = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for t in self.tokens {
            if !t.equal_sign() {
                continue;
            }
            if self.def_equals.contains(&t.begin_pos) {
                continue;
            }
            let line = self.line(t.begin_pos);
            if seen_lines.insert(line) {
                out.push(t.begin_pos);
            }
        }
        out
    }

    /// `processed_source.line_indentation(line)`: number of leading spaces/tabs
    /// of 1-based `line`. We mirror `line[/\A[ \t]*/].length` over the line
    /// content (without newline).
    fn line_indentation(&self, line: usize) -> usize {
        match self.line_text(line) {
            Some(text) => text
                .iter()
                .take_while(|&&b| b == b' ' || b == b'\t')
                .count(),
            None => 0,
        }
    }

    // ----- helpers -----

    /// Byte range `[start, end)` of 1-based `line` excluding the trailing `\n`.
    fn line_byte_range(&self, line: usize) -> Option<(usize, usize)> {
        if line == 0 || line > self.line_count() {
            return None;
        }
        let start = self.line_index.line_starts().get(line - 1).copied()?;
        let end = self
            .line_index
            .line_starts()
            .get(line)
            .map(|&s| {
                if s > start && self.source.get(s - 1) == Some(&b'\n') {
                    s - 1
                } else {
                    s
                }
            })
            .unwrap_or(self.source.len());
        Some((start, end))
    }

    fn source_bytes(&self, start: usize, end: usize) -> &'a [u8] {
        &self.source[start..end]
    }

    /// The chars of `source[start..end]`.
    fn source_chars(&self, start: usize, end: usize) -> Vec<char> {
        match std::str::from_utf8(&self.source[start..end]) {
            Ok(s) => s.chars().collect(),
            Err(_) => self.source[start..end].iter().map(|&b| b as char).collect(),
        }
    }
}

/// Char index of the first non-space char (`=~ /\S/`), or None.
fn first_non_space_index(line: &[u8]) -> Option<usize> {
    let chars = line_chars(line);
    chars.iter().position(|&c| !is_ws_char(c))
}

fn is_ws_char(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{0b}' | '\u{0c}')
}

/// `line.blank?` (ActiveSupport): empty or only whitespace.
fn is_blank_line(line: &[u8]) -> bool {
    line.iter()
        .all(|&b| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c))
}

/// The chars of a line slice.
fn line_chars(line: &[u8]) -> Vec<char> {
    match std::str::from_utf8(line) {
        Ok(s) => s.chars().collect(),
        Err(_) => line.iter().map(|&b| b as char).collect(),
    }
}
