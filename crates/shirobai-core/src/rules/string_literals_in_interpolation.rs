//! `Style/StringLiteralsInInterpolation`.
//!
//! Enforces the configured quote style (`EnforcedStyle`: `single_quotes` /
//! `double_quotes`) for string literals that appear *inside* an interpolation
//! (`#{...}` / `#@x`) of a string, symbol, or regexp. It is the interpolation
//! counterpart of `Style/StringLiterals`: same `StringHelp#on_str` machinery,
//! same `wrong_quotes?` / `to_string_literal` / `inspect` autocorrect, but the
//! `inside_interpolation?` guard is *inverted* (only interpolation-internal
//! strings offend) and there is no `ConsistentQuotesInMultiline` / `on_dstr`
//! handling.
//!
//! Reconstructed over Prism, mirroring stock's single relevant callback:
//!
//! - `StringHelp#on_str` (fires for every `:str` node that has a begin loc and
//!   is not part of an ignored node). The offense test for this cop is
//!   `inside_interpolation?(node) && wrong_quotes?(source)`; a non-offending
//!   such node emits a `correct_style_detected` marker (so
//!   `config_to_allow_offenses` matches stock). Heredocs are excluded: in
//!   parser-gem a heredoc `:str` has no `loc.begin`, whereas Prism's
//!   `StringNode` *does* carry an opening loc (`<<…`), so the heredoc case is
//!   filtered explicitly.
//! - Unlike the other `StringHelp` cops, `on_regexp` is overridden with an empty
//!   body, so string literals inside regexp interpolations are *not* ignored and
//!   are checked normally.
//!
//! A string literal that spans physical lines (a raw newline inside the quotes,
//! not a `\n` escape) is parsed by parser-gem as a `:dstr` whose `:str` children
//! have no begin loc, so `on_str` never fires for it and it produces neither an
//! offense nor a detection marker. Prism keeps it a single `StringNode`, so we
//! detect a raw newline in the node source and `return` early (no record).
//!
//! Division of labour with the Ruby wrapper: Rust decides which nodes offend,
//! with which detection marker, and the decoded string content; the wrapper
//! computes the replacement text with stock's genuine `RuboCop::Cop::Util`
//! helpers (`String#inspect` / `to_string_literal`) and applies it as a single
//! `replace`, and replays the detection markers through the genuine
//! `ConfigurableEnforcedStyle` methods. The source-text quote regexes are reused
//! from `string_literals` (`wrong_quotes`).

use ruby_prism::{Location, Node};

use super::string_literals::{wrong_quotes, STYLE_SINGLE};

/// Which autocorrect to apply (computed by the Ruby wrapper from `content`).
pub const FIX_SINGLE: u8 = 0;
pub const FIX_DOUBLE: u8 = 1;

/// Detection side effect replayed through the genuine stock methods.
pub const DETECT_OPPOSITE: u8 = 0;
pub const DETECT_CORRECT: u8 = 1;

/// `Style/StringLiteralsInInterpolation` configuration.
#[derive(Clone, Copy)]
pub struct Config {
    /// 0 single_quotes, 1 double_quotes.
    pub style: u8,
}

/// One record from the walk, in walk order. When `is_offense` is false it is a
/// pure `correct_style_detected` marker (no caret, no fix).
pub struct StringLiteralsInInterpolationOffense {
    pub is_offense: bool,
    pub start_offset: usize,
    pub end_offset: usize,
    pub detect: u8,
    /// Autocorrect kind for an offense; ignored for non-offense markers.
    pub fix: u8,
    /// Decoded (`unescaped`) string content, for the autocorrect.
    pub content: String,
}

pub fn check_string_literals_in_interpolation(
    source: &[u8],
    cfg: &Config,
) -> Vec<StringLiteralsInInterpolationOffense> {
    let mut rule = build_rule(source, cfg);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

pub(crate) fn build_rule<'a>(source: &'a [u8], cfg: &Config) -> Visitor<'a> {
    Visitor {
        source,
        cfg: *cfg,
        stack: Vec::new(),
        offenses: Vec::new(),
    }
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

/// Mirrors `string_literals::Frame` minus the dstr-ignored field (this cop has
/// no `on_dstr` / `ConsistentQuotesInMultiline` path). Reproduces stock's
/// `inside_interpolation?` semantics: a `#{...}` only counts as
/// "interpolation" when its immediate `:dstr` / `:dsym` / `:regexp` parent
/// (i.e. a `dstr`/`dsym`/`regexp` ancestor strictly *outside* the closest
/// `Embedded*`) is present. Notably **xstr** (backticks) is not on stock's
/// recognised list, so backtick `#{...}` interior strings are *not* inside an
/// interpolation by this cop's contract.
struct Frame {
    /// Frame is an `EmbeddedStatementsNode` / `EmbeddedVariableNode`.
    is_interp: bool,
    /// Frame is a parser-gem `:dstr` / `:dsym` / `:regexp` (Prism
    /// `InterpolatedStringNode` / `InterpolatedSymbolNode` /
    /// `InterpolatedRegularExpressionNode` / `RegularExpressionNode`).
    is_dstr_dsym_or_regexp: bool,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    cfg: Config,
    /// One frame per entered node; `leave` pops in lockstep.
    stack: Vec<Frame>,
    pub offenses: Vec<StringLiteralsInInterpolationOffense>,
}

