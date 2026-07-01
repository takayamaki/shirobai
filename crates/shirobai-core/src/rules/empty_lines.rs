//! `Layout/EmptyLines`.
//!
//! Stock flags any line `L` where both `processed_source[L - 2]` and
//! `processed_source[L - 1]` are textually empty AND `L` falls inside a gap
//! `prev_token_line + 1 < L < cur_token_line` with `cur - prev > 2`. The
//! reported and autocorrected range is the 1-byte `source_range(buffer, L, 0)`.
//!
//! Stock's `on_new_investigation`:
//!
//! 1. `return if processed_source.tokens.empty?`
//! 2. `return unless processed_source.raw_source.include?("\n\n\n")`
//! 3. `lines = Set.new; tokens.each { |t| lines << t.line }`; for every
//!    consecutive pair `(prev, cur)` in `lines.sort`, if `cur - prev > 2` walk
//!    `((prev + 1)...cur)` and yield `source_range(buffer, L, 0)` when both
//!    `processed_source[L - 2]` and `processed_source[L - 1]` are empty.
//!
//! Stock uses parser-gem tokens. Prism has no equivalent token list, so we
//! reconstruct "lines with tokens" from the AST instead:
//!
//! - Walk every node and mark both its start line and end line. parser-gem
//!   emits an opening/closing punctuation token on the line each lives on
//!   (e.g. `tLPAREN` at `start_line`, `tRPAREN` at `end_line` for a multi-line
//!   `foo(\n)`), which is exactly the pair our `start_line` + `end_line`
//!   marking reproduces. Container nodes (Array/Hash/Call) thus contribute
//!   only their delimiters' lines — the body's gap lines stay unmarked, same
//!   as stock.
//! - String-content nodes (StringNode / InterpolatedStringNode /
//!   RegularExpressionNode / InterpolatedRegularExpressionNode / XStringNode /
//!   InterpolatedXStringNode / SymbolNode / InterpolatedSymbolNode /
//!   MatchLastLineNode / InterpolatedMatchLastLineNode) need every line in
//!   their span marked, because parser-gem emits one `tSTRING_CONTENT` per
//!   file line of body. A multi-line string `"a\n\n\nb"` therefore covers
//!   lines 1..4 in both implementations (no gap visible to the cop). Percent
//!   arrays like `%w[a\n\n\nb]` are an ArrayNode whose children are the per-
//!   element StringNodes; only the open/close lines and each element's line
//!   get marked, matching stock's tokens for that shape.
//! - Comments are separate from the AST in prism; pull them from the shared
//!   parse cache and mark each comment's line (parser-gem emits a `tCOMMENT`
//!   for each).
//!
//! `__END__` cuts off both prism and parser; the data segment is unparsed and
//! produces no marks on either side, so neither side flags it.
//!
//! The substring `"\n\n\n"` prefilter is kept — it's exactly what stock does
//! and lets the common no-blank-lines file skip the AST walk entirely.

use ruby_prism::Node;

use super::line_index::LineIndex;
use super::parse_cache;

/// One offense. `[start, end)` is the 1-byte `source_range(buffer, L, 0)`
/// range the wrapper passes to both `add_offense` and `corrector.remove`.
pub struct EmptyLinesOffense {
    pub start: usize,
    pub end: usize,
}

pub fn check_empty_lines(source: &[u8]) -> Vec<EmptyLinesOffense> {
    // Stock's `processed_source.raw_source.include?("\n\n\n")` prefilter —
    // without three consecutive newlines no line could have both `L - 2` and
    // `L - 1` empty.
    if !contains_newline_triple(source) {
        return Vec::new();
    }

    // Collect "lines with parser-gem tokens" via the AST walk + comments.
    let mut rule = build_rule(source);
    super::dispatch::run(source, &mut [&mut rule]);
    finalize(source, rule)
}

/// Quick `"\n\n\n"` substring check; the bundle path uses this to decide
/// whether to push the rule into the shared walk at all. Same semantics as
/// `String#include?` over the raw source.
pub fn contains_newline_triple(source: &[u8]) -> bool {
    contains_subslice(source, b"\n\n\n")
}

