# frozen_string_literal: true

require "spec_helper"

# The `empty_lines` insert arm of `EmptyLineCorrector` (rubocop#16138, 1.92.2):
# stock now replaces the newline before the offense line with `"\n\n"`
# instead of inserting a `"\n"` before it. The final bytes are the same, so
# only the edit list (`Offense#corrections`) tells the two apart.
RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundClassBody do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::EmptyLinesAroundClassBody,
    Shirobai::Cop::Layout::EmptyLinesAroundClassBody
  ]

  def body_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Layout/EmptyLinesAroundClassBody" => { "EnforcedStyle" => style } }, "(test)"),
      "(test)"
    )
  end

  it "builds the same edits as stock for a missing blank line at both ends" do
    source = "class Foo\n  bar\nend\n"
    config = body_config("empty_lines")
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  it "builds the same edits as stock for the empty_lines_special style" do
    source = "class Foo\n  bar\n\n  def baz; end\nend\n"
    config = body_config("empty_lines_special")
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  it "builds the same edits as stock after a non-ASCII line and with CRLF" do
    source = "class Foo\r\n  \"ほげ\"\r\n  bar\r\nend\r\n"
    config = body_config("empty_lines")
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end
end
