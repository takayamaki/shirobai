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
//! The offense decision runs here (an earlier version shipped one record per
//! brace block and decided in Ruby, which made the wire volume scale with the
//! number of blocks). What crosses the wire is offense-count sized: the
//! offense list plus a five-flag [`Summary`] from which the wrapper replays
//! stock's `config_to_allow_offenses` bookkeeping exactly (see the summary
//! docs — the state machine's outcome depends only on these flags because
//! `no_acceptable_style!` replaces the whole hash and freezes every later
//! event, and all non-freezing writes are single-valued).
//!
//! The `EnforcedStyleForEmptyBraces` validation raise stays in the wrapper
//! (stock raises lazily from `check_empty`): with an invalid value the rule
//! skips the empty-brace decisions and the wrapper raises when
//! [`Summary::saw_empty`] is set.

use ruby_prism::Node;

use super::line_index::LineIndex;
use super::space_scan::surrounding_space_left;

/// `EnforcedStyle` value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Space,
    NoSpace,
}

/// Resolved `EnforcedStyleForEmptyBraces` (`nil` falls back to
/// `EnforcedStyle` on the Ruby side; anything unknown is `Invalid` and only
/// matters when an empty-brace block is met).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EmptyStyle {
    Space,
    NoSpace,
    Invalid,
}

/// Config for `Layout/SpaceBeforeBlockBraces`.
#[derive(Clone, Copy)]
pub struct Config {
    pub style: Style,
    pub empty_style: EmptyStyle,
    /// `Style/BlockDelimiters`' `EnforcedStyle == 'line_count_based'` (the
    /// multiline `no_space` conflict skip reads the other cop's config).
    pub bd_line_count_based: bool,
}

/// One offense. `[start, end)` is the reported range (also the autocorrect
/// anchor: whitespace is removed, anything else gets a space inserted
/// before). `detected` picks the message (`DETECTED_MSG` vs `MISSING_MSG`).
/// `from_empty` marks the empty-braces axis: stock calls
/// `opposite_style_detected` inside the offense block only for non-empty
/// braces, so the wrapper mirrors that per offense.
pub struct SpaceBeforeBlockBracesOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub detected: bool,
    pub from_empty: bool,
}

/// The facts the wrapper needs to replay stock's style-detection state
/// byte-identically:
///
/// - `a_correct` — some non-empty block matched the configured style
///   (`correct_style_detected`). The opposite detections ride on the
///   non-empty offenses themselves (inside their `add_offense` blocks, so
///   directive-disabled lines suppress them exactly like stock).
/// - `b_match_first` / `b_match_after` — an empty block matching
///   `style_for_empty_braces` occurred before the first / after some
///   empty-brace offense (`handle_different_styles_for_empty_braces` clears
///   the config when a conflicting value was already recorded, so the order
///   relative to the first write matters; every write stores the same value,
///   so one bit per side is enough).
/// - `b_offense` — some empty block offended (`check_empty` writes
///   `config_to_allow_offenses['EnforcedStyleForEmptyBraces']` before its
///   `add_offense`, unconditionally on the offense path).
/// - `saw_empty` — some empty-brace block reached `check_empty` (the lazy
///   `Unknown EnforcedStyleForEmptyBraces selected!` raise point).
#[derive(Clone, Copy, Default)]
pub struct Summary {
    pub a_correct: bool,
    pub b_match_first: bool,
    pub b_offense: bool,
    pub b_match_after: bool,
    pub saw_empty: bool,
}

/// Offenses in document order plus the style-detection summary.
pub struct SpaceBeforeBlockBracesResult {
    pub offenses: Vec<SpaceBeforeBlockBracesOffense>,
    pub summary: Summary,
}