/// Convert the walk's collected token-bearing lines into the per-source
/// offense list. The shared-walk bundle calls this after `dispatch::run`,
/// reusing the same Rule the standalone entry built.
pub(crate) fn finalize(source: &[u8], rule: Visitor<'_>) -> Vec<EmptyLinesOffense> {
    let (mut lines, line_index, comments) = rule.into_parts();
    for (s, _e) in comments {
        lines.push(line_index.line_of(s));
    }
    if lines.is_empty() {
        return Vec::new();
    }
    lines.sort_unstable();
    lines.dedup();

    let line_starts = line_index.line_starts();
    let total_lines = line_starts.len(); // matches stock's `lines.size`

    let mut out = Vec::new();
    let mut prev_line = 1usize;
    for &cur_line in &lines {
        if cur_line > prev_line + 2 {
            // ((prev + 1)...cur), excluding `cur`. For each L check both
            // `lines[L - 2]` and `lines[L - 1]` are empty.
            let lower = prev_line + 1;
            let upper = cur_line; // exclusive
            for line in lower..upper {
                if line < 2 || line > total_lines {
                    continue;
                }
                if is_line_empty(source, line_starts, line - 1, total_lines)
                    && is_line_empty(source, line_starts, line, total_lines)
                {
                    let line_start = line_starts[line - 1]; // line is 1-based; line - 1 is 0-based index
                    out.push(EmptyLinesOffense {
                        start: line_start,
                        end: line_start + 1,
                    });
                }
            }
        }
        prev_line = cur_line;
    }
    out
}

/// True iff the 1-based `line` (1..=total_lines) has no bytes between its
/// start and the next `\n` (or end-of-source for the last line). Mirrors
/// `processed_source[line - 1].empty?` (`processed_source[]` is the
/// 0-indexed alias for `processed_source.lines`).
///
/// `processed_source.lines` is `buffer.source.lines` with the trailing `\n`
/// chopped from each element. Stock's emptiness check therefore returns true
/// only when the line has no content beyond its terminator. A trailing
/// phantom line (`"x\n".lines.size == 2`-style) is empty as well.
fn is_line_empty(source: &[u8], line_starts: &[usize], line: usize, total_lines: usize) -> bool {
    // `line` is 1-based. line_starts[line - 1] is the start of that line.
    let start = line_starts[line - 1];
    let end = if line < total_lines {
        line_starts[line] - 1 // strip the trailing `\n`
    } else {
        source.len()
    };
    start == end
}

/// Does `haystack` contain `needle` as a contiguous byte subslice?
fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Build the cop's rule for use standalone or in the shared walk. The walk
/// itself owns the collected `lines` set; the bundle entry point drives it
/// alongside the other shared-walk cops.
pub(crate) fn build_rule(source: &[u8]) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        lines: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: std::rc::Rc<LineIndex>,
    /// 1-based line numbers of nodes seen during the walk.
    pub(crate) lines: Vec<usize>,
}

impl<'a> Visitor<'a> {
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    /// Fill `[start, end]` line range. `end_off` is exclusive; the last
    /// occupied line is the line of `end_off - 1` (zero-length ranges produce
    /// nothing).
    fn fill_lines(&mut self, start_off: usize, end_off: usize) {
        if end_off <= start_off {
            return;
        }
        let start_line = self.line_of(start_off);
        let end_line = self.line_of(end_off - 1);
        for l in start_line..=end_line {
            self.lines.push(l);
        }
    }

