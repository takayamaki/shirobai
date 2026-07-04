# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceInsideHashLiteralBraces`.
#
# The vendor spec covers the three styles end-to-end, but several behaviours
# the token-free reconstruction depends on were pinned by stock probing only:
#
#   - a comment directly after `{` on the SAME line suppresses the left check
#     (stock's `return if token2.comment?` fires before any line test);
#   - a `\`-newline continuation between `{` and the first token makes the
#     next token sit on another line, so the left check is skipped even
#     though the byte after `{` is a space;
#   - under `compact`, the same-brace test on the right compares TOKEN types:
#     a `%w{...}` closer is a `tSTRING_END` (not same — space still
#     expected), while a brace block's or a nested hash's `}` is a `tRCURLY`
#     (same — space collapsed);
#   - `{ }` under the `no_space` empty style is hit by BOTH the left-brace
#     check and the whitespace-only check with the identical range;
#     `add_offense` location dedup must leave exactly one offense;
#   - a multiline empty hash `{\n}` fires only the whitespace-only check
#     (`no_space` empty style) and is corrected to `{}`; under the `space`
#     empty style it is clean;
#   - `ADT[a: 1]` hash patterns delimit with brackets, not braces, so the
#     `tokens.first.left_brace?` guard skips them.
RSpec.describe Shirobai::Cop::Layout::SpaceInsideHashLiteralBraces do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::SpaceInsideHashLiteralBraces,
    Shirobai::Cop::Layout::SpaceInsideHashLiteralBraces
  ]

  let(:default_config) { RuboCop::ConfigLoader.default_configuration }

  def config_with(hash)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Layout/SpaceInsideHashLiteralBraces" => hash }, "(test)"),
      "(test)"
    )
  end

  it "skips the left check for a comment directly after { (space style)" do
    src = "h = { # comment\n  a: 1 }\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  it "skips the left check for a comment directly after { (no_space style)" do
    src = "h = { # comment\n  a: 1}\n"
    cfg = config_with("EnforcedStyle" => "no_space")
    expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, cfg)).to be_empty
  end

  it "skips the left check across a backslash continuation" do
    src = "h = { \\\n  a: 1}\n"
    cfg = config_with("EnforcedStyle" => "no_space")
    expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, cfg)).to be_empty
  end

  it "skips the left check when a space precedes the line break" do
    src = "h = { \n  a: 1}\n"
    cfg = config_with("EnforcedStyle" => "no_space")
    expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, cfg)).to be_empty
  end

  it "compact: a %w{...} closer is not a tRCURLY, space is still expected" do
    src = "h = {k => %w{a}}\ng = { k => %w{a} }\n"
    expect_autocorrect_parity(*klasses, src, config_with("EnforcedStyle" => "compact"))
  end

  it "compact: a brace block's } is a tRCURLY and collapses" do
    src = "h = { a: proc {} }\ng = { a: p { 1 } }\n"
    expect_autocorrect_parity(*klasses, src, config_with("EnforcedStyle" => "compact"))
  end

  it "compact: a nested hash's { after { collapses on the left" do
    src = "h = { {a: 1} => 2 }\n"
    expect_autocorrect_parity(*klasses, src, config_with("EnforcedStyle" => "compact"))
  end

  it "dedups the double-emitted { } empty offense" do
    src = "h = { }\n"
    offenses = expect_lint_parity(*klasses, src, default_config)
    expect(offenses.size).to eq(1)
    expect(expect_autocorrect_parity(*klasses, src, default_config)).to eq("h = {}\n")
  end

  it "corrects a multiline empty hash under the no_space empty style" do
    corrected = expect_autocorrect_parity(*klasses, "h = {\n}\n", default_config)
    expect(corrected).to eq("h = {}\n")
  end

  it "accepts a multiline empty hash under the space empty style" do
    src = "h = {\n}\n"
    cfg = config_with("EnforcedStyleForEmptyBraces" => "space")
    expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, cfg)).to be_empty
  end

  it "skips bracketed and parenthesized const hash patterns" do
    src = "case x\nin ADT[a: 1]\n  1\nin ADT(b: 2)\n  2\nend\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  it "checks bare hash patterns like literals" do
    src = "case x\nin {a: 1, b: {c: 2}}\n  1\nend\n"
    expect_autocorrect_parity(*klasses, src, default_config)
  end
end
