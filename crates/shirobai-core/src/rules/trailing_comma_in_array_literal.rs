//! `Style/TrailingCommaInArrayLiteral`.
//!
//! Checks for a trailing comma after the last item of a square-bracketed
//! array literal. `EnforcedStyleForMultiline` decides whether a multi-line
//! literal *should* carry a trailing comma; a trailing comma in a single-line
//! literal is always an offense. All mixin logic lives in
//! [`trailing_comma`](super::trailing_comma) (shared with the hash-literal
//! and arguments cops).
//!
//! Trigger, mirroring stock's `on_array` + `check_literal`:
//!
//! - `node.square_brackets?`: the opening token must be exactly `[`. Percent
//!   arrays (`%w[…]` / `%i(…)` — their opening token is `%w[` / `%i(`) and
//!   implicit no-bracket arrays (`a = 1, 2`, `return 1, 2` — no opening at
//!   all) are excluded (probed on stock).
//! - empty literals (`[]`) are skipped (`node.children.empty?`).

use ruby_prism::Node;

use super::trailing_comma::LiteralChecker;
pub use super::trailing_comma::{Config, TrailingCommaOffense};

pub fn check_trailing_comma_in_array_literal(
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
    fn on_array(&mut self, array: &ruby_prism::ArrayNode<'_>) {
        // `node.square_brackets?`: opening token exactly `[`.
        let Some(opening) = array.opening_loc() else {
            return;
        };
        if opening.as_slice() != b"[" {
            return;
        }
        // A `[`-opened array always has a closing bracket.
        let Some(closing) = array.closing_loc() else {
            return;
        };
        let elements: Vec<Node<'_>> = array.elements().iter().collect();
        self.checker.check_literal(
            &elements,
            array.location().start_offset(),
            array.location().end_offset(),
            closing.start_offset(),
        );
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        // `ArrayNode` is in the dispatcher's LITERAL bucket; `enter` ignores
        // every other node in that bucket (string/symbol/regexp literals),
        // `leave` is empty and the leaf/rescue hooks are unused, so the
        // narrowing is exactly equivalent to `Interest::ALL`.
        Interest(Interest::ENTER_LITERAL)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if let Some(array) = node.as_array_node() {
            self.on_array(&array);
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
        check_trailing_comma_in_array_literal(source.as_bytes(), &Config { style })
            .iter()
            .map(|o| (o.start_offset, o.end_offset, o.message, o.fix))
            .collect()
    }

    #[test]
    fn no_comma_single_line_trailing() {
        // Probed: offense at [9, 10) (the comma).
        let src = "x = [1, 2,]\n";
        let r = run(src, STYLE_NO_COMMA);
        assert_eq!(r, vec![(9, 10, MSG_AVOID_NO_COMMA, FIX_AVOID)]);
    }

    #[test]
    fn avoid_message_tracks_style() {
        let src = "x = [1, 2,]\n";
        assert_eq!(run(src, STYLE_COMMA)[0].2, MSG_AVOID_COMMA);
        assert_eq!(run(src, STYLE_CONSISTENT_COMMA)[0].2, MSG_AVOID_CONSISTENT_COMMA);
        assert_eq!(run(src, STYLE_DIFF_COMMA)[0].2, MSG_AVOID_DIFF_COMMA);
    }

    #[test]
    fn percent_arrays_not_triggered() {
        // The opening token is `%w[` / `%i(`, not `[` (probed).
        assert!(run("x = %w[\n  a\n  b\n]\n", STYLE_CONSISTENT_COMMA).is_empty());
        assert!(run("x = %i(\n  a\n  b\n)\n", STYLE_CONSISTENT_COMMA).is_empty());
    }

    #[test]
    fn implicit_arrays_not_triggered() {
        // No brackets at all (probed).
        assert!(run("a = 1,\n  2\n", STYLE_CONSISTENT_COMMA).is_empty());
        assert!(run("def f\n  return 1,\n    2\nend\n", STYLE_CONSISTENT_COMMA).is_empty());
    }

    #[test]
    fn empty_literal_no_offense() {
        assert!(run("x = []\n", STYLE_NO_COMMA).is_empty());
        assert!(run("x = []\n", STYLE_CONSISTENT_COMMA).is_empty());
    }

    #[test]
    fn splat_last_item() {
        // Probed: put caret [8, 10) = `*a`; avoid at [10, 11).
        let r = run("x = [\n  *a\n]\n", STYLE_CONSISTENT_COMMA);
        assert_eq!(r, vec![(8, 10, MSG_PUT, FIX_PUT)]);
        let r = run("x = [\n  *a,\n]\n", STYLE_NO_COMMA);
        assert_eq!(r, vec![(10, 11, MSG_AVOID_NO_COMMA, FIX_AVOID)]);
        assert!(run("x = [\n  *a,\n]\n", STYLE_CONSISTENT_COMMA).is_empty());
    }

    #[test]
    fn heredoc_last_item() {
        // Probed: avoid at [19, 20) (the comma right after the marker); put
        // caret [13, 19) = `<<~EOS`.
        let r = run("x = [\n  1,\n  <<~EOS,\n    t\n  EOS\n]\n", STYLE_NO_COMMA);
        assert_eq!(r, vec![(19, 20, MSG_AVOID_NO_COMMA, FIX_AVOID)]);
        let r = run("x = [\n  1,\n  <<~EOS\n    t\n  EOS\n]\n", STYLE_CONSISTENT_COMMA);
        assert_eq!(r, vec![(13, 19, MSG_PUT, FIX_PUT)]);
    }

    #[test]
    fn csend_heredoc_body_comma_is_offense() {
        // Stock's `heredoc?` only treats `send_type?`, so a safe-navigation
        // call wrapping a heredoc is NOT heredoc-flagged and the comma inside
        // the body is an offense (probed at [19, 20)); the plain `.` version
        // IS heredoc-flagged -> no offense.
        let src = "x = [\n  a&.b(<<~X)\n, inside\n  X\n]\n";
        let r = run(src, STYLE_NO_COMMA);
        assert_eq!(r, vec![(19, 20, MSG_AVOID_NO_COMMA, FIX_AVOID)]);
        assert!(run("x = [\n  a.b(<<~X)\n, inside\n  X\n]\n", STYLE_NO_COMMA).is_empty());
    }

    #[test]
    fn single_element_closing_on_same_line_allowed() {
        assert!(run("x = [{\n  a: 1\n}]\n", STYLE_CONSISTENT_COMMA).is_empty());
    }

    #[test]
    fn two_per_line_styles() {
        // Probed: consistent wants a comma after `4` [19, 20); comma and
        // diff_comma want nothing.
        let src = "x = [1, 2,\n     3, 4]\n";
        assert!(run(src, STYLE_NO_COMMA).is_empty());
        assert!(run(src, STYLE_COMMA).is_empty());
        assert!(run(src, STYLE_DIFF_COMMA).is_empty());
        let r = run(src, STYLE_CONSISTENT_COMMA);
        assert_eq!(r, vec![(19, 20, MSG_PUT, FIX_PUT)]);
        // With the trailing comma present, consistent accepts; diff_comma
        // avoids (the item does not precede a newline).
        let src = "x = [1, 2,\n     3, 4,]\n";
        assert!(run(src, STYLE_CONSISTENT_COMMA).is_empty());
        let r = run(src, STYLE_DIFF_COMMA);
        assert_eq!(r, vec![(20, 21, MSG_AVOID_DIFF_COMMA, FIX_AVOID)]);
    }

    #[test]
    fn nested_arrays_fire_outer_first() {
        // Probed: stock reports the outer offense first (walk enter order).
        let src = "x = [[1, 2,], [3],]\n";
        let r = run(src, STYLE_NO_COMMA);
        assert_eq!(
            r,
            vec![
                (17, 18, MSG_AVOID_NO_COMMA, FIX_AVOID),
                (10, 11, MSG_AVOID_NO_COMMA, FIX_AVOID),
            ]
        );
    }

    #[test]
    fn no_offense_when_already_clean() {
        assert!(run("x = [1, 2, 3]\n", STYLE_NO_COMMA).is_empty());
        assert!(run("x = [\n  1,\n  2,\n]\n", STYLE_CONSISTENT_COMMA).is_empty());
    }
}
