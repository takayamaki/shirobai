//! `Layout/SpaceAroundEqualsInParameterDefault`.
//!
//! Stock
//! (`vendor/rubocop/lib/rubocop/cop/layout/space_around_equals_in_parameter_default.rb`):
//!
//! - `on_optarg` takes the first three tokens within the optarg node —
//!   `arg`, `equals`, `value` — and checks the space around the `=`:
//!   `space_on_both_sides?` = `arg.space_after? && equals.space_after?`,
//!   `no_surrounding_space?` = `!arg.space_after? && !equals.space_after?`.
//! - Offense unless the configured style is satisfied (`space` wants both
//!   sides, `no_space` wants neither). The offense range is
//!   `range_between(arg.end_pos, value.begin_pos)` — the `=` and its
//!   surrounding whitespace. Autocorrect replaces that range with ` = ` (space)
//!   or `=` (no_space) plus the `/=\s*(\S+)/` remainder.
//!
//! On prism this needs no token stream. An optarg is an `OptionalParameterNode`
//! whose `name_loc` / `operator_loc` / `value` give the three positions
//! directly (probed to match parser-gem's `tokens_within(node).take(3)`,
//! including signed-literal defaults like `-1` / `+1` whose value node begins at
//! the sign). `Token#space_after?` is `/\G\s/` at the token's end byte, so
//! `arg.space_after?` is "is `source[name_loc.end]` a `\s`?" and
//! `equals.space_after?` is "is `source[operator_loc.end]` a `\s`?".
//!
//! Rust emits, per offending optarg, the range `[name_loc.end, value.begin)`
//! (stock's offense range). The message type (`missing` vs `detected`) is fixed
//! by the style, and the autocorrect regex runs on the Ruby side over
//! `range.source`, so the wrapper reproduces stock's autocorrect byte for byte.
//! `on_kwoptarg` is intentionally NOT handled — stock only defines `on_optarg`,
//! so keyword defaults (`def f(a: 1)`) are a different cop's concern.

use ruby_prism::{Node, OptionalParameterNode};

/// `EnforcedStyle`: 0 = `space` (default), 1 = `no_space`.
pub const STYLE_SPACE: u8 = 0;
pub const STYLE_NO_SPACE: u8 = 1;

#[derive(Clone, Copy)]
pub struct Config {
    pub style: u8,
}

/// One offense: `range_between(arg.end_pos, value.begin_pos)`. The message and
/// autocorrect replacement are style-fixed, so the wrapper needs only the range.
pub struct SpaceAroundEqualsOffense {
    /// `arg.end_pos` (`name_loc` end).
    pub start: usize,
    /// `value.begin_pos` (the value node's start).
    pub end: usize,
}

pub fn check_space_around_equals_in_parameter_default(
    source: &[u8],
    config: Config,
) -> Vec<SpaceAroundEqualsOffense> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    Visitor {
        source,
        config,
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    config: Config,
    pub(crate) offenses: Vec<SpaceAroundEqualsOffense>,
}

impl Visitor<'_> {
    fn handle(&mut self, node: &OptionalParameterNode<'_>) {
        let name_end = node.name_loc().end_offset();
        let op_end = node.operator_loc().end_offset();
        let value_begin = node.value().location().start_offset();

        // `arg.space_after?` / `equals.space_after?`: `/\G\s/` at the byte after
        // the token (nil at EOF -> false).
        let space_before_eq = self.is_space_at(name_end);
        let space_after_eq = self.is_space_at(op_end);

        let satisfied = match self.config.style {
            STYLE_NO_SPACE => !space_before_eq && !space_after_eq,
            // STYLE_SPACE (default)
            _ => space_before_eq && space_after_eq,
        };
        if !satisfied {
            self.offenses.push(SpaceAroundEqualsOffense {
                start: name_end,
                end: value_begin,
            });
        }
    }

    fn is_space_at(&self, at: usize) -> bool {
        self.source
            .get(at)
            .is_some_and(|&b| super::duplicate_magic_comment::is_rb_space(b))
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(Interest::ENTER_OTHER)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if let Some(n) = node.as_optional_parameter_node() {
            self.handle(&n);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, style: u8) -> Vec<(usize, usize)> {
        check_space_around_equals_in_parameter_default(src.as_bytes(), Config { style })
            .into_iter()
            .map(|o| (o.start, o.end))
            .collect()
    }

    // Typical (space style): a default without space around `=` offends; the
    // range is `[arg.end, value.begin)`.
    #[test]
    fn space_style_missing() {
        // `def f(x, y=0, z= 1)`: y=0 -> [10,11), z= 1 -> [15,17).
        assert_eq!(
            run("def f(x, y=0, z= 1); end", STYLE_SPACE),
            vec![(10, 11), (15, 17)]
        );
    }

    // Space present on both sides is clean under `space`.
    #[test]
    fn space_style_ok() {
        assert!(run("def f(x, y = 0, z = {}); end", STYLE_SPACE).is_empty());
    }

    // Signed-literal default: the value node begins at the sign, matching
    // parser-gem's third token.
    #[test]
    fn space_style_signed_literals() {
        // `def f(x=-1, y= 0, z =+1)`: x=-1 -> [7,8), y= 0 -> [13,15), z =+1 -> [19,21).
        assert_eq!(
            run("def f(x=-1, y= 0, z =+1); end", STYLE_SPACE),
            vec![(7, 8), (13, 15), (19, 21)]
        );
    }

    // Unary + default with spaces is clean.
    #[test]
    fn space_style_unary_plus_ok() {
        assert!(run("def f(x, y = +1, z = {}); end", STYLE_SPACE).is_empty());
    }

    // no_space style: spaces around `=` offend.
    #[test]
    fn no_space_style_detected() {
        // `def f(x, y = 0, z =1, w= 2)`: y = 0 -> [10,13), z =1 -> [17,19), w= 2 -> [23,25).
        assert_eq!(
            run("def f(x, y = 0, z =1, w= 2); end", STYLE_NO_SPACE),
            vec![(10, 13), (17, 19), (23, 25)]
        );
    }

    // no_space style clean.
    #[test]
    fn no_space_style_ok() {
        assert!(run("def f(x, y=0, z={}); end", STYLE_NO_SPACE).is_empty());
    }

    // Empty-string / empty-array defaults (value node begins at the quote/`[`).
    #[test]
    fn space_style_empty_literals() {
        assert_eq!(run("def f(x, y=\"\"); end", STYLE_SPACE), vec![(10, 11)]);
        assert_eq!(run("def f(x, y=[]); end", STYLE_SPACE), vec![(10, 11)]);
    }

    // Keyword defaults are NOT this cop (stock has no `on_kwoptarg`).
    #[test]
    fn keyword_default_ignored() {
        assert!(run("def f(a: 1); end", STYLE_SPACE).is_empty());
        assert!(run("def f(a:1); end", STYLE_NO_SPACE).is_empty());
    }

    // Nested defs each checked; a block optarg too.
    #[test]
    fn nested_and_block_optargs() {
        // `->(y=0) { }` block optarg (space style) -> one offense.
        let r = run("->(y=0) { }", STYLE_SPACE);
        assert_eq!(r.len(), 1);
    }
}
