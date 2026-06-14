# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: elsif shares the outer `if`'s `end`.
#
# `elsif` is a nested `IfNode` whose `loc.end` is nil (the `end` keyword belongs
# to the outermost `if`). If the wrapper does not exclude the "elsif" syntactic
# subtree from end-alignment, the variable/start-of-line styles drive the
# `end`'s autocorrect to OSCILLATE (column 4 <-> column 15) and never converge,
# which is the worst kind of drop-in breakage (an infinite autocorrect loop).
# The default `keyword` style aligns `end` with the outer `if`; this pins that
# the autocorrect CONVERGES (and is stable on a re-run) and that stock and
# shirobai agree on offenses and corrected source.
#
# Corpus-only before this spec (the vendor + non_ascii fixtures use `var = if
# test\nend` with no elsif).
RSpec.describe Shirobai::Cop::Layout::EndAlignment do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Layout::EndAlignment,
    Shirobai::Cop::Layout::EndAlignment
  ]

  # An elsif chain whose `end` is misaligned: the autocorrect must move the
  # `end` to align with the outer `if` and then STOP (not ping-pong between
  # the `if` column and a `elsif`-derived column).
  misaligned = "var = if c\n  a\nelsif d\n  b\n  end\n"
  # An elsif chain already aligned with the outer `if` (no offense).
  aligned = "var = if c\n  a\nelsif d\n  b\n      end\n"

  it "aligns the shared `end` to the outer `if` and converges" do
    expect_lint_parity(*klasses, misaligned, config)
    corrected = expect_autocorrect_parity(*klasses, misaligned, config)
    # The corrected source is itself offence-free, i.e. the autocorrect reached
    # a fixpoint and does not oscillate on a second pass.
    expect(lint_offenses(klasses.first, corrected, config)).to be_empty
    expect(lint_offenses(klasses.last, corrected, config)).to be_empty
    expect(autocorrect_run(klasses.last, corrected, config).last).to eq(corrected)
  end

  it "emits no offense when the shared `end` already aligns with the outer `if`" do
    expect_lint_parity(*klasses, aligned, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, aligned, config)).to be_empty
  end
end
