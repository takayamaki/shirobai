# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/ArrayAlignment`, pinning the
# bracket-less array shapes and corrector behaviours found by stock probing
# that the vendor spec never exercises:
#
# * parser-gem synthesizes an `array` node in three bracket-less spots —
#   single-assignment RHS lists (`a = 1, 2`, setter sends included), masgn RHS
#   lists (skipped via `parent&.masgn_type?`) and `rescue` exception lists.
#   prism has no ArrayNode for rescue exceptions and different parents for the
#   others, so each mapping is replayed here against stock.
# * `AlignmentCorrector` receives the element NODE from stock, which protects
#   heredoc bodies and multi-line string interiors (taboo ranges) when the
#   element is shifted. A bare range would rewrite them.
# * the `within?` rule downgrades an offense nested inside an
#   already-registered offense range to report-only (`:unsupported`).
RSpec.describe Shirobai::Cop::Layout::ArrayAlignment do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::ArrayAlignment,
    Shirobai::Cop::Layout::ArrayAlignment
  ]

  # A config with `EnforcedStyle` set to `style` on top of defaults.
  # `Config#to_h` returns the default configuration's INTERNAL hash, so it
  # must be duped before the key is reassigned — mutating it in place leaks
  # the style into every later spec that reads the (identity-memoized)
  # default configuration, and `Layout/FirstArrayElementIndentation` derives
  # its enforce flag from `Layout/ArrayAlignment`.
  def config_for(style)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["Layout/ArrayAlignment"] =
      hash["Layout/ArrayAlignment"].merge("EnforcedStyle" => style)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  let(:first_element_config) { config_for("with_first_element") }
  let(:fixed_config) { config_for("with_fixed_indentation") }

  context "rescue exception lists (parser-gem wraps them in a bracket-less array)" do
    it "checks a misaligned exception list under with_first_element" do
      source = "begin\n  x\nrescue FooError,\n    BarError => e\n  y\nend\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
    end

    it "checks the exception list against the rescue keyword's line under with_fixed_indentation" do
      source = "begin\n  x\nrescue FooError,\n       BarError => e\n  y\nend\n"
      expect_lint_parity(*klasses, source, fixed_config)
      expect_autocorrect_parity(*klasses, source, fixed_config)
    end

    it "accepts an aligned exception list in both styles" do
      expect_lint_parity(*klasses,
                         "begin\n  x\nrescue FooError,\n       BarError\n  y\nend\n",
                         first_element_config, expect_offenses: false)
      expect_lint_parity(*klasses,
                         "begin\n  x\nrescue FooError,\n  BarError\n  y\nend\n",
                         fixed_config, expect_offenses: false)
    end

    it "checks each clause of a chained rescue separately" do
      source = "begin\n  x\nrescue FooError,\n    BarError\n  y\nrescue BazError,\n     QuxError\n  z\nend\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
    end

    it "ignores a splat-only exception list (single child)" do
      expect_lint_parity(*klasses, "begin\n  x\nrescue *ERRORS => e\n  y\nend\n",
                         first_element_config, expect_offenses: false)
    end
  end

  context "bracket-less single-assignment RHS lists" do
    it "checks a plain assignment list in both styles" do
      source = "a = 1,\n  2,\n   3\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
      expect_lint_parity(*klasses, source, fixed_config)
      expect_autocorrect_parity(*klasses, source, fixed_config)
    end

    it "checks setter sends (attribute, index and safe navigation)" do
      ["foo.bar = 1,\n   2\n", "foo[0] = 1,\n   2\n", "foo&.bar = 1,\n   2\n"].each do |source|
        expect_lint_parity(*klasses, source, first_element_config)
        expect_autocorrect_parity(*klasses, source, first_element_config)
        expect_lint_parity(*klasses, source, fixed_config)
        expect_autocorrect_parity(*klasses, source, fixed_config)
      end
    end

    it "uses the outer assignment's line for a chained assignment under with_fixed_indentation" do
      source = "a = b = 1,\n   2\n"
      expect_lint_parity(*klasses, source, fixed_config)
      expect_autocorrect_parity(*klasses, source, fixed_config)
    end

    it "checks constant and global assignments" do
      ["A = 1,\n   2\n", "$g = 1,\n   2\n", "A::B = 1,\n   2\n"].each do |source|
        expect_lint_parity(*klasses, source, first_element_config)
        expect_autocorrect_parity(*klasses, source, first_element_config)
      end
    end
  end

  context "masgn RHS lists (skipped like stock's `parent&.masgn_type?`)" do
    it "skips a misaligned bracket-less masgn RHS" do
      expect_lint_parity(*klasses, "a, b = 1,\n        2\n",
                         first_element_config, expect_offenses: false)
      expect_lint_parity(*klasses, "a, b = 1,\n        2\n",
                         fixed_config, expect_offenses: false)
    end

    it "skips a misaligned bracketed masgn RHS too" do
      expect_lint_parity(*klasses, "a, b = [1,\n        2]\n",
                         first_element_config, expect_offenses: false)
    end

    it "still checks an array nested inside a masgn RHS" do
      source = "a, b = [[1,\n   2], 3]\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
    end
  end

  context "percent literal arrays" do
    it "realigns %w and %i elements in both styles" do
      ["x = %w[aa\n    bb\n    cc]\n", "x = %i[aa\n    bb]\n"].each do |source|
        expect_lint_parity(*klasses, source, first_element_config)
        expect_autocorrect_parity(*klasses, source, first_element_config)
        expect_lint_parity(*klasses, source, fixed_config)
        expect_autocorrect_parity(*klasses, source, fixed_config)
      end
    end
  end

  context "corrector taboo ranges (stock passes the element NODE)" do
    it "does not shift the interior of a multi-line string element" do
      source = "x = [1,\n  \"foo\nbar\",\n  2]\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
    end

    it "does not shift a heredoc body inside a shifted element" do
      source = "x = [{ sql: <<EOF\nSELECT 1\nEOF\n     },\n    { sql: <<EOF\nSELECT 2\nEOF\n    }]\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
    end

    it "uses the heredoc opener as the alignment base when it is the first element" do
      source = "x = [<<~EOS,\n  body\nEOS\n   2]\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
    end
  end

  context "the `within?` nested-offense rule" do
    it "reports the inner offense of an already-shifted element without correcting it" do
      source = "x = [[1,\n   2],\n  [3,\n 4]]\n"
      stock = expect_lint_parity(*klasses, source, first_element_config)
      # The `4` offense sits inside the realigned `[3,\n 4]` range: stock
      # keeps it correctable?=false / :unsupported. Pin that shape.
      expect(stock.map { |o| o[4] }).to include(false)
      expect_autocorrect_parity(*klasses, source, first_element_config)
    end
  end

  context "element shapes the vendor spec never mixes in" do
    it "treats a trailing braceless hash as one element" do
      source = "x = [1,\n   foo: 2]\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
    end

    it "ignores an array whose only child is a braceless hash" do
      expect_lint_parity(*klasses, "x = [foo: 1,\n   bar: 2]\n",
                         first_element_config, expect_offenses: false)
    end

    it "aligns splat elements" do
      source = "x = [*a,\n   *b]\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
    end

    it "handles a tab-indented element (display column of a tab)" do
      source = "x = [1,\n\t2]\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
      expect_lint_parity(*klasses, source, fixed_config)
      expect_autocorrect_parity(*klasses, source, fixed_config)
    end

    it "handles comment lines between elements" do
      source = "x = [1,\n  # c\n   2]\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
    end

    it "handles a block comment inside the array" do
      source = "x = [1,\n=begin\nc\n=end\n   2]\n"
      expect_lint_parity(*klasses, source, first_element_config)
      expect_autocorrect_parity(*klasses, source, first_element_config)
    end

    it "ignores multi-value return/break/next (no parser array node)" do
      ["def m\n  return 1,\n2\nend\n",
       "while x\n  break 1,\n2\nend\n",
       "while x\n  next 1,\n2\nend\n"].each do |source|
        expect_lint_parity(*klasses, source, first_element_config, expect_offenses: false)
        expect_lint_parity(*klasses, source, fixed_config, expect_offenses: false)
      end
    end
  end
end
