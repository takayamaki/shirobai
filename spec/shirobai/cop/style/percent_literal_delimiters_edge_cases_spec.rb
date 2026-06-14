# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/PercentLiteralDelimiters`.
#
# The vendor spec covers each percent type's basic "preferred wins / other
# flags / contains preferred chars skips" trio, plus a `%w`/`%i` pairing
# delimiter case and the multi-line autocorrect indentation. Real-machine
# probing turned up several quirks the vendor spec does not exercise, plus a
# couple of prism/parser-AST mapping subtleties that the implementation has
# to get right — pinned here as differential tests so a refactor cannot
# silently regress them.
#
# - `%(string)` (no interpolation) is a parser `:str`; `%(#{...})` (with
#   interpolation) becomes a `:dstr`. The cop must register the offense for
#   both flavours under the same `%` type entry (vendor covers this implicitly
#   but the differential pins it explicitly).
# - **prism only**: `%()` (empty body) is parsed as `InterpolatedStringNode`,
#   not `StringNode` — the visitor must dispatch on the interpolated path
#   even though the literal has zero `parts`.
# - `%r(.*)i` keeps the regex options after autocorrect: stock's `loc.end` is
#   the closer byte alone (so `corrector.replace(loc.end, ']')` keeps the
#   trailing `i`), but prism's `closing_loc` spans `)i`. The rule must trim
#   the replacement range to a single byte.
# - `%w` / `%i`'s pair-character skip uses the BEGIN delimiter's matchpair:
#   `%w(\(some\))` keeps `(` because escaping `[`-`]` would not help — the
#   bytes `(` and `)` are inside, so flipping is fine, but stock's
#   `include_same_character_as_used_for_delimiter?` also skips when the
#   begin's PAIR is present (here, `(` and `)` form the pair).
# - `%w` whose elements have raw newlines (`%w(\nfoo\nbar\n)`) — parser splits
#   on whitespace and yields two `:str` children for `foo` and `bar`; the
#   children's source bytes are what the contains check scans, not the raw
#   literal between the delimiters.
# - `%s[symbol]` is a parser `:sym` whose only child is a Ruby `Symbol`
#   primitive (`:symbol`). Stock's `string_source` returns `nil` for the
#   primitive, so the contains check has zero children — there is no way to
#   skip via "contains preferred delimiter" for `%s`. The implementation must
#   mirror this.
# - `default` only sets the fallback; per-type keys still override. The vendor
#   spec covers this but the differential pins that the bundle path's
#   `bundle_args` resolution matches stock's `PreferredDelimiters` resolution
#   when a `default` is set with one explicit override.
# - The empty `%w()` array offends like any non-preferred delimiter — the
#   contains check sees no children, so nothing skips.
RSpec.describe Shirobai::Cop::Style::PercentLiteralDelimiters do
  include EdgeCaseParity

  # RuboCop default: `default: ()`, %i/%I/%w/%W => [], %r => {}.
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Style::PercentLiteralDelimiters,
    Shirobai::Cop::Style::PercentLiteralDelimiters
  ]

  describe "type dispatch parity" do
    it 'treats `%[no-interp]` (str) the same as `%[#{...}]` (dstr)' do
      # Both are `%` type; the cop's `on_str` / `on_dstr` alias forwards them
      # to the same processor. With the RuboCop default `default: ()`, both
      # offend with non-`()` delimiters.
      expect_lint_parity(*klasses, "x = %[string]\n", config)
      src = +"x = "
      src << "%[\#{val}]\n"
      expect_lint_parity(*klasses, src, config)
    end

    it "flags `%[]` (parsed as InterpolatedStringNode with zero parts)" do
      # prism-only quirk: prism makes an empty `%[]` an InterpolatedStringNode
      # even though there is no interpolation. Stock parser-gem reports this
      # as `:dstr` (matching prism here). The cop must still register because
      # the visitor dispatches on the interpolated form.
      expect_lint_parity(*klasses, "x = %[]\n", config)
    end

    it "flags `%s(symbol)` (sym primitive child, no contains check possible)" do
      # `%s(symbol)` has the begin delimiter `(`. With the RuboCop default
      # `default: ()`, the cop normally accepts `()` for `%s`. Use an explicit
      # override so the offense fires for `%s(symbol)`.
      forced = RuboCop::ConfigLoader.merge_with_default(
        RuboCop::Config.new(
          {
            "Style/PercentLiteralDelimiters" => {
              "PreferredDelimiters" => { "default" => "()", "%s" => "[]" }
            }
          },
          "(test)"
        ),
        "(test)"
      )
      expect_lint_parity(*klasses, "x = %s(symbol)\n", forced)
    end
  end

  describe "regex options preserved through autocorrect" do
    it "keeps the trailing `i` when flipping `%r(.*)i` to `%r[.*]i`" do
      # stock loc.end is the closer alone; prism closing_loc spans `)i`. The
      # rule must hand back the single-byte close range so the corrector
      # leaves `i` untouched.
      _, corrected = autocorrect_run(klasses.last, "x = %r(.*)i\n", config)
      expect(corrected).to eq("x = %r{.*}i\n")
      expect_autocorrect_parity(*klasses, "x = %r(.*)i\n", config)
    end

    it "keeps a multi-char options run (`%r{.*}im`)" do
      forced = RuboCop::ConfigLoader.merge_with_default(
        RuboCop::Config.new(
          { "Style/PercentLiteralDelimiters" => {
              "PreferredDelimiters" => { "default" => "()", "%r" => "()" }
          } },
          "(test)"
        ),
        "(test)"
      )
      expect_autocorrect_parity(*klasses, "x = %r{.*}im\n", forced)
    end
  end

  describe "matchpair / contains-preferred-char skips" do
    it "skips `%w(\\(some\\))` (matchpair of `(`: `(`/`)`)" do
      expect_lint_parity(
        *klasses,
        'x = %w(\(some\))' + "\n",
        config,
        expect_offenses: false
      )
      expect(
        lint_offenses(klasses.first, 'x = %w(\(some\))' + "\n", config)
      ).to be_empty
    end

    it "skips `%i(\\(\\) each)`" do
      expect_lint_parity(
        *klasses,
        'x = %i(\(\) each)' + "\n",
        config,
        expect_offenses: false
      )
      expect(
        lint_offenses(klasses.first, 'x = %i(\(\) each)' + "\n", config)
      ).to be_empty
    end

    it "skips `%w([some] [words])` when `[`/`]` appear in element source" do
      expect_lint_parity(
        *klasses,
        "x = %w([some] [words])\n",
        config,
        expect_offenses: false
      )
      expect(
        lint_offenses(klasses.first, "x = %w([some] [words])\n", config)
      ).to be_empty
    end
  end

  describe "word-array element splitting" do
    it "ignores raw delimiters in whitespace runs (parser splits on whitespace)" do
      # The raw newlines / extra spaces between elements aren't carried into
      # any element source, so the contains check only sees `foo` / `bar`.
      # `%w(...)` => `%w[...]` (preferred `[]`).
      expect_autocorrect_parity(*klasses, "%w(\nfoo\nbar\n)\n", config)
    end

    it 'treats `%w(#{val})` as a literal element (no interpolation)' do
      # `%w` does not interpolate; the element source includes the literal
      # `#{val}` bytes, which the contains check sees as a single token (no
      # `[`/`]`), so the offense still fires.
      src = +"x = "
      src << "%w(\#{val})\n"
      expect_lint_parity(*klasses, src, config)
    end

    it "flags an empty `%w()` array" do
      expect_lint_parity(*klasses, "x = %w()\n", config)
    end
  end

  describe "`default` + per-type override resolution" do
    it "applies `default: '{}'` to types without explicit overrides" do
      forced = RuboCop::ConfigLoader.merge_with_default(
        RuboCop::Config.new(
          {
            "Style/PercentLiteralDelimiters" => {
              "PreferredDelimiters" => { "default" => "{}", "%w" => "[]" }
            }
          },
          "(test)"
        ),
        "(test)"
      )
      # `%w[a b]` keeps preferred, `%i(a)` flags because default is `{}`, and
      # `%(s)` flags too.
      expect_lint_parity(*klasses, "x = %i(a)\n", forced)
      expect_lint_parity(*klasses, "x = %(s)\n", forced)
      expect_lint_parity(
        *klasses,
        "x = %w[a b]\n",
        forced,
        expect_offenses: false
      )
    end
  end
end
