# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/DirectiveScope`.
#
# The Rust side supplies ONLY the candidate list: the `(index, line)` of every
# comment containing `rubocop` (a superset of stock's directive regexp). The
# wrapper runs stock's per-comment body verbatim on those, so detection and
# autocorrect are stock's own code. These differential cases pin what the
# vendor spec under-tests: the superset filter (prose mentioning rubocop, a
# directive inside an `=begin` block, a trailing directive), the index/line
# mapping after multibyte, CRLF and BOM prefixes (bundle and standalone
# paths), and the point of the replacement — a file without a directive must
# never materialize the parser-gem token stream.
RSpec.describe Shirobai::Cop::Style::DirectiveScope do
  include EdgeCaseParity

  let(:config) do
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Style/DirectiveScope" => { "Enabled" => true } }, "(test)"),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Style::DirectiveScope,
    Shirobai::Cop::Style::DirectiveScope
  ]

  pair = "# rubocop:disable Metrics/AbcSize\ndef foo\nend\n# rubocop:enable Metrics/AbcSize\n"
  pair_fixed = "# rubocop:disable-next Metrics/AbcSize\ndef foo\nend\n"

  describe "the typical single-statement pairs" do
    it "converts a disable/enable pair to disable-next" do
      expect(expect_autocorrect_parity(*klasses, pair, config)).to eq(pair_fixed)
    end

    it "converts a signed push/pop to disable-next" do
      source = "# rubocop:push -Metrics/AbcSize\ndef foo\nend\n# rubocop:pop\n"
      expect(expect_autocorrect_parity(*klasses, source, config)).to eq(pair_fixed)
    end

    it "converts an enable/disable pair inside a disabled region to enable-next" do
      source = "# rubocop:disable Metrics/AbcSize\n# rubocop:enable Metrics/AbcSize\n" \
               "def foo\nend\n# rubocop:disable Metrics/AbcSize\n# rubocop:enable Metrics/AbcSize\n"
      corrected = expect_autocorrect_parity(*klasses, source, config)
      expect(corrected).to eq("# rubocop:disable Metrics/AbcSize\n# rubocop:enable-next Metrics/AbcSize\n" \
                              "def foo\nend\n# rubocop:enable Metrics/AbcSize\n")
    end

    it "leaves a region spanning two statements alone" do
      source = "# rubocop:disable Metrics/AbcSize\ndef foo\nend\n\ndef bar\nend\n# rubocop:enable Metrics/AbcSize\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
    end
  end

  describe "the superset candidate filter (any comment containing `rubocop`)" do
    it "ignores prose that mentions rubocop" do
      source = "# rubocop is configured in .rubocop.yml\ndef foo\nend\n# see rubocop docs\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
    end

    it "ignores a directive whose line also has code (not comment-only)" do
      source = "def foo # rubocop:disable Metrics/AbcSize\nend\n# rubocop:enable Metrics/AbcSize\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
    end

    it "ignores a directive inside an `=begin` block comment" do
      source = "=begin\n# rubocop:disable Metrics/AbcSize\n=end\ndef foo\nend\n# rubocop:enable Metrics/AbcSize\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
    end

    it "still finds a pair whose closing directive follows plain comments" do
      source = "# doc\n# more doc\n#{pair}"
      expect(expect_autocorrect_parity(*klasses, source, config)).to eq("# doc\n# more doc\n#{pair_fixed}")
    end
  end

  describe "the comment index/line mapping" do
    it "matches after a multibyte comment (bundle path)" do
      source = "# 多バイト文字を含むコメント\n#{pair}"
      expect(expect_autocorrect_parity(*klasses, source, config)).to eq("# 多バイト文字を含むコメント\n#{pair_fixed}")
    end

    it "matches on a CRLF source (standalone path)" do
      # The corrector rewrites the parser-normalized (LF) buffer on both sides;
      # only the parity matters here.
      expect_autocorrect_parity(*klasses, pair.gsub("\n", "\r\n"), config)
    end

    it "matches on a source with a leading BOM (standalone path)" do
      source = "﻿#{pair}"
      expect(expect_autocorrect_parity(*klasses, source, config)).to eq("﻿#{pair_fixed}")
    end
  end

  describe "the token stream (the replaced cost)" do
    def investigate(klass, source)
      processed = RuboCop::ProcessedSource.new(source, RuboCop::TargetRuby::DEFAULT_VERSION)
      processed.config = config
      processed.registry = RuboCop::Cop::Registry.global
      report = RuboCop::Cop::Commissioner.new([klass.new(config)]).investigate(processed)
      expect(report.errors).to be_empty
      processed
    end

    it "is never materialized on a file without a directive (stock materializes it)" do
      source = "# doc\ndef foo\nend\n"
      expect(investigate(klasses.first, source).instance_variable_get(:@tokens)).not_to be_nil
      expect(investigate(klasses.last, source).instance_variable_get(:@tokens)).to be_nil
    end
  end
end
