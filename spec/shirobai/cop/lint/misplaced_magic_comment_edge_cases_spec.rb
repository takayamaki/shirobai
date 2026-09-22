# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Lint/MisplacedMagicComment`.
#
# The Rust side supplies ONLY the candidates: the shebang comment stock's
# `find` would return, and every magic-comment-shaped comment mentioning
# `coding` / `frozen` with stock's `first_code_token` comparison folded into an
# `after_code` flag. The wrapper runs stock's `check_comment` verbatim on those,
# so detection and autocorrect are stock's own code. These differential cases
# pin what the vendor spec under-tests: the superset filter (quoted magic
# comments, unknown encodings, other magic comments after code), the first
# code token boundary (`;`, `__END__`, comment-only files, a trailing comment),
# the index/line mapping after multibyte, CRLF and BOM prefixes (bundle and
# standalone paths), and the point of the replacement — a file with a leading
# `frozen_string_literal` comment must never materialize the token stream.
RSpec.describe Shirobai::Cop::Lint::MisplacedMagicComment do
  include EdgeCaseParity

  let(:config) do
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Lint/MisplacedMagicComment" => { "Enabled" => true } }, "(test)"),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Lint::MisplacedMagicComment,
    Shirobai::Cop::Lint::MisplacedMagicComment
  ]

  after_code = "require 'foo'\n# frozen_string_literal: true\n"
  after_code_fixed = "# frozen_string_literal: true\nrequire 'foo'\n"

  describe "the typical misplacements" do
    it "moves a frozen_string_literal comment placed after code to the top" do
      expect(expect_autocorrect_parity(*klasses, after_code, config)).to eq(after_code_fixed)
    end

    it "moves an encoding comment placed below a documentation comment" do
      source = "# doc\n# encoding: ascii-8bit\nputs 1\n"
      corrected = expect_autocorrect_parity(*klasses, source, config)
      expect(corrected).to eq("# encoding: ascii-8bit\n# doc\nputs 1\n")
    end

    it "flags a magic comment above a shebang (no autocorrect)" do
      source = "# frozen_string_literal: true\n#!/usr/bin/env ruby\nputs 1\n"
      stock = expect_lint_parity(*klasses, source, config)
      expect(stock.map(&:last)).to all(be(false))
    end

    it "moves a trailing frozen_string_literal comment off its code line" do
      source = "x = 1 # frozen_string_literal: true\n"
      corrected = expect_autocorrect_parity(*klasses, source, config)
      expect(corrected).to eq("# frozen_string_literal: true\nx = 1\n")
    end

    it "moves an emacs-style frozen_string_literal comment placed after code" do
      source = "puts 1\n# -*- frozen_string_literal: true -*-\n"
      expect_autocorrect_parity(*klasses, source, config)
    end
  end

  describe "the superset candidate filter (`coding` / `frozen` mentions)" do
    it "does not flag a leading frozen_string_literal comment" do
      expect_lint_parity(*klasses, "# frozen_string_literal: true\nputs 1\n", config, expect_offenses: false)
    end

    it "does not flag documentation quoting a magic comment (second `#`)" do
      source = "puts 1\n#   # -*- coding: UTF-8 -*-\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
    end

    it "does not flag prose parsing as an unknown encoding" do
      source = "# doc\n# Encoding: force given encoding\nputs 1\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
    end

    it "does not flag another magic comment after code (typed)" do
      source = "puts 1\n# typed: true\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
    end

    it "does not flag prose mentioning frozen strings after code" do
      source = "puts 1\n# strings are frozen here\n# the coding style\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
    end
  end

  describe "the first code token boundary" do
    it "treats a `;` as code" do
      source = "# doc\n;\n# frozen_string_literal: true\n"
      expect_autocorrect_parity(*klasses, source, config)
    end

    it "does not flag a magic comment before `__END__`" do
      source = "# frozen_string_literal: true\n__END__\n# encoding: utf-8\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
    end

    it "does not flag a comment-only file" do
      expect_lint_parity(*klasses, "# doc\n# frozen_string_literal: true\n", config, expect_offenses: false)
    end

    it "does nothing on an empty source" do
      expect_lint_parity(*klasses, "", config, expect_offenses: false)
    end
  end

  describe "the comment index/line mapping" do
    it "matches after a multibyte comment (bundle path)" do
      source = "# 多バイト文字を含むコメント\n#{after_code}"
      expect_autocorrect_parity(*klasses, source, config)
    end

    it "matches on a CRLF source (standalone path)" do
      expect_autocorrect_parity(*klasses, after_code.gsub("\n", "\r\n"), config)
    end

    it "matches on a source with a leading BOM (standalone path)" do
      expect_autocorrect_parity(*klasses, "﻿#{after_code}", config)
    end

    it "matches a shebang candidate after a multibyte comment" do
      source = "# 多バイト文字を含むコメント\n#!/usr/bin/env ruby\nputs 1\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
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

    it "is never materialized on a file with a leading frozen_string_literal comment" do
      source = "# frozen_string_literal: true\nputs 1\n"
      expect(investigate(klasses.first, source).instance_variable_get(:@tokens)).not_to be_nil
      expect(investigate(klasses.last, source).instance_variable_get(:@tokens)).to be_nil
    end
  end
end
