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
