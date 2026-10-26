# frozen_string_literal: true

require "spec_helper"

# Edge cases for the 1.92 tabs autocorrect (rubocop#15845): under
# `Layout/IndentationStyle: tabs` the `end` takes the anchor line's own
# leading whitespace, unless the anchor does not begin its line (then the
# old column shift applies). Covers each EnforcedStyleAlignWith with an
# anchor inside `[...]`, a do-line inside parenthesized arguments and a
# leading-dot `do` line, which the vendor spec does not.
RSpec.describe Shirobai::Cop::Layout::BlockAlignment do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::BlockAlignment,
    Shirobai::Cop::Layout::BlockAlignment
  ]

  source = "\tfoo do |v|\n\t\tv\n end\n" \
           "\t  bar do\n\t\tv\n end\n" \
           "\tx = [baz do\n\t\tv\n   end]\n" \
           "\ty = qux(a,\n\t\tb) do\n\t\tv\n\t\t end\n" \
           "\tz = foo\n\t\t.map do |v|\n\t\tv\n end\n"

  %w[either start_of_block start_of_line].each do |style|
    it "matches stock under tabs (#{style})" do
      cfg = RuboCop::ConfigLoader.merge_with_default(
        RuboCop::Config.new(
          {
            "Layout/IndentationStyle" => { "EnforcedStyle" => "tabs" },
            "Layout/BlockAlignment" => { "EnforcedStyleAlignWith" => style }
          },
          "(test)"
        ),
        "(test)"
      )
      expect_lint_parity(*klasses, source, cfg)
      expect_autocorrect_parity(*klasses, source, cfg)
    end
  end
end
