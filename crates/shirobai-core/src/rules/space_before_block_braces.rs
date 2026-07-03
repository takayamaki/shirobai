//! `Layout/SpaceBeforeBlockBraces`.
//!
//! Checks the space to the left of a block's `{`, per `EnforcedStyle`
//! (`space` / `no_space`) and `EnforcedStyleForEmptyBraces`.
//!
//! Stock's `on_block` (plus the `numblock` / `itblock` aliases, and lambda
//! literals which parser also types as `block`) needs only byte facts around
//! the `{`:
//!
//! - `node.keywords?` — the block is `do ... end` (skipped);
//! - `range_with_surrounding_space(left_brace)` — the `[ \t]`-then-`\n` run
//!   before the `{` (the right extension is never read);
//! - `empty_braces?` — `{` and `}` are adjacent bytes (`{ }` is *not* empty
//!   for this cop);
//! - `node.multiline?` — `RuboCop::AST::BlockNode` overrides `single_line?`
//!   to compare the *brace* lines (`loc.begin.line == loc.end.line`), not the
//!   whole node range, so no ancestor (call start) is needed.
//!
//! Everything else — the used-vs-configured style comparison, the
//! `Style/BlockDelimiters` conflict skip, the `EnforcedStyleForEmptyBraces`
//! validation raise and the `config_to_allow_offenses` state machine (which
//! the vendor spec asserts) — is per-record Ruby state, so the rule is
//! config-free: it reports every brace block's `(left_start, left_end,
//! space_begin, empty, multiline)` in walk order and the wrapper replays
//! stock's `check_empty` / `check_non_empty` verbatim.

use ruby_prism::Node;

use super::line_index::LineIndex;
use super::space_scan::surrounding_space_left;

/// One brace block, in walk (document) order.
pub struct SpaceBeforeBlockBracesRecord {
    /// The `{` byte range.
    pub left_start: usize,
    pub left_end: usize,
    /// Begin of `range_with_surrounding_space(left_brace)`: the `[ \t]` run
    /// then the `\n` run before the `{`. `space_begin < left_start` iff the
    /// used style is `space`.
    pub space_begin: usize,
    /// `{` and `}` are adjacent (`empty_braces?`).
    pub empty: bool,
    /// The braces sit on different lines (`BlockNode#multiline?`).
    pub multiline: bool,
}

pub fn check_space_before_block_braces(source: &[u8]) -> Vec<SpaceBeforeBlockBracesRecord> {
    let mut rule = build_rule(source);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.records
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8]) -> Visitor<'_> {
    Visitor {
        source,
        line_index: LineIndex::new(source),
        records: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: LineIndex,
    pub(crate) records: Vec<SpaceBeforeBlockBracesRecord>,
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        // Parser's `on_block` fires for method-call blocks, numbered / `it`
        // blocks and lambda literals; prism splits those into `BlockNode` and
        // `LambdaNode`.
        let (opening, closing) = if let Some(block) = node.as_block_node() {
            (block.opening_loc(), block.closing_loc())
        } else if let Some(lambda) = node.as_lambda_node() {
            (lambda.opening_loc(), lambda.closing_loc())
        } else if let Some(zsuper) = node.as_forwarding_super_node() {
            // The generated walker reaches a bare super's block through the
            // concretely-typed `block` field, bypassing the branch hooks (the
            // RescueNode trap), so the BlockNode never arrives here on its
            // own. Process it at the super node, which sits right before the
            // block in document order.
            match zsuper.block() {
                Some(block) => (block.opening_loc(), block.closing_loc()),
                None => return,
            }
        } else {
            return;
        };
        // `node.keywords?`: `do ... end` blocks are skipped.
        if self.source.get(opening.start_offset()) != Some(&b'{') {
            return;
        }
        let left_start = opening.start_offset();
        let left_end = opening.end_offset();
        let right_start = closing.start_offset();
        self.records.push(SpaceBeforeBlockBracesRecord {
            left_start,
            left_end,
            space_begin: surrounding_space_left(self.source, left_start),
            empty: left_end == right_start,
            multiline: self.line_index.line_of(left_start) != self.line_index.line_of(right_start),
        });
    }

    fn leave(&mut self) {}

    fn interest(&self) -> super::dispatch::Interest {
        // Pure BlockNode / LambdaNode kind match with an empty fall-through;
        // `leave` / leaf / rescue unused.
        super::dispatch::Interest(
            super::dispatch::Interest::ENTER_BLOCK
                | super::dispatch::Interest::ENTER_LAMBDA
                | super::dispatch::Interest::ENTER_SUPER,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<(usize, usize, usize, bool, bool)> {
        check_space_before_block_braces(source.as_bytes())
            .into_iter()
            .map(|r| {
                (
                    r.left_start,
                    r.left_end,
                    r.space_begin,
                    r.empty,
                    r.multiline,
                )
            })
            .collect()
    }

    #[test]
    fn spaced_and_unspaced_blocks() {
        assert_eq!(
            run("each { puts }\neach{ puts }\n"),
            vec![(5, 6, 4, false, false), (18, 19, 18, false, false)]
        );
    }

    #[test]
    fn empty_braces() {
        assert_eq!(run("7.times {}\n"), vec![(8, 9, 7, true, false)]);
        // `{ }` is not "empty" for this cop.
        assert_eq!(run("7.times { }\n"), vec![(8, 9, 7, false, false)]);
    }

    #[test]
    fn multiline_is_brace_based() {
        // The call spans two lines but the braces are on one: not multiline.
        assert_eq!(
            run("foo.map(a,\n        b) { |x| x }\n"),
            vec![(22, 23, 21, false, false)]
        );
        assert_eq!(run("foo.bar { |x|\n  x\n}\n"), vec![(8, 9, 7, false, true)]);
    }

    #[test]
    fn bare_super_block_braces() {
        // The BlockNode of a bare `super` hides behind ForwardingSuperNode's
        // typed field; the record must still appear.
        assert_eq!(run("super{ 1 }\n"), vec![(5, 6, 5, false, false)]);
    }

    #[test]
    fn lambda_braces() {
        assert_eq!(
            run("->() { }\n->{}\n"),
            vec![(5, 6, 4, false, false), (11, 12, 11, true, false)]
        );
    }

    #[test]
    fn newline_before_brace_counts_as_space() {
        // Continuation line: the surrounding-space run stops at the `\`.
        assert_eq!(
            run("foo.bar \\\n  { |x| x }\n"),
            vec![(12, 13, 9, false, false)]
        );
    }

    #[test]
    fn do_end_and_hashes_are_skipped() {
        assert!(run("x.each do |n| n end\nh = {a: 1}\n").is_empty());
    }
}
