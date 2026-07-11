//! `Layout/SpaceInsideStringInterpolation`.
//!
//! Stock
//! (`vendor/rubocop/lib/rubocop/cop/layout/space_inside_string_interpolation.rb`)
//! is an `Interpolation` + `SurroundingSpace` cop: for every interpolation
//! `#{...}` (a parser `:begin` node) it takes `tokens_within(begin_node)` and
//! checks the space just inside the `#{` and `}` delimiters. That
//! `tokens_within` call materializes the parser-gem token stream (the "toucher"
//! cost). On prism the whole thing is byte work off the delimiter positions:
//! an interpolation is an `EmbeddedStatementsNode` whose `opening_loc` is `#{`
//! and `closing_loc` is `}`, and the checks reduce to the byte right after `#{`
//! (`left.space_after?`) and the byte right before `}` (`right.space_before?`).
//!
//! Behaviour reproduced exactly (probed against stock):
//! - `multiline?` (the `#{...}` spans more than one line) is skipped.
//! - `empty_brackets?` — no token between `#{` and `}` — is skipped. This is
//!   exactly "the bytes between the delimiters are all whitespace" (whitespace
//!   is not a token), so `#{}` and `#{ }` never offend.
//! - `no_space` (default): offend on space/tab right after `#{`
//!   (`SurroundingSpace#extra_space?` is `/[ \t]/`, NOT a newline), and on
//!   space/tab right before `}`. Offense range = the run of space/tab
//!   (`side_space_range`, which extends over `/[ \t]/` only). Autocorrect
//!   removes those runs.
//! - `space`: offend when there is NO space/tab after `#{` / before `}`; the
//!   offense range is the `#{` / `}` token itself (`side_space_range(:none)`).
//!   Autocorrect inserts one space, but gated on `space_after?` / `space_before?`
//!   (any `\s`), so the rare single-line `\r` case reports an offense that the
//!   autocorrect leaves in place — matching stock exactly.
//!
//! The wrapper adds one offense per emitted tuple and, on the FIRST offense of
//! each interpolation, applies that interpolation's edits (stock runs
//! `SpaceCorrector` once per node via `ignore_node`); later offenses of the same
//! interpolation carry no edits. Every offset goes through `SourceOffsets`.

use ruby_prism::{EmbeddedStatementsNode, Node};

/// `EnforcedStyle`: 0 = `no_space` (default), 1 = `space`.
pub const STYLE_NO_SPACE: u8 = 0;
pub const STYLE_SPACE: u8 = 1;

/// Message command: 0 = `NO_SPACE_COMMAND` ("Do not use"), 1 = `SPACE_COMMAND`
/// ("Use").
pub const CMD_NO_SPACE: u8 = 0;
pub const CMD_SPACE: u8 = 1;

#[derive(Clone, Copy)]
pub struct Config {
    pub style: u8,
}

/// One autocorrect edit. `insert` true => insert a single space at `start`
/// (`start == end`); false => remove the byte range `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub insert: bool,
}

/// One offense. `edits` is non-empty only on the FIRST offense of an
/// interpolation (stock applies `SpaceCorrector` once per node).
pub struct SpaceInsideInterpOffense {
    pub start: usize,
    pub end: usize,
    pub command: u8,
    pub edits: Vec<Edit>,
}

fn is_space_or_tab(b: Option<&u8>) -> bool {
    matches!(b, Some(b' ') | Some(b'\t'))
}

