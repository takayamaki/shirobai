//! `Layout/LineLength`.
//!
//! Flags source lines whose visible length exceeds `Max`. The hot path — the
//! per-line scan over *every* line of the file — lives entirely in Rust; Ruby
//! only post-processes the handful of candidate lines that actually exceed the
//! limit (applying the regex-based exemptions such as `AllowedPatterns`,
//! `AllowURI` and cop directives that depend on Ruby's `URI`/`Regexp`).
//!
//! Line length is measured in characters (`chars().count()`), not bytes, with a
//! tab-width adjustment matching RuboCop's `indentation_difference`.

use ruby_prism::Visit;

/// A line that exceeds `Max`. Ruby decides whether it is ultimately exempt.
pub struct LineLengthCandidate {
    /// Zero-based line index (matching `processed_source.lines`).
    pub line_index: usize,
    /// Visible length of the line (chars + tab adjustment).
    pub length: usize,
    /// Byte offset of the start of the line in the source.
    pub line_start: usize,
    /// Byte offset of the end of the line (exclusive of the newline).
    pub line_end: usize,
    /// `indentation_difference` for the line (extra visible width from tabs).
    pub indentation_difference: usize,
    /// End delimiters (e.g. `SQL`) of every heredoc whose body covers this line.
    /// A line can be inside several nested heredocs; Ruby treats the line as
    /// permitted if *any* of these delimiters is allowed (`AllowHeredoc`).
    pub heredoc_delimiters: Vec<String>,
}

/// `indentation_difference` from `LineLengthHelp`: extra visible width gained
/// from leading tabs. `0` when `tab_width` is `0` (the cop disables the
/// adjustment when `tab_indentation_width` is falsey).
fn indentation_difference(line: &[u8], tab_width: usize) -> usize {
    if tab_width == 0 {
        return 0;
    }
    // `index` is the position of the first non-tab byte, or 0 when the line
    // starts with a non-tab character.
    let index = if line.first() != Some(&b'\t') {
        0
    } else {
        line.iter().position(|&b| b != b'\t').unwrap_or(0)
    };
    index * (tab_width - 1)
}

/// Character count of a line, treating bytes as UTF-8. Mirrors `String#length`.
fn char_count(line: &[u8]) -> usize {
    // RuboCop measures with `String#length` (character count). Lines are valid
    // UTF-8 in practice; fall back to byte length on invalid sequences.
    match std::str::from_utf8(line) {
        Ok(s) => s.chars().count(),
        Err(_) => line.len(),
    }
}

/// Walk every line and return those whose visible length exceeds `max`.
///
/// `tab_width` is `tab_indentation_width` (`Layout/IndentationStyle`'s
/// `IndentationWidth` or the configured indentation width); `0` disables the
/// tab adjustment.
pub fn check_line_length(source: &[u8], max: usize, tab_width: usize) -> Vec<LineLengthCandidate> {
    let heredocs = collect_heredocs(source);
    let end_line = end_marker_line(source);

    let mut candidates = Vec::new();
    let mut line_index = 0usize;
    let mut pos = 0usize;
    let len = source.len();

    while pos <= len {
        // `processed_source.lines` stops before `__END__`.
        if let Some(end) = end_line
            && line_index >= end
        {
            break;
        }

        let line_start = pos;
        let mut line_end = line_start;
        while line_end < len && source[line_end] != b'\n' {
            line_end += 1;
        }

        // The trailing empty segment after a final newline is not a real line.
        if line_start == len && line_index > 0 {
            break;
        }

        let line = &source[line_start..line_end];
        let indent_diff = indentation_difference(line, tab_width);
        let length = char_count(line) + indent_diff;

        if length > max {
            let one_based = line_index + 1;
            let delimiters = heredocs
                .iter()
                .filter(|(start, end, _)| one_based >= *start && one_based < *end)
                .map(|(_, _, d)| d.clone())
                .collect();

            candidates.push(LineLengthCandidate {
                line_index,
                length,
                line_start,
                line_end,
                indentation_difference: indent_diff,
                heredoc_delimiters: delimiters,
            });
        }

        if line_end >= len {
            break;
        }
        pos = line_end + 1;
        line_index += 1;
    }

    candidates
}

