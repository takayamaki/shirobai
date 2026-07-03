# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceBeforeBlockBraces`.
#
# The vendor spec asserts the styles and the `config_to_allow_offenses` state
# machine, but a few behaviours were pinned by stock probing only:
#
#   - `node.multiline?` is `RuboCop::AST::BlockNode`'s override — it compares
#     the BRACE lines, not the whole node range. A call spanning two lines
#     with single-line braces is NOT multiline, so the
#     `Style/BlockDelimiters` conflict skip does not apply to it;
#   - the conflict skip reads the OTHER cop's `EnforcedStyle`: with a
#     non-`line_count_based` value the multiline no_space offense comes back;
#   - `{ }` is NOT "empty braces" for this cop (`empty_braces?` is
#     byte-adjacency), so it takes the non-empty path;
#   - the surrounding-space scan stops at a `\`-continuation: the offense
#     range starts at the newline and its removal splices the lines.
RSpec.describe Shirobai::Cop::Layout::SpaceBeforeBlockBraces do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::SpaceBeforeBlockBraces,
    Shirobai::Cop::Layout::SpaceBeforeBlockBraces
  ]

  let(:default_config) { RuboCop::ConfigLoader.default_configuration }

  def config_with(hash, extra = {})
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/SpaceBeforeBlockBraces" => hash }.merge(extra), "(test)"
      ),
      "(test)"
    )
  end

  it "multiline is brace-based: a two-line call with one-line braces fires" do
    src = "foo.map(a,\n        b) { |x| x }\n"
    cfg = config_with("EnforcedStyle" => "no_space")
    corrected = expect_autocorrect_parity(*klasses, src, cfg)
    expect(corrected).to eq("foo.map(a,\n        b){ |x| x }\n")
  end

  it "multiline braces are skipped under no_space + line_count_based" do
    src = "foo.bar { |x|\n  x\n}\n"
    cfg = config_with("EnforcedStyle" => "no_space")
    expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, cfg)).to be_empty
  end

  it "multiline braces fire when BlockDelimiters is not line_count_based" do
    src = "foo.bar { |x|\n  x\n}\n"
    cfg = config_with(
      { "EnforcedStyle" => "no_space" },
      "Style/BlockDelimiters" => { "EnforcedStyle" => "always_braces" }
    )
    expect_autocorrect_parity(*klasses, src, cfg)
  end

  it "treats { } as non-empty braces" do
    src = "7.times { }\n"
    cfg = config_with(
      "EnforcedStyle" => "no_space", "EnforcedStyleForEmptyBraces" => "space"
    )
    corrected = expect_autocorrect_parity(*klasses, src, cfg)
    expect(corrected).to eq("7.times{ }\n")
  end

  it "removes the newline run up to a backslash continuation" do
    src = "foo.bar \\\n  { |x| x }\n"
    cfg = config_with(
      { "EnforcedStyle" => "no_space" },
      "Style/BlockDelimiters" => { "EnforcedStyle" => "always_braces" }
    )
    corrected = expect_autocorrect_parity(*klasses, src, cfg)
    expect(corrected).to eq("foo.bar \\{ |x| x }\n")
  end

  it "handles super, numbered and it blocks and lambda braces" do
    src = "super{ 1 }\nfoo.map{ _1 }\nbar.map{ it }\n->(){ 1 }\n"
    expect_autocorrect_parity(*klasses, src, default_config)
  end

  it "ignores do..end blocks" do
    src = "x.each do |n|\n  n\nend\n"
    [default_config, config_with("EnforcedStyle" => "no_space")].each do |cfg|
      expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, cfg)).to be_empty
    end
  end
end
