# frozen_string_literal: true

require "spec_helper"

# Regression guard for the shared `bundle_eligible?` memo.
#
# A wrapper is "bundle eligible" only when the parser-normalized `buffer.source`
# is byte-identical to the `raw_source` the shared walk scans; a CRLF source
# breaks that (the buffer normalizes `\r\n` to `\n`, the raw source keeps it).
#
# The verdict is memoized. RuboCop's real CLI rebuilds each cop per file (`ready`
# returns a fresh instance), but a REUSED instance investigates several sources
# in turn (vendor specs drive this, and future RuboCop changes might). A plain
# `@bundle_eligible` / `.nil?` / `defined?` memo would freeze the first file's
# verdict and leak it onto later files. `Shirobai::Cop::BundleEligible` keys the
# memo on the `processed_source` identity so each new investigation recomputes.
RSpec.describe Shirobai::Cop::BundleEligible do
  def processed(source)
    ps = RuboCop::ProcessedSource.new(source, RuboCop::TargetRuby::DEFAULT_VERSION)
    ps.config = RuboCop::ConfigLoader.default_configuration
    ps.registry = RuboCop::Cop::Registry.global
    ps
  end

  # Drive one investigation lifecycle and read the (private) verdict.
  def eligible_after(cop, source)
    cop.send(:begin_investigation, processed(source))
    verdict = cop.send(:bundle_eligible?)
    cop.send(:complete_investigation)
    verdict
  end

  let(:cop) { Shirobai::Cop::Layout::SpaceAfterComma.new(RuboCop::ConfigLoader.default_configuration) }

  it "reports true for an LF source whose buffer matches raw" do
    expect(eligible_after(cop, "f(a,b)\n")).to be(true)
  end

  it "reports false for a CRLF source whose buffer diverges from raw" do
    expect(eligible_after(cop, "f(a,b)\r\n")).to be(false)
  end

  it "recomputes the verdict when a reused instance sees a new source" do
    # A plain-ivar memo would freeze the first (eligible) answer and wrongly
    # keep the CRLF file on the bundle path with mismatched offsets.
    expect(eligible_after(cop, "f(a,b)\n")).to be(true)
    expect(eligible_after(cop, "f(a,b)\r\n")).to be(false)
    # And back again, to show it is not a one-way latch.
    expect(eligible_after(cop, "f(a,b)\n")).to be(true)
  end

  it "is a private method on every wrapper that mixes it in" do
    expect(cop.class.private_instance_methods).to include(:bundle_eligible?)
  end
end
