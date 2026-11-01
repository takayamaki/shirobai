# frozen_string_literal: true

require "spec_helper"

# Regression guard for CRLF sources.
#
# `ProcessedSource#raw_source` keeps every "\r", but parser-gem builds
# `buffer.source` with "\r\n" replaced by "\n", and every stock range (AST,
# tokens, `lines`) indexes that normalized string. If Rust scans `raw_source`,
# each offset after the first line break is ahead by the number of "\r" before
# it, so offenses land on the wrong columns or lines. The LF corpora cannot
# see this, so these examples run the stock cop and the shirobai cop side by
# side over CRLF copies of real files.
RSpec.describe "CRLF source offset parity with stock RuboCop" do
  describe "an ASCII-only CRLF file (Ruby stdlib optparse.rb)" do
    it "is ASCII-only and has CRLF line endings (precondition for the sweep)"
    it "matches stock offenses and autocorrect output for every implemented cop"
  end

  describe "a CRLF file with multibyte characters (Ruby stdlib fileutils.rb)" do
    it "is non-ASCII and has CRLF line endings (precondition for the sweep)"
    it "matches stock offenses and autocorrect output for every implemented cop"
  end

  describe "a file that mixes CRLF and LF line endings" do
    it "matches stock offenses and autocorrect output for every implemented cop"
  end

  describe "cops that read the raw line endings" do
    it "matches stock Layout/EndOfLine with EnforcedStyle: crlf on a mixed file"
    it "matches stock Layout/EndOfLine with EnforcedStyle: lf on a CRLF file"
  end
end