impl<'a> Visitor<'a> {
    fn src(&self, a: usize, b: usize) -> &[u8] {
        &self.source[a..b]
    }

    fn decoded(&self, n: &ruby_prism::StringNode<'_>) -> String {
        String::from_utf8_lossy(n.unescaped()).into_owned()
    }

    /// Port of stock `StringHelp#inside_interpolation?`: ancestors are scanned
    /// inner→outer, drop until the first `:begin` (Prism `Embedded*`), then
    /// `any?` over the rest for `:dstr` / `:dsym` / `:regexp`. See the
    /// `string_literals` twin for the full discourse parity rationale; the
    /// short version is that `xstr` (backticks) is **not** on stock's list, so
    /// a `#{...}` inside backticks is not "inside interpolation" and a
    /// double-quoted string literal there is claimed by `Style/StringLiterals`,
    /// not by this cop.
    fn inside_interpolation(&self) -> bool {
        let mut iter = self.stack.iter().rev();
        for f in iter.by_ref() {
            if f.is_interp {
                break;
            }
        }
        iter.any(|f| f.is_dstr_dsym_or_regexp)
    }

    /// Replicate `StringHelp#on_str` for a single `:str` node, with this cop's
    /// inverted interpolation guard.
    fn on_str(&mut self, n: &ruby_prism::StringNode<'_>) {
        // `return unless node.loc?(:begin)`: parser-gem `:str` nodes without an
        // opening (interpolation literal parts / `%w` parts) are skipped. In
        // Prism those have no `opening_loc`.
        let Some(opening) = n.opening_loc() else {
            return;
        };
        // Heredoc: parser-gem heredoc `:str` has no `loc.begin`, but Prism's
        // opening is the `<<…` marker. Exclude it.
        if self
            .src(opening.start_offset(), opening.end_offset())
            .starts_with(b"<<")
        {
            return;
        }

        let (start, end) = loc(&n.location());
        let node_src = self.src(start, end);

        // Parser-gem parses a string literal spanning physical lines (a raw
        // newline inside the quotes, not a `\n` escape) as a `:dstr` with
        // `begin`-less `:str` children, so stock's `on_str` never fires for it —
        // neither an offense nor a `correct_style_detected` marker. Prism keeps
        // it a single `StringNode`; skip it.
        if node_src.contains(&b'\n') {
            return;
        }

        // `offense?(node) = inside_interpolation?(node) && wrong_quotes?(node)`.
        let offending = self.inside_interpolation() && wrong_quotes(node_src, self.cfg.style);

        if offending {
            let fix = if self.cfg.style == STYLE_SINGLE {
                FIX_SINGLE
            } else {
                FIX_DOUBLE
            };
            self.offenses
                .push(StringLiteralsInInterpolationOffense {
                    is_offense: true,
                    start_offset: start,
                    end_offset: end,
                    detect: DETECT_OPPOSITE,
                    fix,
                    content: self.decoded(n),
                });
        } else {
            // `correct_style_detected`.
            self.offenses
                .push(StringLiteralsInInterpolationOffense {
                    is_offense: false,
                    start_offset: 0,
                    end_offset: 0,
                    detect: DETECT_CORRECT,
                    fix: FIX_SINGLE,
                    content: String::new(),
                });
        }
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        let is_interp = matches!(
            node,
            Node::EmbeddedStatementsNode { .. } | Node::EmbeddedVariableNode { .. }
        );
        // Mirrors `string_literals::Visitor::enter` recognition of the
        // parser-gem `:dstr` / `:dsym` / `:regexp` carriers. `xstr` (backticks)
        // is intentionally not on this list, so a `#{...}` inside backticks is
        // *not* "inside interpolation" for this cop either — that string is
        // claimed by `Style/StringLiterals`, not by this cop. See the discourse
        // parity rationale in `string_literals.rs`.
        let is_dstr_dsym_or_regexp = matches!(
            node,
            Node::InterpolatedStringNode { .. }
                | Node::InterpolatedSymbolNode { .. }
                | Node::InterpolatedRegularExpressionNode { .. }
                | Node::RegularExpressionNode { .. }
        );
        self.stack.push(Frame {
            is_interp,
            is_dstr_dsym_or_regexp,
        });
    }

    fn leave(&mut self) {
        self.stack.pop();
    }

    fn enter_leaf(&mut self, node: &Node<'_>) {
        if let Node::StringNode { .. } = node {
            let n = node.as_string_node().unwrap();
            self.on_str(&n);
        }
    }
}
