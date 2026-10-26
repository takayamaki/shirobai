# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: the shape of the autocorrect edits.
#
# Since rubocop 1.92 (#16026) stock moves the dot across the gap in ONE
# `replace` over `dot_range.join(gap)`, so a correction by another cop that
# rewrites a range spanning the dot clashes with it instead of leaving the
# moved dot behind. The final bytes are the same as the old remove + insert
# pair, so `expect_correction` and the corpus byte audit cannot tell them
# apart; only the edit list (`Offense#corrections`, reported by the JSON
# formatter) does. These cases pin the edit list against stock for both
# styles and for a dot standing alone on its line (whole-line removal).
RSpec.describe Shirobai::Cop::Layout::DotPosition do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::DotPosition,
    Shirobai::Cop::Layout::DotPosition
  ]

  def config_for(style)
    default = RuboCop::ConfigLoader.default_configuration
    RuboCop::Config.new(
      default.to_h.merge(
        "Layout/DotPosition" => default["Layout/DotPosition"].merge("EnforcedStyle" => style)
      )
    )
  end

  it "builds one replace edit for a trailing dot under the leading style" do
    source = "foo.\n  bar\n"
    expect_lint_parity(*klasses, source, config_for("leading"))
    expect_correction_edits_parity(*klasses, source, config_for("leading"))
    expect_autocorrect_parity(*klasses, source, config_for("leading"))
  end

  it "builds one replace edit for a leading dot under the trailing style" do
    source = "foo\n  .bar\n"
    expect_lint_parity(*klasses, source, config_for("trailing"))
    expect_correction_edits_parity(*klasses, source, config_for("trailing"))
    expect_autocorrect_parity(*klasses, source, config_for("trailing"))
  end

  it "replaces the whole line when the dot stands alone on it" do
    source = "foo\n  .\n  bar\n"
    expect_lint_parity(*klasses, source, config_for("leading"))
    expect_correction_edits_parity(*klasses, source, config_for("leading"))
    expect_autocorrect_parity(*klasses, source, config_for("leading"))
  end

  it "keeps the safe-navigation operator text in the single edit" do
    source = "foo&.\n  bar\n"
    expect_lint_parity(*klasses, source, config_for("leading"))
    expect_correction_edits_parity(*klasses, source, config_for("leading"))
    expect_autocorrect_parity(*klasses, source, config_for("leading"))
  end
end