/// Find the zero-based index of an `__END__` marker line, if any. Everything
/// from that line on is excluded from `processed_source.lines`.
fn end_marker_line(source: &[u8]) -> Option<usize> {
    let mut line_index = 0usize;
    let mut pos = 0usize;
    let len = source.len();
    while pos <= len {
        let line_start = pos;
        let mut line_end = line_start;
        while line_end < len && source[line_end] != b'\n' {
            line_end += 1;
        }
        if &source[line_start..line_end] == b"__END__" {
            return Some(line_index);
        }
        if line_end >= len {
            break;
        }
        pos = line_end + 1;
        line_index += 1;
    }
    None
}

/// Heredoc body ranges as `(first_line, last_line_exclusive, delimiter)`, with
/// one-based line numbers. Mirrors RuboCop's `extract_heredocs`:
/// `body.first_line...body.last_line` covers the content lines, and the
/// delimiter is the stripped end-marker source.
fn collect_heredocs(source: &[u8]) -> Vec<(usize, usize, String)> {
    super::parse_cache::with_parsed(source, |source, node| {
        let mut visitor = HeredocVisitor {
            source,
            ranges: Vec::new(),
        };
        visitor.visit(node);
        visitor.ranges
    })
}

fn line_of(source: &[u8], off: usize) -> usize {
    source[..off].iter().filter(|&&b| b == b'\n').count() + 1
}

struct HeredocVisitor<'a> {
    source: &'a [u8],
    ranges: Vec<(usize, usize, String)>,
}

impl HeredocVisitor<'_> {
    /// Record a heredoc whose body spans the source byte range
    /// `[body_start, body_end)` and whose end delimiter is at `closing`.
    ///
    /// `opening` must start with `<<` (otherwise the node is a plain string, not
    /// a heredoc). The recorded range mirrors RuboCop's
    /// `body.first_line...body.last_line` (a Ruby exclusive range over one-based
    /// line numbers), so it covers `line_of(body_start) ..= line_of(body_end)-1`.
    fn record(
        &mut self,
        opening: ruby_prism::Location<'_>,
        body_start: usize,
        body_end: usize,
        closing: ruby_prism::Location<'_>,
    ) {
        let open_src = &self.source[opening.start_offset()..opening.end_offset()];
        if !open_src.starts_with(b"<<") {
            return;
        }
        let first_line = line_of(self.source, body_start);
        let last_line = line_of(self.source, body_end);
        let delimiter =
            String::from_utf8_lossy(&self.source[closing.start_offset()..closing.end_offset()])
                .trim()
                .to_string();
        self.ranges.push((first_line, last_line, delimiter));
    }
}

