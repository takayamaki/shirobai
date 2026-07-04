//! `Layout/SpaceInsideParens`.
//!
//! Stock iterates `sorted_tokens` pairwise and reacts to pairs where
//! `token1.left_parens?` (tLPAREN / tLPAREN2) or `token2.right_parens?`
//! (tRPAREN). Byte-side, every unmasked `(` / `)` byte is a paren token
//! (strings, comments, char literals and the `__END__` data segment are the
//! only non-token paren bytes), so the pair checks become the neighbor scans
//! around those bytes:
//!
//! - pair `(t1 = "(", t2)`: `t2` starts at the next token start after the
//!   `(` (whitespace and `\`-newline continuations are not tokens);
//! - pair `(t1, t2 = ")")`: `t1` ends at the previous same-line token end.
//!
//! The one token fact bytes cannot see is `tLPAREN_ARG` — the `(` of a
//! space-separated first argument in a parenless call (`f (3)`,
//! `raise (x)`, `yield (1)`, `super (1)`, `defined? (x)`, `not (x)`), which
//! is NOT `left_parens?` and so never fires the left-side checks (probed:
//! `f ( 3 )` under `no_space` only flags the space before `)`). Those
//! positions are collected from the AST during the shared walk: a
//! parenthesized first argument (or `not` receiver / `defined?` value) that
//! starts with `(` after a gap. `return (1)` / `break (1)` / `next (1)` /
//! `! (x)` lex a plain `tLPAREN` (probed) — their parens are ordinary.
//!
//! Style logic (probed):
//!
//! - `no_space`: a same-line whitespace run after a left paren or before a
//!   `)` is an offense (skipped when the next token is a comment). The
//!   empty-parens pair follows the same rule, so `f(\n)` is NOT flagged.
//! - `space`: a consecutive `( )`-pair whose inner text is not exactly `()`
//!   is flagged for removal EVEN across lines (`f(\n)` → `f()`); non-empty
//!   parens missing the inner space are flagged at the next token's first
//!   byte / at the `)`.
//! - `compact`: like `space`, except a `( (` / `) )` pair is not a
//!   missing-space subject; instead the pair is flagged for removal when the
//!   gap is EXACTLY one space (`" "` — a tab or two spaces pass).
//!
//! Offense ranges and messages mirror stock; the wrapper replays the two
//! corrector shapes (remove range / insert space before range).

use std::collections::HashSet;

use ruby_prism::Node;

use super::space_scan::{next_token_start, prev_token_end_same_line};

/// `EnforcedStyle` value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    NoSpace,
    Space,
    Compact,
}

/// Config for `Layout/SpaceInsideParens`.
#[derive(Clone, Copy)]
pub struct Config {
    pub style: Style,
}

/// One offense: `[start, end)` plus the message/fix selector.
pub struct SpaceInsideParensOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: MessageId,
}

/// The two fixed messages stock emits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MessageId {
    /// `'Space inside parentheses detected.'` — the range is removed.
    Detected,
    /// `'No space inside parentheses detected.'` — a space is inserted
    /// before the range.
    Missing,
}

impl MessageId {
    /// The numeric tag carried over the wire to the Ruby wrapper.
    pub fn code(self) -> u8 {
        match self {
            MessageId::Detected => 0,
            MessageId::Missing => 1,
        }
    }
}

pub fn check_space_inside_parens(source: &[u8], config: Config) -> Vec<SpaceInsideParensOffense> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.into_offenses()
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    let comments = super::parse_cache::comment_ranges(source);
    let data_start = super::parse_cache::data_start(source);
    Visitor {
        source,
        config,
        comments,
        data_start,
        masks: Vec::new(),
        arg_parens: HashSet::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    config: Config,
    comments: Vec<(usize, usize)>,
    data_start: Option<usize>,
    masks: Vec<(usize, usize)>,
    /// Positions of `(` bytes lexing as `tLPAREN_ARG` (not `left_parens?`).
    arg_parens: HashSet<usize>,
}

