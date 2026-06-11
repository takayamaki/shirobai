# frozen_string_literal: true

require "spec_helper"

# Regression guard for a drop-in compat dimension the vendor specs CANNOT see.
#
# `RuboCop::RSpec::ExpectOffense#set_formatter_options` forces
# `@options[:autocorrect] = true` for every example, so the vendor specs only
# ever exercise the autocorrect path. They never check LINT-mode behaviour.
#
# But `Layout/LineLength` etc. default to `AutoCorrect: always`, so even in lint
# mode RuboCop yields the corrector block and a *non-empty* corrector makes the
# offense `:uncorrected` — i.e. `correctable?` — which stock reports as
# "[Correctable]" and counts in the "N offenses auto-correctable" summary. A
# shirobai cop that skips building the corrector in lint mode silently flips the
# offense to `:unsupported`, keeping the offense COUNT identical (so e2e parity
# passes) while diverging from stock's actual lint output.
#
# These examples run stock and shirobai cops side by side in **lint mode**
# (a bare Commissioner, no autocorrect option) and assert identical offenses
# down to `status` / `correctable?`. Each case also asserts stock produced at
# least one offense, so a mistyped source can't make the test pass vacuously.
RSpec.describe "lint-mode correctable parity with stock RuboCop" do
  def lint_offenses(klass, source)
    config = RuboCop::ConfigLoader.default_configuration
    ruby_version = RuboCop::TargetRuby::DEFAULT_VERSION
    cop = klass.new(config)
    processed = RuboCop::ProcessedSource.new(source, ruby_version)
    # A real run always carries the config on the processed source (the Runner
    # sets it); correctors like `AlignmentCorrector` read it even in lint mode.
    processed.config = config
    processed.registry = RuboCop::Cop::Registry.global
    report = RuboCop::Cop::Commissioner.new([cop]).investigate(processed)
    expect(report.errors).to be_empty
    report.offenses.map do |o|
      [o.location.begin_pos, o.location.end_pos, o.message, o.status, o.correctable?]
    end.sort
  end

  cases = {
    "Layout/LineLength" => [
      RuboCop::Cop::Layout::LineLength,
      Shirobai::Cop::Layout::LineLength,
      "x = some_method(aaaaaaaaaa, bbbbbbbbbb, cccccccccc, dddddddddd, " \
      "eeeeeeeeee, ffffffffff, gggggggggg, hhhhhhhhhh, iiiiiiiiii)\n"
    ],
    "Layout/DotPosition" => [
      RuboCop::Cop::Layout::DotPosition,
      Shirobai::Cop::Layout::DotPosition,
      "foo.\n  bar\n"
    ],
    "Style/LineEndConcatenation" => [
      RuboCop::Cop::Style::LineEndConcatenation,
      Shirobai::Cop::Style::LineEndConcatenation,
      "x = 'a' +\n    'b'\n"
    ],
    "Layout/ClosingParenthesisIndentation" => [
      RuboCop::Cop::Layout::ClosingParenthesisIndentation,
      Shirobai::Cop::Layout::ClosingParenthesisIndentation,
      "some_method(a\n)\n"
    ],
    "Layout/FirstArrayElementIndentation" => [
      RuboCop::Cop::Layout::FirstArrayElementIndentation,
      Shirobai::Cop::Layout::FirstArrayElementIndentation,
      "a << [\n 1\n  ]\n"
    ]
  }

  cases.each do |name, (stock_klass, shirobai_klass, source)|
    describe name do
      it "matches stock offense status/correctable? in lint mode" do
        stock = lint_offenses(stock_klass, source)
        expect(stock).not_to be_empty, "fixture produced no stock offense; fix the source"
        expect(lint_offenses(shirobai_klass, source)).to eq(stock)
      end
    end
  end
end
