//! `Layout/SpaceInsideHashLiteralBraces`.
//!
//! Checks the space just inside `{` and `}` of hash literals and hash
//! patterns, per `EnforcedStyle` (`space` / `no_space` / `compact`) and
//! `EnforcedStyleForEmptyBraces` (`space` / `no_space`).
//!
//! Stock's `on_hash` reads `processed_source.tokens_within(node)` and checks
//! the pair `(tokens[0], tokens[1])` (the `{` and the first token after it)
//! and the pair `(tokens[-2], tokens[-1])` (the last inner token and the `}`),
//! plus a whitespace-only-inner check under `no_space` empty style. This is
//! reconstructed token-free:
//!
//! - `token1.line < token2.line` — a `\n` (raw or behind a `\`-newline
//!   continuation) sits between the brace and the adjacent token;
//! - `token2.comment?` — the next token starts with `#` (a `#` in expression
//!   position right after `{` is always a comment);
//! - `token1.space_after?` — the byte adjacent to the brace is `\s`;
//! - `token1.type == token2.type` (only read under `compact`): after `{`, a
//!   `{` byte at token start is always another `tLBRACE`; before `}`, a `}`
//!   byte is `tRCURLY` only when it closes a hash / hash pattern / brace block
//!   / brace lambda (a `%w{...}`-style percent literal's `}` is a
//!   `tSTRING_END`). The closing positions of those four node kinds are
//!   collected during the same walk and right-brace checks that saw a `}`
//!   resolve against the set after the walk.
//!
//! Offense ranges mirror stock: the brace itself for a missing space, the
//! `[ \t]` run (`range_of_space_to_the_right/left`) for a detected one, the
//! whole inner range for a whitespace-only hash. The Ruby wrapper reproduces
//! stock's corrector from the live `range.source` (whitespace → remove, `{` →
//! insert after, else insert before).

use std::collections::HashSet;

use ruby_prism::Node;

use super::space_scan::{
    is_ruby_space, next_token_start, prev_token_end_same_line, skip_space_left, skip_space_right,
};

/// `EnforcedStyle` value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Space,
    NoSpace,
    Compact,
}

/// Config for `Layout/SpaceInsideHashLiteralBraces`.
#[derive(Clone, Copy)]
pub struct Config {
    pub style: Style,
    /// `EnforcedStyleForEmptyBraces == 'no_space'`.
    pub no_space_empty: bool,
}

/// One offense: `[start, end)` is the reported range (also the autocorrect
/// anchor); `message` picks the fixed stock message.
pub struct SpaceInsideHashLiteralBracesOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: MessageId,
}

/// The six fixed messages stock emits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MessageId {
    /// `'Space inside { missing.'`
    LeftMissing,
    /// `'Space inside { detected.'`
    LeftDetected,
    /// `'Space inside } missing.'`
    RightMissing,
    /// `'Space inside } detected.'`
    RightDetected,
    /// `'Space inside empty hash literal braces missing.'`
    EmptyMissing,
    /// `'Space inside empty hash literal braces detected.'`
    EmptyDetected,
}

impl MessageId {
    /// The numeric tag carried over the wire to the Ruby wrapper.
    pub fn code(self) -> u8 {
        match self {
            MessageId::LeftMissing => 0,
            MessageId::LeftDetected => 1,
            MessageId::RightMissing => 2,
            MessageId::RightDetected => 3,
            MessageId::EmptyMissing => 4,
            MessageId::EmptyDetected => 5,
        }
    }
}

pub fn check_space_inside_hash_literal_braces(
    source: &[u8],
    config: Config,
) -> Vec<SpaceInsideHashLiteralBracesOffense> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.into_offenses()
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    Visitor {
        source,
        config,
        items: Vec::new(),
        curly_closers: HashSet::new(),
    }
}

/// A resolved offense or a right-brace check deferred on `tRCURLY`-ness of the
/// preceding `}` (compact style only; the `}` may belong to a node visited
/// after the enclosing hash).
enum Item {
    Ready(SpaceInsideHashLiteralBracesOffense),
    PendingRight {
        rb_s: usize,
        rb_e: usize,
        prev_end: usize,
        has_space: bool,
    },
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    config: Config,
    items: Vec<Item>,
    /// Closing-brace positions that lex as `tRCURLY` (hash / hash pattern /
    /// brace block / brace lambda closings). Only filled under `compact`.
    curly_closers: HashSet<usize>,
}

