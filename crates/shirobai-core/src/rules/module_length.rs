//! `Metrics/ModuleLength`.
//!
//! Drop-in reimplementation of RuboCop's `Metrics/ModuleLength`, which
//! measures every module definition and flags those exceeding `Max` (default
//! 100). There is no autocorrect. Verified against a stock probe
//! (`.tmp/2026-07-02/class-module-length/`).
//!
//! Definitions covered, mirroring stock's hooks:
//!
//! - `on_module`: every `module` node, measured by the *classlike*
//!   line-number count ([`CodeLength::classlike_length`]) — inner
//!   class/module line ranges excluded, `class << self` content included,
//!   0 for a namespace-only body, and stock's one-line-down relevance
//!   sampling reproduced.
//! - `on_casgn`: **only** a direct `Foo = Module.new <block>` (the cop-local
//!   `module_definition?` pattern `(casgn nil? _ (any_block (send (const
//!   {nil? cbase} :Module) :new) ...))`). Unlike `ClassLength`'s
//!   `class_definition?`:
//!   - the casgn scope must be nil (`Foo::Bar = Module.new do..end` never
//!     fires — probed);
//!   - the `Module.new` send takes **no arguments** (`Module.new(1) do..end`
//!     never fires; empty parens `Module.new()` still match — probed);
//!   - `||=`/`&&=`/`+=`/masgn forms never fire (the pattern needs the
//!     expression on the casgn itself — probed).
//!
//!   Stock passes the *casgn* node to `check_code_length`: the measured body
//!   is the block's (`extract_body(:casgn)` recurses into the expression)
//!   and the offense range is `node.loc.name` — the bare constant name.

use ruby_prism::Node;

use super::block_length::top_level_const_name;
use super::code_length::{CodeLength, Fold};

/// A module definition whose measured length exceeds `Max`.
pub struct ModuleLengthCandidate {
    pub start_offset: usize,
    pub end_offset: usize,
    /// End of the offense head for the LSP location mode (the module name; a
    /// casgn offense is the name range already, so it equals `end_offset`).
    pub head_end: usize,
    pub length: usize,
}

pub fn check_module_length(
    source: &[u8],
    max: usize,
    count_comments: bool,
    count_as_one: &[String],
) -> Vec<ModuleLengthCandidate> {
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
        out: Vec::new(),
    }
}

pub(crate) struct Finder<'a> {
    max: usize,
    calc: CodeLength<'a>,
    pub(crate) out: Vec<ModuleLengthCandidate>,
}

impl Finder<'_> {
    /// `on_module`: measure classlike.
    fn process_module(&mut self, node: &Node<'_>) {
        if self.calc.classlike_cannot_exceed(node, self.max) {
            return;
        }
        let length = self.calc.classlike_length(node);
        if length > self.max {
            let loc = node.location();
            let head_end = node
                .as_module_node()
                .map_or(loc.end_offset(), |m| m.constant_path().location().end_offset());
            self.push(loc.start_offset(), loc.end_offset(), head_end, length);
        }
    }

    /// `on_casgn` with the cop-local `module_definition?` pattern.
    fn process_casgn(&mut self, node: &ruby_prism::ConstantWriteNode<'_>) {
        let value = node.value();
        let Some(call) = value.as_call_node() else { return };
        if call.name().as_slice() != b"new" || call.arguments().is_some() {
            return;
        }
        let Some(receiver) = call.receiver() else { return };
        if top_level_const_name(&receiver) != Some(b"Module") {
            return;
        }
        let Some(block) = call.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        if self.calc.cannot_exceed(block.body().as_ref(), self.max) {
            return;
        }
        let length = self.calc.casgn_call_length(&value, block.body());
        if length > self.max {
            let name = node.name_loc();
            self.push(
                name.start_offset(),
                name.end_offset(),
                name.end_offset(),
                length,
            );
        }
    }

    fn push(&mut self, start: usize, end: usize, head_end: usize, length: usize) {
        self.out.push(ModuleLengthCandidate {
            start_offset: start,
            end_offset: end,
            head_end,
            length,
        });
    }
}

