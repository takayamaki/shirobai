# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Naming/AsciiIdentifiers`.
#
# Stock iterates the parser-gem token stream and flags `tIDENTIFIER` (always)
# and `tCONSTANT` (when `AsciiConstants`). shirobai reads prism's own lex tokens
# and maps them to that distinction. These differential cases pin the mapping
# quirks the vendor spec does not exercise: the symbol-body exclusion, the
# instance-method-def `!`/`?` name (parser-gem `tIDENTIFIER`, prism
# `METHOD_NAME`), the `def self.` / call / alias `tFID` skips, the
# ivar/cvar/gvar/label skips, and the `AsciiConstants` gate.
RSpec.describe Shirobai::Cop::Naming::AsciiIdentifiers do
  include EdgeCaseParity

  def ai_config(ascii_constants)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Naming/AsciiIdentifiers" => { "Enabled" => true, "AsciiConstants" => ascii_constants } },
        "(test)"
      ),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Naming::AsciiIdentifiers,
    Shirobai::Cop::Naming::AsciiIdentifiers
  ]

  describe "symbol bodies are tSYMBOL, never flagged" do
    it "does not flag a plain non-ASCII symbol" do
      expect_lint_parity(*klasses, ":サンプル\n", ai_config(true), expect_offenses: false)
      expect(lint_offenses(klasses.first, ":サンプル\n", ai_config(true))).to be_empty
    end

    it "flags the same word used as a method call (tIDENTIFIER)" do
      stock = expect_lint_parity(*klasses, "x.каллε\n", ai_config(true))
      expect(stock.map { |o| o[2] }).to all(include("identifiers"))
    end
  end

  describe "method-name (`!`/`?`) tokenization asymmetry" do
    it "flags an instance-method def name with `!` (parser-gem tIDENTIFIER)" do
      stock = expect_lint_parity(*klasses, "def кир!; end\n", ai_config(true))
      expect(stock.map { |o| o[2] }).to all(include("identifiers"))
    end

    it "flags an instance-method def name with `?`" do
      expect_lint_parity(*klasses, "def кир?; end\n", ai_config(true))
    end

    it "does not flag a `?`/`!` method CALL (tFID under prism Latest)" do
      expect_lint_parity(*klasses, "x.каллε!\n", ai_config(true), expect_offenses: false)
      expect(lint_offenses(klasses.first, "x.каллε!\n", ai_config(true))).to be_empty
    end
  end

  # KNOWN LIMITATION (documented in docs/cop-status.md and README): parser-gem
  # tokenizes a `!`/`?` method NAME differently by TargetRubyVersion. Under the
  # default target (2.7) `def self.foo!`, `undef foo!` and `alias foo!` are
  # `tIDENTIFIER` (stock flags a non-ASCII name); under prism's Latest grammar
  # (what shirobai always uses) they are `tFID` (skipped). The instance-method
  # def name `def foo!` is `tIDENTIFIER` under BOTH, so it is NOT a divergence.
  # No real corpus code has a non-ASCII `!`/`?` method name, so this never
  # affects parity — verified stock-vs-shirobai over every non-ASCII file in all
  # five corpora (0 divergences) under the default target.
  describe "TargetRubyVersion `!`/`?` method-name divergence (known limitation)" do
    it "shows stock (target 2.7) flags `def self.`/`undef` bang names, shirobai does not" do
      ["def self.кир!; end\n", "undef кир!\n"].each do |src|
        stock = lint_offenses(klasses.first, src, ai_config(true))
        expect(stock).not_to be_empty # stock @2.7 flags it (tIDENTIFIER)
        expect(lint_offenses(klasses.last, src, ai_config(true))).to be_empty # shirobai (Latest) skips
      end
    end
  end

  describe "non-identifier name kinds are their own token types (skipped)" do
    it "does not flag ivar / cvar / gvar / label names" do
      %W[@名前\n @@клас=1\n $グ=1\n {\ 名前:\ 1\ }\n].each do |src|
        expect_lint_parity(*klasses, src, ai_config(true), expect_offenses: false)
        expect(lint_offenses(klasses.first, src, ai_config(true))).to be_empty
      end
    end
  end

  describe "AsciiConstants gate" do
    it "flags a non-ASCII constant only when AsciiConstants is on" do
      stock = expect_lint_parity(*klasses, "Foö = 1\n", ai_config(true))
      expect(stock.map { |o| o[2] }).to all(include("constants"))
      expect_lint_parity(*klasses, "Foö = 1\n", ai_config(false), expect_offenses: false)
      expect(lint_offenses(klasses.first, "Foö = 1\n", ai_config(false))).to be_empty
    end
  end

  # parser-gem calls a name `tCONSTANT` only when it starts with an ASCII A-Z; a
  # Unicode-uppercase (non-ASCII) start is a `tIDENTIFIER` (prism lexes both as
  # CONSTANT). So `ФУ` / `Öo` / `Ωμ` are IDENTIFIER offenses, flagged even under
  # AsciiConstants: false. `Foö` (ASCII `F` start) stays a constant.
  describe "Unicode-uppercase start is a tIDENTIFIER, not a constant" do
    it "flags a Cyrillic-uppercase name as an identifier under both gates" do
      [true, false].each do |ac|
        stock = expect_lint_parity(*klasses, "ФУ = 1\n", ai_config(ac))
        expect(stock.map { |o| o[2] }).to all(include("identifiers"))
      end
    end

    it "flags Greek and full-width uppercase starts as identifiers" do
      expect_lint_parity(*klasses, "Ωμέγα = 1\n", ai_config(false))
      expect_lint_parity(*klasses, "Ａb = 1\n", ai_config(false))
    end
  end

  describe "offense range is the first non-ASCII run" do
    it "marks only the middle non-ASCII run of a mixed identifier" do
      # `foo∂∂bar`: the run is the two ∂ (byte offsets converted to chars).
      stock = expect_lint_parity(*klasses, "foo∂∂bar = baz\n", ai_config(true))
      expect(stock.first[0]).to eq(3) # begin char
      expect(stock.first[1]).to eq(5) # end char (2 chars)
    end
  end

  describe "leading BOM does not shift offsets" do
    it "flags an identifier after a BOM at the right position" do
      stock = expect_lint_parity(*klasses, "﻿класс = 1\n", ai_config(true))
      expect(stock.map { |o| o[2] }).to all(include("identifiers"))
    end
  end
end
