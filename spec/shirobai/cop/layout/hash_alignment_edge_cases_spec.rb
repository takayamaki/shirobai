# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: clobber confinement keeps hash groups separate.
#
# When adjacent key/value corrections inside ONE hash collide
# (`ClobberingError`), stock drops the remaining offenses of THAT hash but
# leaves every OTHER hash intact. shirobai reproduces this by handing a hash
# group id from Rust and confining the ClobberingError inside the group. If the
# group boundary collapses, a clobber in one hash would swallow another hash's
# offenses (or resurrect them), a divergence that is invisible to count parity
# when it nets out.
#
# Corpus-only before this spec (it was first seen on a specific corpus file;
# the vendor + non_ascii fixtures only ever use a SINGLE hash). These cases pin
# that two independently-misaligned hashes each keep their own offense and are
# each corrected, with stock and shirobai agreeing throughout.
RSpec.describe Shirobai::Cop::Layout::HashAlignment do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Layout::HashAlignment,
    Shirobai::Cop::Layout::HashAlignment
  ]

  # Two separate multiline hashes, each with its own misaligned key. Each hash
  # is its own correction group: both offenses survive and both are corrected.
  it "confines each hash's offenses to its own group (two misaligned hashes)" do
    source = "a = {\n  x: 1,\n   yy: 2\n}\nb = {\n  p: 1,\n   qq: 2\n}\n"
    stock = expect_lint_parity(*klasses, source, config)
    # Both hashes must contribute an offense (not just one) — guards that the
    # group split does not collapse the two hashes into one swallowed group.
    expect(stock.size).to eq(2)
    expect_autocorrect_parity(*klasses, source, config)
  end

  # Control: one hash misaligned, one already aligned. The aligned hash must
  # stay untouched (no spurious offense leaking across the group boundary).
  it "leaves an already-aligned neighbouring hash untouched" do
    source = "a = {\n  x: 1,\n   yy: 2\n}\nb = {\n  p: 1,\n  qq: 2\n}\n"
    stock = expect_lint_parity(*klasses, source, config)
    expect(stock.size).to eq(1)
    expect_autocorrect_parity(*klasses, source, config)
  end
end
