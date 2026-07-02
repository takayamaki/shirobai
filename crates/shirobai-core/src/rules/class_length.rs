//! `Metrics/ClassLength`.
//!
//! Drop-in reimplementation of RuboCop's `Metrics/ClassLength`, which measures
//! every class definition and flags those exceeding `Max` (default 100).
//! There is no autocorrect. Every quirk below was verified against a stock
//! probe (`.tmp/2026-07-02/class-module-length/`).
//!
//! Definitions covered, mirroring stock's hooks:
//!
//! - `on_class`: every `class` node, measured by the *classlike* line-number
//!   count ([`CodeLength::classlike_length`]): the lines strictly between the
//!   `class` and `end` lines, minus every inner class/module's full line
//!   range, sampled with stock's one-line-down off-by-one, and 0 for a
//!   namespace-only body.
//! - `on_sclass`: a `class << self` with **no `class` ancestor** (modules and
//!   other sclasses do not suppress it). It is measured like a block body
//!   (stock's `extract_body(:sclass)`), heredoc extension included.
//! - `on_casgn`: a constant assigned a `class_definition?` expression. The
//!   expression comes from the casgn itself, from a surrounding
//!   `||=`/`&&=`/`+=` assignment, or from a surrounding masgn (direct
//!   targets only). Matching expressions are:
//!   - `Class.new`/`::Class.new`/`Struct.new` with a literal block (any
//!     arguments): measured like a block body, offense on the whole
//!     call-with-block;
//!   - a `class << self` expression: measured like a block body, offense on
//!     the sclass, **regardless of a surrounding class** (no ancestor check
//!     on this path — probed);
//!   - a plain `class` expression: stock measures the same class node
//!     `on_class` already measured and `add_offense` dedups the identical
//!     range, so this path emits nothing here.
//!
//! Stock's masgn form fires `on_casgn` once per constant target with an
//! identical range that `add_offense` dedups; one candidate per masgn is
//! byte-identical. The toplevel `Foo = class << self` form fires both
//! `on_sclass` and `on_casgn` with identical ranges — both candidates are
//! emitted (matching stock's two `add_offense` calls) and the wrapper's
//! `add_offense` dedups exactly like stock's.
//!
//! In LSP mode stock reads `node.loc.begin` for an sclass offense, which
//! `Parser::Source::Map::Definition` does not define — stock raises
//! (`NoMethodError`, swallowed into a cop error) and reports nothing. The
//! candidates carry an `sclass` flag so the Ruby wrapper can skip them in LSP
//! mode (offense output matches; the error channel is not reproduced).

use ruby_prism::Node;

use super::block_length::top_level_const_name;
use super::code_length::{CodeLength, Fold};

/// A class definition whose measured length exceeds `Max`.
pub struct ClassLengthCandidate {
    pub start_offset: usize,
    pub end_offset: usize,
    /// End of the offense head for the LSP location mode (the class name for
    /// `class`, the block opening for a constructor block). Unused for an
    /// sclass candidate (the wrapper skips those in LSP mode).
    pub head_end: usize,
    pub length: usize,
    /// True for a `class << self` candidate (stock errors out on these in LSP
    /// mode instead of reporting).
    pub sclass: bool,
}

pub fn check_class_length(
    source: &[u8],
    max: usize,
    count_comments: bool,
    count_as_one: &[String],
) -> Vec<ClassLengthCandidate> {
    let mut rule = build_rule(source, max, count_comments, count_as_one);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.out
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule<'a>(
    source: &'a [u8],
    max: usize,
    count_comments: bool,
    count_as_one: &[String],
) -> Finder<'a> {
    Finder {
        max,
        calc: CodeLength::new(source, count_comments, Fold::from_types(count_as_one)),
        class_ends: Vec::new(),
        out: Vec::new(),
    }
}

pub(crate) struct Finder<'a> {
    max: usize,
    calc: CodeLength<'a>,
    /// End offsets of the `class` nodes still open at the current pre-order
    /// position (a monotone stack: entries whose end is at or before the
    /// current node's start are popped lazily). Because the walk is pre-order
    /// and class ranges nest properly, "some remaining entry covers this
    /// offset" is exactly `each_ancestor(:class).any?`.
    class_ends: Vec<usize>,
    pub(crate) out: Vec<ClassLengthCandidate>,
}