/// Ruby `\s`: space, tab, CR, LF, form feed, vertical tab.
fn is_ws(b: Option<&u8>) -> bool {
    matches!(b, Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n') | Some(&0x0c) | Some(&0x0b))
}

pub fn check_space_inside_string_interpolation(
    source: &[u8],
    config: Config,
) -> Vec<SpaceInsideInterpOffense> {
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
    pub(crate) offenses: Vec<SpaceInsideInterpOffense>,
}

impl Visitor<'_> {
    fn handle(&mut self, node: &EmbeddedStatementsNode<'_>) {
        let src = self.source;
        let l_start = node.opening_loc().start_offset();
        let l_end = node.opening_loc().end_offset();
        let r_start = node.closing_loc().start_offset();
        let r_end = node.closing_loc().end_offset();

        // `multiline?`: the `#{...}` spans more than one line.
        if src.get(l_start..r_end).is_some_and(|s| s.contains(&b'\n')) {
            return;
        }
        // `empty_brackets?`: no token between the delimiters (all whitespace).
        if src.get(l_end..r_start).is_some_and(|s| s.iter().all(|&b| is_ws(Some(&b)))) {
            return;
        }

        let space_after_left = is_space_or_tab(src.get(l_end));
        let space_before_right = r_start > 0 && is_space_or_tab(src.get(r_start - 1));

        let mut pending: Vec<SpaceInsideInterpOffense> = Vec::new();
        let mut edits: Vec<Edit> = Vec::new();

        if self.config.style == STYLE_SPACE {
            // `space_offenses`: offend when a side has no space/tab.
            if !space_after_left {
                pending.push(offense(l_start, l_end, CMD_SPACE));
            }
            if !space_before_right {
                pending.push(offense(r_start, r_end, CMD_SPACE));
            }
            // `add_space`: gated on `space_after?` / `space_before?` (any `\s`),
            // so a single-line `\r` neighbour reports but is not corrected.
            if !is_ws(src.get(l_end)) {
                edits.push(Edit { start: l_end, end: l_end, insert: true });
            }
            if r_start == 0 || !is_ws(src.get(r_start - 1)) {
                edits.push(Edit { start: r_start, end: r_start, insert: true });
            }
        } else {
            // `no_space_offenses`: offend on a space/tab just inside a delimiter.
            if space_after_left {
                let end = l_end + count_space_tab_forward(src, l_end);
                pending.push(offense(l_end, end, CMD_NO_SPACE));
            }
            if space_before_right {
                let start = r_start - count_space_tab_backward(src, r_start);
                pending.push(offense(start, r_start, CMD_NO_SPACE));
            }
            // `remove_space`: the space/tab runs (empty range for a bare `\r`,
            // so gating on space/tab matches stock's effective removal).
            if space_after_left {
                let end = l_end + count_space_tab_forward(src, l_end);
                edits.push(Edit { start: l_end, end, insert: false });
            }
            if space_before_right {
                let start = r_start - count_space_tab_backward(src, r_start);
                edits.push(Edit { start, end: r_start, insert: false });
            }
        }

        if pending.is_empty() {
            return;
        }
        // Attach every edit to the FIRST offense of this interpolation.
        pending[0].edits = edits;
        self.offenses.append(&mut pending);
    }
}

fn offense(start: usize, end: usize, command: u8) -> SpaceInsideInterpOffense {
    SpaceInsideInterpOffense { start, end, command, edits: Vec::new() }
}

/// Count consecutive space/tab bytes starting at `pos` (stock's `reposition`
/// rightward, `/[ \t]/` only).
fn count_space_tab_forward(src: &[u8], pos: usize) -> usize {
    let mut n = 0;
    while is_space_or_tab(src.get(pos + n)) {
        n += 1;
    }
    n
}

