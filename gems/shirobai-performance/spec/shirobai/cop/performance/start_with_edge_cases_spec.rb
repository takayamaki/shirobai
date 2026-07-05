# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Performance/StartWith`.
#
# Mirror of the `Performance/EndWith` edge cases, gated on the START anchor.
# Quirks probed against stock rubocop-performance 1.26.1
# (.tmp/2026-07-05/probe-perf/startwith_sm_{true,false}.rb):
#
# - **Union order**: `/\Ax/.match?(/\Aab/)` matches the regexp-ARGUMENT branch
#   first, so the rewrite receiver stays `/\Ax/`. When only the receiver is
#   anchored (`/\Aab/.match?(/xy/)`) the receiver branch fires.
# - **`&.` asymmetry**: `str&.match /\Aabc/` is flagged and keeps the `&.`
#   dot; `/\Aabc/&.match?(str)` (regexp receiver) is a csend and the receiver
#   branches are `send`-only.
# - **Method set**: `RESTRICT_ON_SEND` is `[match, =~, match?]` — `===` and
#   `!~` never match.
# - **Anchor gate**: an escaped backslash before `A` (`/\\Aabc/`) is NOT a
#   `\A` anchor, and a bare `/\A/` has no literal suffix — neither is flagged.
# - **`SafeMultiline`**: `^` counts as a start anchor only when `SafeMultiline`
#   is false; `\A` always counts.
# - **Escape interpretation / quote selection**: stock drops the anchor, then
#   runs `interpret_string_escapes` + `to_string_literal`.
RSpec.describe Shirobai::Cop::Performance::StartWith do
  include EdgeCaseParity

  def config_for(safe_multiline)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    # `Enabled: pending` cops are dropped by Team-based runs; force-enable
    # like a real adopter's config would, and pin SafeMultiline explicitly.
    hash["Performance/StartWith"] =
      (hash["Performance/StartWith"] || {}).merge(
        "Enabled" => true, "SafeMultiline" => safe_multiline
      )
    RuboCop::Config.new(hash, default.loaded_path)
  end

  let(:config) { config_for(true) }
  let(:klasses) { [RuboCop::Cop::Performance::StartWith, described_class] }

  describe "pattern union order" do
    it "keeps the receiver when both sides are anchored regexps" do
      corrected = expect_autocorrect_parity(*klasses, "/\\Ax/.match?(/\\Aab/)\n", config)
      expect(corrected).to eq("/\\Ax/.start_with?('ab')\n")
    end

    it "uses the receiver pattern when only the receiver is anchored" do
      corrected = expect_autocorrect_parity(*klasses, "/\\Aab/.match?(/xy/)\n", config)
      expect(corrected).to eq("/xy/.start_with?('ab')\n")
    end
  end

  describe "safe navigation asymmetry" do
    it "flags str&.match /\\Aabc/ and keeps the &. dot" do
      corrected = expect_autocorrect_parity(*klasses, "str&.match /\\Aabc/\n", config)
      expect(corrected).to eq("str&.start_with?('abc')\n")
    end

    it "does not flag a csend on a regexp receiver" do
      expect_lint_parity(*klasses, "/\\Aabc/&.match?(str)\n", config, expect_offenses: false)
    end
  end

  describe "method set" do
    it "does not flag === or !~" do
      expect_lint_parity(*klasses, "/\\Aabc/ === str\n", config, expect_offenses: false)
      expect_lint_parity(*klasses, "str !~ /\\Aabc/\n", config, expect_offenses: false)
    end
  end

  describe "anchor gate" do
    it "does not flag an escaped backslash before A" do
      # `/\\Aabc/` is an escaped backslash then a literal `A`, not a `\A` anchor.
      expect_lint_parity(*klasses, "str.match?(/\\\\Aabc/)\n", config, expect_offenses: false)
    end

    it "does not flag a bare \\A with no literal suffix" do
      expect_lint_parity(*klasses, "str.match?(/\\A/)\n", config, expect_offenses: false)
    end

    it "does not flag a bare multibyte literal after the anchor" do
      expect_lint_parity(*klasses, "str.match?(/\\Aあ/)\n", config, expect_offenses: false)
    end
  end

  describe "SafeMultiline gating of the ^ anchor" do
    it "does not flag a ^ anchor when SafeMultiline is true" do
      expect_lint_parity(*klasses, "str.match?(/^abc/)\n", config, expect_offenses: false)
    end

    it "flags a ^ anchor when SafeMultiline is false" do
      sm_false = config_for(false)
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/^abc/)\n", sm_false)
      expect(corrected).to eq("str.start_with?('abc')\n")
    end

    it "does not flag a non-initial ^ even when SafeMultiline is false" do
      sm_false = config_for(false)
      expect_lint_parity(*klasses, "str.match?(/a^b/)\n", sm_false, expect_offenses: false)
    end
  end

  describe "escape interpretation and quote selection" do
    it "interprets \\n into a double-quoted literal" do
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/\\A\\n/)\n", config)
      expect(corrected).to eq('str.start_with?("\n")' + "\n")
    end

    it "keeps an escaped backslash in single quotes" do
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/\\A\\\\/)\n", config)
      expect(corrected).to eq("str.start_with?('\\\\')\n")
    end

    it "collapses an escaped metacharacter to the bare character" do
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/\\A\\^/)\n", config)
      expect(corrected).to eq("str.start_with?('^')\n")
    end
  end

  describe "CRLF sources (bundle-ineligible fallback)" do
    # See `Detect`'s CRLF case: the parser buffer normalizes `\r\n` to `\n`,
    # so `bundle_eligible?` must route CRLF files to the standalone entry
    # point over `buffer.source`.
    it "matches stock offenses and autocorrect on a CRLF source" do
      src = "x = 1\r\ny = str.match?(/\\Aabc/)\r\n"
      expect_lint_parity(*klasses, src, config)
      expect_autocorrect_parity(*klasses, src, config)
    end
  end
end
