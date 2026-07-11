//! `Metrics/MethodLength`.
//!
//! Drop-in reimplementation of RuboCop's `Metrics/MethodLength`, which counts
//! the body length of every method definition and flags those exceeding `Max`
//! (default 10). Method bodies are measured by the shared
//! [`CodeLength`](super::code_length::CodeLength) calculator (same machinery as
//! `Metrics/BlockLength`), with `CountComments` / `CountAsOne` honored.
//!
//! Definitions covered, mirroring stock's `on_def` / `on_defs` / `on_block`
//! (aliased to `on_numblock` / `on_itblock`):
//!
//! - `def` / `def self.` (incl. endless methods).
//! - `define_method :name do ... end` blocks. Stock's `node.method?(:define_method)`
//!   gate ignores the receiver and the argument type, so `foo.define_method`,
//!   numbered/`it` blocks, brace blocks and a non-literal name argument all count.
//!   A `define_method` block with *no* argument makes stock raise (the CLI
//!   swallows it into a cop error, not an offense), so it is skipped here.
//!
//! `AllowedMethods` / `AllowedPatterns` filtering stays on the Ruby side (the
//! wrapper has the exact symbol/regexp semantics). Stock only applies the
//! allow-filter to a `define_method` whose name argument is a basic literal, so
//! a candidate carries a `filterable` flag the wrapper respects. Every quirk was
//! verified against a stock probe (`.tmp/2026-06-14/method-length/`).

use ruby_prism::{Node, Visit, visit_call_node, visit_def_node};

use super::code_length::{CodeLength, Fold};

/// A method whose body length exceeds `Max`.
pub struct MethodLengthCandidate {
    pub start_offset: usize,
    pub end_offset: usize,
    /// End of the offense head (method name for `def`, block opening for
    /// `define_method`), used by the LSP location mode.
    pub head_end: usize,
    pub length: usize,
    /// The method name for the allow-filter. For a `define_method` whose name
    /// argument is not a basic literal this is empty and `filterable` is false.
    pub name: String,
    /// Whether the Ruby wrapper should apply the `AllowedMethods` /
    /// `AllowedPatterns` filter to this candidate.
    pub filterable: bool,
}

pub fn check_method_length(
    source: &[u8],
    max: usize,
    count_comments: bool,
    count_as_one: &[String],
) -> Vec<MethodLengthCandidate> {
    let mut finder = build_rule(source, max, count_comments, count_as_one);
    super::parse_cache::with_parsed(source, |_source, node| finder.visit(node));
    finder.out
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
    pub(crate) out: Vec<MethodLengthCandidate>,
}

impl Finder<'_> {
    fn push(
        &mut self,
        start: usize,
        end: usize,
        head_end: usize,
        length: usize,
        name: String,
        filterable: bool,
    ) {
        self.out.push(MethodLengthCandidate {
            start_offset: start,
            end_offset: end,
            head_end,
            length,
            name,
            filterable,
        });
    }

    /// `on_def` / `on_defs`: measure the `def` body. An empty-bodied method has
    /// length 0 and never exceeds a non-negative `Max`.
    fn process_def(&mut self, node: &ruby_prism::DefNode<'_>) {
        if self.calc.cannot_exceed(node.body().as_ref(), self.max) {
            return;
        }
        let length = self.calc.body_length(node.body());
        if length > self.max {
            let loc = node.location();
            let name = String::from_utf8_lossy(node.name().as_slice()).into_owned();
            self.push(
                loc.start_offset(),
                loc.end_offset(),
                node.name_loc().end_offset(),
                length,
                name,
                true,
            );
        }
    }

    /// `on_block` for `define_method`: stock's `node.method?(:define_method)`
    /// gate (receiver-agnostic, argument-type-agnostic). A no-argument
    /// `define_method` block no longer raises in stock (rubocop#15404 made the
    /// name-argument lookup nil-safe: `method_name&.basic_literal?`), so it is
    /// measured too — just never filterable by `AllowedMethods`.
    fn process_call(&mut self, node: &ruby_prism::CallNode<'_>) {
        if node.name().as_slice() != b"define_method" {
            return;
        }
        let Some(block) = node.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        if self.calc.cannot_exceed(block.body().as_ref(), self.max) {
            return;
        }
        let length = self.calc.body_length(block.body());
        if length <= self.max {
            return;
        }
        // `allowed?` only runs when the name argument is present and a basic
        // literal; map that to a filterable name, else mark the candidate
        // unfilterable (a missing / dynamic / interpolated name).
        let first = node.arguments().and_then(|a| a.arguments().iter().next());
        let (name, filterable) = match first {
            Some(first) => basic_literal_name(&first),
            None => (String::new(), false),
        };
        let loc = node.location();
        self.push(
            loc.start_offset(),
            loc.end_offset(),
            block.opening_loc().end_offset(),
            length,
            name,
            filterable,
        );
    }
}

