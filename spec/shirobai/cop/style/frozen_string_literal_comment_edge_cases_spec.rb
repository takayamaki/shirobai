# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/FrozenStringLiteralComment`: quirks the
# vendor spec does not exercise, all found by probing the real stock CLI.
#
# Pinned quirks:
#
# - The token-empty gate: whitespace-only files have NO tokens (early return);
#   a comment-only / `=begin` file DOES have a token.
# - The existence check (`leading_comment_lines`, LINE based) versus the
#   offense-location search (`frozen_string_literal_comment`, comment-TOKEN
#   based). When the gate fires from a magic line INSIDE a `=begin/=end` block
#   but no comment token carries the setting, stock calls `nil.pos` and RAISES;
#   RuboCop swallows the per-file error and emits NO offense. shirobai must
#   likewise emit nothing (and never raise).
# - `never` removes the FIRST frozen-string-literal-SPECIFIED comment (which may
#   be a `token`/`false` line) even though the gate matched a later valid one.
# - The `Style/Encoding` UTF-8-only insertion pattern: `# encoding: utf-8`
#   drives an insert-after; `# encoding: ascii-8bit` does not (prepend).
# - CRLF line endings around the insertion and removal correctors (`buffer`
#   is LF-normalized, so shirobai takes its standalone path).
#
# Every case runs stock and shirobai side by side and asserts identical
# offenses and identical autocorrect output. The `=begin` raise case is the
# one exception (stock raises inside the cop, so the differential helpers
# cannot run stock): there we assert shirobai alone stays silent and does not
# raise.
RSpec.describe "Style/FrozenStringLiteralComment edge cases" do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Style::FrozenStringLiteralComment,
    Shirobai::Cop::Style::FrozenStringLiteralComment
  ]

  def style_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Style/FrozenStringLiteralComment" => { "EnforcedStyle" => style } }, "(test)"
      ),
      "(test)"
    )
  end

  # ---- oracle self-test: stock fires under each style ----
  it "stock fires (missing) under always" do
    stock = lint_offenses(klasses.first, "puts 1\n", style_config("always"))
    expect(stock).not_to be_empty
  end

  it "stock fires (unnecessary) under never" do
    stock = lint_offenses(klasses.first, "# frozen_string_literal: true\nputs 1\n", style_config("never"))
    expect(stock).not_to be_empty
  end

  it "stock fires (disabled) under always_true" do
    stock = lint_offenses(klasses.first, "# frozen_string_literal: false\nputs 1\n", style_config("always_true"))
    expect(stock).not_to be_empty
  end

  # ---- gate ----
  %w[always never always_true].each do |style|
    it "agrees on whitespace-only files having no tokens (#{style})" do
      ["", " ", "  \n\t\n"].each do |src|
        expect(lint_offenses(klasses.last, src, style_config(style)))
          .to eq(lint_offenses(klasses.first, src, style_config(style)))
      end
    end
  end

  # ---- always: insertion points ----
  {
    "prepend when no special comment" => "puts 1\n",
    "after a shebang" => "#!/usr/bin/env ruby\nputs 1\n",
    "after an encoding (utf-8) comment" => "# encoding: utf-8\nputs 1\n",
    "prepend past a non-utf8 encoding comment" => "# encoding: ascii-8bit\nputs 1\n",
    "after an encoding comment under a shebang" => "#!/usr/bin/env ruby\n# encoding: utf-8\nputs 1\n",
    "prepend when a shebang-shape is not on line 1" => "x = 1\n#!/bin/ruby\n"
  }.each do |desc, source|
    it "matches stock insertion #{desc} (always)" do
      config = style_config("always")
      expect_lint_parity(*klasses, source, config)
      expect_autocorrect_parity(*klasses, source, config)
    end
  end

  # ---- never: removal + surrounding whitespace ----
  {
    "own line then code" => "# frozen_string_literal: true\nputs 1\n",
    "under a shebang" => "#!/bin/ruby\n# frozen_string_literal: true\nputs 1\n",
    "at EOF with no trailing newline" => "#!/bin/ruby\n# frozen_string_literal: true",
    "then a blank line" => "# frozen_string_literal: true\n\nputs 1\n",
    "a disabled comment" => "# frozen_string_literal: false\nputs 1\n",
    "an emacs comment between others" =>
      "# -*- encoding: utf-8 -*-\n# -*- frozen_string_literal: true -*-\n# -*- warn_indent: true -*-\nputs 1\n"
  }.each do |desc, source|
    it "matches stock removal #{desc} (never)" do
      config = style_config("never")
      expect_lint_parity(*klasses, source, config)
      expect_autocorrect_parity(*klasses, source, config)
    end
  end

  it "removes the first specified comment, not the gate-matching one (never)" do
    # Gate matches the `true` line (valid literal); the removed comment is the
    # earlier `foo` line (specified, not valid).
    source = "# frozen_string_literal: foo\n# frozen_string_literal: true\nputs 1\n"
    config = style_config("never")
    expect_lint_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  it "does not fire on a token-only comment (never)" do
    source = "# frozen_string_literal: foo\nputs 1\n"
    config = style_config("never")
    expect_lint_parity(*klasses, source, config, expect_offenses: false)
    expect_autocorrect_parity(*klasses, source, config)
  end

  # ---- always_true: missing vs disabled ----
  {
    "disabled false line (whole-line replace)" => "# frozen_string_literal: false\nputs 1\n",
    "arbitrary token line" => "# frozen_string_literal: token\nputs 1\n",
    "emacs false comment (emacs replacement form)" => "# -*- frozen_string_literal: false -*-\nputs 1\n",
    "missing under a shebang and encoding" => "#!/usr/bin/env ruby\n# encoding: utf-8\nputs 1\n",
    "first specified false wins over a later true" =>
      "# frozen_string_literal: false\n# frozen_string_literal: true\nputs 1\n"
  }.each do |desc, source|
    it "matches stock #{desc} (always_true)" do
      config = style_config("always_true")
      expect_lint_parity(*klasses, source, config)
      expect_autocorrect_parity(*klasses, source, config)
    end
  end

  it "accepts a first-specified true over a later false (always_true)" do
    source = "# frozen_string_literal: true\n# frozen_string_literal: false\nputs 1\n"
    config = style_config("always_true")
    expect_lint_parity(*klasses, source, config, expect_offenses: false)
    expect_autocorrect_parity(*klasses, source, config)
  end

  # ---- mid-file frozen_string_literal comment is invisible to the leading gate ----
  %w[always never always_true].each do |style|
    it "ignores a frozen_string_literal comment under ruby code (#{style})" do
      source = "puts 1\n# frozen_string_literal: true\nputs 2\n"
      config = style_config(style)
      # Under `always`/`always_true` this fires (missing); under `never` it does
      # not (the leading gate sees no comment). Let the differential decide.
      stock = lint_offenses(klasses.first, source, config)
      expect(lint_offenses(klasses.last, source, config)).to eq(stock)
      expect_autocorrect_parity(*klasses, source, config)
    end
  end

  # ---- CRLF around insertion / removal (high-risk) ----
  {
    ["always", "prepend"] => "puts 1\r\n",
    ["always", "after shebang"] => "#!/bin/ruby\r\nputs 1\r\n",
    ["always", "after encoding"] => "# encoding: utf-8\r\nputs 1\r\n",
    ["never", "removal own line"] => "# frozen_string_literal: true\r\nputs 1\r\n",
    ["never", "removal then blank"] => "# frozen_string_literal: true\r\n\r\nputs 1\r\n",
    ["always_true", "disabled replace"] => "# frozen_string_literal: false\r\nputs 1\r\n"
  }.each do |(style, desc), source|
    it "matches stock on CRLF #{desc} (#{style})" do
      config = style_config(style)
      expect_lint_parity(*klasses, source, config)
      expect_autocorrect_parity(*klasses, source, config)
    end
  end

  # ---- the =begin raise quirk (stock raises → no offense; shirobai stays silent) ----
  %w[never always_true].each do |style|
    it "stays silent (no raise) when the setting is only inside a =begin block (#{style})" do
      source = "=begin\n# frozen_string_literal: false\n=end\nputs 1\n"
      # `lint_offenses` asserts the shirobai report has no errors and returns [].
      expect(lint_offenses(klasses.last, source, style_config(style))).to eq([])
    end
  end
end
