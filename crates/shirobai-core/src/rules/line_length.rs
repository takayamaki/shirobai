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

use ruby_prism::{Node, Visit};

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
    /// Heredoc end delimiter (e.g. `SQL`) if this line sits inside a heredoc
    /// body, otherwise empty. Used by Ruby for the `AllowHeredoc` exemption.
    pub heredoc_delimiter: String,
}

/// Walk every line and return those whose visible length exceeds `max`.
///
/// `tab_width` is `tab_indentation_width` (`Layout/IndentationStyle`'s
/// `IndentationWidth` or the configured indentation width); `0` disables the
/// tab adjustment, matching `indentation_difference` returning `0` when
/// `tab_indentation_width` is falsey.
pub fn check_line_length(source: &[u8], max: usize, tab_width: usize) -> Vec<LineLengthCandidate> {
    let _ = (source, max, tab_width);
    Vec::new()
}

#[allow(dead_code)]
struct HeredocVisitor<'a> {
    source: &'a [u8],
    ranges: Vec<(usize, usize, String)>,
}

impl<'pr> Visit<'pr> for HeredocVisitor<'_> {
    fn visit_branch_node_enter(&mut self, _node: Node<'pr>) {}
}
