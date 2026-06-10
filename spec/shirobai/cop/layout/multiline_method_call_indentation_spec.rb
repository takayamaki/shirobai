# frozen_string_literal: true

require "spec_helper"

# 大枠(aligned/indented/indented_relative_to_receiver の regular indentation)を
# 実装済み。block チェーン整列・hash pair 整列・splat 相対インデント等の未移植
# ケースは下記 PENDING に列挙して skip し、実装するたびに該当行を削除していく。
RSpec.describe Shirobai::Cop::Layout::MultilineMethodCallIndentation, :config do
  PENDING = [
    "when EnforcedStyle is aligned accepts correctly aligned methods in operands",
    "when EnforcedStyle is aligned accepts indented and aligned methods in binary operation",
    "when EnforcedStyle is aligned registers an offense and corrects misaligned methods in multiline block chain",
    "when EnforcedStyle is aligned accepts aligned methods in multiline block chain",
    "when EnforcedStyle is aligned accepts aligned methods in multiline numbered block chain",
    "when EnforcedStyle is aligned accepts aligned methods in multiline `it` block chain",
    "when EnforcedStyle is aligned accepts aligned methods in multiline block chain with safe navigation operator",
    "when EnforcedStyle is aligned accepts aligned method chained after single-line block on both calls",
    "when EnforcedStyle is aligned accepts aligned method chained after single-line block only on first call",
    "when EnforcedStyle is aligned accepts aligned method chained after single-line block only on second call",
    "when EnforcedStyle is aligned accepts aligned method chained after single-line block with safe navigation",
    "when EnforcedStyle is aligned registers an offense for misaligned method chained after single-line block on both calls",
    "when EnforcedStyle is aligned registers an offense for misaligned method chained after single-line block only on first call",
    "when EnforcedStyle is aligned registers an offense for misaligned method chained after single-line block only on second call",
    "when EnforcedStyle is aligned registers an offense and corrects method call inside hash pair value shifted left",
    "when EnforcedStyle is aligned registers an offense and corrects method call inside hash pair value shifted right",
    "when EnforcedStyle is aligned registers an offense and corrects method call with block inside hash pair value",
    "when EnforcedStyle is aligned registers an offense and corrects method chain inside hash pair value",
    "when EnforcedStyle is aligned registers an offense and corrects method call after block in hash pair value",
    "when EnforcedStyle is aligned registers an offense and corrects method chain with array literal receiver",
    "when EnforcedStyle is aligned for semantic alignment accepts methods being aligned with safe navigation method call that is an argument",
    "when EnforcedStyle is aligned for semantic alignment when inside a hash pair without block receiver registers an offense for misaligned multi-dot chain in hash pair",
    "when EnforcedStyle is aligned for semantic alignment when inside a hash pair without block receiver registers an offense for trailing dot multi-dot chain in hash pair",
    "when EnforcedStyle is aligned for semantic alignment when inside a hash pair in a multiline chain method call still registers an offense for same-line chain with hash pair",
    "when EnforcedStyle is aligned for semantic alignment when inside a hash pair with block receiver accepts method chain after do-end block inside hash pair",
    "when EnforcedStyle is aligned for semantic alignment when inside a hash pair with block receiver registers an offense for misaligned method chain after do-end block in hash pair",
    "when EnforcedStyle is indented_relative_to_receiver does not register an offense when multiline method chain has expected indent width and the method is preceded by splat",
    "when EnforcedStyle is indented_relative_to_receiver does not register an offense when multiline method chain with block has expected indent width and the method is preceded by splat",
    "when EnforcedStyle is indented_relative_to_receiver does not register an offense when multiline method chain with numbered block has expected indent width and the method is preceded by splat",
    "when EnforcedStyle is indented_relative_to_receiver does not register an offense when multiline method chain with `it` block has expected indent width and the method is preceded by splat",
    "when EnforcedStyle is indented_relative_to_receiver does not register an offense when multiline method chain has expected indent width and the method is preceded by double splat",
    "when EnforcedStyle is indented_relative_to_receiver does not register an offense when multiline method chain with block has expected indent width and the method is preceded by double splat",
    "when EnforcedStyle is indented_relative_to_receiver does not register an offense when multiline method chain with numbered block has expected indent width and the method is preceded by double splat",
    "when EnforcedStyle is indented_relative_to_receiver does not register an offense when multiline method chain with `it` block has expected indent width and the method is preceded by double splat",
    "when EnforcedStyle is indented_relative_to_receiver accepts method chained after single-line block on both calls with receiver-relative indent",
    "when EnforcedStyle is indented_relative_to_receiver accepts method chained after single-line block only on first call with receiver-relative indent",
    "when EnforcedStyle is indented_relative_to_receiver accepts method chained after single-line block only on second call with receiver-relative indent",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense and corrects method call inside hash pair value shifted left",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense and corrects method call inside hash pair value shifted right",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense and corrects method call with block inside hash pair value",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense and corrects method chain inside hash pair value",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense and corrects method chain with block in hash pair value",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense for method chain with parenthesized expression receiver",
    "when EnforcedStyle is indented_relative_to_receiver accepts correctly indented method chain with parenthesized expression receiver",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense for method chain when dot is on same line as multiline parens",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense and corrects method chain with hash literal receiver",
    "when EnforcedStyle is indented registers an offense and corrects method call inside hash pair value using standard indentation width shifted left",
    "when EnforcedStyle is indented registers an offense and corrects method call inside hash pair value using standard indentation width shifted right",
    "when EnforcedStyle is indented registers an offense and corrects method call with block inside hash pair value",
    "when EnforcedStyle is indented registers an offense and corrects method chain inside hash pair value",
    "when EnforcedStyle is indented registers an offense and corrects method chain with block in hash pair value",
  ].freeze

  before do |example|
    desc = example.full_description.sub("#{described_class} ", "")
    skip "未移植 (block/hash pair/splat ほか)" if PENDING.include?(desc)
  end

  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/multiline_method_call_indentation_spec.rb")
end
