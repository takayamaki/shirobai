# frozen_string_literal: true

require "spec_helper"

# 3スタイルの regular indentation + block チェーン整列/block補正まで実装済み。
# 残る hash pair 整列の未移植ケースを PENDING に列挙して skip し、実装するたびに
# 該当行を削除する。
RSpec.describe Shirobai::Cop::Layout::MultilineMethodCallIndentation, :config do
  PENDING = [
    "when EnforcedStyle is aligned registers an offense and corrects method call inside hash pair value shifted left",
    "when EnforcedStyle is aligned registers an offense and corrects method call inside hash pair value shifted right",
    "when EnforcedStyle is aligned registers an offense and corrects method call with block inside hash pair value",
    "when EnforcedStyle is aligned registers an offense and corrects method chain inside hash pair value",
    "when EnforcedStyle is aligned registers an offense and corrects method call after block in hash pair value",
    "when EnforcedStyle is aligned for semantic alignment when inside a hash pair without block receiver registers an offense for misaligned multi-dot chain in hash pair",
    "when EnforcedStyle is aligned for semantic alignment when inside a hash pair without block receiver registers an offense for trailing dot multi-dot chain in hash pair",
    "when EnforcedStyle is aligned for semantic alignment when inside a hash pair in a multiline chain method call still registers an offense for same-line chain with hash pair",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense and corrects method call inside hash pair value shifted left",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense and corrects method call inside hash pair value shifted right",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense and corrects method call with block inside hash pair value",
    "when EnforcedStyle is indented_relative_to_receiver registers an offense and corrects method chain with block in hash pair value",
    "when EnforcedStyle is indented registers an offense and corrects method call inside hash pair value using standard indentation width shifted left",
    "when EnforcedStyle is indented registers an offense and corrects method call inside hash pair value using standard indentation width shifted right",
    "when EnforcedStyle is indented registers an offense and corrects method call with block inside hash pair value",
    "when EnforcedStyle is indented registers an offense and corrects method chain inside hash pair value",
    "when EnforcedStyle is indented registers an offense and corrects method chain with block in hash pair value",
  ].freeze

  before do |example|
    desc = example.full_description.sub("#{described_class} ", "")
    skip "未移植 (hash pair 整列)" if PENDING.include?(desc)
  end

  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/multiline_method_call_indentation_spec.rb")
end
