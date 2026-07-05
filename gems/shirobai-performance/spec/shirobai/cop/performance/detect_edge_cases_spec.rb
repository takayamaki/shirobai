# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Performance/Detect`.
#
# The vendor spec covers the canonical select/find_all/filter x
# first/last/[0]/[-1] matrix, but several structural quirks were uncovered
# by probing stock rubocop-performance 1.26.1 directly
# (.tmp/2026-07-05/probe-perf). Pinned here because corpus parity is
# disposable and a refactor could silently regress them:
#
# - **Explicit `.[](0)` form**: parser's `loc.selector` for an explicit
#   index call is the bare `[]` token, so stock's autocorrect removes
#   `.[]` and leaves the `(0)` argument list behind — the rewrite is
#   knowingly broken Ruby. Byte parity beats prettiness, so the wrapper
#   must reproduce the broken output exactly.
# - **`&.` sends**: the outer `&.first` is a csend and never matches the
#   pattern head `(send ...)`; the inner `foo&.select { ... }` matches via
#   `call` for `first`/`last` but NOT for the `[]` branches (they are
#   `send`-only in the stock pattern).
# - **`accept_first_call?` gates**: an empty block body (`{ }`, `{ |i| }`)
#   is accepted; a sole block-pass argument is flagged; a plain argument or
#   an argument plus block-pass is accepted; `lazy` chains are accepted
#   only when `lazy` itself has a receiver.
# - **Numbered-parameter blocks** are parser `numblock` nodes and never
#   match `(block ...)` — no offense on either side.
# - **Index by value**: stock matches `(int {0 -1})` by VALUE, so `[0x0]`
#   is flagged like `[0]`.
RSpec.describe Shirobai::Cop::Performance::Detect do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }
  let(:klasses) { [RuboCop::Cop::Performance::Detect, described_class] }

  describe "explicit index call forms" do
    it "reproduces stock's broken autocorrect for .[](0) byte for byte" do
      corrected = expect_autocorrect_parity(*klasses, "foo.select { |i| i.odd? }.[](0)\n", config)
      expect(corrected).to eq("foo.find { |i| i.odd? }(0)\n")
    end

    it "matches stock on the parenless .[] 0 form" do
      expect_autocorrect_parity(*klasses, "foo.select { |i| i.odd? }.[] 0\n", config)
    end
  end

  describe "safe-navigation gating" do
    it "does not flag a csend outer (&.first)" do
      expect_lint_parity(*klasses, "foo.select { |i| i.odd? }&.first\n", config,
                         expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo.select { |i| i.odd? }&.first\n", config))
        .to be_empty
    end

    it "flags a csend inner (foo&.select { }.first) and corrects it" do
      expect_autocorrect_parity(*klasses, "foo&.select { |i| i.odd? }.first\n", config)
    end

    it "does not flag a csend inner on the index branch (send-only pattern)" do
      expect_lint_parity(*klasses, "foo&.select { |i| i.odd? }[0]\n", config,
                         expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo&.select { |i| i.odd? }[0]\n", config))
        .to be_empty
    end
  end

  describe "accept_first_call? gates" do
    it "does not flag an empty block body" do
      expect_lint_parity(*klasses, "foo.select { }.first\n", config, expect_offenses: false)
      expect_lint_parity(*klasses, "foo.select { |i| }.first\n", config, expect_offenses: false)
    end

    it "flags a sole block-pass argument and corrects it" do
      corrected = expect_autocorrect_parity(*klasses, "foo.select(&:odd?).first\n", config)
      expect(corrected).to eq("foo.find(&:odd?)\n")
    end

    it "does not flag a plain argument" do
      expect_lint_parity(*klasses, "foo.select(:x).first\n", config, expect_offenses: false)
    end

    it "does not flag an argument plus block-pass" do
      expect_lint_parity(*klasses, "foo.select(1, &:odd?).first\n", config,
                         expect_offenses: false)
    end

    it "does not flag a bare select" do
      expect_lint_parity(*klasses, "foo.select.first\n", config, expect_offenses: false)
    end

    it "does not flag a lazy chain" do
      expect_lint_parity(*klasses, "foo.lazy.select { |i| i.odd? }.first\n", config,
                         expect_offenses: false)
    end

    it "flags when lazy has no receiver" do
      expect_autocorrect_parity(*klasses, "lazy.select { |i| i.odd? }.first\n", config)
    end

    it "does not flag when select has both args and a literal block" do
      expect_lint_parity(*klasses, "foo.select(x) { |i| i.odd? }.first\n", config,
                         expect_offenses: false)
    end
  end

  describe "block parameter styles" do
    it "does not flag a numbered-parameter block on either side" do
      expect_lint_parity(*klasses, "foo.select { _1.odd? }.first\n", config,
                         expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo.select { _1.odd? }.first\n", config))
        .to be_empty
    end

    # `it`-parameter blocks are the documented TargetRubyVersion limitation:
    # shirobai parses with prism Latest, where `{ it.odd? }` is an it-block
    # (parser `itblock`) and the stock pattern (`block` only) does not match.
    # Stock on whitequark (TargetRubyVersion <= 3.3) parses `it` as a plain
    # method call and flags it; RuboCop itself switches to the prism engine
    # (and stops flagging) for 3.3+ targets. shirobai follows prism Latest,
    # same family as Lint/DuplicateMethods' `it` note in the README.
    it "does not flag an it-parameter block (prism Latest semantics)" do
      expect(lint_offenses(described_class, "foo.select { it.odd? }.first\n", config))
        .to be_empty
    end
  end

  describe "selector range variants" do
    it "matches stock for the ::first call operator" do
      corrected = expect_autocorrect_parity(*klasses, "foo.select { |i| i.odd? }::first\n", config)
      expect(corrected).to eq("foo.find { |i| i.odd? }\n")
    end

    it "matches stock for an offense mid-chain" do
      corrected = expect_autocorrect_parity(*klasses, "foo.select { |i| i.odd? }.first.to_s\n",
                                            config)
      expect(corrected).to eq("foo.find { |i| i.odd? }.to_s\n")
    end

    it "flags first with a literal block" do
      expect_autocorrect_parity(*klasses, "foo.select { |i| i.odd? }.first { bar }\n", config)
    end
  end

  describe "index literal matching" do
    it "flags [0x0] by value like [0]" do
      expect_autocorrect_parity(*klasses, "foo.select { |i| i.odd? }[0x0]\n", config)
    end

    it "does not flag other indices" do
      expect_lint_parity(*klasses, "foo.select { |i| i.odd? }[1]\n", config,
                         expect_offenses: false)
    end

    it "does not flag first(n)" do
      expect_lint_parity(*klasses, "foo.select { |i| i.odd? }.first(2)\n", config,
                         expect_offenses: false)
    end
  end

  describe "CRLF sources (bundle-ineligible fallback)" do
    # A CRLF source normalizes to LF in the parser buffer while `raw_source`
    # keeps the `\r`s, so shared-walk offsets no longer line up with parser
    # positions. `bundle_eligible?` must route these files to the standalone
    # entry point over `buffer.source` — before the guard, both the offense
    # positions and the autocorrected bytes were shifted.
    it "matches stock offenses and autocorrect on a CRLF source" do
      src = "x = 1\r\narr.select { |i| i > 1 }.first\r\n"
      expect_lint_parity(*klasses, src, config)
      expect_autocorrect_parity(*klasses, src, config)
    end
  end
end
