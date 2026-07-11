# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/Semicolon`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. Path (a) groups PARSER tokens by line, and the tokens include comments:
#      a trailing comment makes the last token a `tCOMMENT`, so `foo; # x` is
#      NOT flagged even though the `;` looks like a terminator.
#   2. Path (b) walks PARSER tokens for real `;` tokens once a line carries 2+
#      expressions (1.88.2 rubocop#15376), so a `;` INSIDE a string/regexp
#      literal on such a line is NOT flagged; only the separator `;` is.
#   3. The trailing `;` of `foo = 1; bar = 2;` is registered by BOTH paths
#      (path (a) remove, path (b) newline); stock's add_offense order and
#      corrector conflict resolution decide the result. shirobai calls
#      add_offense in the same order, so RuboCop reproduces it.
#   4. The leading token-index patterns (`{ ;`, `#{ ;`, `-> { ;`) fire ONLY
#      when the opener is at the exact leading token index; a receiver, an
#      argument list, an assignment or lambda `()` params shift it out.
#   5. Autocorrect: an endless range before `;` is wrapped in parens; a hash
#      value omission is wrapped and its selector space removed; a heredoc
#      opened earlier on the line suppresses the newline replacement.
#   6. CRLF source (the wrapper's non-`bundle_eligible?` fallback path) and
#      multibyte characters before the `;` keep offsets aligned with stock.
#
# All cases are differential against the pinned vendor cop.
RSpec.describe Shirobai::Cop::Style::Semicolon do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Style::Semicolon
  shirobai_klass = Shirobai::Cop::Style::Semicolon

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  def allow_separator_config
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Style/Semicolon" => { "AllowAsExpressionSeparator" => true } }, "(test)"
      ),
      "(test)"
    )
  end

  # --- Quirk 1: parser tokens include comments. ---

  it "does not flag a terminator `;` followed by a comment" do
    expect_lint_parity(stock_klass, shirobai_klass, "foo; # comment\n", cfg,
                       expect_offenses: false)
  end

  it "still flags a bare terminator `;` (no comment)" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo;\n", cfg)
  end

  # --- Quirk 2: path (b) token scan skips `;` inside a string/regexp. ---

  it "does not flag a string `;` even when the line carries two expressions" do
    # Only the separator `;` after `a = 1` is flagged, not the one in "x;y".
    expect_autocorrect_parity(stock_klass, shirobai_klass, "a = 1; b = \"x;y\"\n", cfg)
  end

  it "does not flag a regexp `;` even when the line carries two expressions" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = /a;b/; y = 2\n", cfg)
  end

  it "does not flag a string `;` on a single-expression line" do
    expect_lint_parity(stock_klass, shirobai_klass, "b = \"x;y\"\n", cfg,
                       expect_offenses: false)
  end

  # --- Quirk 3: a trailing `;` flagged by both paths. ---

  it "resolves the both-paths trailing-`;` conflict like stock" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo = 1; bar = 2;\n", cfg)
  end

  it "handles one-line method with several statements (all path (b))" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "def foo(a) x(1); y(2); z(3); end\n", cfg)
  end

  # --- Quirk 4: leading token-index strictness. ---

  it "flags `{ ;`, `-> { ;` and `#{ ;` at the leading index" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo {; bar }\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo -> {; bar }\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "-> {; x }\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "\"\#{;foo}\"\n", cfg)
  end

  it "does not flag shifted openers" do
    ["a.b {; x }\n", "foo(1) {; x }\n", "z = foo {; x }\n",
     "baz ->() {; qux }\n", "z = -> {; x }\n", "x = \"\#{;foo}\"\n"].each do |src|
      expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
    end
  end

  it "flags `; }` before a `}` closing brace and before an interpolation end" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo { bar; }\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "\"\#{foo;}\"\n", cfg)
  end

  it "does not flag `; }` when content follows the interpolation end" do
    expect_lint_parity(stock_klass, shirobai_klass, "\"\#{foo;}bar\"\n", cfg,
                       expect_offenses: false)
  end

  # --- Quirk 5: autocorrect special cases. ---

  it "wraps an endless range before the `;` in parentheses" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "42..;\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "42...;\n", cfg)
  end

  it "wraps a hash value omission and removes the selector space" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "m key1:, key2:;\ndo_something\n", cfg)
  end

  it "suppresses the newline replacement when a heredoc opened before the `;`" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "x = <<~TEXT; y = 2\n  text\nTEXT\n", cfg)
  end

  it "removes a terminator `;` after a heredoc opener" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "x = <<~TEXT;\n  text\nTEXT\n", cfg)
  end

  # --- Quirk 6: CRLF and multibyte offsets. ---

  it "matches on CRLF source (the non-bundle-eligible fallback path)" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "foo = 1; bar = 2\r\nbaz;\r\n", cfg)
  end

  it "keeps offsets aligned with a multibyte character before the `;`" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "puts \"日本語\";\n", cfg)
    # Path (b) with a multibyte string on a two-expression line.
    expect_autocorrect_parity(stock_klass, shirobai_klass, "a = \"あ\"; b = 2\n", cfg)
  end

  # --- Config: AllowAsExpressionSeparator kills path (b) only. ---

  it "keeps path (a) but drops path (b) when AllowAsExpressionSeparator is true" do
    allow = allow_separator_config
    # A pure separator line: no offense at all under the option.
    expect_lint_parity(stock_klass, shirobai_klass, "puts 1; puts 2\n", allow,
                       expect_offenses: false)
    # A trailing terminator `;` is still path (a): still flagged.
    expect_autocorrect_parity(stock_klass, shirobai_klass, "puts 1;\n", allow)
  end

  # --- Oracle self-test: one fixture firing every index pattern. ---

  it "matches on a fixture exercising every path (a) index pattern" do
    src = +""
    src << "foo;\n"          # pattern -1 (last token)
    src << "; bar\n"         # pattern 0  (first token)
    src << "baz { qux; }\n"  # pattern -3 (`; }`)
    src << "quux {; corge }\n" # pattern 2 (`{ ;`)
    src << "-> {; x }\n"     # pattern 2 (bare lambda `{ ;`)
    src << "grault -> {; y }\n" # pattern 3 (`-> { ;`)
    src << "\"\#{;a}\"\n"    # pattern 2 (`#{ ;`)
    src << "\"\#{b;}\"\n"    # pattern -4 (`; }` before interp end)
    stock = lint_offenses(stock_klass, src, cfg)
    expect(stock.size).to be >= 8
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end
end
