//! `Layout/DefEndAlignment`.
//!
//! Checks that a method definition's `end` keyword is aligned properly, under
//! one of two `EnforcedStyleAlignWith` styles: `start_of_line` (the default,
//! align the `end` with the first non-space column of the `def` keyword's line)
//! or `def` (align it with the `def` keyword itself).
//!
//! Stock fires two callbacks:
//!
//! - `on_def` / `on_defs` (a bare definition): `check_end_kw_in_node` checks
//!   the `end` against a single range `{ style => node.loc.keyword }` (only the
//!   configured style is in the hash). The message always names the `def`
//!   keyword.
//! - `on_send` (a `def_modifier?` send such as `private def foo`): the def is
//!   reached as the send's sole argument. Here `align_with` carries *both*
//!   styles — `def: method_def.loc.keyword` and
//!   `start_of_line: range_between(send.begin, def.keyword.end)` (the
//!   `"private def"` prefix). The inner def is then `ignore_node`d so its own
//!   `on_def` is a no-op. For a *chain* of modifiers (`private module_function
//!   def foo`) every enclosing send `def_modifier?`s and would check, but the
//!   first (outermost, entered first) wins and ignores the def.
//!
//! The autocorrect column (`AlignmentCorrector#align_end`) comes from a
//! separate anchor: under `def` style always the def keyword; under
//! `start_of_line` the outermost send of the def's modifier chain
//! (`outermost_def_modifier`, 1.92.3, rubocop#16251), or the def keyword when
//! the def is not under a modifier. Before 1.92.3 it was the def's *parent*
//! send, which split the message and the corrected column for chained
//! modifiers and for a def argument of a call with a receiver.
//!
//! Reconstructed over Prism in one ancestor-stack walk; each definition's `end`
//! is checked once, in stock's callback order, with a `handled` set standing in
//! for `ignore_node`.

use std::collections::HashSet;
use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// Style selector (`EnforcedStyleAlignWith`): 0 = start_of_line, 1 = def.
const STYLE_START_OF_LINE: u8 = 0;
const STYLE_DEF: u8 = 1;

/// One checked definition `end`, emitted in walk order. `matching` is the set
/// of styles the `end` already aligns with, in the path's hash-insertion order
/// (drives `style_detected` / `correct_style_detected` on the Ruby side). When
/// the configured style is not in `matching`, `offense` carries the location,
/// message, and autocorrect target column.
#[derive(Debug)]
pub struct DefEndAlignmentRecord {
    /// `end` keyword range (the offense location when misaligned).
    pub end_start: usize,
    pub end_end: usize,
    /// The matched style ids in hash order (stock's `matching.keys`).
    /// 0 = start_of_line, 1 = def.
    pub matching: Vec<u8>,
    /// `Some(offense)` when the configured style is not matched.
    pub offense: Option<DefEndAlignmentOffense>,
}

/// The offense detail for a misaligned definition `end`.
#[derive(Debug)]
pub struct DefEndAlignmentOffense {
    /// Formatted stock message.
    pub message: String,
    /// `AlignmentCorrector#align_end`: target column for the `end`.
    pub align_column: usize,
}

#[derive(Clone, Copy)]
pub struct Config {
    pub style: u8,
}

pub fn check_def_end_alignment(source: &[u8], config: Config) -> Vec<DefEndAlignmentRecord> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.records
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        style: config.style,
        records: Vec::new(),
        handled: HashSet::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    style: u8,
    pub(crate) records: Vec<DefEndAlignmentRecord>,
    /// Start offsets of definitions already checked through the modifier
    /// (`on_send`) path (`ignore_node`); their own `on_def` callback is a no-op.
    handled: HashSet<usize>,
}

/// A resolved alignment range: a source range whose `column` / `line` the
/// matcher and message use.
#[derive(Clone, Copy)]
struct AlignRange {
    start: usize,
    end: usize,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

impl<'a> Visitor<'a> {
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn column(&self, off: usize) -> usize {
        self.line_index.column(self.source, off)
    }

    /// `effective_column`: the char column, minus one when the offset is on
    /// line 1 and the source begins with a UTF-8 BOM (matches RangeHelp).
    fn effective_column(&self, off: usize) -> usize {
        let col = self.column(off);
        if self.line_of(off) == 1 && self.source.starts_with(&[0xef, 0xbb, 0xbf]) && col > 0 {
            col - 1
        } else {
            col
        }
    }

    /// Source text of a range, for the message (`align_with.source`).
    fn text(&self, start: usize, end: usize) -> String {
        String::from_utf8_lossy(&self.source[start..end]).into_owned()
    }

    /// `matching_ranges`: a range matches the `end` when it is on the same line
    /// as the `end` or shares its effective column.
    fn range_matches(&self, range: AlignRange, end_start: usize) -> bool {
        self.line_of(range.start) == self.line_of(end_start)
            || self.effective_column(range.start) == self.effective_column(end_start)
    }

    /// Format the stock `MSG` for a misaligned `end` against `align`.
    fn message(&self, end_start: usize, align: AlignRange) -> String {
        format!(
            "`end` at {}, {} is not aligned with `{}` at {}, {}.",
            self.line_of(end_start),
            self.column(end_start),
            self.text(align.start, align.end),
            self.line_of(align.start),
            self.column(align.start),
        )
    }

