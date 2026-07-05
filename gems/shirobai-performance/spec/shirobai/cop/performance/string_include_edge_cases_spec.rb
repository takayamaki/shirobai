# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Performance/StringInclude`.
#
# Quirks probed against stock rubocop-performance 1.26.1
# (.tmp/2026-07-05/probe-perf) that the vendor spec does not pin:
#
# - **Union order**: `/xy/.match?(/ab/)` matches the regexp-ARGUMENT branch
#   first, so the rewrite receiver stays `/xy/` and the pattern comes from
#   the argument.
# - **`!~` asymmetry**: `str !~ /ab/` is flagged with negation;
#   `/ab/ !~ str` matches no branch at all.
# - **`&.` asymmetry**: `str&.match?(/ab/)` is flagged and keeps the `&.`
#   dot in the rewrite; `/ab/&.match?(str)` (regexp receiver) is a csend
#   and the receiver branches are `send`-only.
# - **Escape interpretation / quote selection**: stock runs
#   `interpret_string_escapes` then `to_string_literal`, so `\n` becomes a
#   real newline in double quotes, `'` flips to double quotes, `"` stays
#   in single quotes, and `\.` collapses to `.`.
# - **Literal-only gate**: ASCII `\w`/`\s` semantics — a bare multibyte
#   char is NOT literal (no offense), but an ESCAPED multibyte char matches
#   the `\\[^...]` alternative (offense).
# - **Non-string receivers** (symbols, string literals) are flagged and
#   rewritten verbatim; `SafeAutoCorrect: false` owns the semantics risk.
RSpec.describe Shirobai::Cop::Performance::StringInclude do
  include EdgeCaseParity

  let(:config) do
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    # `Enabled: pending` cops are dropped by Team-based runs; force-enable
    # like a real adopter's config would.
    hash["Performance/StringInclude"] =
      (hash["Performance/StringInclude"] || {}).merge("Enabled" => true)
    RuboCop::Config.new(hash, default.loaded_path)
  end
  let(:klasses) { [RuboCop::Cop::Performance::StringInclude, described_class] }

  describe "pattern union order" do
    it "keeps the receiver when both sides are regexps" do
      corrected = expect_autocorrect_parity(*klasses, "/xy/.match?(/ab/)\n", config)
      expect(corrected).to eq("/xy/.include?('ab')\n")
    end
  end

  describe "!~ asymmetry" do
    it "flags str !~ /ab/ with negation" do
      corrected = expect_autocorrect_parity(*klasses, "str !~ /ab/\n", config)
      expect(corrected).to eq("!str.include?('ab')\n")
    end

    it "does not flag /ab/ !~ str" do
      expect_lint_parity(*klasses, "/ab/ !~ str\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "/ab/ !~ str\n", config)).to be_empty
    end
  end

  describe "safe navigation asymmetry" do
    it "flags str&.match?(/ab/) and keeps the &. dot" do
      corrected = expect_autocorrect_parity(*klasses, "str&.match?(/ab/)\n", config)
      expect(corrected).to eq("str&.include?('ab')\n")
    end

    it "does not flag a csend on a regexp receiver" do
      expect_lint_parity(*klasses, "/ab/&.match?(str)\n", config, expect_offenses: false)
      expect_lint_parity(*klasses, "/ab/&.match(str)\n", config, expect_offenses: false)
    end
  end

  describe "escape interpretation and quote selection" do
    it "interprets \\n into a double-quoted literal" do
      # `interpret_string_escapes` turns `\n` into a real newline;
      # `to_string_literal` (String#inspect) re-escapes it inside double
      # quotes, so the OUTPUT text is `"a\nb"` (escaped), not a raw newline.
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/a\\nb/)\n", config)
      expect(corrected).to eq('str.include?("a\nb")' + "\n")
    end

    it "collapses escaped literal chars" do
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/a\\.b/)\n", config)
      expect(corrected).to eq("str.include?('a.b')\n")
    end

    it "keeps escaped backslashes in single quotes" do
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/a\\\\b/)\n", config)
      expect(corrected).to eq("str.include?('a\\\\b')\n")
    end

    it "flips to double quotes for a single-quote pattern" do
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/a'b/)\n", config)
      expect(corrected).to eq(%(str.include?("a'b")\n))
    end

    it "keeps single quotes for a double-quote pattern" do
      corrected = expect_autocorrect_parity(*klasses, "str.match?(/a\"b/)\n", config)
      expect(corrected).to eq("str.include?('a\"b')\n")
    end

    it "matches stock on mixed quote patterns" do
      expect_autocorrect_parity(*klasses, "str.match?(/a-b,c\"d'e/)\n", config)
    end
  end

  describe "literal-only gate" do
    it "does not flag a bare multibyte pattern" do
      expect_lint_parity(*klasses, "str.match?(/あい/)\n", config,
                         expect_offenses: false)
    end

    it "flags an escaped multibyte char" do
      expect_autocorrect_parity(*klasses, "str.match?(/a\\あb/)\n", config)
    end

    it "does not flag an empty regexp" do
      expect_lint_parity(*klasses, "str.match?(//)\n", config, expect_offenses: false)
    end

    it "does not flag flagged regexps or extra args" do
      expect_lint_parity(*klasses, "str.match?(/ab/i)\n", config, expect_offenses: false)
      expect_lint_parity(*klasses, "str.match(/ab/, 1)\n", config, expect_offenses: false)
    end
  end

  describe "receiver shapes" do
    it "flags a symbol receiver verbatim" do
      corrected = expect_autocorrect_parity(*klasses, ":sym.match?(/ab/)\n", config)
      expect(corrected).to eq(":sym.include?('ab')\n")
    end

    it "flags percent-r patterns with slash content" do
      corrected = expect_autocorrect_parity(*klasses, "str.match?(%r{a/b})\n", config)
      expect(corrected).to eq("str.include?('a/b')\n")
    end

    it "does not flag a receiverless call" do
      expect_lint_parity(*klasses, "match?(/ab/)\n", config, expect_offenses: false)
    end
  end

  describe "CRLF sources (bundle-ineligible fallback)" do
    # See `Detect`'s CRLF case: the parser buffer normalizes `\r\n` to `\n`,
    # so `bundle_eligible?` must route CRLF files to the standalone entry
    # point over `buffer.source`.
    it "matches stock offenses and autocorrect on a CRLF source" do
      src = "x = 1\r\ny = str.match?(/abc/)\r\n"
      expect_lint_parity(*klasses, src, config)
      expect_autocorrect_parity(*klasses, src, config)
    end
  end
end
