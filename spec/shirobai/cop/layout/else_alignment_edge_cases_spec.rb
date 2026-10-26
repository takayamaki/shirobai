# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: the shape and the gating of the autocorrect.
#
# Stock hands `AlignmentCorrector.correct` the bare `else` keyword range, so
# the edit anchors at the keyword, not at the start of its line: a positive
# delta inserts right before `else`, a negative one removes the spaces that
# end at `else`. The final bytes equal a line-start edit, so only the edit
# list (`Offense#corrections`, reported by the JSON formatter since rubocop
# 1.92) tells them apart. Since rubocop 1.92 a range argument also makes the
# corrector treat every string body in the file as taboo (#15750), so an
# `else` inside a multiline string interpolation is reported but not moved.
RSpec.describe Shirobai::Cop::Layout::ElseAlignment do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::ElseAlignment,
    Shirobai::Cop::Layout::ElseAlignment
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "inserts before the keyword when else sits left of its if" do
    source = "  x = if a\n    1\n else\n    2\n  end\n"
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  it "removes the spaces that end at the keyword when else sits right of its if" do
    source = "  if a\n    1\n      else\n    2\n  end\n"
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  it "reports but does not move an else inside a multiline string interpolation" do
    source = "x = \"\#{if a\n  1\n    else\n  2\nend}\"\n"
    expect_lint_parity(*klasses, source, config)
    stock = correction_edits(klasses.first, source, config)
    expect(stock).not_to be_empty
    expect(stock.flat_map(&:last)).to be_empty
    expect(correction_edits(klasses.last, source, config)).to eq(stock)
    expect_autocorrect_parity(*klasses, source, config)
  end

  it "reports but does not move an else inside a heredoc interpolation" do
    source = "x = <<~TEXT\n  \#{if a\n  1\n      else\n  2\nend}\nTEXT\n"
    expect_lint_parity(*klasses, source, config)
    stock = correction_edits(klasses.first, source, config)
    expect(stock).not_to be_empty
    expect(stock.flat_map(&:last)).to be_empty
    expect(correction_edits(klasses.last, source, config)).to eq(stock)
    expect_autocorrect_parity(*klasses, source, config)
  end
end
