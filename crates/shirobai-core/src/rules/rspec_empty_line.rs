//! The RSpec "empty-line family": five cops that all wrap rubocop-rspec's
//! `EmptyLineSeparation` mixin — `RSpec/EmptyLineAfterExample`,
//! `RSpec/EmptyLineAfterExampleGroup`, `RSpec/EmptyLineAfterFinalLet`,
//! `RSpec/EmptyLineAfterHook`, `RSpec/EmptyLineAfterSubject`.
//!
//! The walk that produces these offenses lives in
//! [`rspec_dispatcher`](super::rspec_dispatcher): the single `RSpecDispatcherRule`
//! classifies every node against `RSpec/Language` once and feeds every RSpec
//! cop, including this family. This module owns only the family's result
//! types and the standalone (per-cop fallback) entry point, which is a thin
//! wrapper over the dispatcher rule.
//!
//! # What the mixin does (probed against stock rubocop-rspec 3.10.2)
//!
//! Every one of the five cops resolves a "concept" node (an example / example
//! group / final let / hook / subject) and calls
//! `missing_separating_line_offense(node)`:
//!
//! 1. `last_child?(node)` — return (no offense) unless the node's parser parent
//!    is a `:begin` (a multi-statement sequence) AND the node is not that
//!    sequence's last child. So a concept is only a candidate when it has a
//!    following sibling inside a statement sequence.
//! 2. `missing_separating_line` walks the comment lines directly after the
//!    node's `final_end_location` (heredoc-aware end), tracks the last enabled
//!    `# rubocop:enable` directive, and suppresses the offense when the line
//!    after the last such comment is blank.
//! 3. The offense location is the trimmed content of `final_end_line` (or the
//!    enabled-directive line when present); autocorrect inserts one `"\n"`
//!    after it.
//!
//! Steps 2 and 3 are pure `ProcessedSource` line/comment work, so the Ruby
//! wrappers replay the mixin verbatim (byte-for-byte parity guaranteed). The
//! Rust rule owns step 1 plus the heredoc-aware `final_end_line` and the
//! per-cop concept classification, and emits, per cop, one
//! `(final_end_line, method_name)` for every candidate that clears `last_child?`
//! and the one-liner allowances.
//!
//! # parser `:begin` recovery from prism (probed)
//!
//! prism has no `:begin`. A concept's parser parent is `:begin` (offense
//! eligible when multi-statement) in exactly these shapes, recovered from the
//! immediate frame the walk is under (`RSpecDispatcherRule::resolve`):
//!
//! - a nested body `StatementsNode` (block / def / class / module / if / while
//!   / ensure / else body) with `>= 2` children — always `:begin`;
//! - the transparent top-level `ProgramNode` statements with `>= 2` children;
//! - a `RescueNode`'s own body (`rescue ...; a; b`) with `>= 2` children —
//!   parser wraps it in `:begin`;
//! - a `BeginNode`'s MAIN statements (visited via `visit_statements_node`
//!   directly, so the frame is the `BeginNode` itself) — `:begin` ONLY when the
//!   begin has a `rescue`/`ensure` clause (`begin a; b rescue ... end`); a plain
//!   `begin a; b end` keeps `a`,`b` as direct `:kwbegin` children, which is NOT
//!   `begin_type?` (probed: no offense).
//!
//! Every other parent shape (a single-statement body, an assignment value, an
//! argument, ...) is not `:begin`, so the concept is its parent's last/only
//! child and never an offense.

pub use super::rspec_dispatcher::check_rspec_empty_line;

/// One empty-line-family offense: the concept's 1-based `final_end` line and
/// the concept's method name (for the per-cop message). The Ruby wrapper
/// runs stock's comment/blank walk from `final_end_line` and decides the
/// exact offense location + suppression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyLineOffense {
    pub final_end_line: usize,
    pub method_name: String,
}

