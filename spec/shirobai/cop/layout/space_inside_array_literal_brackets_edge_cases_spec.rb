# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceInsideArrayLiteralBrackets`.
#
# The vendor spec covers the styles broadly, but the trickiest behaviours of
# the token-free reconstruction were pinned by stock probing only:
#
#   - under `no_space`, `start_ok` is `next_to_comment?` regardless of the
#     line, while a space run after `[` offends even when a newline follows;
#   - `empty_brackets?` means adjacent TOKENS: whitespace and continuations
#     keep the brackets "empty" while a comment does not (and the
#     comment-only body produces no offense on any path);
#   - under `compact`, the multi-dimension test compares TOKEN types: `%w[a]`
#     closers (`tSTRING_END`) and `?]` character literals (`tCHARACTER`) are
#     not right brackets, while index-call closers (`h[0]`) are;
#   - the compact left offense next to a bare newline has a ZERO-WIDTH range
#     (the `[ \t]` run is empty) and its correction joins the lines;
#   - stock's `find_node_with_brackets` redirects array patterns to the
#     nearest `const_pattern` ancestor: a bare pattern nested under `ADT[...]`
#     duplicates the ancestor check (its own brackets are never checked!),
#     while `ADT(...)` hunts the FIRST `[` / LAST `]` tokens inside — a
#     mismatched pair when several sibling patterns are present, so only the
#     last `]`'s space offends.
RSpec.describe Shirobai::Cop::Layout::SpaceInsideArrayLiteralBrackets do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::SpaceInsideArrayLiteralBrackets,
    Shirobai::Cop::Layout::SpaceInsideArrayLiteralBrackets
  ]

  let(:default_config) { RuboCop::ConfigLoader.default_configuration }

  def config_with(hash)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Layout/SpaceInsideArrayLiteralBrackets" => hash }, "(test)"),
      "(test)"
    )
  end

  it "no_space: a comment after [ sets start_ok even on the same line" do
    src = "a = [ # c\n  1]\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  it "no_space: a space run before a newline still offends" do
    corrected = expect_autocorrect_parity(*klasses, "a = [ \n  1]\n", default_config)
    expect(corrected).to eq("a = [\n  1]\n")
  end

  it "space: a comment with no space after [ offends at the bracket" do
    src = "a = [# c\n  1 ]\n"
    expect_autocorrect_parity(*klasses, src, config_with("EnforcedStyle" => "space"))
  end

  it "keeps brackets empty across a backslash continuation" do
    src = "a = [\\\n]\n"
    corrected = expect_autocorrect_parity(*klasses, src, default_config)
    expect(corrected).to eq("a = []\n")
  end

  it "a comment between empty brackets blocks every path" do
    src = "a = [ # c\n]\n"
    [default_config,
     config_with("EnforcedStyle" => "space"),
     config_with("EnforcedStyle" => "compact"),
     config_with("EnforcedStyleForEmptyBrackets" => "space")].each do |cfg|
      expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, cfg)).to be_empty
    end
  end

  it "empty brackets across a bare newline offend per empty style" do
    expect_autocorrect_parity(*klasses, "a = [\n]\n", default_config)
    expect_autocorrect_parity(
      *klasses, "a = [\n]\n", config_with("EnforcedStyleForEmptyBrackets" => "space")
    )
  end

  it "compact: a %w[a] closer is not a right bracket" do
    src = "a = [x, %w[a]]\nb = [x, %w[a] ]\n"
    expect_autocorrect_parity(*klasses, src, config_with("EnforcedStyle" => "compact"))
  end

  it "compact: a ?] character literal is not a right bracket" do
    src = "g = [x, ?] ]\n"
    expect_autocorrect_parity(*klasses, src, config_with("EnforcedStyle" => "compact"))
  end

  it "compact: an index call closer is a right bracket" do
    src = "c = [x, h[0]]\n"
    expect_autocorrect_parity(*klasses, src, config_with("EnforcedStyle" => "compact"))
  end

  it "compact: nested array corrections compose per node" do
    src = "d = [ [1] ]\ne = [[1] ]\n"
    expect_autocorrect_parity(*klasses, src, config_with("EnforcedStyle" => "compact"))
  end

  it "compact: newline-adjacent left bracket has a zero-width offense and joins lines" do
    src = "f = [\n  [1], [2]]\n"
    corrected = expect_autocorrect_parity(*klasses, src, config_with("EnforcedStyle" => "compact"))
    expect(corrected).to eq("f = [[ 1 ], [ 2 ]]\n")
  end

  it "bracketed const pattern swallows the checks of bare patterns below" do
    src = "case v\nin ADT[[a, b ]]\n  1\nend\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  it "the same bare pattern without the const ancestor is checked" do
    src = "case v\nin [[a, b ]]\n  1\nend\n"
    expect_autocorrect_parity(*klasses, src, default_config)
  end

  it "bracketed const pattern checks its own pair only" do
    src = "case v\nin ADT[ i, [j ]]\n  1\nend\n"
    corrected = expect_autocorrect_parity(*klasses, src, default_config)
    expect(corrected).to eq("case v\nin ADT[i, [j ]]\n  1\nend\n")
  end

  it "parenthesized const pattern hunts the min/max bracket pair" do
    src = "case v\nin ADT([e, f ])\n  1\nend\n"
    expect_autocorrect_parity(*klasses, src, default_config)
  end

  it "parenthesized const pattern with siblings checks a mismatched pair" do
    src = "case v\nin ADT([g ], [h ])\n  1\nend\n"
    corrected = expect_autocorrect_parity(*klasses, src, default_config)
    expect(corrected).to eq("case v\nin ADT([g ], [h])\n  1\nend\n")
  end

  it "percent arrays and reference brackets never fire" do
    src = "a = %w[ x y ]\nb = %i[ x ]\nc[ 3]\nd[ foo ]\n"
    [default_config, config_with("EnforcedStyle" => "space"),
     config_with("EnforcedStyle" => "compact")].each do |cfg|
      expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, cfg)).to be_empty
    end
  end
end