impl<'a> Visitor<'a> {
    /// Resolve deferred right-brace checks and return the offenses in stock's
    /// emission order.
    pub(crate) fn into_offenses(self) -> Vec<SpaceInsideHashLiteralBracesOffense> {
        let Visitor {
            source,
            config,
            items,
            curly_closers,
        } = self;
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Item::Ready(o) => out.push(o),
                Item::PendingRight {
                    rb_s,
                    rb_e,
                    prev_end,
                    has_space,
                } => {
                    // `is_same_braces && style == :compact` => expect no space;
                    // otherwise (compact != no_space) expect a space.
                    let same = curly_closers.contains(&(prev_end - 1));
                    if let Some(o) = right_offense(source, config, !same, has_space, rb_s, rb_e) {
                        out.push(o);
                    }
                }
            }
        }
        out
    }

    fn push(&mut self, start: usize, end: usize, message: MessageId) {
        self.items
            .push(Item::Ready(SpaceInsideHashLiteralBracesOffense {
                start_offset: start,
                end_offset: end,
                message,
            }));
    }

    /// Stock `on_hash` / `on_hash_pattern` for a braced node.
    fn check_hash(&mut self, lb_s: usize, lb_e: usize, rb_s: usize, rb_e: usize) {
        let src = self.source;
        let style = self.config.style;

        if lb_e == rb_s {
            // Adjacent braces `{}`: tokens are exactly `{` `}`. `is_same_braces`
            // is false (tLBRACE vs tRCURLY), `is_empty_braces` is true, no
            // space after `{`. Only the `space` empty style flags this, with
            // the `{` itself as the range. The right check needs tokens
            // between (`tokens.size > 2`) and the whitespace-only check needs
            // a non-empty inner, so both are skipped.
            if !self.config.no_space_empty {
                self.push(lb_s, lb_e, MessageId::EmptyMissing);
            }
            return;
        }

        // check(tokens[0], tokens[1]) — the `{` and the next token.
        let (tok_pos, crossed) = next_token_start(src, lb_e);
        if !crossed && src.get(tok_pos) != Some(&b'#') {
            // `token2.comment?` also returns early (stock skips `{ # comment`
            // even though the comment is on the same line).
            let is_empty = tok_pos == rb_s;
            let same = src.get(tok_pos) == Some(&b'{');
            let expect_space = if same && style == Style::Compact {
                false
            } else if is_empty {
                !self.config.no_space_empty
            } else {
                style != Style::NoSpace
            };
            // `token1.space_after?`: the byte right after `{` is `\s`. The gap
            // to the next token is whitespace-only here (a continuation would
            // have crossed a line), so this is "the gap is non-empty".
            let has_space = tok_pos > lb_e;
            if expect_space && !has_space {
                let message = if is_empty {
                    MessageId::EmptyMissing
                } else {
                    MessageId::LeftMissing
                };
                self.push(lb_s, lb_e, message);
            } else if !expect_space && has_space {
                // range_of_space_to_the_right: begin_pos + 1 .. end of [ \t].
                let message = if is_empty {
                    MessageId::EmptyDetected
                } else {
                    MessageId::LeftDetected
                };
                self.push(lb_s + 1, skip_space_right(src, lb_e), message);
            }
        }

        // check(tokens[-2], tokens[-1]) if tokens.size > 2 — there is at least
        // one token between the braces.
        if tok_pos != rb_s
            && let Some(prev_end) = prev_token_end_same_line(src, rb_s)
        {
            let has_space = prev_end < rb_s;
            if style == Style::Compact && src.get(prev_end.wrapping_sub(1)) == Some(&b'}') {
                // `is_same_braces` needs the token type of that `}`; defer
                // until every closer position is known.
                self.items.push(Item::PendingRight {
                    rb_s,
                    rb_e,
                    prev_end,
                    has_space,
                });
            } else {
                // Not compact, or the previous token cannot be a `}`:
                // `is_same_braces` is false and `is_empty_braces` is false,
                // so a space is expected iff style != no_space.
                let expect_space = style != Style::NoSpace;
                if let Some(o) =
                    right_offense(src, self.config, expect_space, has_space, rb_s, rb_e)
                {
                    self.items.push(Item::Ready(o));
                }
            }
        }

        // check_whitespace_only_hash, only under the `no_space` empty style.
        if self.config.no_space_empty && rb_s > lb_e {
            let inner = &src[lb_e..rb_s];
            if inner.iter().all(|&b| is_ruby_space(b)) {
                self.push(lb_e, rb_s, MessageId::EmptyDetected);
            }
        }
    }
}