    /// Record the node's start and end lines (parser-gem emits an opening
    /// and a closing token on each, respectively). For string-content node
    /// types fill the whole content span — for heredocs the node's own
    /// `location` covers only the `<<~MARK` marker, so the body lines must be
    /// picked up via `content_loc` separately. For plain strings the content
    /// span is inside `location` so the fill is a superset, not a conflict.
    fn mark(&mut self, node: &Node<'_>) {
        let loc = node.location();
        let start_off = loc.start_offset();
        let end_off = loc.end_offset();
        let start_line = self.line_of(start_off);
        let end_line = if end_off == 0 { start_line } else { self.line_of(end_off - 1) };
        self.lines.push(start_line);
        if end_line != start_line {
            self.lines.push(end_line);
        }
        // Pick up the per-line body tokens parser-gem emits for string-like
        // literals (tSTRING_CONTENT, one per file line of body).
        if let Some(s) = node.as_string_node() {
            let c = s.content_loc();
            self.fill_lines(c.start_offset(), c.end_offset());
            if let Some(o) = s.closing_loc() {
                self.fill_lines(o.start_offset(), o.end_offset());
            }
        } else if let Some(s) = node.as_x_string_node() {
            let c = s.content_loc();
            self.fill_lines(c.start_offset(), c.end_offset());
            self.fill_lines(s.closing_loc().start_offset(), s.closing_loc().end_offset());
        } else if let Some(s) = node.as_regular_expression_node() {
            let c = s.content_loc();
            self.fill_lines(c.start_offset(), c.end_offset());
            self.fill_lines(s.closing_loc().start_offset(), s.closing_loc().end_offset());
        } else if let Some(s) = node.as_match_last_line_node() {
            let c = s.content_loc();
            self.fill_lines(c.start_offset(), c.end_offset());
            self.fill_lines(s.closing_loc().start_offset(), s.closing_loc().end_offset());
        } else if let Some(s) = node.as_symbol_node() {
            if let Some(v) = s.value_loc() {
                self.fill_lines(v.start_offset(), v.end_offset());
            }
            if let Some(c) = s.closing_loc() {
                self.fill_lines(c.start_offset(), c.end_offset());
            }
        } else if let Some(s) = node.as_interpolated_string_node() {
            // Heredoc body lives in `parts`; each `StringNode` part carries
            // its own content_loc on a real body line (visited later). The
            // closing_loc covers the heredoc terminator line (e.g. `TEXT\n`)
            // which is itself a parser-gem token line.
            if let Some(c) = s.closing_loc() {
                self.fill_lines(c.start_offset(), c.end_offset());
            }
        } else if let Some(s) = node.as_interpolated_x_string_node() {
            let c = s.closing_loc();
            self.fill_lines(c.start_offset(), c.end_offset());
        } else if let Some(s) = node.as_interpolated_regular_expression_node() {
            self.fill_lines(s.closing_loc().start_offset(), s.closing_loc().end_offset());
        } else if let Some(s) = node.as_interpolated_match_last_line_node() {
            self.fill_lines(s.closing_loc().start_offset(), s.closing_loc().end_offset());
        } else if let Some(s) = node.as_interpolated_symbol_node()
            && let Some(c) = s.closing_loc()
        {
            self.fill_lines(c.start_offset(), c.end_offset());
        }
    }

