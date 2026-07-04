//! `Style/TrailingCommaInHashLiteral`.
//!
//! Checks for a trailing comma after the last item of a braced hash literal.
//! `EnforcedStyleForMultiline` decides whether a multi-line literal *should*
//! carry a trailing comma; a trailing comma in a single-line literal is always
//! an offense. All mixin logic lives in
//! [`trailing_comma`](super::trailing_comma) (shared with the array-literal
//! and arguments cops).
//!
//! Trigger, mirroring stock's `on_hash` + `check_literal`:
//!
//! - stock skips a hash without `node.loc.end` — a braceless keyword hash,
//!   which "is the last parameter of a method call and will be checked as
//!   such" (by `Style/TrailingCommaInArguments`). Prism splits the two shapes
//!   by node type: a braced hash is a `HashNode` (braces always present), a
//!   braceless one is a `KeywordHashNode`. So the trigger is exactly
//!   `HashNode` (probed: `m(a: 1, b: 2,)` never fires this cop).
//! - empty literals (`{}`) are skipped (`node.children.empty?`).

use ruby_prism::Node;

use super::trailing_comma::LiteralChecker;
pub use super::trailing_comma::{Config, TrailingCommaOffense};

pub fn check_trailing_comma_in_hash_literal(
    source: &[u8],
    cfg: &Config,
) -> Vec<TrailingCommaOffense> {
    let mut rule = build_rule(source, cfg);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.checker.offenses
}

pub(crate) fn build_rule<'a>(source: &'a [u8], cfg: &Config) -> Visitor<'a> {
    Visitor {
        checker: LiteralChecker::new(source, *cfg),
    }
}

pub(crate) struct Visitor<'a> {
    pub checker: LiteralChecker<'a>,
}

impl Visitor<'_> {
    fn on_hash(&mut self, hash: &ruby_prism::HashNode<'_>) {
        let elements: Vec<Node<'_>> = hash.elements().iter().collect();
        // A prism `HashNode` always has braces, so stock's `brackets?` guard
        // is satisfied by construction.
        self.checker.check_literal(
            &elements,
            hash.location().start_offset(),
            hash.location().end_offset(),
            hash.closing_loc().start_offset(),
        );
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        // `HashNode` is in the dispatcher's OTHER bucket (not an explicit
        // class); `enter` ignores every other node, `leave` is empty and the
        // leaf/rescue hooks are unused, so the narrowing is exactly
        // equivalent to `Interest::ALL`.
        Interest(Interest::ENTER_OTHER)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if let Some(hash) = node.as_hash_node() {
            self.on_hash(&hash);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::super::trailing_comma::{
        FIX_AVOID, FIX_PUT, MSG_AVOID_COMMA, MSG_AVOID_CONSISTENT_COMMA, MSG_AVOID_DIFF_COMMA,
        MSG_AVOID_NO_COMMA, MSG_PUT, STYLE_COMMA, STYLE_CONSISTENT_COMMA, STYLE_DIFF_COMMA,
        STYLE_NO_COMMA,
    };
    use super::*;

    fn run(source: &str, style: u8) -> Vec<(usize, usize, u8, u8)> {
        check_trailing_comma_in_hash_literal(source.as_bytes(), &Config { style })
            .iter()
            .map(|o| (o.start_offset, o.end_offset, o.message, o.fix))
            .collect()
    }

    #[test]
    fn no_comma_single_line_trailing() {
        // Probed: offense at [16, 17) (the comma).
        let src = "h = { a: 1, b: 2, }\n";
        let r = run(src, STYLE_NO_COMMA);
        assert_eq!(r, vec![(16, 17, MSG_AVOID_NO_COMMA, FIX_AVOID)]);
    }

    #[test]
    fn avoid_message_tracks_style() {
        let src = "h = { a: 1, b: 2, }\n";
        assert_eq!(run(src, STYLE_COMMA)[0].2, MSG_AVOID_COMMA);
        assert_eq!(run(src, STYLE_CONSISTENT_COMMA)[0].2, MSG_AVOID_CONSISTENT_COMMA);
        assert_eq!(run(src, STYLE_DIFF_COMMA)[0].2, MSG_AVOID_DIFF_COMMA);
    }

    #[test]
    fn braceless_hash_not_triggered() {
        // A braceless keyword hash is `Style/TrailingCommaInArguments`'
        // business (probed: stock never fires the hash cop here).
        assert!(run("m(a: 1, b: 2,)\n", STYLE_NO_COMMA).is_empty());
        assert!(run("m(a: 1,\n  b: 2)\n", STYLE_CONSISTENT_COMMA).is_empty());
    }

    #[test]
    fn empty_literal_no_offense() {
        assert!(run("h = {}\n", STYLE_NO_COMMA).is_empty());
        assert!(run("h = {}\n", STYLE_CONSISTENT_COMMA).is_empty());
    }

    #[test]
    fn consistent_comma_put_on_multiline() {
        // Probed: caret [8, 11) = `**a`; kwsplat is a valid last item.
        let r = run("h = {\n  **a\n}\n", STYLE_CONSISTENT_COMMA);
        assert_eq!(r, vec![(8, 11, MSG_PUT, FIX_PUT)]);
    }

    #[test]
    fn kwsplat_trailing_comma_ok_under_consistent() {
        assert!(run("h = {\n  **a,\n}\n", STYLE_CONSISTENT_COMMA).is_empty());
        let r = run("h = {\n  **a,\n}\n", STYLE_NO_COMMA);
        assert_eq!(r, vec![(11, 12, MSG_AVOID_NO_COMMA, FIX_AVOID)]);
    }

    #[test]
    fn put_caret_is_last_line_of_multiline_value() {
        // Probed: `a: [\n    1\n  ]` puts the caret on the `]` [21, 22).
        let r = run("h = {\n  a: [\n    1\n  ]\n}\n", STYLE_CONSISTENT_COMMA);
        assert_eq!(r, vec![(21, 22, MSG_PUT, FIX_PUT)]);
    }

    #[test]
    fn single_pair_closing_on_value_line_allowed() {
        // The outer hash's `}` does not begin its line -> allowed multiline;
        // the inner hash's `}` does -> put on `b: 1` [13, 17) (probed).
        let r = run("h = { a: {\n  b: 1\n} }\n", STYLE_CONSISTENT_COMMA);
        assert_eq!(r, vec![(13, 17, MSG_PUT, FIX_PUT)]);
    }

    #[test]
    fn heredoc_value_comma_in_body_not_offense() {
        // The comma on the heredoc body's first line is body text; the
        // heredoc-aware regex must not cross the newline.
        assert!(run("h = {\n  a: b.c(<<~X)\n, inside\n  X\n}\n", STYLE_NO_COMMA).is_empty());
    }

    #[test]
    fn csend_heredoc_body_comma_is_offense() {
        // Stock's `heredoc?` only treats `send_type?`, so a safe-navigation
        // call wrapping a heredoc is NOT heredoc-flagged and the comma inside
        // the body is an offense (probed at [22, 23)).
        let src = "h = {\n  a: b&.c(<<~X)\n, inside\n  X\n}\n";
        let r = run(src, STYLE_NO_COMMA);
        assert_eq!(r, vec![(22, 23, MSG_AVOID_NO_COMMA, FIX_AVOID)]);
    }

    #[test]
    fn no_offense_when_already_clean() {
        assert!(run("h = { a: 1, b: 2 }\n", STYLE_NO_COMMA).is_empty());
        assert!(run("h = {\n  a: 1,\n  b: 2,\n}\n", STYLE_CONSISTENT_COMMA).is_empty());
    }
}