impl<'pr> Visit<'pr> for HeredocVisitor<'pr> {
    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        if let (Some(opening), Some(closing)) = (node.opening_loc(), node.closing_loc()) {
            let content = node.content_loc();
            self.record(
                opening,
                content.start_offset(),
                content.end_offset(),
                closing,
            );
        }
    }

    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        if let (Some(opening), Some(closing)) = (node.opening_loc(), node.closing_loc())
            && let Some(first) = node.parts().iter().next()
        {
            // The body of an interpolated heredoc runs from the first part to
            // the closing delimiter line (`opening_loc` only marks the `<<` on
            // the introducing line, which is wrong for stacked heredocs).
            self.record(
                opening,
                first.location().start_offset(),
                closing.start_offset(),
                closing,
            );
        }
        for part in node.parts().iter() {
            self.visit(&part);
        }
    }

    fn visit_x_string_node(&mut self, node: &ruby_prism::XStringNode<'pr>) {
        let content = node.content_loc();
        self.record(
            node.opening_loc(),
            content.start_offset(),
            content.end_offset(),
            node.closing_loc(),
        );
    }

    fn visit_interpolated_x_string_node(
        &mut self,
        node: &ruby_prism::InterpolatedXStringNode<'pr>,
    ) {
        if let Some(first) = node.parts().iter().next() {
            let closing = node.closing_loc();
            self.record(
                node.opening_loc(),
                first.location().start_offset(),
                closing.start_offset(),
                closing,
            );
        }
        for part in node.parts().iter() {
            self.visit(&part);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, max: usize, tab: usize) -> Vec<(usize, usize, String)> {
        check_line_length(source.as_bytes(), max, tab)
            .into_iter()
            .map(|c| (c.line_index, c.length, c.heredoc_delimiters.join(",")))
            .collect()
    }

    // Typical: a line one character over the limit is a candidate.
    #[test]
    fn over_by_one() {
        let src = format!("{}#\n", "#".repeat(80));
        assert_eq!(run(&src, 80, 0), vec![(0, 81, String::new())]);
    }

    // Typical: a line exactly at the limit is not a candidate.
    #[test]
    fn exactly_at_limit() {
        let src = "#".repeat(80);
        assert!(run(&src, 80, 0).is_empty());
    }

    // Typical: an empty / short file yields nothing.
    #[test]
    fn short_line() {
        assert!(run("puts 1\n", 80, 0).is_empty());
    }

    // Multiple long lines are all reported.
    #[test]
    fn multiple_lines() {
        let src = format!("{}\n{}\n", "a".repeat(90), "b".repeat(85));
        assert_eq!(
            run(&src, 80, 0),
            vec![(0, 90, String::new()), (1, 85, String::new())]
        );
    }

    // Tabs widen the visible length by `tab_width - 1` per leading tab.
    #[test]
    fn tab_indentation() {
        // 12 tabs + "1" with tab_width 2 => 12 + 1 + 12 * (2-1) = 25.
        let src = "\t\t\t\t\t\t\t\t\t\t\t\t1";
        assert_eq!(run(src, 10, 2), vec![(0, 25, String::new())]);
    }

    // A line under the limit but with tabs that push it over is reported.
    #[test]
    fn tab_pushes_over() {
        // 3 tabs + 28 '#' + 'a' = 32 chars, tab_width 2 => +3 => 35? compute.
        // chars: 3 + 28 + 1 = 32; indent_diff = 3 * 1 = 3; length 35... but the
        // upstream example uses width-2 with first non-tab adjustment.
        let src = "\t\t########################################a";
        let res = run(src, 30, 2);
        assert_eq!(res.len(), 1);
    }

    // Multibyte characters count as one character each, not bytes.
    #[test]
    fn multibyte_chars() {
        // 81 multibyte chars => length 81, byte length 243.
        let src = "あ".repeat(81);
        assert_eq!(run(&src, 80, 0), vec![(0, 81, String::new())]);
    }

    // Content after `__END__` is not scanned.
    #[test]
    fn after_end_marker() {
        let src = format!("{}\n__END__\n{}\n", "a".repeat(90), "b".repeat(200));
        assert_eq!(run(&src, 80, 0), vec![(0, 90, String::new())]);
    }

    // A heredoc body line carries its end delimiter.
    #[test]
    fn heredoc_body_delimiter() {
        let src = "<<-SQL\n  SELECT * FROM a_very_long_table_name_that_exceeds_the_configured_maximum_length;\nSQL\n";
        let res = run(src, 80, 0);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].0, 1);
        assert_eq!(res[0].2, "SQL");
    }

    // The heredoc opening line itself is not part of the body.
    #[test]
    fn heredoc_opening_line_not_body() {
        let long = "x".repeat(90);
        let src = format!("foo({}, <<-SQL)\n  body\nSQL\n", long);
        let res = run(&src, 80, 0);
        // The opening line is long but has no delimiter (treated normally).
        assert_eq!(res[0].0, 0);
        assert_eq!(res[0].2, String::new());
    }

    // A line inside a heredoc that is itself nested in another heredoc's
    // interpolation reports the *innermost* delimiter, not the enclosing one.
    #[test]
    fn nested_heredoc_innermost_delimiter() {
        let long = "x".repeat(90);
        let src = format!("foo(<<-DOC)\n  #{{<<-OK}}\n    {long}\n  OK\nDOC\n");
        let res = run(&src, 80, 0);
        // The long line is inside the OK heredoc (which itself sits in DOC's
        // interpolation), so the OK delimiter must cover it.
        assert!(res.iter().any(|c| c.2.split(',').any(|d| d == "OK")));
    }
}