    fn into_parts(self) -> (Vec<usize>, std::rc::Rc<LineIndex>, Vec<(usize, usize)>) {
        // Pull comments from the same cached parse (single re-borrow; cheap).
        let comments = parse_cache::comment_ranges(self.source);
        (self.lines, self.line_index, comments)
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_ALL
                    | Interest::LEAF,
        )
    }
    
    fn enter(&mut self, node: &Node<'_>) {
        self.mark(node);
    }
    fn leave(&mut self) {}
    fn enter_leaf(&mut self, node: &Node<'_>) {
        self.mark(node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(src: &str) -> Vec<(usize, usize)> {
        check_empty_lines(src.as_bytes())
            .into_iter()
            .map(|o| (o.start, o.end))
            .collect()
    }

    // Typical: two consecutive blank lines between statements is one offense
    // at the SECOND blank line (parser yields `source_range(buffer, L, 0)`
    // for `L = 3`).
    #[test]
    fn two_blank_lines() {
        // "a = 1\n\n\nb = 2\n": ws starts at 6 (the second `\n`). line 3
        // start = 7. Offense [7, 8).
        let got = ranges("a = 1\n\n\nb = 2\n");
        assert_eq!(got, vec![(7, 8)]);
    }

    // Three blank lines yields TWO offenses (the second and third blank
    // lines); the loop reports each L that has both `L-2` and `L-1` empty.
    #[test]
    fn three_blank_lines_two_offenses() {
        // "a = 1\n\n\n\nb = 2\n": line starts at 0, 6, 7, 8, 9.
        // L=3: start 7, L=4: start 8.
        let got = ranges("a = 1\n\n\n\nb = 2\n");
        assert_eq!(got, vec![(7, 8), (8, 9)]);
    }

    // No `\n\n\n` substring: prefilter skips the AST walk entirely.
    #[test]
    fn no_consecutive_blank_lines() {
        assert!(ranges("a\n\nb\n").is_empty());
    }

    // String with literal newlines inside: stock emits `tSTRING_CONTENT` per
    // line, so the blank lines have tokens and no offense fires.
    #[test]
    fn blank_lines_in_string_body() {
        assert!(ranges("x = \"a\n\n\nb\"\n").is_empty());
    }

    // Heredoc body with internal blank lines: stock emits content tokens for
    // every line, including the blanks.
    #[test]
    fn blank_lines_in_heredoc_body() {
        assert!(ranges("x = <<~T\nline 1\n\n\nline 2\nT\nputs x\n").is_empty());
    }

    // Percent array (`%w[]`) does NOT have per-line tokens for blanks inside,
    // so stock flags the blank-line gap. We must too.
    #[test]
    fn blank_lines_in_percent_word_array() {
        // "x = %w[a\n\n\nb]\n": bytes 0-6 = "x = %w[", 7 = "a", 8 = "\n",
        // 9 = "\n", 10 = "\n", 11 = "b", 12 = "]", 13 = "\n". L=3 starts at 10.
        let got = ranges("x = %w[a\n\n\nb]\n");
        assert_eq!(got, vec![(10, 11)]);
    }

    // Array literal with blank lines between elements: same as percent
    // arrays — the gap is visible.
    #[test]
    fn blank_lines_in_array_literal() {
        // "x = [1,\n\n\n2]\n": bytes 0-6 = "x = [1,", 7-9 = "\n\n\n",
        // 10 = "2", 11 = "]", 12 = "\n". L=3 starts at 9.
        let got = ranges("x = [1,\n\n\n2]\n");
        assert_eq!(got, vec![(9, 10)]);
    }

    // Blank lines between comment lines are still tracked (comments have
    // their own line via `processed_source.tokens`).
    #[test]
    fn blank_lines_between_comments() {
        // "# a\n\n\n# b\n": L=3 start = 5.
        let got = ranges("# a\n\n\n# b\n");
        assert_eq!(got, vec![(5, 6)]);
    }

    // A blank-line gap with comment on the line BEFORE the gap: the comment's
    // line counts as a token line, breaking the gap.
    #[test]
    fn comment_between_breaks_gap() {
        assert!(ranges("a\n\n# foo\nb\n").is_empty());
    }

    // `__END__` cuts off both prism and parser; nothing past it is parsed and
    // no offense fires for blanks before it.
    #[test]
    fn end_marker_stops_parse() {
        assert!(ranges("x = 1\n\n\n\n__END__\nfoo\n\n\nbar\n").is_empty());
    }

    // `__END__` inside a string is NOT the marker; parsing continues.
    #[test]
    fn end_marker_inside_string_not_marker() {
        let got = ranges("x = \"__END__\"\n\n\ny\n");
        assert_eq!(got.len(), 1);
    }

    // An empty file produces no offenses.
    #[test]
    fn empty_source() {
        assert!(ranges("").is_empty());
    }

    // A single trailing-blank-line gap at end of file: there's no second
    // token line after, so no offense (matches stock's loop, which only
    // checks gaps BETWEEN token lines).
    #[test]
    fn trailing_blank_lines_no_offense() {
        assert!(ranges("x = 1\n\n\n\n").is_empty());
    }

    // Blank lines before the first token line: stock seeds `prev_line = 1`
    // (the implicit "line 1" anchor). When the first token line is far
    // enough away, the loop checks every intermediate L; both L=2 and L=3
    // see two empty predecessor lines and each yields its own offense.
    #[test]
    fn blank_lines_before_first_token() {
        // "\n\n\nfoo\n": first token line = 4. prev_line = 1, gap = 3.
        // L=2 start = 1, L=3 start = 2.
        let got = ranges("\n\n\nfoo\n");
        assert_eq!(got, vec![(1, 2), (2, 3)]);
    }

    // A blank line at top followed by a real line: only one blank, no
    // offense.
    #[test]
    fn one_blank_at_top_no_offense() {
        assert!(ranges("\nfoo\n").is_empty());
    }

    // Multi-line def with a blank-line gap inside: offense at the second
    // blank line.
    #[test]
    fn blank_gap_inside_def() {
        // "def foo\n  bar\n\n\n  baz\nend\n": line starts 0, 8, 14, 15, 16, 22, 26.
        // Token lines: {1, 2, 5, 6}. Gap 2→5: check L=3 (lines[1]=`  bar` not
        // empty → skip), L=4 (lines[2] empty, lines[3] empty → offense at L=4
        // start = 15).
        let got = ranges("def foo\n  bar\n\n\n  baz\nend\n");
        assert_eq!(got, vec![(15, 16)]);
    }
}
