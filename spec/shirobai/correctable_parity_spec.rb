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
    # The argument is an INTERPOLATED string: the `#` of `#{` must not be
    # taken for a comment, or the line loses its breakable insertion point and
    # the offense flips from correctable to `:unsupported` (regression seen on
    # stdlib fileutils.rb `raise ArgumentError, "...#{...}"` lines).
    "Layout/LineLength (interpolated string argument)" => [
      RuboCop::Cop::Layout::LineLength,
      Shirobai::Cop::Layout::LineLength,
      "raise ArgumentError, \"#{"a" * 95}xxxx #\{path.inspect} tail tail\"\n"
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
    ],
    "Layout/FirstHashElementIndentation" => [
      RuboCop::Cop::Layout::FirstHashElementIndentation,
      Shirobai::Cop::Layout::FirstHashElementIndentation,
      "a << {\n a: 1\n  }\n"
    ],
    # A nested offense-within-offense: the inner block's misindentation is
    # reported but stays correctable-without-correction (`within?` the outer
    # range) while the outer one carries a corrector. Both statuses must match
    # stock in lint mode.
    "Layout/IndentationConsistency" => [
      RuboCop::Cop::Layout::IndentationConsistency,
      Shirobai::Cop::Layout::IndentationConsistency,
      "describe A do\n  render_views\n    describe B do\n        it C do\n      end\n    end\nend\n"
    ],
    "Style/HashEachMethods" => [
      RuboCop::Cop::Style::HashEachMethods,
      Shirobai::Cop::Style::HashEachMethods,
      "foo.keys.each { |k| p k }\nbar.each { |unused_key, v| p v }\n"
    ],
    # One corrected offense (`self`) plus one whose corrector block stays
    # empty (a literal in a modifier-conditional branch): both statuses must
    # match stock.
    "Lint/Void" => [
      RuboCop::Cop::Lint::Void,
      Shirobai::Cop::Lint::Void,
      "self; top\n42 unless condition\nfoo\n"
    ],
    # An unused trailing modifier plus a repeated one: both corrected by
    # whole-line removal (`AutoCorrect: contextual` still yields the corrector
    # in a plain run).
    "Lint/UselessAccessModifier" => [
      RuboCop::Cop::Lint::UselessAccessModifier,
      Shirobai::Cop::Lint::UselessAccessModifier,
      "class C\n  private\n  private\n  def m\n  end\n  protected\nend\n"
    ],
    # Extra blank at the body beginning (removal corrector).
    "Layout/EmptyLinesAroundMethodBody" => [
      RuboCop::Cop::Layout::EmptyLinesAroundMethodBody,
      Shirobai::Cop::Layout::EmptyLinesAroundMethodBody,
      "def m\n\n  x\nend\n"
    ],
    # Both blanks of an empty body land on the same range: stock's
    # add_offense dedup must keep a single (beginning) offense.
    "Layout/EmptyLinesAroundClassBody" => [
      RuboCop::Cop::Layout::EmptyLinesAroundClassBody,
      Shirobai::Cop::Layout::EmptyLinesAroundClassBody,
      "class C\n\nend\n"
    ],
    "Layout/EmptyLinesAroundModuleBody" => [
      RuboCop::Cop::Layout::EmptyLinesAroundModuleBody,
      Shirobai::Cop::Layout::EmptyLinesAroundModuleBody,
      "module M\n\n  x\nend\n"
    ],
    "Layout/EmptyLinesAroundBlockBody" => [
      RuboCop::Cop::Layout::EmptyLinesAroundBlockBody,
      Shirobai::Cop::Layout::EmptyLinesAroundBlockBody,
      "foo do\n  x\n\nend\n"
    ],
    # Blanks after `begin` and before `end` across rescue sections.
    "Layout/EmptyLinesAroundBeginBody" => [
      RuboCop::Cop::Layout::EmptyLinesAroundBeginBody,
      Shirobai::Cop::Layout::EmptyLinesAroundBeginBody,
      "begin\n\n  x\nrescue\n  y\n\nend\n"
    ],
    # Blanks around `rescue` and `ensure` keywords (removal correctors).
    "Layout/EmptyLinesAroundExceptionHandlingKeywords" => [
      RuboCop::Cop::Layout::EmptyLinesAroundExceptionHandlingKeywords,
      Shirobai::Cop::Layout::EmptyLinesAroundExceptionHandlingKeywords,
      "def m\n  x\n\nrescue\n\n  y\nensure\n\n  z\nend\n"
    ],
    # One corrected offense (single-line do-end to braces) plus one whose
    # corrector stays empty (`correction_would_break_code?`: unparenthesized
    # send arguments): both statuses must match stock.
    "Style/BlockDelimiters" => [
      RuboCop::Cop::Style::BlockDelimiters,
      Shirobai::Cop::Style::BlockDelimiters,
      "each do |x| end\ns.subspec 'Subspec' do |sp| end\n"
    ],
    # A pure metric cop: no autocorrect, so both stock and shirobai offenses
    # must stay `:unsupported` (never correctable). Guards against the wrapper
    # accidentally attaching a corrector block. The default `Max` is 17, so the
    # body needs an ABC score above it (18 assignments => vector <18, 0, 0>).
    "Metrics/AbcSize" => [
      RuboCop::Cop::Metrics::AbcSize,
      Shirobai::Cop::Metrics::AbcSize,
      "def m\n#{(1..18).map { |i| "  v#{i} = #{i}" }.join("\n")}\nend\n"
    ],
    "Metrics/MethodLength" => [
      RuboCop::Cop::Metrics::MethodLength,
      Shirobai::Cop::Metrics::MethodLength,
      "def m\n#{(1..11).map { |i| "  v = #{i}" }.join("\n")}\nend\n"
    ],
    "Layout/EmptyLineBetweenDefs" => [
      RuboCop::Cop::Layout::EmptyLineBetweenDefs,
      Shirobai::Cop::Layout::EmptyLineBetweenDefs,
      "def a\nend\ndef b\nend\n"
    ],
    "Layout/EmptyLinesAroundArguments" => [
      RuboCop::Cop::Layout::EmptyLinesAroundArguments,
      Shirobai::Cop::Layout::EmptyLinesAroundArguments,
      "foo(\n\n  bar\n)\n"
    ],
    "Layout/EndAlignment" => [
      RuboCop::Cop::Layout::EndAlignment,
      Shirobai::Cop::Layout::EndAlignment,
      "var = if test\nend\n"
    ],
    "Layout/DefEndAlignment" => [
      RuboCop::Cop::Layout::DefEndAlignment,
      Shirobai::Cop::Layout::DefEndAlignment,
      "def foo\n  end\n"
    ],
    "Layout/BlockAlignment" => [
      RuboCop::Cop::Layout::BlockAlignment,
      Shirobai::Cop::Layout::BlockAlignment,
      "test do\n  end\n"
    ],
    "Layout/ElseAlignment" => [
      RuboCop::Cop::Layout::ElseAlignment,
      Shirobai::Cop::Layout::ElseAlignment,
      "if test\n  x\n else\n  y\nend\n"
    ],
    "Layout/HashAlignment" => [
      RuboCop::Cop::Layout::HashAlignment,
      Shirobai::Cop::Layout::HashAlignment,
      "h = {\n  a: 0,\n   bb: 1\n}\n"
    ],
    "Style/HashSyntax" => [
      RuboCop::Cop::Style::HashSyntax,
      Shirobai::Cop::Style::HashSyntax,
      "h = { :a => 0, :b => 1 }\n"
    ],
    # A double-quoted string under the default single_quotes style: the offense
    # carries a corrector (correctable), and the unaffected single-quoted string
    # emits no offense. Guards that the wrapper attaches the corrector block in
    # lint mode like stock.
    "Style/StringLiterals" => [
      RuboCop::Cop::Style::StringLiterals,
      Shirobai::Cop::Style::StringLiterals,
      "a = \"double\"\nb = 'single'\n"
    ],
    # Default style is `no_comma`: the single-line trailing comma is an
    # `avoid_comma` offense whose corrector removes the comma.
    "Style/TrailingCommaInArguments" => [
      RuboCop::Cop::Style::TrailingCommaInArguments,
      Shirobai::Cop::Style::TrailingCommaInArguments,
      "some_method(a, b, c,)\n"
    ],
    # A double-quoted string inside an interpolation under the default
    # single_quotes style: the offense carries a corrector (correctable), and
    # the outer double-quoted string is unaffected (not inside interpolation).
    "Style/StringLiteralsInInterpolation" => [
      RuboCop::Cop::Style::StringLiteralsInInterpolation,
      Shirobai::Cop::Style::StringLiteralsInInterpolation,
      "a = \"x \#{\"inner\"} y\"\n"
    ],
    # Multiple trailing blank lines under the default `final_newline` style: the
    # offense carries a `replace` corrector (correctable) and its caret range
    # starts one byte after the first trailing newline.
    "Layout/TrailingEmptyLines" => [
      RuboCop::Cop::Layout::TrailingEmptyLines,
      Shirobai::Cop::Layout::TrailingEmptyLines,
      "x = 0\n\n\n"
    ],
    # Spaces around a `.` call operator: the offense carries a removal corrector
    # (correctable). Guards that the wrapper attaches the corrector block in
    # lint mode like stock.
    "Layout/SpaceAroundMethodCallOperator" => [
      RuboCop::Cop::Layout::SpaceAroundMethodCallOperator,
      Shirobai::Cop::Layout::SpaceAroundMethodCallOperator,
      "foo . bar\n"
    ],
    # A missing space after a keyword: the offense carries an `insert_after`
    # corrector (correctable). Guards that the wrapper attaches the corrector
    # block in lint mode like stock.
    "Layout/SpaceAroundKeyword" => [
      RuboCop::Cop::Layout::SpaceAroundKeyword,
      Shirobai::Cop::Layout::SpaceAroundKeyword,
      "if\"\"; end\n"
    ],
    # A block brace missing its inner spaces under the default `space` style:
    # both offenses carry an `insert_before` corrector (correctable). Guards
    # that the wrapper attaches the corrector block in lint mode like stock.
    "Layout/SpaceInsideBlockBraces" => [
      RuboCop::Cop::Layout::SpaceInsideBlockBraces,
      Shirobai::Cop::Layout::SpaceInsideBlockBraces,
      "foo.each {puts x}\n"
    ],
    # A predicate-style call with an `&&` argument: stock has no autocorrect,
    # so both stock and shirobai offenses must stay `:unsupported` (never
    # correctable). Guards that the wrapper does not accidentally attach a
    # corrector block.
    "Lint/RequireParentheses" => [
      RuboCop::Cop::Lint::RequireParentheses,
      Shirobai::Cop::Lint::RequireParentheses,
      "day.is? 'monday' && month == :jan\n"
    ],
    # `foo = foo` is a stock no-autocorrect cop, so both stock and shirobai
    # offenses must stay `:unsupported` (never correctable). Guards against
    # the wrapper accidentally attaching a corrector block.
    "Lint/SelfAssignment" => [
      RuboCop::Cop::Lint::SelfAssignment,
      Shirobai::Cop::Lint::SelfAssignment,
      "foo = foo\nfoo.bar = foo.bar\nfoo['k'] = foo['k']\n"
    ],
    # A nested unparenthesized call inside a parenthesized one: the offense
    # carries a `replace`+`insert_after` corrector (correctable). Guards that
    # the wrapper attaches the corrector block in lint mode like stock.
    "Style/NestedParenthesizedCalls" => [
      RuboCop::Cop::Style::NestedParenthesizedCalls,
      Shirobai::Cop::Style::NestedParenthesizedCalls,
      "puts(compute something)\n"
    ],
    # A space-before-`(` call: the offense carries a `remove(range)` corrector
    # (correctable). Guards that the wrapper attaches the corrector block in
    # lint mode like stock.
    "Lint/ParenthesesAsGroupedExpression" => [
      RuboCop::Cop::Lint::ParenthesesAsGroupedExpression,
      Shirobai::Cop::Lint::ParenthesesAsGroupedExpression,
      "a.func (x)\n"
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
