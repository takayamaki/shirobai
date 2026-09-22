//! `Style/DirectiveScope`.
//!
//! Stock's `on_new_investigation`
//! (`vendor/rubocop/lib/rubocop/cop/style/directive_scope.rb`):
//!
//! ```ruby
//! def on_new_investigation
//!   processed_source.comments.each do |comment|
//!     directive = DirectiveComment.new(comment)
//!     next unless comment_config.comment_only_line?(directive.line_number)
//!     ...
//!   end
//! end
//! ```
//!
//! Every comment in the file pays `DirectiveComment.new` (the
//! `DIRECTIVE_COMMENT_REGEXP` match) and `comment_config.comment_only_line?`,
//! and the first `comment_only_line?` call materializes the parser-gem token
//! stream (`processed_source.tokens`) — so the cop is a "toucher" on every file
//! with at least one comment. Only comments that match the directive regexp
//! (`#\s*rubocop\s*:\s*(disable|enable|todo|push|pop|...)`) can ever reach
//! `check_pair` / `check_push_pop` / `check_enable_pair`; for every other
//! comment the loop body is a no-op.
//!
//! The Rust side returns the `(index, line)` of every comment whose text
//! contains the bytes `rubocop` — a strict superset of the regexp matches
//! (the regexp is case-sensitive and requires that word). The wrapper runs
//! stock's loop body verbatim on those comments only, so a file without a
//! `rubocop` comment never touches tokens, and the directive logic
//! (`DirectiveComment`, `comment_config` ranges, statement scopes,
//! autocorrect) stays stock's own code — byte-identical by construction.
//!
//! `index` is the comment's position in the parse's comment list (document
//! order, the same list `processed_source.comments` is built from); `line` is
//! its 1-based start line, which the wrapper uses to verify the index and to
//! fall back to `comment_at_line` if the two comment lists ever disagree.

use super::line_index;
use super::parse_cache;

/// `(comment index, 1-based line)` of a comment containing `rubocop`.
pub type DirectiveCandidate = (usize, usize);

const MARKER: &[u8] = b"rubocop";

/// Whether `text` contains the bytes `rubocop` (case-sensitive, like stock's
/// `DIRECTIVE_COMMENT_REGEXP`).
pub(crate) fn contains_marker(text: &[u8]) -> bool {
    text.len() >= MARKER.len() && text.windows(MARKER.len()).any(|w| w == MARKER)
}

/// Every comment whose text contains `rubocop`, as `(index, line)`.
pub fn check_directive_scope(source: &[u8]) -> Vec<DirectiveCandidate> {
    let comments = parse_cache::comment_ranges(source);
    let mut out: Vec<DirectiveCandidate> = comments
        .iter()
        .enumerate()
        .filter(|&(_, &(start, end))| contains_marker(&source[start..end]))
        .map(|(index, &(start, _))| (index, start))
        .collect();
    if out.is_empty() {
        return out;
    }
    line_index::with_line_index(source, |li| {
        for candidate in &mut out {
            candidate.1 = li.line_of(candidate.1);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<DirectiveCandidate> {
        check_directive_scope(src.as_bytes())
    }

    // Typical: a disable/enable pair around one statement -> both comments
    // are candidates, with their indices and lines.
    #[test]
    fn disable_enable_pair() {
        let src =
            "# rubocop:disable Metrics/AbcSize\ndef foo\nend\n# rubocop:enable Metrics/AbcSize\n";
        assert_eq!(run(src), vec![(0, 1), (1, 4)]);
    }

    // Plain comments never become candidates: no `rubocop` anywhere.
    #[test]
    fn plain_comments_are_skipped() {
        assert_eq!(run("# hello\nputs 1 # trailing\n"), vec![]);
    }

    // Indices count every comment (plain ones too), so the wrapper can read
    // `processed_source.comments[index]` directly.
    #[test]
    fn index_counts_all_comments() {
        let src = "# doc\n# more\nx = 1 # rubocop:disable Style/Foo\n";
        assert_eq!(run(src), vec![(2, 3)]);
    }

    // A superset filter: any mention of `rubocop` is passed through (stock's
    // regexp rejects it later); `RuboCop` (different case) is not.
    #[test]
    fn marker_is_case_sensitive_substring() {
        assert_eq!(run("# see the rubocop docs\n# RuboCop\n"), vec![(0, 1)]);
    }

    // No comments, empty source.
    #[test]
    fn empty_source() {
        assert_eq!(run(""), vec![]);
        assert_eq!(run("puts 1\n"), vec![]);
    }

    // An `=begin` block comment is a comment in both parsers; the marker
    // search covers its whole text.
    #[test]
    fn embdoc_comment() {
        let src = "=begin\nrubocop:disable Foo\n=end\nputs 1\n";
        assert_eq!(run(src), vec![(0, 1)]);
    }
}