/// Everything the five empty-line cops produced for one file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RSpecEmptyLineResult {
    pub example: Vec<EmptyLineOffense>,
    pub example_group: Vec<EmptyLineOffense>,
    pub final_let: Vec<EmptyLineOffense>,
    pub hook: Vec<EmptyLineOffense>,
    pub subject: Vec<EmptyLineOffense>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::rspec_language;
    use crate::rules::rspec_language::RSpecConfig;

    fn cfg() -> RSpecConfig {
        RSpecConfig::from_role_lists(&rspec_language::tests::default_role_lists()).unwrap()
    }

    /// `(final_end_line, method_name)` per offense of each cop.
    fn run(src: &str) -> RSpecEmptyLineResult {
        check_rspec_empty_line(src.as_bytes(), &cfg())
    }

    fn pairs(v: &[EmptyLineOffense]) -> Vec<(usize, &str)> {
        v.iter()
            .map(|o| (o.final_end_line, o.method_name.as_str()))
            .collect()
    }

    #[test]
    fn example_basic_multiline() {
        let src = "RSpec.describe Foo do\n  it 'a' do\n    x\n  end\n  it 'b' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(4, "it")]);
    }

    #[test]
    fn example_consecutive_one_liners_allowed_by_default() {
        let src = "RSpec.describe Foo do\n  it { one }\n  it { two }\nend\n";
        assert!(run(src).example.is_empty());
    }

    #[test]
    fn example_one_liner_flagged_when_next_is_not_an_example() {
        // single-line example followed by a multi-line example: not the
        // allowed consecutive-one-liner shape.
        let src = "RSpec.describe Foo do\n  it { one }\n  it 'b' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(2, "it")]);
    }

    #[test]
    fn example_disabled_allow_consecutive_flags_one_liners() {
        let mut c = cfg();
        c.example_allow_consecutive = false;
        let src = "RSpec.describe Foo do\n  it { one }\n  it { two }\nend\n";
        let res = check_rspec_empty_line(src.as_bytes(), &c);
        assert_eq!(pairs(&res.example), vec![(2, "it")]);
    }

    #[test]
    fn example_kwbegin_without_handler_is_not_begin() {
        let src = "RSpec.describe Foo do\n  begin\n    it 'a' do\n      x\n    end\n    it 'b' do\n      y\n    end\n  end\nend\n";
        assert!(run(src).example.is_empty());
    }

    #[test]
    fn example_kwbegin_with_rescue_is_begin() {
        let src = "RSpec.describe Foo do\n  begin\n    it 'a' do\n      x\n    end\n    it 'b' do\n      y\n    end\n  rescue\n    z\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(5, "it")]);
    }

    #[test]
    fn example_in_rescue_body_multi_is_begin() {
        let src = "RSpec.describe Foo do\n  work\nrescue\n  it 'a' do\n    x\n  end\n  it 'b' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(6, "it")]);
    }

    #[test]
    fn example_in_ensure_body_multi_is_begin() {
        let src = "RSpec.describe Foo do\n  work\nensure\n  it 'a' do\n    x\n  end\n  it 'b' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(6, "it")]);
    }

    #[test]
    fn example_three_multiline() {
        let src = "RSpec.describe Foo do\n  it 'a' do\n    x\n  end\n  it 'b' do\n    y\n  end\n  it 'c' do\n    z\n  end\nend\n";
        assert_eq!(pairs(&run(src).example), vec![(4, "it"), (7, "it")]);
    }

    #[test]
    fn example_single_is_last_child() {
        let src = "RSpec.describe Foo do\n  it 'a' do\n    x\n  end\nend\n";
        assert!(run(src).example.is_empty());
    }

    #[test]
    fn hook_consecutive_one_liner_chain() {
        // `before` chains to `after` (both one-liner hooks) => allowed;
        // `after` precedes `it` => flagged.
        let src = "RSpec.describe Foo do\n  before { a }\n  after { b }\n  it { c }\nend\n";
        assert_eq!(pairs(&run(src).hook), vec![(3, "after")]);
    }

    #[test]
    fn hook_numblock_fires() {
        let src = "RSpec.describe Foo do\n  before { _1 }\n  it 'x' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).hook), vec![(2, "before")]);
    }

    #[test]
    fn final_let_multi() {
        let src = "RSpec.describe Foo do\n  let(:a) { 1 }\n  let(:b) { 2 }\n  it 'x' do\n    y\n  end\nend\n";
        assert_eq!(pairs(&run(src).final_let), vec![(3, "let")]);
    }

    #[test]
    fn final_let_send_form() {
        let src = "describe 'x' do\n  let(:a) { 1 }\n  let(:b, &blk)\n  it 'y' do\n    z\n  end\nend\n";
        assert_eq!(pairs(&run(src).final_let), vec![(3, "let")]);
    }

    #[test]
    fn final_let_it_then_let_is_last_child() {
        let src = "RSpec.describe Foo do\n  it 'x' do\n    y\n  end\n  let(:a) { 1 }\nend\n";
        assert!(run(src).final_let.is_empty());
    }

    #[test]
    fn final_let_single_is_last_child() {
        let src = "RSpec.describe Foo do\n  let(:a) { 1 }\nend\n";
        assert!(run(src).final_let.is_empty());
    }

    #[test]
    fn subject_heredoc_final_end_follows_terminator() {
        let src = "RSpec.describe Foo do\n  subject(:obj) { described_class.new(<<~ARGS) }\n    a\n    b\n  ARGS\n  let(:foo) { bar }\nend\n";
        assert_eq!(pairs(&run(src).subject), vec![(5, "subject")]);
    }

    #[test]
    fn subject_top_level_is_not_inside_group() {
        let src = "subject(:obj) { described_class }\nlet(:foo) { bar }\n";
        assert!(run(src).subject.is_empty());
    }

    #[test]
    fn subject_inside_class_wrapped_group_is_excluded() {
        let src = "class Wrap\n  RSpec.describe Foo do\n    subject(:obj) { described_class }\n    let(:foo) { bar }\n  end\nend\n";
        assert!(run(src).subject.is_empty());
    }

    #[test]
    fn example_group_fires_on_shared_group() {
        let src = "RSpec.describe Foo do\n  shared_examples 'x' do\n    it { a }\n  end\n  describe '#bar' do\n    it { b }\n  end\nend\n";
        assert_eq!(pairs(&run(src).example_group), vec![(4, "shared_examples")]);
    }

    #[test]
    fn example_group_two_top_level_groups() {
        let src = "RSpec.describe Foo do\n  it { a }\nend\nRSpec.describe Bar do\n  it { b }\nend\n";
        assert_eq!(pairs(&run(src).example_group), vec![(3, "describe")]);
    }

    #[test]
    fn numblock_example_is_not_an_example() {
        // `it('a') { _1 }` is a numblock, so on_block never fires => no offense.
        let src = "RSpec.describe Foo do\n  it('a') { _1 }\n  it('b') { _1 }\nend\n";
        assert!(run(src).example.is_empty());
    }
}