impl Finder<'_> {
    fn pop_closed(&mut self, start: usize) {
        while self.class_ends.last().is_some_and(|&end| end <= start) {
            self.class_ends.pop();
        }
    }

    /// `on_class`: track the open-class stack, then measure classlike.
    fn process_class(&mut self, node: &Node<'_>) {
        let loc = node.location();
        self.pop_closed(loc.start_offset());
        self.class_ends.push(loc.end_offset());
        if self.calc.classlike_cannot_exceed(node, self.max) {
            return;
        }
        let length = self.calc.classlike_length(node);
        if length > self.max {
            let head_end = node
                .as_class_node()
                .map_or(loc.end_offset(), |c| c.constant_path().location().end_offset());
            self.push(loc.start_offset(), loc.end_offset(), head_end, length, false);
        }
    }

    /// `on_sclass`: skipped inside a `class` (stock's
    /// `each_ancestor(:class).any?`); modules / sclasses do not suppress it.
    fn process_sclass(&mut self, node: &ruby_prism::SingletonClassNode<'_>) {
        self.pop_closed(node.location().start_offset());
        if !self.class_ends.is_empty() {
            return;
        }
        self.measure_sclass(node);
    }

    /// Measure a `class << self` like a block body. The fold walk of
    /// `body_length` is rooted at the body; stock roots it at the sclass whose
    /// only other child is the `self` expression (never foldable), so the two
    /// rootings are equivalent.
    fn measure_sclass(&mut self, node: &ruby_prism::SingletonClassNode<'_>) {
        if self.calc.cannot_exceed(node.body().as_ref(), self.max) {
            return;
        }
        let length = self.calc.body_length(node.body());
        if length > self.max {
            let loc = node.location();
            self.push(loc.start_offset(), loc.end_offset(), loc.end_offset(), length, true);
        }
    }

    /// `on_casgn`, applied to the assigned expression (`node.expression` or
    /// stock's `find_expression_within_parent` for `||=`/`&&=`/`+=`/masgn —
    /// prism's constant write nodes carry the value directly).
    fn process_casgn_value(&mut self, value: &Node<'_>) {
        // `class_definition?`'s `(class _ _ $_)` arm measures the same class
        // node `on_class` measures; stock's duplicate offense is deduped by
        // `add_offense`, so emitting nothing here is byte-identical.
        if value.as_class_node().is_some() {
            return;
        }
        // `(sclass _ $_)`: fires with no ancestor check (probed: a
        // `FOO = class << self` nested in a class still offends).
        if let Some(sclass) = value.as_singleton_class_node() {
            self.measure_sclass(&sclass);
            return;
        }
        // `(any_block (send #global_const?({:Struct :Class}) :new ...) _ $_)`.
        let Some(call) = value.as_call_node() else { return };
        if call.name().as_slice() != b"new" {
            return;
        }
        let Some(receiver) = call.receiver() else { return };
        let Some(name) = top_level_const_name(&receiver) else {
            return;
        };
        if name != b"Class" && name != b"Struct" {
            return;
        }
        let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        if self.calc.cannot_exceed(block.body().as_ref(), self.max) {
            return;
        }
        let length = self.calc.casgn_call_length(value, block.body());
        if length > self.max {
            let loc = value.location();
            self.push(
                loc.start_offset(),
                loc.end_offset(),
                block.opening_loc().end_offset(),
                length,
                false,
            );
        }
    }

    fn push(&mut self, start: usize, end: usize, head_end: usize, length: usize, sclass: bool) {
        self.out.push(ClassLengthCandidate {
            start_offset: start,
            end_offset: end,
            head_end,
            length,
            sclass,
        });
    }
}

