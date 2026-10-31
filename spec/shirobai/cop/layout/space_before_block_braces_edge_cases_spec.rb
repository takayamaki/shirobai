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
#     range starts at the newline. Since 1.92.2 the correction also removes
#     the backslash and the spaces, tabs and backslashes before it, and does
#     nothing when a heredoc body sits between the `\` and the brace;
#   - CRLF sources take the standalone path on `buffer.source`, so the
#     offense range lines up with the CR-stripped buffer.
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

  context "with a brace after a backslash continuation" do
    let(:cfg) do
      config_with(
        { "EnforcedStyle" => "no_space" },
        "Style/BlockDelimiters" => { "EnforcedStyle" => "always_braces" }
      )
    end

    it "removes the backslash and the spaces before it" do
      src = "foo.bar \\\n  { |x| x }\n"
      expect_correction_edits_parity(*klasses, src, cfg)
      corrected = expect_autocorrect_parity(*klasses, src, cfg)
      expect(corrected).to eq("foo.bar{ |x| x }\n")
    end

    it "removes a run of tabs and spaces before the continuation" do
      src = "foo.bar\t \t\\\n  { |x| x }\n"
      expect_correction_edits_parity(*klasses, src, cfg)
      expect_autocorrect_parity(*klasses, src, cfg)
    end

    it "does not correct when a heredoc body sits before the brace" do
      src = "foo(<<~X) \\\n  a\nX\n  { baz }\n"
      expect_lint_parity(*klasses, src, cfg)
      expect(correction_edits(klasses.first, src, cfg).flat_map(&:last)).to be_empty
      expect(correction_edits(klasses.last, src, cfg).flat_map(&:last)).to be_empty
    end

    it "corrects on a CRLF source" do
      src = "a\r\nfoo(1,     x) \\\r\n  { baz }\r\n"
      expect_lint_parity(*klasses, src, cfg)
      expect_correction_edits_parity(*klasses, src, cfg)
      corrected = expect_autocorrect_parity(*klasses, src, cfg)
      expect(corrected).to eq("a\nfoo(1,     x){ baz }\n")
    end

    it "keeps char offsets after a non-ASCII prefix" do
      src = "# あいう\nfoo(\"日本\") \\\n  { baz }\n"
      expect_lint_parity(*klasses, src, cfg)
      expect_correction_edits_parity(*klasses, src, cfg)
      corrected = expect_autocorrect_parity(*klasses, src, cfg)
      expect(corrected).to eq("# あいう\nfoo(\"日本\"){ baz }\n")
    end

    it "keeps char offsets on a non-ASCII CRLF source" do
      src = "# あいう\r\nfoo(\"日本\") \\\r\n  { baz }\r\n"
      expect_correction_edits_parity(*klasses, src, cfg)
      expect_autocorrect_parity(*klasses, src, cfg)
    end
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