/// Shared-walk driver. `enter` reacts only to `ModuleNode` (in
/// `ENTER_CLASS_MOD`) and `ConstantWriteNode` (in `ENTER_WRITE`); every other
/// kind falls through empty, `leave` is empty and the leaf/rescue hooks are
/// the defaults, so narrowing the interest mask to exactly those two classes
/// is equivalent to receiving every node.
impl super::dispatch::Rule for Finder<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(Interest::ENTER_CLASS_MOD | Interest::ENTER_WRITE)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if node.as_module_node().is_some() {
            self.process_module(node);
        } else if let Some(w) = node.as_constant_write_node() {
            self.process_casgn(&w);
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
    }

    fn run(source: &str, max: usize, count_comments: bool) -> Got {
        run_fold(source, max, count_comments, &[])
    }

    fn run_fold(source: &str, max: usize, count_comments: bool, fold: &[&str]) -> Got {
        let fold: Vec<String> = fold.iter().map(|s| s.to_string()).collect();
        let c = check_module_length(source.as_bytes(), max, count_comments, &fold);
        Got {
            ranges: c.iter().map(|o| (o.start_offset, o.end_offset)).collect(),
            lengths: c.iter().map(|o| o.length).collect(),
        }
    }

    // Typical: a module over the limit (vendor: 6 body lines at Max 5).
    #[test]
    fn long_module() {
        let src = "module Test\n  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\nend";
        let got = run(src, 5, false);
        assert_eq!(got.lengths, vec![6]);
        assert_eq!(got.ranges, vec![(0, src.len())]);
        assert!(run(src, 6, false).lengths.is_empty());
    }

    // Stock's relevance sampling is one line below each body line number
    // (probed [6/5] with a blank first body line).
    #[test]
    fn blank_first_body_line_off_by_one() {
        let src = "module Test\n\n  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\nend";
        assert_eq!(run(src, 5, false).lengths, vec![6]);
    }

    // Inner module/class line ranges are excluded from the outer count.
    #[test]
    fn inner_modules_and_classes_excluded() {
        let inner = "  class Inner\n    a = 1\n    a = 2\n    a = 3\n    a = 4\n    a = 5\n  end\n";
        let outer_body = "  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n";
        let src = format!("module Outer\n{inner}{outer_body}end");
        assert!(run(&src, 5, false).lengths.is_empty());
        let src6 = format!("module Outer\n{inner}{outer_body}  a = 6\nend");
        assert_eq!(run(&src6, 5, false).lengths, vec![6]);
    }

    // `class << self` content counts toward the module (vendor [8/5]).
    #[test]
    fn sclass_content_counts() {
        let src = "module Test\n  class << self\n    a = 1\n    a = 2\n    a = 3\n    a = 4\n    a = 5\n    a = 6\n  end\nend";
        assert_eq!(run(src, 5, false).lengths, vec![8]);
    }

    // A namespace-only body (one classlike statement) counts 0.
    #[test]
    fn namespace_module_counts_zero() {
        let src = "module M\n  class C\n    a = 1\n    a = 2\n    a = 3\n    a = 4\n    a = 5\n    a = 6\n    a = 7\n  end\nend";
        assert!(run(src, 5, false).lengths.is_empty());
    }

    // casgn: `Foo = Module.new do ... end` offends on the constant NAME.
    #[test]
    fn casgn_module_new() {
        let body = "  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\n";
        let src = format!("Foo = Module.new do\n{body}end");
        let got = run(&src, 5, false);
        assert_eq!(got.lengths, vec![6]);
        assert_eq!(got.ranges, vec![(0, 3)]);
        // `::Module.new`, empty parens, brace and numbered blocks all match.
        for head in [
            "Foo = ::Module.new do",
            "Foo = Module.new() do",
        ] {
            let src = format!("{head}\n{body}end");
            assert_eq!(run(&src, 5, false).lengths, vec![6], "{head}");
        }
        let numbered = "Foo = Module.new do\n  a(_1)\n  b(_1)\n  c(_1)\n  d(_1)\n  e(_1)\n  f(_1)\nend";
        assert_eq!(run(numbered, 5, false).ranges, vec![(0, 3)]);
        // Chained assignment fires on the inner casgn's name (probed).
        let chained = format!("Foo = Bar = Module.new do\n{body}end");
        assert_eq!(run(&chained, 5, false).ranges, vec![(6, 9)]);
    }

    // Forms the cop-local pattern does NOT match (all probed against stock).
    #[test]
    fn casgn_non_matching_forms() {
        let body = "  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\n";
        for head in [
            "Foo::Bar = Module.new do", // scoped casgn: `nil?` scope required
            "Foo = Module.new(1) do",   // the send must take no arguments
            "Foo ||= Module.new do",    // casgn under or_asgn has no expression
            "Foo &&= Module.new do",
            "Foo, Bar = Module.new do", // masgn casgn has no expression
            "Foo = NotModule.new do",
        ] {
            let src = format!("{head}\n{body}end");
            assert!(run(&src, 5, false).lengths.is_empty(), "{head}");
        }
        assert!(run("Foo = Module.new", 5, false).lengths.is_empty());
    }

    // CountAsOne folds inside a module body (vendor array case).
    #[test]
    fn count_as_one_array() {
        let src = "module Test\n  a = 1\n  a = [\n    2,\n    3,\n    4,\n    5\n  ]\nend";
        assert!(run_fold(src, 5, false, &["array"]).lengths.is_empty());
        assert_eq!(run(src, 5, false).lengths, vec![7]);
    }

    // CountAsOne method_call on the casgn form folds nothing extra here (the
    // send is one line) but the body foldables still collapse.
    #[test]
    fn casgn_count_as_one() {
        let src = "Foo = Module.new do\n  a = 1\n  a = [\n    2,\n    3,\n    4,\n    5\n  ]\nend";
        assert!(run_fold(src, 5, false, &["array"]).lengths.is_empty());
        assert_eq!(run(src, 5, false).lengths, vec![7]);
    }

    // Boundary around the classlike fast reject.
    #[test]
    fn classlike_span_boundary_at_max() {
        let src = "module Test\n  a = 1\n  a = 2\n  a = 3\nend";
        assert!(run(src, 3, false).lengths.is_empty());
        assert_eq!(run(src, 2, false).lengths, vec![3]);
    }
}