/// Shared-walk driver. `enter` reacts only to class/module-family nodes
/// (`ENTER_CLASS_MOD`) and assignment forms (`ENTER_WRITE`); every other kind
/// falls through empty, `leave` is empty and the leaf/rescue hooks are the
/// defaults, so narrowing the interest mask to exactly those two classes is
/// equivalent to receiving every node. The open-class stack is only written
/// at `ClassNode` enters and only read at `SingletonClassNode` enters, both
/// inside `ENTER_CLASS_MOD`.
impl super::dispatch::Rule for Finder<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(Interest::ENTER_CLASS_MOD | Interest::ENTER_WRITE)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if node.as_class_node().is_some() {
            self.process_class(node);
        } else if let Some(sclass) = node.as_singleton_class_node() {
            self.process_sclass(&sclass);
        } else if let Some(w) = node.as_constant_write_node() {
            self.process_casgn_value(&w.value());
        } else if let Some(w) = node.as_constant_path_write_node() {
            self.process_casgn_value(&w.value());
        } else if let Some(w) = node.as_constant_or_write_node() {
            self.process_casgn_value(&w.value());
        } else if let Some(w) = node.as_constant_and_write_node() {
            self.process_casgn_value(&w.value());
        } else if let Some(w) = node.as_constant_operator_write_node() {
            self.process_casgn_value(&w.value());
        } else if let Some(w) = node.as_constant_path_or_write_node() {
            self.process_casgn_value(&w.value());
        } else if let Some(w) = node.as_constant_path_and_write_node() {
            self.process_casgn_value(&w.value());
        } else if let Some(w) = node.as_constant_path_operator_write_node() {
            self.process_casgn_value(&w.value());
        } else if let Some(w) = node.as_multi_write_node() {
            // Stock's masgn arm fires only for a casgn directly under the
            // mlhs (`parent.parent.masgn_type?`): direct lefts/rights only —
            // nested `(A, B), c = ...` targets and a splatted `*C` do not
            // reach the masgn expression.
            let has_const_target = w.lefts().iter().chain(w.rights().iter()).any(|n| {
                n.as_constant_target_node().is_some()
                    || n.as_constant_path_target_node().is_some()
            });
            if has_const_target {
                self.process_casgn_value(&w.value());
            }
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Got {
        ranges: Vec<(usize, usize)>,
        lengths: Vec<usize>,
        sclass: Vec<bool>,
    }

    fn run(source: &str, max: usize, count_comments: bool) -> Got {
        run_fold(source, max, count_comments, &[])
    }

    fn run_fold(source: &str, max: usize, count_comments: bool, fold: &[&str]) -> Got {
        let fold: Vec<String> = fold.iter().map(|s| s.to_string()).collect();
        let c = check_class_length(source.as_bytes(), max, count_comments, &fold);
        Got {
            ranges: c.iter().map(|o| (o.start_offset, o.end_offset)).collect(),
            lengths: c.iter().map(|o| o.length).collect(),
            sclass: c.iter().map(|o| o.sclass).collect(),
        }
    }

    // Typical: a class over the limit (vendor: 6 body lines at Max 5).
    #[test]
    fn long_class() {
        let src = "class Test\n  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\nend";
        let got = run(src, 5, false);
        assert_eq!(got.lengths, vec![6]);
        assert_eq!(got.ranges, vec![(0, src.len())]);
        assert_eq!(got.sclass, vec![false]);
        assert!(run(src, 6, false).lengths.is_empty());
    }

    // Stock's relevance sampling is one line below each body line number
    // (probed): a blank FIRST body line still counts 6, not 5.
    #[test]
    fn blank_first_body_line_off_by_one() {
        let src = "class Test\n\n  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\nend";
        assert_eq!(run(src, 5, false).lengths, vec![6]);
    }

    // Same sampling shift: a comment-only class counts 1 at Max 0 (the `end`
    // line is sampled for the last body number) — probed.
    #[test]
    fn comment_only_class_counts_one_at_max_zero() {
        let src = "class Test\n  # c1\n  # c2\nend";
        assert_eq!(run(src, 0, false).lengths, vec![1]);
        assert!(run(src, 1, false).lengths.is_empty());
    }

    // Blank lines inside the (shifted) window are not counted.
    #[test]
    fn blank_lines_excluded() {
        let src = "class Test\n  a = 1\n  a = 2\n  a = 3\n  a = 4\n\n\n  a = 7\nend";
        assert!(run(src, 5, false).lengths.is_empty());
    }

    // CountComments: comments skipped by default, counted when enabled.
    #[test]
    fn count_comments() {
        let src = "class Test\n  a = 1\n  #a = 2\n  a = 3\n  #a = 4\n  a = 5\n  a = 6\nend";
        assert!(run(src, 5, false).lengths.is_empty());
        assert_eq!(run(src, 5, true).lengths, vec![6]);
    }

    // Inner class/module line ranges are excluded from the outer count.
    #[test]
    fn inner_classes_excluded() {
        let inner = "  class Inner\n    a = 1\n    a = 2\n    a = 3\n    a = 4\n    a = 5\n  end\n";
        let outer_body = "  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n";
        let src = format!("class Outer\n{inner}{outer_body}end");
        // Outer counts 5 (its own lines); Inner counts 5. Neither exceeds 5.
        assert!(run(&src, 5, false).lengths.is_empty());
        let src6 = format!("class Outer\n{inner}{outer_body}  a = 6\nend");
        assert_eq!(run(&src6, 5, false).lengths, vec![6]);
    }

    // The inner-range subtraction shifts with the sampling (probed [7/5]): a
    // blank line right after the inner `end` samples the line below it.
    #[test]
    fn inner_class_blank_after_end_off_by_one() {
        let src = "class Outer\n  class Inner\n    x = 1\n  end\n\n  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\nend";
        assert_eq!(run(src, 5, false).lengths, vec![7]);
    }

    // A namespace-only body (one classlike statement) counts 0.
    #[test]
    fn namespace_class_counts_zero() {
        let src = "class C\n  module M\n    a = 1\n    a = 2\n    a = 3\n    a = 4\n    a = 5\n    a = 6\n    a = 7\n  end\nend";
        assert!(run(src, 5, false).lengths.is_empty());
    }

    // Heredoc content lines are sampled per line: blanks and comment-looking
    // lines inside the heredoc body are skipped (probed: no offense at 5).
    #[test]
    fn heredoc_content_lines_sampled_per_line() {
        let src = "class Test\n  x = <<~H\n    a\n\n    # looks like comment\n    b\n  H\n  y = 1\nend";
        assert!(run(src, 5, false).lengths.is_empty());
        assert_eq!(run(src, 4, false).lengths, vec![5]);
    }

    // A multi-line superclass expression's continuation lines count (probed).
    #[test]
    fn multiline_superclass_lines_count() {
        let src = "class A < Struct.new(:a,\n                     :b,\n                     :c)\n  x = 1\n  x = 2\n  x = 3\nend";
        assert!(run(src, 5, false).lengths.is_empty());
        assert_eq!(run(src, 4, false).lengths, vec![5]);
    }

    // `class << self` fires at the toplevel and inside a module, but not
    // inside a class (stock's `each_ancestor(:class)` check).
    #[test]
    fn sclass_ancestor_check() {
        let body = "  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\n";
        let top = format!("class << self\n{body}end");
        let got = run(&top, 5, false);
        assert_eq!(got.lengths, vec![6]);
        assert_eq!(got.sclass, vec![true]);

        let in_module = format!("module M\n  class << self\n  {body}  end\nend");
        let got = run(&in_module, 5, false);
        assert_eq!(got.lengths, vec![6]);
        assert_eq!(got.sclass, vec![true]);

        let in_class = format!("class C\n  class << self\n  {body}  end\nend");
        let got = run(&in_class, 5, false);
        // Only the outer class offends ([8/5]); the sclass is suppressed.
        assert_eq!(got.lengths, vec![8]);
        assert_eq!(got.sclass, vec![false]);
    }

    // An sclass nested in one another still fires for both (no :class
    // ancestor) — probed [8/5] + [6/5].
    #[test]
    fn sclass_in_sclass_fires_twice() {
        let src = "class << self\n  class << self\n    a = 1\n    a = 2\n    a = 3\n    a = 4\n    a = 5\n    a = 6\n  end\nend";
        assert_eq!(run(src, 5, false).lengths, vec![8, 6]);
    }

    // casgn: `Foo = Class.new do ... end` offends on the call-with-block.
    #[test]
    fn casgn_class_new() {
        let body = "  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\n";
        let src = format!("Foo = Class.new do\n{body}end");
        let got = run(&src, 5, false);
        assert_eq!(got.lengths, vec![6]);
        assert_eq!(got.ranges, vec![(6, src.len())]);
        // `::Class.new`, `Struct.new(...)`, brace / numbered / it blocks.
        for head in [
            "Foo = ::Class.new do",
            "Foo = Struct.new(:foo, :bar) do",
            "Foo = Class.new(1) do",
            "Foo = Class.new() do",
        ] {
            let src = format!("{head}\n{body}end");
            assert_eq!(run(&src, 5, false).lengths, vec![6], "{head}");
        }
        let brace = format!("Foo = Struct.new(:a) {{\n{body}}}");
        assert_eq!(run(&brace, 5, false).lengths, vec![6]);
        let numbered = "Foo = Class.new do\n  a(_1)\n  b(_1)\n  c(_1)\n  d(_1)\n  e(_1)\n  f(_1)\nend";
        assert_eq!(run(numbered, 5, false).lengths, vec![6]);
    }

    // casgn variants: scoped constant, `||=`, `&&=`, `+=`, masgn.
    #[test]
    fn casgn_assignment_forms() {
        let body = "  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\n";
        for head in [
            "Foo::Bar = Class.new do",
            "Foo ||= Struct.new(:foo, :bar) do",
            "Foo &&= Class.new do",
            "Foo += Class.new do",
            "Foo, Bar = Struct.new(:foo, :bar) do",
            "Foo = Bar = Class.new do",
        ] {
            let src = format!("{head}\n{body}end");
            assert_eq!(run(&src, 5, false).lengths, vec![6], "{head}");
        }
    }

    // Non-matching casgn values produce nothing.
    #[test]
    fn casgn_non_matching_values() {
        for src in [
            "X = Y = Z = do_something",
            "Foo = Class.new",
            "Foo = Class.new(&blk)",
            "Foo = Klass.new do\n  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\nend",
            "Foo::Class.new do\n  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\nend",
            "for FOO in [1] do end",
            "begin\nrescue => FOO\nend",
        ] {
            assert!(run(src, 5, false).lengths.is_empty(), "{src}");
        }
    }

    // `Foo = class Bar ... end`: only the `on_class` candidate is emitted
    // (stock's casgn duplicate is deduped by `add_offense`).
    #[test]
    fn casgn_plain_class_value_single_candidate() {
        let src = "Foo = class Bar\n  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\nend";
        let got = run(src, 5, false);
        assert_eq!(got.lengths, vec![6]);
        assert_eq!(got.ranges, vec![(6, src.len())]);
    }

    // `Foo = class << self` fires via casgn even inside a class (probed); at
    // the toplevel both hooks fire with the same range (wrapper dedups).
    #[test]
    fn casgn_sclass_value() {
        let body = "  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\n";
        let nested = format!("class Outer\n  FOO = class << self\n  {body}  end\nend");
        let got = run(&nested, 5, false);
        // Outer class [8/5] + the casgn sclass [6/5].
        assert_eq!(got.lengths, vec![8, 6]);
        assert_eq!(got.sclass, vec![false, true]);

        let top = format!("Foo = class << self\n{body}end");
        let got = run(&top, 5, false);
        // casgn + on_sclass with identical ranges (add_offense dedups).
        assert_eq!(got.lengths, vec![6, 6]);
        assert_eq!(got.ranges[0], got.ranges[1]);
    }

    // A `class << self` inside a casgn constructor block has no :class
    // ancestor, so both the block and the sclass offend (probed).
    #[test]
    fn sclass_in_casgn_block_fires() {
        let src = "Foo = Class.new do\n  class << self\n    a = 1\n    a = 2\n    a = 3\n    a = 4\n    a = 5\n    a = 6\n  end\nend";
        let got = run(src, 5, false);
        assert_eq!(got.lengths, vec![8, 6]);
        assert_eq!(got.sclass, vec![false, true]);
    }

    // CountAsOne folds inside a class body (vendor array case).
    #[test]
    fn count_as_one_array() {
        let src = "class Test\n  a = 1\n  a = [\n    2,\n    3,\n    4,\n    5\n  ]\nend";
        assert!(run_fold(src, 5, false, &["array"]).lengths.is_empty());
        assert_eq!(run(src, 5, false).lengths, vec![7]);
    }

    // CountAsOne method_call on a casgn constructor: the multi-line
    // `Struct.new(...)` send part folds too (probed: 6 body lines fold to 5).
    #[test]
    fn casgn_method_call_fold_includes_send() {
        let src = "Foo = Struct.new(:a,\n                 :b) do\n  x = 1\n  x = 2\n  x = 3\n  x = 4\n  x = 5\n  x = 6\nend";
        assert!(run_fold(src, 5, false, &["method_call"]).lengths.is_empty());
        assert_eq!(run_fold(src, 4, false, &["method_call"]).lengths, vec![5]);
        assert_eq!(run(src, 5, false).lengths, vec![6]);
    }

    // A nested class inside a casgn constructor block counts its source
    // lines (body-based measure has no inner-class exclusion) — probed.
    #[test]
    fn casgn_block_counts_nested_class_lines() {
        let src = "Foo = Class.new do\n  class Bar\n    y = 1\n    y = 2\n  end\n  x = 1\n  x = 2\nend";
        assert_eq!(run(src, 5, false).lengths, vec![6]);
    }

    // Boundary around the classlike fast reject: interior span == max is
    // skipped, one more line is measured and reported.
    #[test]
    fn classlike_span_boundary_at_max() {
        let src = "class Test\n  a = 1\n  a = 2\n  a = 3\nend";
        assert!(run(src, 3, false).lengths.is_empty());
        assert_eq!(run(src, 2, false).lengths, vec![3]);
    }
}