impl<'a> Visitor<'a> {
    /// Register `start` as a `tLPAREN_ARG` when the expression beginning
    /// there starts with `(` and sits after a gap from `prev_end`.
    fn collect_arg_paren(&mut self, prev_end: usize, start: usize) {
        if start > prev_end && self.source.get(start) == Some(&b'(') {
            self.arg_parens.insert(start);
        }
    }

    pub(crate) fn into_offenses(self) -> Vec<SpaceInsideParensOffense> {
        let Visitor {
            source,
            config,
            comments,
            data_start,
            masks,
            arg_parens,
        } = self;
        let style = config.style;
        let masks = super::opaque_mask::merge(masks, &comments, data_start, source.len());
        let mut out = Vec::new();
        let mut push = |s: usize, e: usize, message: MessageId| {
            out.push(SpaceInsideParensOffense {
                start_offset: s,
                end_offset: e,
                message,
            });
        };

        let scan_end = data_start.unwrap_or(source.len()).min(source.len());
        let mut mask_i = 0;
        for p in 0..scan_end {
            let b = source[p];
            if b != b'(' && b != b')' {
                continue;
            }
            while mask_i < masks.len() && masks[mask_i].1 <= p {
                mask_i += 1;
            }
            if mask_i < masks.len() && masks[mask_i].0 <= p {
                continue;
            }
            if b == b'(' {
                if arg_parens.contains(&p) {
                    // tLPAREN_ARG is not `left_parens?`: no left-side checks.
                    continue;
                }
                let (q, crossed) = next_token_start(source, p + 1);
                if q >= source.len() {
                    continue;
                }
                if source[q] == b')' {
                    // The consecutive `( )` pair: `no_space` treats it like
                    // any gap (same line only); `space` / `compact` flag any
                    // inner text that is not exactly `()`, across lines too.
                    let gap = q > p + 1;
                    match style {
                        Style::NoSpace => {
                            if gap && !crossed {
                                push(p + 1, q, MessageId::Detected);
                            }
                        }
                        Style::Space | Style::Compact => {
                            if gap {
                                push(p + 1, q, MessageId::Detected);
                            }
                        }
                    }
                    continue;
                }
                if crossed || source[q] == b'#' {
                    // A line break or a comment after the `(`: the inside
                    // rules do not apply.
                    continue;
                }
                match style {
                    Style::NoSpace => {
                        if q > p + 1 {
                            push(p + 1, q, MessageId::Detected);
                        }
                    }
                    Style::Space => {
                        if q == p + 1 {
                            push(q, q + 1, MessageId::Missing);
                        }
                    }
                    Style::Compact => {
                        if source[q] == b'(' && !arg_parens.contains(&q) {
                            // A `( (` pair: flagged only when the gap is
                            // exactly one space.
                            if q == p + 2 && source[p + 1] == b' ' {
                                push(p + 1, q, MessageId::Detected);
                            }
                        } else if q == p + 1 {
                            push(q, q + 1, MessageId::Missing);
                        }
                    }
                }
            } else {
                // b == b')'
                let Some(m) = prev_token_end_same_line(source, p) else {
                    continue;
                };
                if m == 0 {
                    continue;
                }
                let prev_is_paren_byte =
                    |ch: u8| m >= 1 && source[m - 1] == ch && !super::opaque_mask::contains(&masks, m - 1);
                if prev_is_paren_byte(b'(') {
                    if arg_parens.contains(&(m - 1)) {
                        // Pair (tLPAREN_ARG, tRPAREN): `left_parens?` is
                        // false, so the empty-parens special never runs;
                        // only `no_space`'s generic gap check fires.
                        if style == Style::NoSpace && p > m {
                            push(m, p, MessageId::Detected);
                        }
                    }
                    // A real left paren: the pair was fully handled from the
                    // `(` side.
                    continue;
                }
                match style {
                    Style::NoSpace => {
                        if p > m {
                            push(m, p, MessageId::Detected);
                        }
                    }
                    Style::Space => {
                        if p == m {
                            push(p, p + 1, MessageId::Missing);
                        }
                    }
                    Style::Compact => {
                        if prev_is_paren_byte(b')') {
                            // A `) )` pair: flagged only when the gap is
                            // exactly one space.
                            if p == m + 1 && source[m] == b' ' {
                                push(m, p, MessageId::Detected);
                            }
                        } else if p == m {
                            push(p, p + 1, MessageId::Missing);
                        }
                    }
                }
            }
        }
        out
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        match node {
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if call.opening_loc().is_some() {
                    return;
                }
                let Some(message) = call.message_loc() else {
                    return;
                };
                // `f (3)` / `raise (x)` / `x.f (3)`: the first argument of a
                // parenless call whose selector is a plain name. Operators
                // and attribute writes put the lexer in a state that yields
                // a plain `tLPAREN` instead.
                let selector_ok = message
                    .as_slice()
                    .first()
                    .is_some_and(|&c| c.is_ascii_alphabetic() || c == b'_' || !c.is_ascii());
                if selector_ok
                    && !call.is_attribute_write()
                    && let Some(args) = call.arguments()
                    && let Some(first) = args.arguments().iter().next()
                {
                    self.collect_arg_paren(message.end_offset(), first.location().start_offset());
                    return;
                }
                // `not (x)`: the receiver form of the keyword.
                if call.arguments().is_none()
                    && message.as_slice() == b"not"
                    && let Some(recv) = call.receiver()
                {
                    self.collect_arg_paren(message.end_offset(), recv.location().start_offset());
                }
            }
            Node::YieldNode { .. } => {
                let n = node.as_yield_node().unwrap();
                if n.lparen_loc().is_none()
                    && let Some(args) = n.arguments()
                    && let Some(first) = args.arguments().iter().next()
                {
                    self.collect_arg_paren(
                        n.keyword_loc().end_offset(),
                        first.location().start_offset(),
                    );
                }
            }
            Node::SuperNode { .. } => {
                let n = node.as_super_node().unwrap();
                if n.lparen_loc().is_none()
                    && let Some(args) = n.arguments()
                    && let Some(first) = args.arguments().iter().next()
                {
                    self.collect_arg_paren(
                        n.keyword_loc().end_offset(),
                        first.location().start_offset(),
                    );
                }
            }
            Node::DefinedNode { .. } => {
                let n = node.as_defined_node().unwrap();
                if n.lparen_loc().is_none() {
                    self.collect_arg_paren(
                        n.keyword_loc().end_offset(),
                        n.value().location().start_offset(),
                    );
                }
            }
            _ => super::opaque_mask::collect_enter(node, &mut self.masks),
        }
    }

    fn leave(&mut self) {}

    fn enter_leaf(&mut self, node: &Node<'_>) {
        super::opaque_mask::collect_leaf(node, &mut self.masks);
    }

    fn interest(&self) -> super::dispatch::Interest {
        // `enter` reads CallNode (ARG parens + `not`), YieldNode / DefinedNode
        // (OTHER), SuperNode (SUPER) and the opaque-mask branch nodes
        // (ISTRING / LITERAL / WRITE / OTHER); `enter_leaf` masks the leaf
        // literals.
        super::dispatch::Interest(
            super::dispatch::Interest::LEAF
                | super::dispatch::Interest::ENTER_CALL
                | super::dispatch::Interest::ENTER_SUPER
                | super::dispatch::Interest::ENTER_ISTRING
                | super::dispatch::Interest::ENTER_LITERAL
                | super::dispatch::Interest::ENTER_WRITE
                | super::dispatch::Interest::ENTER_OTHER,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: Style) -> Vec<(usize, usize, u8)> {
        check_space_inside_parens(source.as_bytes(), Config { style })
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset, o.message.code()))
            .collect()
    }

    #[test]
    fn no_space_flags_inner_spaces() {
        assert_eq!(run("f( 3)\n", Style::NoSpace), vec![(2, 3, 0)]);
        assert_eq!(run("f(3 )\n", Style::NoSpace), vec![(3, 4, 0)]);
        assert_eq!(run("f( 3 )\n", Style::NoSpace), vec![(2, 3, 0), (4, 5, 0)]);
        assert_eq!(run("f(3\t)\n", Style::NoSpace), vec![(3, 4, 0)]);
        assert!(run("f(3)\n", Style::NoSpace).is_empty());
    }

    #[test]
    fn no_space_empty_parens() {
        assert_eq!(run("f( )\n", Style::NoSpace), vec![(2, 3, 0)]);
        assert!(run("f()\n", Style::NoSpace).is_empty());
        // Across a line break the pair is not same-line: nothing.
        assert!(run("f(\n)\n", Style::NoSpace).is_empty());
        assert!(run("f(\\\n)\n", Style::NoSpace).is_empty());
    }

    #[test]
    fn no_space_skips_comments_and_line_breaks() {
        assert!(run("f( # c\n3)\n", Style::NoSpace).is_empty());
        assert!(run("f(3 # c\n)\n", Style::NoSpace).is_empty());
        assert!(run("f( \n3)\n", Style::NoSpace).is_empty());
        assert!(run("x = ( \na)\n", Style::NoSpace).is_empty());
        // Right side on a later line with same-line prev token still fires.
        assert_eq!(run("x = (a;\nb )\n", Style::NoSpace), vec![(9, 10, 0)]);
    }

    #[test]
    fn no_space_arg_paren_skips_left_side_only() {
        // `f ( 3 )`: the ARG `(` fires nothing; the `)` side still does.
        assert_eq!(run("f ( 3 )\n", Style::NoSpace), vec![(5, 6, 0)]);
        assert_eq!(run("f ( )\n", Style::NoSpace), vec![(3, 4, 0)]);
        assert_eq!(run("yield ( 3 )\n", Style::NoSpace), vec![(9, 10, 0)]);
        assert_eq!(run("not ( x )\n", Style::NoSpace), vec![(7, 8, 0)]);
        assert_eq!(run("x.f ( 3 )\n", Style::NoSpace), vec![(7, 8, 0)]);
        // `return (1)` lexes a plain tLPAREN: both sides fire.
        assert_eq!(
            run("return ( 3 )\n", Style::NoSpace),
            vec![(8, 9, 0), (10, 11, 0)]
        );
        assert_eq!(
            run("break ( 3 ) while x\n", Style::NoSpace),
            vec![(7, 8, 0), (9, 10, 0)]
        );
    }

    #[test]
    fn no_space_masked_parens_are_not_tokens() {
        assert!(run("x = ?)\n", Style::NoSpace).is_empty());
        assert!(run("x = \"( )\"\n", Style::NoSpace).is_empty());
        assert!(run("x = %w( a )\n", Style::NoSpace).is_empty());
        assert!(run("x = 1 # ( )\n", Style::NoSpace).is_empty());
        // A char-literal `?)` inside real parens: both sides fire.
        assert_eq!(
            run("x = ( ?) )\n", Style::NoSpace),
            vec![(5, 6, 0), (8, 9, 0)]
        );
        // `%w()`'s closer is a tSTRING_END, not a tRPAREN.
        assert_eq!(run("f(%w() )\n", Style::NoSpace), vec![(6, 7, 0)]);
    }

    #[test]
    fn space_style_flags_missing_and_empty() {
        assert_eq!(run("f(3)\n", Style::Space), vec![(2, 3, 1), (3, 4, 1)]);
        assert_eq!(run("f( 3)\n", Style::Space), vec![(4, 5, 1)]);
        assert_eq!(run("f(3 )\n", Style::Space), vec![(2, 3, 1)]);
        assert!(run("f( 3 )\n", Style::Space).is_empty());
        assert!(run("f()\n", Style::Space).is_empty());
        assert_eq!(run("f( )\n", Style::Space), vec![(2, 3, 0)]);
        // The empty pair is flagged across lines too.
        assert_eq!(run("f(\n)\n", Style::Space), vec![(2, 3, 0)]);
        assert_eq!(run("f(\\\n)\n", Style::Space), vec![(2, 4, 0)]);
        // Tabs count as the required space.
        assert!(run("f(\t3\t)\n", Style::Space).is_empty());
        assert_eq!(run("f(3\t)\n", Style::Space), vec![(2, 3, 1)]);
    }

    #[test]
    fn space_style_arg_parens_are_inert() {
        assert!(run("f ( 3 )\n", Style::Space).is_empty());
        assert_eq!(run("f (3)\n", Style::Space), vec![(4, 5, 1)]);
        assert!(run("f ( )\n", Style::Space).is_empty());
        assert!(run("f ()\n", Style::Space).is_empty());
    }

    #[test]
    fn space_style_nested_parens() {
        assert_eq!(
            run("g(( 3 ))\n", Style::Space),
            vec![(2, 3, 1), (7, 8, 1)]
        );
        assert!(run("g( ( 3 ) )\n", Style::Space).is_empty());
    }

    #[test]
    fn space_style_heredoc_and_comment() {
        assert_eq!(
            run("f(<<~EOS)\n  b\nEOS\n", Style::Space),
            vec![(2, 3, 1), (8, 9, 1)]
        );
        assert!(run("f( <<~EOS )\n  b\nEOS\n", Style::Space).is_empty());
        assert!(run("f( # c\n3 )\n", Style::Space).is_empty());
        assert!(run("f( 3 # c\n)\n", Style::Space).is_empty());
        assert!(run("f(\n  3\n)\n", Style::Space).is_empty());
    }

    #[test]
    fn compact_consecutive_parens() {
        // `) )` with exactly one space is collapsed.
        assert_eq!(run("g( f( x ) )\n", Style::Compact), vec![(9, 10, 0)]);
        assert!(run("g( f( x ))\n", Style::Compact).is_empty());
        assert!(run("g( f( x )  )\n", Style::Compact).is_empty());
        // `( (` with exactly one space too.
        assert_eq!(
            run("g( ( 3 + 5 ) * f )\n", Style::Compact),
            vec![(2, 3, 0)]
        );
        // Tight `((` asks for nothing; the lone right paren still needs its
        // space.
        assert_eq!(
            run("g(( 3 + 5 ) * f)\n", Style::Compact),
            vec![(15, 16, 1)]
        );
        // Two spaces between lefts pass; the `) )` pair is flagged.
        assert_eq!(run("g(  ( 3 ) )\n", Style::Compact), vec![(9, 10, 0)]);
        // Missing left space on a non-paren next token.
        assert_eq!(run("g(f( x ))\n", Style::Compact), vec![(2, 3, 1)]);
        // Across a line break the `) )` pair is untouched.
        assert!(run("g( f( x )\n)\n", Style::Compact).is_empty());
        // Empty parens behave like the space style.
        assert_eq!(run("f( )\n", Style::Compact), vec![(2, 3, 0)]);
        assert_eq!(run("f(\n)\n", Style::Compact), vec![(2, 3, 0)]);
        // An ARG `(` is not a left paren for the `( (` pair test.
        assert_eq!(run("g( f ( x ) )\n", Style::Compact), vec![(10, 11, 0)]);
    }

    #[test]
    fn interpolation_and_data_segment() {
        assert_eq!(
            run("x = \"#{f( 3 )}\"\n", Style::NoSpace),
            vec![(9, 10, 0), (11, 12, 0)]
        );
        assert!(run("x = 1\n__END__\nf( 3 )\n", Style::NoSpace).is_empty());
    }

    #[test]
    fn def_lambda_pattern_parens_are_ordinary() {
        assert_eq!(
            run("def f( a ); end\n", Style::NoSpace),
            vec![(6, 7, 0), (8, 9, 0)]
        );
        assert_eq!(
            run("->( a ) { }\n", Style::NoSpace),
            vec![(3, 4, 0), (5, 6, 0)]
        );
        assert_eq!(
            run("case x\nin Foo( 1 )\n  y\nend\n", Style::NoSpace),
            vec![(14, 15, 0), (16, 17, 0)]
        );
        assert_eq!(
            run("a.( 1 )\n", Style::NoSpace),
            vec![(3, 4, 0), (5, 6, 0)]
        );
    }
}
