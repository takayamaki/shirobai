# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/MultilineOperationIndentation`.
#
# The keyword message tail names the enclosing keyword from
# `node.loc.keyword.source` (stock `keyword_message_tail`). Prism models an
# `elsif` branch as a NESTED `IfNode` whose keyword token is `elsif`, not `if`,
# so stock reports "a condition in a `elsif` statement" while shirobai used to
# hardcode `if` and report "a condition in an `if` statement" (both keyword and
# article diverged: `elsif` -> `a`, `if` -> `an`).
#
# This was invisible on the corpus for a long time because redmine's real
# config excludes the file / cop; it only surfaced under DEFAULT config on
# redmine `app/helpers/queries_helper.rb` (the `elsif api_request? || ...`
# chain). Pinned here differentially, together with the adjacent keyword
# shapes, against the 1.88-pinned stock.
RSpec.describe Shirobai::Cop::Layout::MultilineOperationIndentation do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::MultilineOperationIndentation,
    Shirobai::Cop::Layout::MultilineOperationIndentation
  ]

  let(:aligned_config) { RuboCop::ConfigLoader.default_configuration }

  it "names the keyword `elsif` for a multiline operation in an elsif condition" do
    # Minimised from redmine `app/helpers/queries_helper.rb` 363-365.
    src = <<~RUBY
      if a
        1
      elsif bbbb ||
          cccc
        2
      end
    RUBY
    expect_lint_parity(*klasses, src, aligned_config)
    expect_autocorrect_parity(*klasses, src, aligned_config)
  end

  it "names `elsif` for each branch of a 3-deep elsif chain" do
    src = <<~RUBY
      if a
        1
      elsif bbbb ||
          cccc
        2
      elsif dddd ||
          eeee
        3
      end
    RUBY
    expect_lint_parity(*klasses, src, aligned_config)
    expect_autocorrect_parity(*klasses, src, aligned_config)
  end

  it "still names `if` for the leading if condition" do
    src = <<~RUBY
      if aaaa ||
          bbbb
        1
      elsif cccc
        2
      end
    RUBY
    expect_lint_parity(*klasses, src, aligned_config)
    expect_autocorrect_parity(*klasses, src, aligned_config)
  end

  it "names the nearest keyword `elsif`, not an inner `if` used in the condition" do
    src = <<~RUBY
      if a
        1
      elsif (b ? c : d) ||
          eeee
        2
      end
    RUBY
    expect_lint_parity(*klasses, src, aligned_config)
    expect_autocorrect_parity(*klasses, src, aligned_config)
  end

  it "keeps naming `unless` / `while` / `until` unchanged" do
    src = <<~RUBY
      unless gggg ||
          hhhh
        1
      end

      while iiii ||
          jjjj
        2
      end

      until kkkk ||
          llll
        3
      end
    RUBY
    expect_lint_parity(*klasses, src, aligned_config)
    expect_autocorrect_parity(*klasses, src, aligned_config)
  end
end
