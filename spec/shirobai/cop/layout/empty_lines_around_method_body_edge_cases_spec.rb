# frozen_string_literal: true

require "spec_helper"

# CRLF files: `buffer.source` is LF-normalized while `raw_source` keeps the
# `\r`. The shared walk scans `buffer.source`, so its byte offsets line up
# with the parser-gem buffer.
RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundMethodBody do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::EmptyLinesAroundMethodBody,
    Shirobai::Cop::Layout::EmptyLinesAroundMethodBody
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "reports and removes the same blank lines as stock in a CRLF file" do
    source = "def foo\r\n\r\n  bar\r\n\r\nend\r\n"
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  it "reports and removes the same blank lines as stock in a CRLF file after a non-ASCII line" do
    source = "x = \"ほげ\"\r\ndef foo\r\n\r\n  bar\r\n\r\nend\r\n"
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end
end
