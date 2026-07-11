# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceInsideStringInterpolation`.
#
# Rust reproduces stock's `SurroundingSpace` detection and `SpaceCorrector`
# autocorrect from the `#{` / `}` delimiter byte positions (no
# `tokens_within`). These differential cases pin the quirks the vendor spec
# under-tests:
#
# - the offense gate is `/[ \t]/` (space OR tab), but the space-style insert is
#   gated on `space_after?` / `space_before?` (any `\s`), so a single-line `\r`
#   neighbour reports an offense that the autocorrect leaves in place — the one
#   place the offense gate and the edit gate disagree;
# - whitespace-only interpolations (`#{}` / `#{ }`) never offend;
# - the autocorrect runs once per interpolation (both sides fixed together);
# - symbol / regexp interpolation hosts behave the same.
RSpec.describe Shirobai::Cop::Layout::SpaceInsideStringInterpolation do
  include EdgeCaseParity

  def sisi_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/SpaceInsideStringInterpolation" => { "Enabled" => true, "EnforcedStyle" => style } },
        "(test)"
      ),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Layout::SpaceInsideStringInterpolation,
    Shirobai::Cop::Layout::SpaceInsideStringInterpolation
  ]

  describe "no_space style (default)" do
    it "removes space on both sides in one pass" do
      expect_autocorrect_parity(*klasses, "\"\#{ x }\"\n", sisi_config("no_space"))
    end

    it "removes a run of spaces / a tab" do
      expect_autocorrect_parity(*klasses, "\"\#{  x  }\"\n", sisi_config("no_space"))
      expect_autocorrect_parity(*klasses, "\"\#{\tx}\"\n", sisi_config("no_space"))
    end

    it "flags only the offending side" do
      expect_autocorrect_parity(*klasses, "\"\#{ x}\"\n", sisi_config("no_space"))
    end
  end

  describe "space style" do
    it "inserts space on both missing sides" do
      expect_autocorrect_parity(*klasses, "\"\#{x}\"\n", sisi_config("space"))
    end

    # The `\r` quirk: a single-line CR right after `#{` makes the offense fire
    # (CR is not space/tab) but the autocorrect skip that side (CR is `\s`), so
    # only the `}` side is corrected. Autocorrect must match byte for byte.
    it "reports but does not correct a bare CR neighbour" do
      expect_autocorrect_parity(*klasses, "\"\#{\rx}\"\n", sisi_config("space"))
      expect_autocorrect_parity(*klasses, "\"\#{x\r}\"\n", sisi_config("space"))
    end
  end

  describe "cases that must NOT offend" do
    it "ignores empty and whitespace-only interpolations" do
      ["\"\#{}\"\n", "\"\#{ }\"\n", "\"\#{  }\"\n"].each do |src|
        expect_lint_parity(*klasses, src, sisi_config("no_space"), expect_offenses: false)
        expect_lint_parity(*klasses, src, sisi_config("space"), expect_offenses: false)
      end
    end

    it "ignores multiline interpolations" do
      expect_lint_parity(*klasses, "\"\#{ x\n}\"\n", sisi_config("no_space"), expect_offenses: false)
    end
  end

  describe "other interpolation hosts and multiple interpolations" do
    it "handles symbol and regexp interpolation" do
      expect_autocorrect_parity(*klasses, ":\"\#{ x }\"\n", sisi_config("no_space"))
      expect_autocorrect_parity(*klasses, "/\#{ x }/\n", sisi_config("no_space"))
    end

    it "handles two interpolations, one clean" do
      expect_autocorrect_parity(*klasses, "\"a\#{ x }b\#{y}\"\n", sisi_config("no_space"))
    end
  end
end