/// The right-brace half of stock's `check`, shared by the immediate and the
/// deferred (compact) paths.
fn right_offense(
    source: &[u8],
    _config: Config,
    expect_space: bool,
    has_space: bool,
    rb_s: usize,
    rb_e: usize,
) -> Option<SpaceInsideHashLiteralBracesOffense> {
    if expect_space && !has_space {
        Some(SpaceInsideHashLiteralBracesOffense {
            start_offset: rb_s,
            end_offset: rb_e,
            message: MessageId::RightMissing,
        })
    } else if !expect_space && has_space {
        // range_of_space_to_the_left: begin of [ \t] run .. end_pos - 1.
        Some(SpaceInsideHashLiteralBracesOffense {
            start_offset: skip_space_left(source, rb_s),
            end_offset: rb_e - 1,
            message: MessageId::RightDetected,
        })
    } else {
        None
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(hash) = node.as_hash_node() {
            let (o, c) = (hash.opening_loc(), hash.closing_loc());
            self.check_hash(
                o.start_offset(),
                o.end_offset(),
                c.start_offset(),
                c.end_offset(),
            );
            if self.config.style == Style::Compact {
                self.curly_closers.insert(c.start_offset());
            }
        } else if let Some(pat) = node.as_hash_pattern_node() {
            // Braced hash patterns only: `in ADT[a: 1]` / `in ADT(a: 1)` lex
            // their delimiters as brackets / parens, so stock's
            // `tokens.first.left_brace?` guard skips them; braceless patterns
            // have no delimiters at all.
            if let (Some(o), Some(c)) = (pat.opening_loc(), pat.closing_loc())
                && self.source.get(o.start_offset()) == Some(&b'{')
            {
                self.check_hash(
                    o.start_offset(),
                    o.end_offset(),
                    c.start_offset(),
                    c.end_offset(),
                );
                if self.config.style == Style::Compact {
                    self.curly_closers.insert(c.start_offset());
                }
            }
        } else if self.config.style == Style::Compact {
            // Brace blocks and brace lambdas close with a `tRCURLY` too; a
            // `do ... end` block's closer is a keyword, not a `}`.
            let closer = if let Some(block) = node.as_block_node() {
                Some((block.opening_loc(), block.closing_loc()))
            } else if let Some(lambda) = node.as_lambda_node() {
                Some((lambda.opening_loc(), lambda.closing_loc()))
            } else if let Some(zsuper) = node.as_forwarding_super_node() {
                // A bare super's BlockNode hides behind the concretely-typed
                // `block` field (the RescueNode trap): collect its closer here.
                zsuper.block().map(|b| (b.opening_loc(), b.closing_loc()))
            } else {
                None
            };
            if let Some((o, c)) = closer
                && self.source.get(o.start_offset()) == Some(&b'{')
            {
                self.curly_closers.insert(c.start_offset());
            }
        }
    }

    fn leave(&mut self) {}

    fn interest(&self) -> super::dispatch::Interest {
        // HashNode / HashPatternNode live in the ENTER_OTHER bucket. The
        // compact style additionally reads BlockNode / LambdaNode closings.
        // `enter` is a pure kind match with an empty fall-through and `leave`
        // / leaf / rescue are unused, so the mask is exact.
        let mut mask = super::dispatch::Interest::ENTER_OTHER;
        if self.config.style == Style::Compact {
            mask |= super::dispatch::Interest::ENTER_BLOCK
                | super::dispatch::Interest::ENTER_LAMBDA
                | super::dispatch::Interest::ENTER_SUPER;
        }
        super::dispatch::Interest(mask)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: Style, no_space_empty: bool) -> Vec<(usize, usize, u8)> {
        check_space_inside_hash_literal_braces(
            source.as_bytes(),
            Config {
                style,
                no_space_empty,
            },
        )
        .into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.message.code()))
        .collect()
    }

    #[test]
    fn space_style_flags_missing_spaces() {
        // "h = {a: 1}" -> `{` at 4, `}` at 9.
        assert_eq!(
            run("h = {a: 1}\n", Style::Space, true),
            vec![(4, 5, 0), (9, 10, 2)]
        );
    }

    #[test]
    fn space_style_accepts_spaced_hash() {
        assert!(run("h = { a: 1 }\n", Style::Space, true).is_empty());
    }

    #[test]
    fn no_space_style_flags_spaces() {
        // "h = { a: 1 }" -> left space range [5,6), right space range [10,11).
        assert_eq!(
            run("h = { a: 1 }\n", Style::NoSpace, true),
            vec![(5, 6, 1), (10, 11, 3)]
        );
    }

    #[test]
    fn comment_after_brace_is_skipped() {
        assert!(run("h = { # c\n  a: 1 }\n", Style::Space, true).is_empty());
        assert!(run("h = { # c\n  a: 1}\n", Style::NoSpace, true).is_empty());
    }

    #[test]
    fn multiline_is_skipped() {
        assert!(run("h = {\n  a: 1\n}\n", Style::NoSpace, true).is_empty());
        assert!(run("h = {\n  a: 1\n}\n", Style::Space, true).is_empty());
    }

    #[test]
    fn continuation_after_brace_is_skipped() {
        assert!(run("h = { \\\n  a: 1}\n", Style::NoSpace, true).is_empty());
    }

    #[test]
    fn empty_braces_no_space_style() {
        assert!(run("h = {}\n", Style::Space, true).is_empty());
        // `{ }`: the left check and the whitespace-only check both produce
        // the space range (stock emits two offenses; add_offense dedups).
        assert_eq!(
            run("h = { }\n", Style::Space, true),
            vec![(5, 6, 5), (5, 6, 5)]
        );
        // `{\n}`: only the whitespace-only check fires.
        assert_eq!(run("h = {\n}\n", Style::Space, true), vec![(5, 6, 5)]);
    }

    #[test]
    fn empty_braces_space_style() {
        assert_eq!(run("h = {}\n", Style::Space, false), vec![(4, 5, 4)]);
        assert!(run("h = { }\n", Style::Space, false).is_empty());
        assert!(run("h = {\n}\n", Style::Space, false).is_empty());
    }

    #[test]
    fn compact_collapses_same_braces() {
        // "h = { a: { b: 1 } }" -> only the outer right space is flagged.
        assert_eq!(
            run("h = { a: { b: 1 } }\n", Style::Compact, true),
            vec![(17, 18, 3)]
        );
        assert!(run("h = { a: { b: 1 }}\n", Style::Compact, true).is_empty());
    }

    #[test]
    fn compact_percent_brace_is_not_a_curly() {
        // "{k => %w{a}}": the `}` before the outer `}` is a tSTRING_END, so a
        // space is still expected on the right.
        assert_eq!(
            run("h = {k => %w{a}}\n", Style::Compact, true),
            vec![(4, 5, 0), (15, 16, 2)]
        );
    }

    #[test]
    fn compact_block_brace_is_a_curly() {
        // "{ a: proc {} }": the block's `}` is a tRCURLY, so the outer right
        // space is flagged.
        assert_eq!(
            run("h = { a: proc {} }\n", Style::Compact, true),
            vec![(16, 17, 3)]
        );
    }

    #[test]
    fn compact_bare_super_block_brace_is_a_curly() {
        // "z = { a: super {} }": the block's `}` hides behind
        // ForwardingSuperNode's typed field but is still a tRCURLY.
        assert_eq!(
            run("z = { a: super {} }\n", Style::Compact, true),
            vec![(17, 18, 3)]
        );
    }

    #[test]
    fn hash_pattern_is_checked() {
        assert_eq!(
            run("case x\nin {a: 1}\n  y\nend\n", Style::Space, true),
            vec![(10, 11, 0), (15, 16, 2)]
        );
        assert!(run("case x\nin ADT[a: 1]\n  y\nend\n", Style::Space, true).is_empty());
    }

    #[test]
    fn keyword_hash_is_skipped() {
        assert!(run("foo(a: 1)\n", Style::Space, true).is_empty());
        assert!(run("f(get: \"#{x}\")\n", Style::Space, true).is_empty());
    }
}