    /// `check_end_kw_alignment`: with the per-style ranges in the path's
    /// hash-insertion order, compute the matched styles (`matching.keys`,
    /// order-preserving) and emit a record (offense iff the configured style is
    /// not matched). `align_column` is the autocorrect target column.
    fn check(
        &mut self,
        end_start: usize,
        end_end: usize,
        ordered_styles: &[(u8, AlignRange)],
        align_column: usize,
    ) {
        let mut matching: Vec<u8> = Vec::with_capacity(ordered_styles.len());
        for &(id, range) in ordered_styles {
            if self.range_matches(range, end_start) {
                matching.push(id);
            }
        }

        let offense = if matching.contains(&self.style) {
            None
        } else {
            let align = ordered_styles
                .iter()
                .find(|(id, _)| *id == self.style)
                .map(|(_, r)| *r)
                .expect("configured style is always present");
            Some(DefEndAlignmentOffense {
                message: self.message(end_start, align),
                align_column,
            })
        };

        self.records.push(DefEndAlignmentRecord {
            end_start,
            end_end,
            matching,
            offense,
        });
    }
}

// --- Modifier (`def_modifier?`) resolution. ---

/// rubocop-ast `def_modifier`: a send with no receiver whose *first*
/// argument (`node.children[2]`) is a `def`/`defs` node or another such
/// modifier send. Other arguments, a block argument and a block do not
/// matter. Returns the `DefNode` reached, or `None` if `node` is not a def
/// modifier.
fn def_modifier_target<'pr>(node: &Node<'pr>) -> Option<ruby_prism::DefNode<'pr>> {
    let call = node.as_call_node()?;
    if call.receiver().is_some() {
        return None;
    }
    let first = call.arguments()?.arguments().iter().next()?;
    if let Some(def) = first.as_def_node() {
        return Some(def);
    }
    def_modifier_target(&first)
}

impl<'a> Visitor<'a> {
    /// The `def`/`end` facts of a `DefNode`, or `None` for an endless method
    /// (`def foo = ...`, which has no `end`).
    fn def_construct(d: &ruby_prism::DefNode<'_>) -> Option<(usize, usize, usize, usize)> {
        let (kw_start, kw_end) = loc(&d.def_keyword_loc());
        let (end_start, end_end) = loc(&d.end_keyword_loc()?);
        Some((kw_start, kw_end, end_start, end_end))
    }

    /// `on_def` / `on_defs`: `check_end_kw_in_node` checks the single configured
    /// style against the def keyword.
    ///
    /// The autocorrect anchor is the def keyword under both styles: a def
    /// that reaches this path under a modifier chain is single-line (the
    /// modifier path skipped it), and a single-line def's `end` never begins
    /// its line, so it is never an offense.
    fn check_bare_def(&mut self, def_start: usize, d: &ruby_prism::DefNode<'_>) {
        if self.handled.contains(&def_start) {
            return;
        }
        let Some((kw_start, kw_end, end_start, end_end)) = Self::def_construct(d) else {
            return;
        };
        let kw = AlignRange {
            start: kw_start,
            end: kw_end,
        };
        let anchor = self.column(kw_start);
        // `{ style => node.loc.keyword }`: only the configured style is present.
        self.check(end_start, end_end, &[(self.style, kw)], anchor);
    }

    /// `on_send` for a `def_modifier?` send: check the inner def against both
    /// styles (`def` = the def keyword, `start_of_line` = the send-start to
    /// def-keyword-end range), then mark the def handled. `send_start` is the
    /// *outermost* enclosing send (the firing callback). It is used for the
    /// message's `start_of_line` range and, under `start_of_line`, as the
    /// autocorrect anchor: the outermost send whose `def_modifier` is this
    /// def (`outermost_def_modifier`) is exactly the send that fires first.
    fn check_modifier(&mut self, send_start: usize, d: &ruby_prism::DefNode<'_>) {
        let def_start = d.location().start_offset();
        if self.handled.contains(&def_start) {
            return;
        }
        // 1.92: `return if method_def.single_line?` — before `ignore_node`, so
        // the def is left to its own `on_def` (which accepts an `end` that
        // does not begin its line).
        let def_end = d.location().end_offset();
        if self.line_of(def_start) == self.line_of(def_end.saturating_sub(1).max(def_start)) {
            return;
        }
        let Some((kw_start, kw_end, end_start, end_end)) = Self::def_construct(d) else {
            // An endless def has no `end`; nothing to check, but still mark it
            // handled so its own callback stays consistent.
            self.handled.insert(def_start);
            return;
        };
        let def_range = AlignRange {
            start: kw_start,
            end: kw_end,
        };
        // `line_start = range_between(send.begin_pos, def.keyword.end_pos)`.
        let line_start = AlignRange {
            start: send_start,
            end: kw_end,
        };
        let anchor = if self.style == STYLE_DEF {
            self.column(kw_start)
        } else {
            self.column(send_start)
        };
        // `align_with = { def: ..., start_of_line: ... }` (insertion order).
        self.check(
            end_start,
            end_end,
            &[(STYLE_DEF, def_range), (STYLE_START_OF_LINE, line_start)],
            anchor,
        );
        self.handled.insert(def_start);
    }

