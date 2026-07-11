# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/FileNull`.
#
# The vendor spec covers plain literals, the array / hash exemption and the
# bare-`NUL` gate, but several quirks surfaced only by probing stock during
# the implementation and are pinned here as differential against stock (fresh
# stock cop per file, matching the real CLI). Corpus parity is disposable, so
# a refactor could silently regress them.
#
# Probed quirks:
# - A parser `:str` fires `on_str` even inside a regexp / xstr / interpolated
#   string. prism has no `StringNode` child for a non-interpolated regexp /
#   xstr, so shirobai synthesizes the `:str`-child from the node's
#   `content_loc` + `unescaped`; the offense range is that content (no
#   delimiters), and the body also feeds the file-level `/dev/null` gate.
# - A plain symbol is `:sym`, never `:str`: ignored, and it does NOT feed the
#   gate.
# - A heredoc whose body is `/dev/null` carries a trailing newline, so it is
#   not a full match and neither fires nor feeds the gate.
# - The bare-`nul` gate is file-wide and order-independent: a `/dev/null`
#   appearing AFTER the `nul` still unlocks it, and a `/dev/null` hidden inside
#   an exempt array member unlocks it too.
# - `NUL:` has no gate.
# - Empty / invalid-encoding literals are skipped by `valid_string?`.
RSpec.describe Shirobai::Cop::Style::FileNull do
  include EdgeCaseParity

  # `Style/FileNull` is `Enabled: pending`, so a Team run drops its offenses
  # (and its autocorrect) under the default config. Force-enable it so the
  # autocorrect differential actually exercises the `File::NULL` rewrite.
  let(:config) do
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Style/FileNull" => { "Enabled" => true } }, "(test)"),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Style::FileNull,
    Shirobai::Cop::Style::FileNull
  ]

  describe "regexp / xstr / interpolation `:str` bodies" do
    it "flags and rewrites a `%r{/dev/null}` body (content range, no delimiters)" do
      corrected = expect_autocorrect_parity(*klasses, "x = %r{/dev/null}\n", config)
      expect(corrected).to eq("x = %r{File::NULL}\n")
    end

    it "flags and rewrites a `/\\/dev\\/null/` body (escaped source, unescaped value)" do
      corrected = expect_autocorrect_parity(*klasses, "x = /\\/dev\\/null/\n", config)
      expect(corrected).to eq("x = /File::NULL/\n")
    end

    it "flags and rewrites an xstr (backtick) `/dev/null` body" do
      corrected = expect_autocorrect_parity(*klasses, "x = `/dev/null`\n", config)
      expect(corrected).to eq("x = `File::NULL`\n")
    end

    it "does NOT flag a `/dev/null` `str` part inside an interpolated string (dstr parent)" do
      # rubocop#15333: a `str` that is part of a `:dstr` is not a standalone
      # null device; rewriting it in isolation would corrupt the string.
      corrected = expect_autocorrect_parity(*klasses, "x = \"\#{y}/dev/null\"\n", config)
      expect(corrected).to eq("x = \"\#{y}/dev/null\"\n")
    end

    it "does NOT flag a `/dev/null` part of adjacent string concatenation (dstr parent)" do
      corrected = expect_autocorrect_parity(*klasses, "x = '/dev/null' '/dev/null'\n", config)
      expect(corrected).to eq("x = '/dev/null' '/dev/null'\n")
    end

    it "does NOT flag a plain symbol, and it does NOT feed the gate" do
      expect_lint_parity(*klasses, "a = :\"/dev/null\"\nx = 'NUL'\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "a = :\"/dev/null\"\nx = 'NUL'\n", config)).to be_empty
    end
  end

  describe "the file-level `/dev/null` gate for bare `NUL`" do
    it "unlocks the gate from a `/dev/null` that appears AFTER the `nul`" do
      source = "x = 'nul'\na = '/dev/null'\n"
      stock = expect_lint_parity(*klasses, source, config)
      expect(stock.size).to eq(2)
    end

    it "unlocks the gate from a `/dev/null` hidden in an exempt array member" do
      stock = expect_lint_parity(*klasses, "a = %w[/dev/null]\nx = 'NUL'\n", config)
      expect(stock.size).to eq(1) # the array member itself is exempt
    end

    it "unlocks the gate from a regexp body `/dev/null`" do
      stock = expect_lint_parity(*klasses, "r = %r{/dev/null}\nx = 'NUL'\n", config)
      expect(stock.size).to eq(2)
    end

    it "does NOT flag a bare `nul` with no `/dev/null` anywhere" do
      expect_lint_parity(*klasses, "x = 'nul'\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "x = 'nul'\n", config)).to be_empty
    end

    it "flags `NUL:` regardless of the gate" do
      expect_lint_parity(*klasses, "x = 'NUL:'\n", config)
    end
  end

  describe "non-matching shapes" do
    it "does NOT flag a heredoc body of `/dev/null` (trailing newline breaks the full match)" do
      source = "x = <<~H\n/dev/null\nH\ny = 'NUL'\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "does NOT flag an empty string or a `/dev/null` substring" do
      source = "a = ''\nb = 'see /dev/null here'\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end

  describe "CRLF sources (the bundle path falls back to the standalone scan)" do
    # A CRLF file makes `buffer.source` (normalized to LF) differ from
    # `raw_source`, so `bundle_eligible?` is false and the wrapper scans
    # `buffer.source`. Offsets and autocorrect must still match stock.
    it "matches stock offenses and autocorrect on a CRLF source" do
      # The parser buffer normalizes CRLF to LF, so both stock and shirobai
      # rewrite over the LF `buffer.source` (the differential is the real
      # guarantee; the literal below just documents stock's output).
      corrected = expect_autocorrect_parity(*klasses, "x = '/dev/null'\r\ny = 'NUL'\r\n", config)
      expect(corrected).to eq("x = File::NULL\ny = File::NULL\n")
    end
  end

  describe "case-insensitivity and original-case message" do
    it "flags `/DEV/NULL` and keeps the original case in the message" do
      stock = expect_lint_parity(*klasses, "x = \"/DEV/NULL\"\n", config)
      expect(stock.first[2]).to eq("Style/FileNull: Use `File::NULL` instead of `/DEV/NULL`.")
    end
  end
end