/// The `method_name.value` stock filters on when `method_name.basic_literal?`.
/// Returns `(name, true)` for a symbol / string / integer literal, else
/// `("", false)` (a dynamic/interpolated/variable name stock never filters).
fn basic_literal_name(node: &Node<'_>) -> (String, bool) {
    if let Some(sym) = node.as_symbol_node() {
        // A plain symbol literal; `:"#{x}"` is an InterpolatedSymbolNode and
        // does not reach here, matching `basic_literal?`.
        return (String::from_utf8_lossy(sym.unescaped()).into_owned(), true);
    }
    if let Some(s) = node.as_string_node() {
        return (String::from_utf8_lossy(s.unescaped()).into_owned(), true);
    }
    if let Some(i) = node.as_integer_node() {
        // Stock compares `value.to_s`; an integer-literal method name is
        // pathological but faithful. The literal source equals its `to_s` for
        // a plain decimal integer (the only basic-literal integer arg form).
        return (
            String::from_utf8_lossy(i.location().as_slice()).into_owned(),
            true,
        );
    }
    (String::new(), false)
}

impl<'pr> Visit<'pr> for Finder<'_> {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        self.process_def(node);
        visit_def_node(self, node);
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.process_call(node);
        visit_call_node(self, node);
    }
}

/// Shared-walk driver. The generic branch hook fires for every `DefNode` /
/// `CallNode` the typed visits see except the `CallNode` reached through
/// `MatchWriteNode`'s concretely-typed `call` field — an `=~` operator call,
/// which is never a `define_method`, so `process_call` skips it anyway.
impl super::dispatch::Rule for Finder<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_CALL
                    | Interest::ENTER_DEF,
        )
    }
    
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(def) = node.as_def_node() {
            self.process_def(&def);
        } else if let Some(call) = node.as_call_node() {
            self.process_call(&call);
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
        names: Vec<String>,
        filterable: Vec<bool>,
    }

    fn run(source: &str, max: usize, count_comments: bool) -> Got {
        run_fold(source, max, count_comments, &[])
    }

    fn run_fold(source: &str, max: usize, count_comments: bool, fold: &[&str]) -> Got {
        let fold: Vec<String> = fold.iter().map(|s| s.to_string()).collect();
        let c = check_method_length(source.as_bytes(), max, count_comments, &fold);
        Got {
            ranges: c.iter().map(|o| (o.start_offset, o.end_offset)).collect(),
            lengths: c.iter().map(|o| o.length).collect(),
            names: c.iter().map(|o| o.name.clone()).collect(),
            filterable: c.iter().map(|o| o.filterable).collect(),
        }
    }

    // Typical: a def over the limit.
    #[test]
    fn long_def() {
        let src = "def m\n  a = 1\n  a = 2\n  a = 3\nend";
        let got = run(src, 2, false);
        assert_eq!(got.lengths, vec![3]);
        assert_eq!(got.ranges, vec![(0, src.len())]);
        assert_eq!(got.names, vec!["m"]);
        assert_eq!(got.filterable, vec![true]);
    }

    // A def at exactly the limit is not flagged.
    #[test]
    fn def_at_limit() {
        assert!(run("def m\n  a = 1\n  a = 2\nend", 2, false).lengths.is_empty());
    }

    // Blank lines are not counted.
    #[test]
    fn blank_lines_excluded() {
        assert!(
            run("def m\n  a = 1\n\n\n  a = 4\nend", 2, false)
                .lengths
                .is_empty()
        );
    }

    // Comments are excluded by default, counted with CountComments.
    #[test]
    fn comments() {
        let src = "def m\n  a = 1\n  # c\n  # c2\n  a = 2\nend";
        assert!(run(src, 2, false).lengths.is_empty());
        assert_eq!(run(src, 2, true).lengths, vec![4]);
    }

    // Empty methods produce nothing.
    #[test]
    fn empty_def() {
        assert!(run("def m\nend", 0, false).lengths.is_empty());
    }

    // `def self.m` (a `defs`) counts.
    #[test]
    fn defs() {
        let got = run("def self.m\n  a = 1\n  a = 2\n  a = 3\nend", 2, false);
        assert_eq!(got.lengths, vec![3]);
        assert_eq!(got.names, vec!["m"]);
    }

    // Endless method body counts.
    #[test]
    fn endless() {
        let got = run("def m = (a = 1\nb = 2\nc = 3)", 2, false);
        assert_eq!(got.lengths, vec![3]);
    }

    // A method whose body ends with a block counts the block's source lines.
    #[test]
    fn ends_with_block() {
        let src = "def m\n  something do\n    a = 2\n    a = 3\n    a = 4\n    a = 5\n  end\nend";
        assert_eq!(run(src, 5, false).lengths, vec![6]);
    }

    // `define_method` with a symbol name counts and is filterable.
    #[test]
    fn define_method_symbol() {
        let got = run("define_method(:m) do\n  a = 1\n  a = 2\n  a = 3\nend", 2, false);
        assert_eq!(got.lengths, vec![3]);
        assert_eq!(got.names, vec!["m"]);
        assert_eq!(got.filterable, vec![true]);
    }

    // `define_method` with a string name is filterable.
    #[test]
    fn define_method_string() {
        let got = run("define_method('m') do\n  a = 1\n  a = 2\n  a = 3\nend", 2, false);
        assert_eq!(got.names, vec!["m"]);
        assert_eq!(got.filterable, vec![true]);
    }

    // A receiver does not stop the gate (stock's `method?` ignores it).
    #[test]
    fn define_method_with_receiver() {
        let got = run(
            "foo.define_method(:m) do\n  a = 1\n  a = 2\n  a = 3\nend",
            2,
            false,
        );
        assert_eq!(got.lengths, vec![3]);
    }

    // A non-literal name argument is unfilterable.
    #[test]
    fn define_method_dynamic_name() {
        let got = run(
            "define_method(name) do\n  a = 1\n  a = 2\n  a = 3\nend",
            2,
            false,
        );
        assert_eq!(got.lengths, vec![3]);
        assert_eq!(got.filterable, vec![false]);
        assert_eq!(got.names, vec![""]);
    }

    // A dynamic-symbol name (`:"#{x}="`) is unfilterable too.
    #[test]
    fn define_method_dsym_name() {
        let got = run(
            "define_method(:\"a#{x}\") do\n  a = 1\n  a = 2\n  a = 3\nend",
            2,
            false,
        );
        assert_eq!(got.lengths, vec![3]);
        assert_eq!(got.filterable, vec![false]);
    }

    // A `define_method` block with no argument is measured (rubocop#15404 made
    // stock nil-safe instead of raising), and is never filterable.
    #[test]
    fn define_method_no_args_measured() {
        let got = run("define_method do\n  a = 1\n  a = 2\n  a = 3\nend", 2, false);
        assert_eq!(got.lengths, vec![3]);
        assert_eq!(got.filterable, vec![false]);
    }

    // A brace `define_method` block also counts.
    #[test]
    fn define_method_brace() {
        let got = run("define_method(:m) {\n  a = 1\n  a = 2\n  a = 3\n}", 2, false);
        assert_eq!(got.lengths, vec![3]);
    }

    // Numbered- and it-parameter blocks count.
    #[test]
    fn define_method_numblock_itblock() {
        assert_eq!(
            run("define_method(:m) do\n  _1\n  a = 2\n  a = 3\nend", 2, false).lengths,
            vec![3]
        );
        assert_eq!(
            run("define_method(:m) do\n  it\n  a = 2\n  a = 3\nend", 2, false).lengths,
            vec![3]
        );
    }

    // CountAsOne folds an array.
    #[test]
    fn count_as_one_array() {
        let src = "def m\n  a = 1\n  x = [\n    2,\n    3\n  ]\nend";
        assert!(run_fold(src, 2, false, &["array"]).lengths.is_empty());
        assert_eq!(run(src, 2, false).lengths, vec![5]);
    }

    // CountAsOne folds a heredoc, and the no-fold case extends to the heredoc end.
    #[test]
    fn count_as_one_heredoc() {
        let src = "def m\n  x = <<~H\n    l1\n    l2\n    l3\n  H\nend";
        // No fold: x=<<H, l1, l2, l3, H => 5 lines.
        assert_eq!(run(src, 1, false).lengths, vec![5]);
        assert!(run_fold(src, 1, false, &["heredoc"]).lengths.is_empty());
    }

    // CountAsOne folds a braceless keyword-hash argument (omit_length).
    #[test]
    fn count_as_one_braceless_hash() {
        let src = "def m\n  a = 1\n  foo(\n    x: 1,\n    y: 2\n  )\nend";
        assert!(run_fold(src, 2, false, &["hash"]).lengths.is_empty());
    }

    // Nested methods are each counted (def inside a define_method block).
    #[test]
    fn nested_methods() {
        let src = "define_method(:outer) do\n  a = 1\n  def inner\n    b = 1\n    b = 2\n    b = 3\n  end\n  a = 2\nend";
        let got = run(src, 2, false);
        // The outer block (8 body lines) and the inner def (3) both exceed 2.
        assert_eq!(got.lengths.len(), 2);
    }

    // The fast reject (span <= max and no `<<` in the body) must not hide a
    // heredoc that extends the measured lines past the body's own span: the
    // def body is one physical line, but the heredoc content pushes the count
    // to 5 (marker line + 3 body lines + closing delimiter line).
    #[test]
    fn heredoc_extension_still_measured_when_span_fits_max() {
        let src = "def m\n  x = <<~S\n    a\n    b\n    c\n  S\nend";
        let got = run(src, 3, false);
        assert_eq!(got.lengths, vec![5]);
        assert!(run(src, 5, false).lengths.is_empty());
    }

    // Boundary around the fast reject: a 3-line body is skipped at max=3 and
    // measured (and reported) at max=2.
    #[test]
    fn span_boundary_at_max() {
        let src = "def m\n  a = 1\n  a = 2\n  a = 3\nend";
        assert!(run(src, 3, false).lengths.is_empty());
        assert_eq!(run(src, 2, false).lengths, vec![3]);
    }
}
