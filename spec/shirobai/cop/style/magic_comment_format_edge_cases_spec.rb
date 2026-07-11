# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/MagicCommentFormat`.
#
# The Rust side supplies ONLY stock's `leading_comment_lines` boundary (the
# first non-comment token line, read by stock from `processed_source.tokens` --
# the "toucher" cost). The wrapper reuses stock's `CommentRange` and copies the
# offense/message/correction helpers verbatim, so detection and autocorrect are
# stock's own code. These differential cases pin the quirks the vendor spec
# under-tests: the `ast`-nil gate, the leading boundary, a keyword found INSIDE
# a value, a leading BOM, and the emacs-closer swept into the last value.
RSpec.describe Shirobai::Cop::Style::MagicCommentFormat do
  include EdgeCaseParity

  def mcf_config(style: "snake_case", directive: nil, value: nil)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Style/MagicCommentFormat" => {
          "Enabled" => true,
          "EnforcedStyle" => style,
          "DirectiveCapitalization" => directive,
          "ValueCapitalization" => value,
          "SupportedStyles" => %w[snake_case kebab_case],
          "SupportedCapitalizations" => %w[lowercase uppercase]
        } }, "(test)"
      ),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Style::MagicCommentFormat,
    Shirobai::Cop::Style::MagicCommentFormat
  ]

  describe "the ast-nil gate (stock's `return unless processed_source.ast`)" do
    it "does not flag a comment-only file (no ast, no leading boundary)" do
      source = "# frozen-string-literal: true\n"
      expect_lint_parity(*klasses, source, mcf_config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, mcf_config)).to be_empty
    end

    it "does not flag a whitespace-only source" do
      expect_lint_parity(*klasses, " ", mcf_config, expect_offenses: false)
    end
  end

  describe "the leading-line boundary (Rust supplies it without tokens)" do
    it "does not flag an incorrectly-cased magic comment after the first statement" do
      source = "puts 1\n# frozen-string-literal: true\n"
      expect_lint_parity(*klasses, source, mcf_config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, mcf_config)).to be_empty
    end

    it "flags a leading kebab directive above a `;` statement token" do
      # A `;` is a non-comment token, so the boundary is line 2; line 1 is still
      # leading and its kebab directive is flagged (self-test: stock fires).
      source = "# frozen-string-literal: true\n;\nputs 1\n"
      stock = expect_autocorrect_parity(*klasses, source, mcf_config)
      expect(stock).to eq("# frozen_string_literal: true\n;\nputs 1\n")
    end
  end

  describe "a directive keyword found INSIDE a value" do
    it "flags and corrects a kebab keyword that appears in another value" do
      # `DIRECTIVE_REGEXP.scan` matches keywords anywhere in the comment text, so
      # `frozen-string-literal` inside the `typed` value is itself an offense.
      source = "# typed: frozen-string-literal\nputs 1\n"
      stock = expect_autocorrect_parity(*klasses, source, mcf_config)
      expect(stock).to eq("# typed: frozen_string_literal\nputs 1\n")
    end
  end

  describe "a leading UTF-8 BOM" do
    it "still flags the leading magic comment below the BOM" do
      # Stock reads the parser-gem comment (BOM stripped from the comment text);
      # the Rust boundary scan must skip the BOM so the comment stays leading.
      source = "﻿# frozen-string-literal: true\nputs 1\n"
      stock = expect_autocorrect_parity(*klasses, source, mcf_config)
      expect(stock).to eq("﻿# frozen_string_literal: true\nputs 1\n")
    end
  end

  describe "the emacs closer swept into the last value" do
    it "corrects the last value even when it captures the trailing ` -*-`" do
      # `VALUE_REGEXP` captures up to `;` or end-of-text, so the last value of an
      # emacs comment includes the ` -*-` closer.
      source = "# -*- coding: UTF-8; frozen_string_literal: TRUE -*-\nputs 1\n"
      config = mcf_config(value: "lowercase")
      stock = expect_autocorrect_parity(*klasses, source, config)
      expect(stock).to eq("# -*- coding: utf-8; frozen_string_literal: true -*-\nputs 1\n")
    end
  end
end
