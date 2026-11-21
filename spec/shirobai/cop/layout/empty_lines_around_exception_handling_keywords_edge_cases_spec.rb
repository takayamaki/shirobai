# frozen_string_literal: true

require "spec_helper"

# Edge cases for the 1.92 hand-off to Layout/EmptyLinesAroundBlockBody
# (rubocop#15814): the blank line after the last keyword of a block body
# is left to that cop only when it is enabled with `empty_lines`. The
# vendor spec covers the enabled case; these pin the disabled and
# `no_empty_lines` cases, lambdas, `ensure`, a non-blank line before the
# block end, and def bodies (never handed off).
RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundExceptionHandlingKeywords do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::EmptyLinesAroundExceptionHandlingKeywords,
    Shirobai::Cop::Layout::EmptyLinesAroundExceptionHandlingKeywords
  ]

  def ehk_config(block_cop)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Layout/EmptyLinesAroundBlockBody" => block_cop }, "(test)"),
      "(test)"
    )
  end

  source = <<~RUBY
    foo do
      bar
    rescue

    end
    lambda do
      bar
    ensure
    \t
    end
    foo do
      bar
    rescue

      baz

    end
    def m
      bar
    rescue

    end
    foo { bar rescue nil

    }
  RUBY

  [
    { "EnforcedStyle" => "empty_lines" },
    { "Enabled" => false, "EnforcedStyle" => "empty_lines" },
    { "EnforcedStyle" => "no_empty_lines" }
  ].each do |block_cop|
    it "matches stock with Layout/EmptyLinesAroundBlockBody #{block_cop}" do
      cfg = ehk_config(block_cop)
      expect_lint_parity(*klasses, source, cfg)
      expect_autocorrect_parity(*klasses, source, cfg)
    end
  end

  # 1.92.3: the blank line before the first keyword of a block body is also
  # left to the block body cop when only blank lines sit between the body
  # start line (after multiline arguments, not for numbered / `it` / empty
  # `||` parameters) and the keyword (rubocop#16262); `it` blocks are now
  # checked (rubocop#16325; the vendor spec pins those under Ruby 3.4).
  beginning_source = <<~RUBY
    foo do

    rescue
      x
    end
    foo do |a,
           b|
    \t
    ensure
      x
    end
    foo do ||

    rescue
    end
    foo do
      _1

    rescue
    end
    foo do

      it
    rescue

    end
    -> (a,
        b) do

    rescue
    end
    foo do # c

    rescue
    end
  RUBY

  [
    { "EnforcedStyle" => "empty_lines" },
    { "Enabled" => false, "EnforcedStyle" => "empty_lines" },
    { "EnforcedStyle" => "no_empty_lines" }
  ].each do |block_cop|
    it "matches stock at the block body beginning with Layout/EmptyLinesAroundBlockBody #{block_cop}" do
      cfg = ehk_config(block_cop)
      expect_lint_parity(*klasses, beginning_source, cfg)
      expect_autocorrect_parity(*klasses, beginning_source, cfg)
    end
  end
end

# CRLF files: `buffer.source` is LF-normalized while `raw_source` keeps the
# `\r`. The shared walk scans `buffer.source`, so its byte offsets line up
# with the parser-gem buffer.
RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundExceptionHandlingKeywords, "CRLF source" do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::EmptyLinesAroundExceptionHandlingKeywords,
    Shirobai::Cop::Layout::EmptyLinesAroundExceptionHandlingKeywords
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "reports and removes the same blank lines as stock in a CRLF file" do
    source = "begin\r\n  foo\r\n\r\nrescue\r\n\r\n  bar\r\nensure\r\n\r\n  baz\r\nend\r\n"
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end
end
