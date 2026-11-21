# frozen_string_literal: true

require "spec_helper"

# Edge cases for the 1.92 block body start line (rubocop#15953 / #16059):
# the body starts after `{` / `do` or after the block arguments, whichever
# ends later. The vendor spec covers `do` after a line continuation and
# multiline `|a, b|` arguments; these pin the parser `arguments?` corners
# (block-local-only `|;z|`, empty `||`, numbered parameters, multiline
# lambda parameters) and the leading-dot skip (rubocop#15697).
RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundBlockBody do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::EmptyLinesAroundBlockBody,
    Shirobai::Cop::Layout::EmptyLinesAroundBlockBody
  ]

  def block_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Layout/EmptyLinesAroundBlockBody" => { "EnforcedStyle" => style } }, "(test)"),
      "(test)"
    )
  end

  source = <<~RUBY
    foo(a,
        b) { |x,
              y|

      x
    }
    bar do |;z|

      z
    end
    baz { ||

      1
    }
    qux = ->(a,
             b) do

      a
    end
    foo \\
      do |x|

      x
    end
    items.each { _1

    }
    aa { bb { cc }
    .dd }
    aa {

      bb
      &.cc }
  RUBY

  %w[no_empty_lines empty_lines].each do |style|
    it "matches stock on argument and leading-dot corners (#{style})" do
      cfg = block_config(style)
      expect_lint_parity(*klasses, source, cfg)
      expect_autocorrect_parity(*klasses, source, cfg)
    end
  end
end

# The `empty_lines` insert arm of `EmptyLineCorrector` (rubocop#16138, 1.92.2):
# stock now replaces the newline before the offense line with `"\n\n"`
# instead of inserting a `"\n"` before it. The final bytes are the same, so
# only the edit list (`Offense#corrections`) tells the two apart.
RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundBlockBody, "empty_lines insert edits" do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::EmptyLinesAroundBlockBody,
    Shirobai::Cop::Layout::EmptyLinesAroundBlockBody
  ]

  let(:config) do
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Layout/EmptyLinesAroundBlockBody" => { "EnforcedStyle" => "empty_lines" } }, "(test)"),
      "(test)"
    )
  end

  it "builds the same edits as stock for a missing blank line at both ends" do
    source = "foo do\n  bar\nend\n"
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  it "builds the same edits as stock after a non-ASCII line and with CRLF" do
    source = "foo do\r\n  \"ほげ\"\r\n  bar\r\nend\r\n"
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  # 1.92.3 (rubocop#16305): under `empty_lines` no blank line is required at
  # an end the body shares with the opening (or closing) line. With
  # `rescue` / `ensure` clauses the parser body ends at the last clause, not
  # at the block `end`.
  [
    "foo do |x| bar\n  baz\nend\n",
    "foo {\n  bar\n  baz }\n",
    "foo do\n  a\nrescue\n  b end\n",
    "foo do\n  a\nrescue Foo,\n  Bar end\n",
    "foo do\n  a\nrescue => e then end\n",
    "foo do\n  a\nensure end\n",
    "foo do\n  a\nensure\n  b\nend\n",
    "foo do\n  a\nrescue\nelse end\n",
    "foo do\nrescue\n  a\nend\n",
    "foo do rescue\n  a\nend\n",
    "foo do ensure\nend\n",
    "-> { foo(<<~A) }\n  x\nA\n",
    "foo { bar(\n  1) }\n"
  ].each do |src|
    it "matches stock under empty_lines for #{src.inspect}" do
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
      expect_autocorrect_parity(*klasses, src, config)
    end
  end
end