/// Count consecutive space/tab bytes ending just before `pos` (stock's
/// `reposition` leftward).
fn count_space_tab_backward(src: &[u8], pos: usize) -> usize {
    let mut n = 0;
    while pos > n && is_space_or_tab(src.get(pos - n - 1)) {
        n += 1;
    }
    n
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(Interest::ENTER_OTHER)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if let Some(n) = node.as_embedded_statements_node() {
            self.handle(&n);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, style: u8) -> Vec<(usize, usize, u8, Vec<Edit>)> {
        check_space_inside_string_interpolation(src.as_bytes(), Config { style })
            .into_iter()
            .map(|o| (o.start, o.end, o.command, o.edits))
            .collect()
    }

    fn rm(s: usize, e: usize) -> Edit {
        Edit { start: s, end: e, insert: false }
    }
    fn ins(p: usize) -> Edit {
        Edit { start: p, end: p, insert: true }
    }

    // no_space: `"#{ x }"` -> two offenses, both space runs; edits on the first.
    #[test]
    fn no_space_both_sides() {
        assert_eq!(
            run("\"#{ x }\"", STYLE_NO_SPACE),
            vec![(3, 4, CMD_NO_SPACE, vec![rm(3, 4), rm(5, 6)]), (5, 6, CMD_NO_SPACE, vec![])]
        );
    }

    // no_space clean: `"#{x}"`.
    #[test]
    fn no_space_clean() {
        assert!(run("\"#{x}\"", STYLE_NO_SPACE).is_empty());
    }

    // Empty / whitespace-only interpolation never offends.
    #[test]
    fn empty_interpolation() {
        assert!(run("\"#{}\"", STYLE_NO_SPACE).is_empty());
        assert!(run("\"#{ }\"", STYLE_NO_SPACE).is_empty());
        assert!(run("\"#{  }\"", STYLE_SPACE).is_empty());
    }

    // space: `"#{x}"` -> two "missing" offenses on the `#{` / `}` tokens.
    #[test]
    fn space_missing_both() {
        assert_eq!(
            run("\"#{x}\"", STYLE_SPACE),
            vec![(1, 3, CMD_SPACE, vec![ins(3), ins(4)]), (4, 5, CMD_SPACE, vec![])]
        );
    }

    // space clean: `"#{ x }"`.
    #[test]
    fn space_clean() {
        assert!(run("\"#{ x }\"", STYLE_SPACE).is_empty());
    }

    // no_space one side: `"#{ x}"`.
    #[test]
    fn no_space_left_only() {
        assert_eq!(run("\"#{ x}\"", STYLE_NO_SPACE), vec![(3, 4, CMD_NO_SPACE, vec![rm(3, 4)])]);
    }

    // space one side: `"#{x }"` -> only the left is missing.
    #[test]
    fn space_left_only() {
        assert_eq!(run("\"#{x }\"", STYLE_SPACE), vec![(1, 3, CMD_SPACE, vec![ins(3)])]);
    }

    // Tab counts as space.
    #[test]
    fn tab_counts() {
        assert_eq!(run("\"#{\tx}\"", STYLE_NO_SPACE), vec![(3, 4, CMD_NO_SPACE, vec![rm(3, 4)])]);
    }

    // Multiple spaces on each side: the whole run is one offense range.
    #[test]
    fn multiple_spaces() {
        assert_eq!(
            run("\"#{  x  }\"", STYLE_NO_SPACE),
            vec![(3, 5, CMD_NO_SPACE, vec![rm(3, 5), rm(6, 8)]), (6, 8, CMD_NO_SPACE, vec![])]
        );
    }

    // Two interpolations: the second (`#{y}`) is clean.
    #[test]
    fn multiple_interpolations() {
        assert_eq!(
            run("\"a#{ x }b#{y}\"", STYLE_NO_SPACE),
            vec![(4, 5, CMD_NO_SPACE, vec![rm(4, 5), rm(6, 7)]), (6, 7, CMD_NO_SPACE, vec![])]
        );
    }

    // Multiline interpolation is skipped.
    #[test]
    fn multiline_skipped() {
        assert!(run("\"#{\n x }\"", STYLE_NO_SPACE).is_empty());
    }

    // Symbol / regexp / xstr interpolations are handled the same way.
    #[test]
    fn other_interpolation_hosts() {
        assert_eq!(run(":\"#{ x }\"", STYLE_NO_SPACE).len(), 2);
        assert_eq!(run("/#{ x }/", STYLE_NO_SPACE).len(), 2);
    }
}
