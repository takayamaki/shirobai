# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/EmptyLineAfterMagicComment`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. CRLF line endings. Prism's comment `location` includes the trailing
#      `\r` while parser-gem's `comment.source_range` excludes it. The Rust
#      side snaps the end back so candidate offsets line up.  And the bundle
#      path requires `buffer.source == raw_source`; CRLF / BOM hits the
#      standalone fallback that scans `buffer.source` (LF-normalized).
#   2. Emacs-style magic: `# -*- coding: utf-8 -*-`. Stock's
#      `MagicComment.parse` routes this through `EmacsComment` and reports
#      `encoding_specified? == true`, so it counts as a magic comment.
#   3. Vim-style magic: `# vim: foo=bar, fileencoding=ascii-8bit`. Stock
#      routes this through `VimComment` (encoding only effective when
#      `tokens.size > 1`).
#   4. Magic comment with a whitespace-only next line: `strip.empty?` is
#      true, so no offense fires (whitespace-only counts as blank).
#   5. Header doc + magic: stock takes the LAST magic in the prefix; only
#      the magic's next line is checked.  When that next line is code, the
#      offense fires after the magic, not after the doc.
#   6. Block comment (`=begin/=end`) before a magic: prism reports the
#      block as one EmbDocComment whose text won't match any
#      MagicComment.parse regex; stock ignores it but it is still in the
#      "before code" prefix.
#   7. Magic comment AFTER the first AST line: not in the prefix at all;
#      stock's `take_while { line < ast.line }` drops it before the
#      magic-comment check.
#   8. Magic comment at end-of-file (no next line): stock's
#      `processed_source[last.loc.line]` returns nil → `return unless` exit;
#      no offense.
#   9. Multiple magic comments back to back without a blank line: stock
#      reports ONE offense at the LAST magic's `loc.line + 1`.
#  10. `rbs_inline: invalid_value` is NOT a magic comment for stock's
#      `MagicComment.parse.any?` (`valid_rbs_inline_value?` rejects it).
#
# All cases are differential against the 1.87-pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::EmptyLineAfterMagicComment do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::EmptyLineAfterMagicComment
  shirobai_klass = Shirobai::Cop::Layout::EmptyLineAfterMagicComment

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  it "snaps prism's CRLF-included comment range back to parser-gem's range" do
    # CRLF source: parser-gem normalizes to LF in buffer.source; the
    # standalone fallback scans buffer.source (LF) directly.
    expect_autocorrect_parity(stock_klass, shirobai_klass, "# frozen_string_literal: true\r\nclass Foo; end\n", cfg)
  end

  it "treats Emacs-style `# -*- coding: utf-8 -*-` as a magic comment" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "# -*- coding: utf-8 -*-\nclass Foo; end\n", cfg)
  end

  it "treats Vim-style `# vim: foo=bar, fileencoding=ascii-8bit` as a magic comment" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "# vim: foo=bar, fileencoding=ascii-8bit\nclass Foo; end\n", cfg)
  end

  it "treats a whitespace-only next line as blank (strip.empty?)" do
    expect_lint_parity(stock_klass, shirobai_klass, "# frozen_string_literal: true\n\t \nclass Foo; end\n", cfg,
                       expect_offenses: false)
  end

  it "ignores a non-magic header comment and flags the magic that follows" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "# header\n# frozen_string_literal: true\nclass Foo; end\n", cfg)
  end

  it "skips a `=begin/=end` block comment in the before-code prefix" do
    src = "=begin\nblock comment\n=end\n# frozen_string_literal: true\nclass Foo; end\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "ignores a magic comment that appears AFTER the first code line" do
    expect_lint_parity(stock_klass, shirobai_klass, "puts 'hi'\n# frozen_string_literal: true\nfoo\n", cfg,
                       expect_offenses: false)
  end

  it "does not flag when the magic comment is the file's last line" do
    expect_lint_parity(stock_klass, shirobai_klass, "# frozen_string_literal: true", cfg,
                       expect_offenses: false)
    expect_lint_parity(stock_klass, shirobai_klass, "# frozen_string_literal: true\n", cfg,
                       expect_offenses: false)
  end

  it "reports one offense for back-to-back magic comments before code" do
    src = "# encoding: utf-8\n# frozen_string_literal: true\nclass Foo; end\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "does not flag `# rbs_inline: invalid_value` (not a magic comment)" do
    expect_lint_parity(stock_klass, shirobai_klass, "# rbs_inline: invalid_value\nclass Foo; end\n", cfg,
                       expect_offenses: false)
  end

  it "flags after a shebang + magic comment" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "#!/usr/bin/env ruby\n# frozen_string_literal: true\nclass Foo; end\n", cfg)
  end

  it "accepts a magic comment with no code (comments only) and a following blank" do
    expect_lint_parity(stock_klass, shirobai_klass, "# frozen_string_literal: true\n\n# Hello\n", cfg,
                       expect_offenses: false)
  end

  it "flags a comments-only file when the magic is immediately followed by text" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "# frozen_string_literal: true\n# Hello\n", cfg)
  end

  it "flags an `# encoding: utf-8` standalone magic comment" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "# encoding: utf-8\nclass Foo; end\n", cfg)
  end

  it "flags a magic comment ending in trailing whitespace before the code" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "# frozen_string_literal: true \nclass Foo; end\n", cfg)
  end

  it "flags after a leading blank line followed by a magic comment" do
    # Stock's `take_while { line < ast.line }` still picks up the magic on
    # line 2 since the first code line is 3.
    expect_autocorrect_parity(stock_klass, shirobai_klass, "\n# frozen_string_literal: true\nclass Foo; end\n", cfg)
  end
end
