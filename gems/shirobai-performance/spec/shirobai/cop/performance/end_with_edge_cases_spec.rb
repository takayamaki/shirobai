# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Performance/EndWith`.
#
# Quirks probed against stock rubocop-performance 1.26.1
# (.tmp/2026-07-05/probe-perf/endwith_sm_{true,false}.rb) that the vendor
# spec does not pin:
#
# - **Union order**: `/x\z/.match?(/ab\z/)` matches the regexp-ARGUMENT branch
#   first, so the rewrite receiver stays `/x\z/` and the pattern comes from
#   the argument. When only the receiver is anchored (`/ab\z/.match?(/xy/)`)
#   the receiver branch fires and the argument becomes the rewrite receiver.
# - **`&.` asymmetry**: `str&.match /abc\z/` is flagged and keeps the `&.`
#   dot; `/abc\z/&.match?(str)` (regexp receiver) is a csend and the receiver
#   branches are `send`-only.
# - **Method set**: `RESTRICT_ON_SEND` is `[match, =~, match?]` — `===` and
#   `!~` never match (the difference from `Performance/StringInclude`).
# - **Anchor gate**: an escaped backslash before `z` (`/a\\z/`) is NOT a `\z`
#   anchor (the prefix is a dangling backslash), and a bare `/\z/` has no
#   literal prefix — neither is flagged.
# - **`SafeMultiline`**: `$` counts as an end anchor only when `SafeMultiline`
#   is false; `\z` always counts.
# - **Escape interpretation / quote selection**: stock drops the anchor, then
#   runs `interpret_string_escapes` + `to_string_literal`, so `\n` becomes a
#   real newline in double quotes, an escaped backslash stays in single
#   quotes, and an escaped metacharacter collapses to the bare character.
RSpec.describe Shirobai::Cop::Performance::EndWith do
  include EdgeCaseParity

  def config_for(safe_multiline)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    # `Enabled: pending` cops are dropped by Team-based runs; force-enable
    # like a real adopter's config would, and pin SafeMultiline explicitly.
    hash["Performance/EndWith"] =
      (hash["Performance/EndWith"] || {}).merge(
        "Enabled" => true, "SafeMultiline" => safe_multiline
      )
    RuboCop::Config.new(hash, default.loaded_path)
  end

  let(:config) { config_for(true) }
  let(:klasses) { [RuboCop::Cop::Performance::EndWith, described_class] }

  describe "pattern union order" do
    it "keeps the receiver when both sides are anchored regexps" do
      corrected = expect_autocorrect_parity(*klasses, "/x\\z/.match?(/ab\\z/)\n", config)
      expect(corrected).to eq("/x\\z/.end_with?('ab')\n")
    end

    it "uses the receiver pattern when only the receiver is anchored" do
      corrected = expect_autocorrect_parity(*klasses, "/ab\\z/.match?(/xy/)\n", config)
      expect(corrected).to eq("/xy/.end_with?('ab')\n")
    end
  end

  describe "safe navigation asymmetry" do
    it "flags str&.match /abc\\z/ and keeps the &. dot" do
      corrected = expect_autocorrect_parity(*klasses, "str&.match /abc\\z/\n", config)
      expect(corrected).to eq("str&.end_with?('abc')\n")
    end

    it "does not flag a csend on a regexp receiver" do
      expect_lint_parity(*klasses, "/abc\\z/&.match?(str)\n", config, expect_offenses: false)
    end
  end

  describe "method set" do
    it "does not flag === or !~" do
      expect_lint_parity(*klasses, "/abc\\z/ === str\n", config, expect_offenses: false)
      expect_lint_parity(*klasses, "str !~ /abc\\z/\n", config, expect_offenses: false)
    end
  end

  describe "anchor gate" do
    it "does not flag an escaped backslash before z" do
      # `/a\\z/` is an escaped backslash then a literal `z`, not a `\z` anchor.
      expect_lint_parity(*klasses, "str.match?(/a\\\\z/)\n", config, expect_offenses: false)
    end

    it "does not flag a bare \\z with no literal prefix" do
      expect_lint_parity(*klasses, "str.match?(/\\z/)\n", config, expect_offenses: false)
    end

    it "does not flag a bare multibyte literal before the anchor" do
      expect_lint_parity(*klasses, "str.match?(/あ\\z/)\n", config, expect_offenses: false)
    end
  end

  describe "SafeMultiline gating of the $ anchor" do
    it "does not flag a $ anchor when SafeMultiline is true" do
      expect_lint_parity(*klasses, "str.match?(/abc$/)\n", config, expect_offenses: false)
    end

    it "flags a $ anchor when SafeMultiline is false" do
      sm_false = config_for(false)
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/abc$/)\n", sm_false)
      expect(corrected).to eq("str.end_with?('abc')\n")
    end

    it "does not flag a non-terminal $ even when SafeMultiline is false" do
      sm_false = config_for(false)
      expect_lint_parity(*klasses, "str.match?(/a$b/)\n", sm_false, expect_offenses: false)
    end
  end

  describe "escape interpretation and quote selection" do
    it "interprets \\n into a double-quoted literal" do
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/\\n\\z/)\n", config)
      expect(corrected).to eq('str.end_with?("\n")' + "\n")
    end

    it "keeps an escaped backslash in single quotes" do
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/\\\\\\z/)\n", config)
      expect(corrected).to eq("str.end_with?('\\\\')\n")
    end

    it "collapses an escaped metacharacter to the bare character" do
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/a\\$\\z/)\n", config)
      expect(corrected).to eq("str.end_with?('a$')\n")
    end
  end
end
