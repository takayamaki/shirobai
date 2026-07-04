# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Lint/DuplicateMagicComment`.
#
# Quirks probed against stock that the vendor spec does not pin. Two
# families:
#
# `leading_comment_lines` (the token-prefix line slice):
# - the prefix is a RAW LINE slice up to the first non-comment token's
#   line: blank lines, `\` continuations and `=begin/=end` bodies stay in
#   it, so a magic-shaped line INSIDE a block comment counts;
# - a `;` is a non-comment token and ends the prefix even with no AST;
# - `__END__` at column 0 stops the lexer: with no code token at all,
#   EVERY line (data section included) becomes a leading line;
# - shebang lines are comments.
#
# `MagicComment.parse` classification:
# - SimpleComment encoding needs EXACTLY one space after the colon, is
#   case-insensitive, and its tail is unanchored (trailing text allowed);
# - SimpleComment fsl is fully anchored (trailing text disqualifies) and
#   any TOKEN value counts as "specified" (`false`, `yes`, ...);
# - the `frozen_string_literal: true coding: utf-8` combined form counts
#   as ENCODING (stock's if/elsif bucket priority);
# - EmacsComment keyword matching is case-SENSITIVE (`-*- CODING: x -*-`
#   does not count) and multi-token comments prioritize encoding;
# - VimComment needs at least two `", "`-separated tokens for
#   `fileencoding` to count, and never sets fsl;
# - `typed` / `shareable_constant_value` / `rbs_inline` never count.
RSpec.describe Shirobai::Cop::Lint::DuplicateMagicComment do
  include EdgeCaseParity

  # The cop is `Enabled: pending`; Team-based autocorrect runs would filter
  # it out under the plain default config on BOTH sides (vacuously green),
  # so force-enable it.
  let(:config) do
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Lint/DuplicateMagicComment" => { "Enabled" => true } }, "(test)"),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Lint::DuplicateMagicComment,
    Shirobai::Cop::Lint::DuplicateMagicComment
  ]

  describe "leading comment line semantics" do
    it "keeps counting past blank lines and shebangs" do
      expect_autocorrect_parity(*klasses,
                                "#!/usr/bin/env ruby\n# encoding: utf-8\n\n# encoding: utf-8\nx = 1\n",
                                config)
    end

    it "stops the prefix at a lone semicolon (a non-comment token)" do
      source = "# encoding: utf-8\n;\n# encoding: utf-8\nx = 1\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "continues past a backslash line continuation" do
      expect_autocorrect_parity(*klasses,
                                "# encoding: utf-8\n\\\n# encoding: utf-8\nx = 1\n",
                                config)
    end

    it "counts magic-shaped lines inside a =begin/=end block" do
      expect_autocorrect_parity(*klasses,
                                "=begin\n# encoding: utf-8\n=end\n# encoding: utf-8\nx = 1\n",
                                config)
    end

    it "treats every line as leading when __END__ starts line 1" do
      # No token at all: the data-section lines join the prefix.
      expect_autocorrect_parity(*klasses,
                                "__END__\n# encoding: a\n# encoding: a\n",
                                config)
    end

    it "sees an indented __END__ as an identifier token (prism Latest)" do
      # Parsers before 3.4 stop lexing at an indented `__END__` (stock then
      # counts the following lines as leading); prism's Latest grammar
      # tokenizes it as an identifier. shirobai follows prism — the
      # documented TargetRubyVersion limitation — so this is a
      # shirobai-only expectation, not a differential one.
      source = "# encoding: utf-8\n  __END__\n# encoding: utf-8\n"
      expect(lint_offenses(klasses.last, source, config)).to be_empty
    end

    it "cuts the line array at a mid-file __END__ (ProcessedSource#lines)" do
      # `buffer.source` keeps the data section, but `ProcessedSource#lines`
      # cuts at the `__END__` line after the last (comment) token, so the
      # data-section duplicate does not count.
      expect_autocorrect_parity(*klasses,
                                "# encoding: utf-8\n# encoding: utf-8\n__END__\n# encoding: a\n",
                                config)
    end

    it "flags duplicates in a comment-only file" do
      expect_autocorrect_parity(*klasses, "# encoding: utf-8\n# encoding: utf-8\n", config)
    end

    it "flags a duplicate on the last line without a trailing newline" do
      expect_autocorrect_parity(*klasses, "# encoding: utf-8\n# encoding: utf-8", config)
    end
  end

  describe "MagicComment classification quirks" do
    it "requires exactly one space after the simple encoding colon" do
      ["# encoding:utf-8\n# encoding: utf-8\nx = 1\n",
       "# encoding:  utf-8\n# encoding: utf-8\nx = 1\n"].each do |source|
        expect_lint_parity(*klasses, source, config, expect_offenses: false)
        expect(lint_offenses(klasses.first, source, config)).to be_empty
      end
    end

    it "matches simple encoding case-insensitively with an unanchored tail" do
      expect_autocorrect_parity(*klasses,
                                "# ENCODING: UTF-8 extra text\n# Coding: utf-8\nx = 1\n",
                                config)
    end

    it "does not match `##`-prefixed comments" do
      source = "## encoding: utf-8\n# encoding: utf-8\nx = 1\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "anchors the fsl form (trailing text disqualifies)" do
      source = "# frozen_string_literal: true extra\n# frozen_string_literal: true\nx = 1\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "accepts dashed fsl keywords and arbitrary fsl values" do
      expect_autocorrect_parity(*klasses,
                                "# frozen-string-literal: yes\n# frozen_string_literal: maybe\nx = 1\n",
                                config)
    end

    it "buckets the combined fsl+coding form as encoding" do
      expect_autocorrect_parity(
        *klasses,
        "# frozen_string_literal: true coding: utf-8\n# encoding: ascii\n# frozen_string_literal: true\nx = 1\n",
        config
      )
    end

    it "matches Emacs comments case-sensitively" do
      expect_autocorrect_parity(*klasses,
                                "# -*- coding: utf-8 -*-\n# encoding: ascii\nx = 1\n",
                                config)
      source = "# -*- CODING: utf-8 -*-\n# encoding: ascii\nx = 1\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "buckets an Emacs fsl+coding comment as encoding" do
      expect_autocorrect_parity(
        *klasses,
        "# -*- frozen_string_literal: true; coding: utf-8 -*-\n# encoding: ascii\n# frozen_string_literal: true\nx = 1\n",
        config
      )
    end

    it "needs two comma-space vim tokens for fileencoding" do
      expect_autocorrect_parity(*klasses,
                                "# vim: ft=ruby, fileencoding=utf-8\n# encoding: ascii\nx = 1\n",
                                config)
      ["# vim: fileencoding=utf-8\n# encoding: ascii\nx = 1\n",
       "# vim: ft=ruby,fileencoding=utf-8\n# encoding: ascii\nx = 1\n"].each do |source|
        expect_lint_parity(*klasses, source, config, expect_offenses: false)
        expect(lint_offenses(klasses.first, source, config)).to be_empty
      end
    end

    it "matches a vim comment mid-line (unanchored)" do
      expect_autocorrect_parity(*klasses,
                                "# stuff # vim: ft=ruby, fileencoding=utf-8\n# encoding: ascii\nx = 1\n",
                                config)
    end

    it "ignores typed / shareable_constant_value / rbs_inline magic" do
      source = "# typed: true\n# typed: true\n# shareable_constant_value: literal\n" \
               "# shareable_constant_value: literal\n# rbs_inline: enabled\n# rbs_inline: enabled\nx = 1\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "flags three encoding comments twice, then fsl duplicates" do
      # Encoding values must be real encodings: the parser honors the first
      # one, and an unknown name makes the file unparsable (no cop runs).
      expect_autocorrect_parity(
        *klasses,
        "# encoding: utf-8\n# encoding: ascii\n# encoding: us-ascii\n# frozen_string_literal: true\n# frozen_string_literal: false\nx = 1\n",
        config
      )
    end

    it "accepts a Unicode token value in a non-first encoding comment" do
      expect_autocorrect_parity(*klasses,
                                "# encoding: utf-8\n# encoding: utf-é\nx = 1\n",
                                config)
    end
  end
end