    fn enter_node(&mut self, node: &Node<'_>) {
        // 1. A `def_modifier?` send checks (and ignores) its inner def first.
        if let Some(def) = def_modifier_target(node) {
            let send_start = node.location().start_offset();
            self.check_modifier(send_start, &def);
            return;
        }
        // 2. A bare `def` / `defs` checks itself (unless already ignored).
        if let Some(d) = node.as_def_node() {
            self.check_bare_def(node.location().start_offset(), &d);
        }
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        self.enter_node(node);
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Vec<DefEndAlignmentRecord> {
        check_def_end_alignment(source.as_bytes(), Config { style })
    }

    // Stock 1.92.1: a single-line def under a modifier after a line
    // continuation is not an offense.
    #[test]
    fn single_line_def_after_continuation() {
        let got = run("foo \\\n  def test; end\n", 0);
        assert!(got.iter().all(|r| r.offense.is_none()), "{:?}", got.iter().map(|r| (r.end_start, &r.matching)).collect::<Vec<_>>());
    }

    #[test]
    fn aligned_def_no_offense() {
        let r = run("def foo\nend\n", STYLE_START_OF_LINE);
        assert_eq!(r.len(), 1);
        assert!(r[0].offense.is_none());
    }

    #[test]
    fn misaligned_def() {
        let r = run("def foo\n  end\n", STYLE_START_OF_LINE);
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o
            .message
            .contains("`end` at 2, 2 is not aligned with `def` at 1, 0."));
        assert_eq!(o.align_column, 0);
    }

    #[test]
    fn defs_self_method() {
        // `def self.foo` keyword is still `def` (column 0).
        let r = run("def self.foo\n    end\n", STYLE_START_OF_LINE);
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`def` at 1, 0"));
        assert_eq!(o.align_column, 0);
    }

    #[test]
    fn endless_method_no_check() {
        assert!(run("def foo = 42\n", STYLE_START_OF_LINE).is_empty());
        assert!(run("def foo = 42\n", STYLE_DEF).is_empty());
    }

    #[test]
    fn modifier_start_of_line() {
        // `private def foo` misaligned: start_of_line names `private def` col 0.
        let r = run("private def foo\n            end\n", STYLE_START_OF_LINE);
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`private def` at 1, 0"));
        assert_eq!(o.align_column, 0);
    }

    #[test]
    fn modifier_def_style_aligns_to_def_column() {
        // `private def foo`: under `def` style the def keyword is column 8.
        let r = run("private def foo\nend\n", STYLE_DEF);
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`def` at 1, 8"));
        assert_eq!(o.align_column, 8);
    }

    #[test]
    fn modifier_chained_aligns_to_outermost() {
        // `private module_function def foo` start_of_line: the message and
        // (since 1.92.3) the autocorrect column both use the outer send.
        let r = run(
            "private module_function def foo\n  end\n",
            STYLE_START_OF_LINE,
        );
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o
            .message
            .contains("`private module_function def` at 1, 0"));
        assert_eq!(o.align_column, 0);
    }

    #[test]
    fn def_as_call_argument_with_receiver() {
        // `obj.foo def bar` is not a modifier (receiver present); on_def fires
        // and both the message and the autocorrect use the def (col 8).
        let r = run("obj.foo def bar\n      end\n", STYLE_START_OF_LINE);
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`def` at 1, 8"));
        assert_eq!(o.align_column, 8);
    }

    #[test]
    fn modifier_uses_first_argument_only() {
        // rubocop-ast `def_modifier` looks at the first argument only.
        let r = run("foo def bar\n  end, 1\n", STYLE_START_OF_LINE);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`foo def` at 1, 0"));
        let r = run("foo(bar, def baz\nend)\n", STYLE_START_OF_LINE);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`def` at 1, 9"));
        assert_eq!(o.align_column, 9);
    }

    #[test]
    fn assignment_rhs_def_aligns_to_def() {
        // `x = def foo`: parent is an assignment (not a send), so start_of_line
        // aligns to the def keyword (col 4).
        let r = run("x = def foo\n      end\n", STYLE_START_OF_LINE);
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`def` at 1, 4"));
        assert_eq!(o.align_column, 4);
    }

    #[test]
    fn nested_def_in_class() {
        let r = run("class A\n  def foo\n    end\nend\n", STYLE_DEF);
        assert_eq!(r.len(), 1);
        let o = r[0].offense.as_ref().unwrap();
        assert!(o.message.contains("`def` at 2, 2"));
        assert_eq!(o.align_column, 2);
    }

    #[test]
    fn single_line_def_no_offense() {
        // `def foo; end`: end on the same line as the keyword always matches.
        assert!(run("def foo; end\n", STYLE_DEF)[0].offense.is_none());
        assert!(run("private def foo; end\n", STYLE_START_OF_LINE)[0]
            .offense
            .is_none());
    }
}
