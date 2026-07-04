# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceAfterColon`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. `on_pair` fires for hash-pattern pairs too — braced (`in {a:1}`),
#      braceless (`in a:1`) and typed (`in {b:Integer}`) all flag the colon.
#   2. Pattern value omissions (`in {a:}`) are skipped like hash omissions.
#   3. Quoted and interpolated labels put the colon at the end of the key's
#      closing (`"a":1`, `"#{x}":1`).
#   4. `on_kwoptarg` covers block and lambda parameters and parenless defs;
#      required keyword arguments (`a:`) are not kwoptargs.
#   5. Index keyword arguments (`x[a:1]`) are pairs.
#   6. A newline after the colon counts as space.
#
# All cases are differential against the pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::SpaceAfterColon do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::SpaceAfterColon
  shirobai_klass = Shirobai::Cop::Layout::SpaceAfterColon

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  it "flags hash-pattern pairs, braced and braceless" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "case x\nin {a:1}\n  y\nend\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "case x\nin a:1\n  y\nend\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "case x\nin {b:Integer}\n  y\nend\n", cfg)
  end

  it "skips pattern value omissions" do
    src = "case x\nin {a:, b: 1}\n  y\nend\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "skips hash and call value omissions" do
    src = "h = {a:}\nf(a:)\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags quoted and interpolated labels" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "h = {\"a\":1}\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "h = {\"\#{x}\":1}\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "h = {a?: 1, b:2}\n", cfg)
  end

  it "flags kwoptargs in blocks, lambdas and parenless defs" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo { |a:1| }\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "->(a:1) {}\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "def f a:1\nend\n", cfg)
  end

  it "does not flag required keyword arguments" do
    src = "def f(a:, b: 2); end\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags index keyword arguments" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x[a:1]\n", cfg)
  end

  it "accepts a newline after the colon" do
    src = "h = {a:\n1}\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "does not flag ternaries or plain symbols" do
    src = "x = a ? :b : :c\ny = h[:a]\nz = A::B\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags a colon pair at the last line without a trailing newline" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "h = {a:1}", cfg)
  end
end