pub fn check_space_before_block_braces(
    source: &[u8],
    config: Config,
) -> SpaceBeforeBlockBracesResult {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.into_result()
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    Visitor {
        source,
        config,
        line_index: LineIndex::new(source),
        offenses: Vec::new(),
        summary: Summary::default(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    config: Config,
    line_index: LineIndex,
    offenses: Vec<SpaceBeforeBlockBracesOffense>,
    summary: Summary,
}

impl Visitor<'_> {
    pub(crate) fn into_result(self) -> SpaceBeforeBlockBracesResult {
        SpaceBeforeBlockBracesResult {
            offenses: self.offenses,
            summary: self.summary,
        }
    }

    fn push(&mut self, start: usize, end: usize, detected: bool, from_empty: bool) {
        self.offenses.push(SpaceBeforeBlockBracesOffense {
            start_offset: start,
            end_offset: end,
            detected,
            from_empty,
        });
    }

    /// Stock's `on_block` body for one brace block.
    fn on_block(&mut self, left_start: usize, left_end: usize, right_start: usize) {
        let multiline = self.line_index.line_of(left_start) != self.line_index.line_of(right_start);
        // conflict_with_block_delimiters?
        if self.config.bd_line_count_based && self.config.style == Style::NoSpace && multiline {
            return;
        }
        let space_begin = surrounding_space_left(self.source, left_start);
        let used_space = space_begin < left_start;

        if left_end == right_start {
            // check_empty.
            self.summary.saw_empty = true;
            let empty_space = match self.config.empty_style {
                EmptyStyle::Space => true,
                EmptyStyle::NoSpace => false,
                // The wrapper raises on `saw_empty`; nothing after the raise
                // point is observable.
                EmptyStyle::Invalid => return,
            };
            if empty_space == used_space {
                // handle_different_styles_for_empty_braces: order relative to
                // the first empty-brace write decides whether it clears.
                if self.summary.b_offense {
                    self.summary.b_match_after = true;
                } else {
                    self.summary.b_match_first = true;
                }
                return;
            }
            self.summary.b_offense = true;
            if empty_space {
                self.push(left_start, left_end, false, true);
            } else {
                self.push(space_begin, left_start, true, true);
            }
        } else {
            // check_non_empty.
            if used_space == (self.config.style == Style::Space) {
                self.summary.a_correct = true;
            } else if used_space {
                self.push(space_begin, left_start, true, false);
            } else {
                self.push(left_start, left_end, false, false);
            }
        }
    }
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
        self.on_block(
            opening.start_offset(),
            opening.end_offset(),
            closing.start_offset(),
        );
    }

    fn leave(&mut self) {}

    fn interest(&self) -> super::dispatch::Interest {
        // Pure BlockNode / LambdaNode / ForwardingSuperNode kind match with an
        // empty fall-through; `leave` / leaf / rescue unused.
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

    const DEFAULT: Config = Config {
        style: Style::Space,
        empty_style: EmptyStyle::Space,
        bd_line_count_based: true,
    };

    fn cfg(style: Style, empty: EmptyStyle, bd: bool) -> Config {
        Config {
            style,
            empty_style: empty,
            bd_line_count_based: bd,
        }
    }

    fn run(source: &str, config: Config) -> Vec<(usize, usize, bool, bool)> {
        check_space_before_block_braces(source.as_bytes(), config)
            .offenses
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset, o.detected, o.from_empty))
            .collect()
    }

    fn summary(source: &str, config: Config) -> (bool, bool, bool, bool, bool) {
        let s = check_space_before_block_braces(source.as_bytes(), config).summary;
        (
            s.a_correct,
            s.b_match_first,
            s.b_offense,
            s.b_match_after,
            s.saw_empty,
        )
    }

    #[test]
    fn space_style_flags_missing() {
        assert_eq!(
            run("each { puts }\neach{ puts }\n", DEFAULT),
            vec![(18, 19, false, false)]
        );
        assert_eq!(
            summary("each { puts }\neach{ puts }\n", DEFAULT),
            (true, false, false, false, false)
        );
    }

    #[test]
    fn no_space_style_flags_detected() {
        let c = cfg(Style::NoSpace, EmptyStyle::NoSpace, true);
        assert_eq!(run("each { puts }\n", c), vec![(4, 5, true, false)]);
    }

    #[test]
    fn empty_braces_offense_and_match_order() {
        // Offense then match: b_match_after.
        let c = cfg(Style::Space, EmptyStyle::NoSpace, true);
        assert_eq!(run("7.times {}\n7.times{}\n", c), vec![(7, 8, true, true)]);
        assert_eq!(
            summary("7.times {}\n7.times{}\n", c),
            (false, false, true, true, true)
        );
        // Match then offense: b_match_first.
        assert_eq!(
            summary("7.times{}\n7.times {}\n", c),
            (false, true, true, false, true)
        );
    }

    #[test]
    fn empty_braces_missing_space() {
        let c = cfg(Style::NoSpace, EmptyStyle::Space, true);
        assert_eq!(run("->{}\n", c), vec![(2, 3, false, true)]);
    }

    #[test]
    fn invalid_empty_style_reports_saw_empty_only() {
        let c = cfg(Style::Space, EmptyStyle::Invalid, true);
        assert_eq!(run("each {}\n", c), vec![]);
        assert_eq!(summary("each {}\n", c), (false, false, false, false, true));
        // Non-empty blocks stay decided (stock only raises from check_empty).
        assert_eq!(run("each{ puts }\n", c), vec![(4, 5, false, false)]);
    }

    #[test]
    fn multiline_is_brace_based_and_conflict_skips() {
        let c = cfg(Style::NoSpace, EmptyStyle::NoSpace, true);
        // Two-line call, one-line braces: not multiline, fires.
        assert_eq!(
            run("foo.map(a,\n        b) { |x| x }\n", c),
            vec![(21, 22, true, false)]
        );
        // Multiline braces + line_count_based: skipped.
        assert_eq!(run("foo.bar { |x|\n  x\n}\n", c), vec![]);
        assert_eq!(
            summary("foo.bar { |x|\n  x\n}\n", c),
            (false, false, false, false, false)
        );
        // Non-line_count_based: fires.
        let c2 = cfg(Style::NoSpace, EmptyStyle::NoSpace, false);
        assert_eq!(
            run("foo.bar { |x|\n  x\n}\n", c2),
            vec![(7, 8, true, false)]
        );
    }

    #[test]
    fn newline_before_brace_counts_as_space() {
        // Continuation line: the surrounding-space run stops at the `\`.
        let c = cfg(Style::NoSpace, EmptyStyle::NoSpace, false);
        assert_eq!(
            run("foo.bar \\\n  { |x| x }\n", c),
            vec![(9, 12, true, false)]
        );
    }

    #[test]
    fn bare_super_block_braces() {
        // The BlockNode of a bare `super` hides behind ForwardingSuperNode's
        // typed field; the check must still run.
        assert_eq!(run("super{ 1 }\n", DEFAULT), vec![(5, 6, false, false)]);
    }

    #[test]
    fn do_end_and_hashes_are_skipped() {
        assert!(run("x.each do |n| n end\nh = {a: 1}\n", DEFAULT).is_empty());
    }
}
