# frozen_string_literal: true

require "spec_helper"

# Regression guard for the byte-vs-character offset dimension that COUNT
# parity cannot see.
#
# Rust reports prism BYTE offsets, but `Parser::Source::Range` indexes the
# buffer by CHARACTERS. On an ASCII-only file the two coincide, so the vendor
# specs and the full-corpus offense-count parity both pass even when a wrapper
# hands byte offsets straight to `Parser::Source::Range` — yet on a non-ASCII
# file every offense located after a multibyte character lands shifted (or, if
# the shifted range falls outside the buffer, the cop raises and silently
# drops its offenses in a default Commissioner run).
#
# These examples run the stock cop and the shirobai cop side by side over
# sources that put a multibyte comment BEFORE the offense, in autocorrect
# mode, and assert that the first-pass offenses (down to begin/end position,
# message, status, correctable?) and the fully autocorrected source are
# identical. Each fixture is chosen to exercise every offset field the cop's
# wrapper receives from Rust (autocorrect ranges included), and each case also
# asserts stock produced at least one offense so a mistyped fixture cannot
# pass vacuously. A final group replays the same comparison over Ruby's own
# `fileutils.rb` (multibyte comments; the file where the divergence was first
# demonstrated) for every implemented cop.
RSpec.describe "non-ASCII source offset parity with stock RuboCop" do
  # A multibyte line shifting every later byte offset ahead of its char offset.
  prefix = "# 多バイト文字を含むコメント\n"

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  # Runs `klass` over `source` in autocorrect mode with the vendor-spec
  # iteration semantics (same cop instance across passes, loop until the
  # corrector is empty or a fixpoint). Returns the first-pass offense
  # snapshots and the final source.
  def autocorrect_run(klass, source, config)
    cop = klass.new(config)
    cop.instance_variable_get(:@options)[:autocorrect] = true
    src = source
    first_offenses = nil
    11.times do |iteration|
      processed = RuboCop::ProcessedSource.new(src, RuboCop::TargetRuby::DEFAULT_VERSION)
      processed.config = config
      processed.registry = RuboCop::Cop::Registry.global
      team = RuboCop::Cop::Team.new([cop], config, raise_error: true)
      report = team.investigate(processed)
      offenses = report.offenses.map do |o|
        [o.location.begin_pos, o.location.end_pos, o.message, o.status, o.correctable?]
      end.sort
      first_offenses ||= offenses
      corrector = report.correctors.first
      break if corrector.nil? || corrector.empty?

      rewritten = corrector.rewrite
      break if rewritten == src
      raise "autocorrect loop did not converge" if iteration == 10

      src = rewritten
    end
    [first_offenses, src]
  end

  def stock_and_shirobai(cop_name)
    department, name = cop_name.split("/")
    [RuboCop::Cop.const_get(department).const_get(name),
     Shirobai::Cop.const_get(department).const_get(name)]
  end

  def expect_parity(cop_name, source, config)
    stock_klass, shirobai_klass = stock_and_shirobai(cop_name)
    stock_offenses, stock_corrected = autocorrect_run(stock_klass, source, config)
    shirobai_offenses, shirobai_corrected = autocorrect_run(shirobai_klass, source, config)
    expect(shirobai_offenses).to eq(stock_offenses)
    expect(shirobai_corrected).to eq(stock_corrected)
    stock_offenses
  end

  # Every fixture exercises the cop's full offset tuple: the offense range
  # plus any autocorrect ranges its wrapper receives from Rust (noted inline).
  cases = {
    # start/fin + AlignmentCorrector on the offense range.
    "Layout/ArgumentAlignment" => "foo(a,\n  b)\n",
    # start/fin (the hanging `)`) + AlignmentCorrector.
    "Layout/ClosingParenthesisIndentation" => "some_method(a\n)\n",
    # dot range + remove range + insert position.
    "Layout/DotPosition" => "foo.\n  bar\n",
    # start/fin + the separate correction range (cs/ce).
    "Layout/FirstArgumentIndentation" => "run(\n:foo)\n",
    # first element and hanging `]` ranges + node-resolved realignment.
    "Layout/FirstArrayElementIndentation" => "a << [\n 1\n  ]\n",
    # first pair and hanging `}` ranges + node-resolved / key-line realignment.
    "Layout/FirstHashElementIndentation" => "a << {\n a: 1\n  }\n",
    # start/fin (the offending child node range) + the node-resolved
    # AlignmentCorrector realignment by column_delta.
    "Layout/IndentationConsistency" => "def m\n  a\n   b\nend\n",
    # start/fin + cs/ce node range + the prior-offense-range accumulation
    # (the second offense is suppressed as within the first correction).
    "Layout/IndentationWidth" => "def m\n    begin\n    x\n    end\nend\n",
    # candidate line data + the breakable insertion offset.
    "Layout/LineLength" =>
      "x = some_method(aaaaaaaaaa, bbbbbbbbbb, cccccccccc, dddddddddd, " \
      "eeeeeeeeee, ffffffffff, gggggggggg, hhhhhhhhhh, iiiiiiiiii)\n",
    # start/fin + the block body and block `end` ranges (block-aware path).
    "Layout/MultilineMethodCallIndentation" => "foo.bar\n  .baz do\n    x\n  end\n",
    # start/fin + AlignmentCorrector.
    "Layout/MultilineOperationIndentation" => "x = 1 +\n2\n",
    # start/fin only (no autocorrect).
    "Lint/Debugger" => "binding.irb\n",
    # offense range + replacement, and the wrap range (parenthesization).
    "Lint/SafeNavigationChain" => "x&.foo.bar\ndo_something && x&.foo >= bar\n",
    # start/fin + whole-line removal derived in Ruby from the range.
    "Lint/UselessAccessModifier" =>
      "class C\n  private\n  private\n  def m\n  end\n  protected\nend\n",
    # start/fin + replace range and remove range (plus an empty-corrector case).
    "Lint/Void" => "self; top\n42 unless condition\nfoo\n",
    # start/fin/head_end.
    "Metrics/BlockLength" => "foo do\n#{"  x = 1\n" * 26}end\n",
    # start/fin.
    "Metrics/BlockNesting" => "if a\n if b\n  if c\n   if d\n    x\n   end\n  end\n end\nend\n",
    # start/fin/head_end.
    "Metrics/CyclomaticComplexity" => "def m\n#{(1..8).map { |i| "  x#{i} if c#{i}\n" }.join}end\n",
    # start/fin/head_end.
    "Metrics/PerceivedComplexity" => "def m\n#{(1..9).map { |i| "  x#{i} if c#{i}\n" }.join}end\n",
    # start/fin/head_end (default Max 17, so 18 assignments => vector <18, 0, 0>).
    "Metrics/AbcSize" => "def m\n#{(1..18).map { |i| "  v#{i} = #{i}\n" }.join}end\n",
    # start/fin/head_end (default Max 100 => 101 counted lines; no autocorrect).
    "Metrics/ClassLength" => "class Test\n#{"  x = 1\n" * 101}end\n",
    # start/fin/head_end for the module and the `Foo = Module.new do` name range.
    "Metrics/ModuleLength" =>
      "module Test\n#{"  x = 1\n" * 101}end\nFoo = Module.new do\n#{"  x = 1\n" * 101}end\n",
    # start/fin (fb_start/fb_end are covered by the forbidden-identifier case below).
    "Naming/MethodName" => "def fooBar; end\n",
    # start/fin.
    "Naming/PredicatePrefix" => "def is_foo; end\n",
    # start/fin.
    "Naming/VariableNumber" => "foo_1 = 1\n",
    # start/fin + replace range and unused-argument remove range.
    "Style/HashEachMethods" => "foo.keys.each { |k| p k }\nbar.each { |unused_key, v| p v }\n",
    # operator range + replace range.
    "Style/LineEndConcatenation" => "x = 'a' +\n    'b'\n",
    # `self` range + dot range (both removed).
    "Style/RedundantSelf" => "def m\n  self.foo\nend\n",
    # Offense line ranges (removal corrector) after a multibyte comment.
    "Layout/EmptyLinesAroundMethodBody" => "def m\n\n  x\nend\n",
    # Same-range begin/end dedup + the insertion corrector is covered by the
    # missing-at-end path of the autocorrect loop (removal first pass).
    "Layout/EmptyLinesAroundClassBody" => "class C\n\n  x\n\nend\n",
    "Layout/EmptyLinesAroundModuleBody" => "module M\n\n  x\nend\n",
    "Layout/EmptyLinesAroundBlockBody" => "foo do\n  x\n\nend\n",
    "Layout/EmptyLinesAroundBeginBody" => "begin\n\n  x\nrescue\n  y\n\nend\n",
    "Layout/EmptyLinesAroundExceptionHandlingKeywords" =>
      "def m\n  x\n\nrescue\n\n  y\nensure\n\n  z\nend\n",
    # The offense range (`def b` location) plus the autocorrect `newline_pos`
    # (the byte offset of the `\n` after `end`), all shifted by the multibyte
    # comment; the insert arm adds the missing empty line.
    "Layout/EmptyLineBetweenDefs" => "def a\nend\ndef b\nend\n",
    # The whole-modifier offense range and the autocorrect `range_by_whole_lines`
    # anchor, both shifted by the multibyte comment.  The guard's `if x` arm
    # also exercises the parser-parent walk through DefNode/StatementsNode.
    "Layout/EmptyLineAfterGuardClause" =>
      "def foo\n  return if 日本\n  bar\nend\n",
    # The offense line range (the whole empty line plus its `\n`), which is also
    # the removal corrector range, shifted by the multibyte comment.
    "Layout/EmptyLinesAroundArguments" => "foo(\n\n  bar\n)\n",
    # The `do` token range, the correction ops (delimiter replacements plus a
    # comment relocation with a multibyte comment text) and the cross-pass
    # ignored-range accumulation (the autocorrect loop's second pass must not
    # resurrect the nested block suppressed by the first offense).
    "Style/BlockDelimiters" =>
      "foo {\n  bar do |x| x end\n} # マルチバイト末尾コメント\neach do |y| end\n",
    # The `end` keyword range and the autocorrect whitespace range (`end`'s
    # column run) plus the alignment column, all shifted by the multibyte
    # comment; the replace arm re-indents `end` to the keyword column.
    "Layout/EndAlignment" => "var = if test\nend\n",
    # The `end` keyword range and the autocorrect whitespace range (`end`'s
    # column run) plus the alignment column, all shifted by the multibyte
    # comment; the replace arm re-indents the misaligned def `end` to the `def`
    # keyword column under the default `start_of_line` style.
    "Layout/DefEndAlignment" => "def foo\n  end\n",
    # The closing-token range and the autocorrect insert/remove arms (the
    # over-indented `end` is de-indented to the block start column), all shifted
    # by the multibyte comment.
    "Layout/BlockAlignment" => "test do\n  end\n",
    # The `else` keyword range and the autocorrect line-shift arm (the
    # over-indented `else` is de-indented to the `if` column), all shifted by
    # the multibyte comment.
    "Layout/ElseAlignment" => "if a\n  b\n else\n  c\nend\n",
    # The offending pair node range plus the per-part key/separator/value
    # ranges Rust hands the wrapper for `insert_before` / `remove`, all shifted
    # by the multibyte comment; the misaligned key is de-indented to alignment.
    "Layout/HashAlignment" => "h = {\n  a: 0,\n   bb: 1\n}\n",
    # The offending pair range plus every corrector op offset (key replace,
    # surrounding-space remove) handed to the wrapper, all shifted by the
    # multibyte comment; each rocket pair is rewritten to ruby19 syntax.
    "Style/HashSyntax" => "h = { :a => 0, :b => 1 }\n",
    # The offending string node range plus the autocorrect replacement, all
    # shifted by the multibyte comment. The string *content* is itself
    # multibyte, so the wrapper's `to_string_literal` on the decoded content
    # must round-trip the UTF-8 bytes (double quotes -> single under the default
    # single_quotes style).
    "Style/StringLiterals" => "x = \"日本語の文字列\"\n",
    # The trailing-comma offense (default `no_comma`) carries an `avoid_comma`
    # caret range and a removal corrector; both offsets sit after the multibyte
    # comment, so the byte->char conversion must shift them.
    "Style/TrailingCommaInArguments" => "some_method(あ, い,)\n",
    # Same `avoid_comma` shape on a braced hash literal: the caret range and
    # the removal corrector both sit after multibyte keys/values, so every
    # offset must be byte->char converted.
    "Style/TrailingCommaInHashLiteral" => "h = { a: \"あ\", b: \"い\", }\n",
    # Same `avoid_comma` shape on an array literal with multibyte elements.
    "Style/TrailingCommaInArrayLiteral" => "x = [\"あ\", \"い\",]\n",
    # The offending interpolation-internal string node range plus the autocorrect
    # replacement, all shifted by the multibyte comment. The inner string content
    # is itself multibyte, so the wrapper's `to_string_literal` on the decoded
    # content must round-trip the UTF-8 bytes (double -> single under the default
    # single_quotes style); the outer double-quoted string stays untouched.
    "Style/StringLiteralsInInterpolation" => "x = \"前 \#{\"日本語\"} 後\"\n",
    # Trailing blank lines: the caret range and the autocorrect replacement
    # range both sit at end-of-source, after the multibyte comment, so their
    # byte offsets are shifted ahead of the char offsets and must be converted.
    "Layout/TrailingEmptyLines" => "x = 0\n\n\n",
    # The offending whitespace runs (before/after a `.` and after a `::`) all
    # sit after the multibyte comment, so their byte offsets are shifted ahead
    # of the char offsets and must be converted; the removal corrector range is
    # the same range.
    "Layout/SpaceAroundMethodCallOperator" => "あ.foo . bar\nRuboCop:: Cop\n",
    # The keyword ranges (the `case` and `when` here) sit after the multibyte
    # comment, so their byte offsets are shifted ahead of the char offsets and
    # must be converted; the insert_before / insert_after corrector anchors at
    # the same range.
    "Layout/SpaceAroundKeyword" => "x = 1 # あ\ncase a when\"\"; end\n",
    # The inner-brace offense ranges (the `insert_before` anchors after `{` and
    # before `}`) sit after the multibyte comment and contain a multibyte inner
    # body, so their byte offsets are shifted ahead of the char offsets and must
    # be converted; both spaces are inserted under the default `space` style.
    "Layout/SpaceInsideBlockBraces" => "foo.each {puts 日本語}\n",
    # Both brace offenses (the `{` and `}` anchors of the insert correctors)
    # sit after the multibyte comment and the hash value is itself multibyte,
    # so every offense/corrector byte offset must be converted.
    "Layout/SpaceInsideHashLiteralBraces" => "h = {a: \"日本語\", b: 2}\n",
    # The two space-run offense ranges and the node's removal corrector
    # ranges all sit after the multibyte comment with multibyte elements
    # between them, so the byte offsets must be converted.
    "Layout/SpaceInsideArrayLiteralBrackets" => "a = [ \"日本語\", 2 ]\n",
    # The `{` offense range (also the `insert_before` anchor) sits after the
    # multibyte comment and after a multibyte receiver on the same line.
    "Layout/SpaceBeforeBlockBraces" => "あいう.each{ puts 1 }\n",
    # Predicate-style call with an `&&` argument: the whole-call offense range
    # sits after the multibyte comment, so the byte offsets must be converted
    # to character offsets. No autocorrect, so no other ranges to check.
    "Lint/RequireParentheses" => "day.is? '日本語' && month == :jan\n",
    # `foo = foo` sits after the multibyte comment; the byte offsets of the
    # whole-assignment offense range must be converted to character offsets.
    # The body mixes assignment shapes (lvasgn, masgn, casgn, attribute setter
    # and `[]=`) so each shape's offset path runs through `SourceOffsets`.
    # No autocorrect, so no other ranges to check.
    "Lint/SelfAssignment" =>
      "x = x\nfoo, bar = foo, bar\nFoo = Foo\nself.あ = self.あ\nh[\"日本語\"] = h[\"日本語\"]\n",
    # Nested unparenthesized call inside a parenthesized one, with multibyte
    # method names and arguments after the multibyte comment. Exercises the
    # offense range and both autocorrect anchors (the surrounding-space
    # replace `[ac_open_start, ac_open_end)` and the zero-width
    # `insert_after` at `ac_close_pos`).
    "Style/NestedParenthesizedCalls" =>
      "puts(compute 日本語)\n",
    # Space-before-`(` call where both the offense (the space) and the message
    # source (the `(...)` argument) come after the multibyte comment. The
    # message embeds the multibyte argument source verbatim, so the wrapper's
    # `arg_start..arg_end` byte→char conversion is on the offense path.
    "Lint/ParenthesesAsGroupedExpression" =>
      "あ.func (日本語)\n",
    # Multi-statement body where the unreachable expression sits after a
    # multibyte comment: the offense range is the `bar` token after `return`,
    # whose byte offset comes through the bundle wire and must be converted
    # to character offset via `SourceOffsets`.
    "Lint/UnreachableCode" =>
      "def f\n  # 日本語\n  return\n  bar\nend\n",
    # Multibyte content inside the percent literal AND the offense sitting after
    # the multibyte comment: exercises the offense range plus both autocorrect
    # anchors (`begin_loc` for `%w(`→`%w[` and the single-byte closer at
    # `end_loc`). The element bytes themselves include multibyte so the
    # contains-delimiter check has to scan them correctly too.
    "Style/PercentLiteralDelimiters" =>
      "x = %w(日本 中)\n",
    # The close `)` offense range sits AFTER the multibyte comment, and stock's
    # `MultilineLiteralBraceCorrector` reads the parser-gem node's
    # `loc.begin`/`loc.end`/`children.last` ranges (CHARACTER offsets). The
    # wrapper feeds the send node located via byte→char-converted
    # `(send_node_start, send_node_end)` so every range the corrector touches
    # comes out byte-correct on a multibyte source.
    "Layout/MultilineMethodCallBraceLayout" =>
      "日本.foo(a,\n  b\n)\n",
    # A misaligned `private` in a multi-statement class body sits after the
    # multibyte prefix; the send range Rust hands the wrapper is in BYTES and
    # the autocorrect line-shift's leading-whitespace range also has to be
    # computed in character space. Exercises both the offense highlight and
    # `AlignmentCorrector.correct`'s insert/remove arm in non-ASCII source.
    "Layout/AccessModifierIndentation" =>
      "class A\n  X = 1\nprivate\nend\n",
    # An assignment whose multibyte LHS sits after the multibyte comment; the
    # RHS sits on a fresh line at column 0 and the offense range plus the
    # autocorrect target both fall after the multibyte prefix. Exercises the
    # `(rhs_start, rhs_end)` byte→char conversion as well as relocating the
    # `Parser::AST::Node` by char offset for `AlignmentCorrector#correct`.
    "Layout/AssignmentIndentation" =>
      "日本語 =\nif b ; end\n",
    # Both flavors (variable assignment + setter) plus the rhs slice the
    # wrapper does `source.byteslice` on for kind=0 — every offset must be
    # converted from prism's byte basis to `Parser::Source::Range` characters,
    # and `byteslice(rhs_start, rhs_end - rhs_start)` must use the *byte*
    # offset (Rust hands us byte indices) so the multibyte rhs source survives
    # round-trip.
    "Style/RedundantSelfAssignment" =>
      "foo = foo.concat(日本)\nother.bar = other.bar.concat(日本)\n",
    # The `::` token sits AFTER the multibyte comment. The offense range and
    # the autocorrect `replace` range are the same `[dot_start, dot_end)` two
    # bytes; both must come out byte-correct on a multibyte source.
    "Style/ColonMethodCall" =>
      "あ::method_name(arg)\n",
    # A stabby lambda missing parentheses, with multibyte argument names. The
    # args range Rust hands the wrapper is in BYTES; the offense highlight
    # and the wrap `(`/`)` insertion anchors all need byte→char conversion.
    "Style/StabbyLambdaParentheses" =>
      "->あ,い,う { あ + い + う }\n",
    # `_.each_with_object({}) {|(k,v),h| h[transform(k)] = v}` ⇒
    # `_.transform_keys {|k| transform(k)}`. The offense range, the four
    # corrector edits (selector + args + body, plus the leading `Hash[` /
    # trailing `]` strip in the brackets shape) are all returned as byte
    # offsets from Rust; the wrapper must SourceOffsets-translate every one
    # before handing them to `Parser::Source::Range`. A multibyte prefix
    # shifts each offset and exposes any spot we forgot.
    "Style/HashTransformKeys" =>
      "{a: 1, b: 2}.each_with_object({}) {|(k, v), h| h[foo(k)] = v}\n",
    # `some_method a { … }` with a multibyte block body: the whole-call offense
    # range, the autocorrect `remove` + `insert_before` whitespace anchor, the
    # `insert_after` zero-width range, and the `%<param>s` / `%<method>s` MSG
    # substitutions all run through `SourceOffsets`. The multibyte content
    # exercises the byte->char shift on every offset.
    "Lint/AmbiguousBlockAssociation" =>
      "some_method あ { |el| puts 日本語 }\n",
    # Two empty `#` lines (in default `AllowMarginComment: true` mode they
    # chunk together — joined `#\n#\n` matches `/\A(#\n)+\z/` and both get
    # flagged). The offense range and the whole-line removal range both sit
    # after the multibyte comment, so the byte→char conversion runs on every
    # offset Rust returns.
    "Layout/EmptyComment" =>
      "x = 0\n#\n#\n",
    # A magic comment immediately followed by code, after the multibyte
    # prefix comment. The offense is a 1-byte range at column 0 of the line
    # below the magic, and the autocorrect inserts `"\n"` there. Both
    # offsets fall after the multibyte prefix, so the byte→char conversion
    # runs on every offset.
    "Layout/EmptyLineAfterMagicComment" =>
      "# frozen_string_literal: true\nclass Foo; end\n",
    # Two blank lines between statements after the multibyte comment: the
    # 1-byte offense range and the same range used as the removal corrector
    # both sit after the multibyte prefix, so the byte->char conversion runs
    # on every offset Rust returns.
    "Layout/EmptyLines" =>
      "a = 1\n\n\nb = 2\n",
    # NOTE: `Layout/LeadingEmptyLines` does not appear in this synthetic
    # `prefix + body` sweep — the shared prefix puts a comment on line 1,
    # which IS a token, so the cop never fires once prepended. The cop's
    # byte→char path still gets non-ASCII coverage from the fileutils.rb
    # sweep below (which uses the cop's own multibyte first token).
    # keyword offense range + node replace range and the Rust-built
    # replacement text (multibyte body slice), both directions.
    "Style/IfUnlessModifier" =>
      "if condition\n  do_stuff(\"あいう\")\nend\n" \
      "foo_bar(\"えお\") if #{"a" * 100}_condition\n",
  }

  cases.each do |cop_name, body|
    describe cop_name do
      it "matches stock offenses and autocorrect output after a multibyte comment" do
        offenses = expect_parity(cop_name, prefix + body, config)
        expect(offenses).not_to be_empty, "fixture produced no stock offense; fix the source"
      end
    end
  end

  # The forbidden-identifier branch reports a separate Rust-provided range
  # (fb_start/fb_end) and runs through the cop's standalone (non-bundled)
  # entry point, which must convert offsets all the same.
  describe "Naming/MethodName with ForbiddenIdentifiers" do
    it "matches stock offenses after a multibyte comment" do
      forbidden_config = RuboCop::ConfigLoader.merge_with_default(
        RuboCop::Config.new(
          { "Naming/MethodName" => { "ForbiddenIdentifiers" => %w[fooBar] } }, "(test)"
        ),
        "(test)"
      )
      offenses = expect_parity("Naming/MethodName", "#{prefix}def fooBar; end\n", forbidden_config)
      expect(offenses).not_to be_empty, "fixture produced no stock offense; fix the source"
    end
  end

  # Real-file sweep: Ruby's own fileutils.rb carries multibyte comments and is
  # the file where the byte/char divergence was first demonstrated against
  # stock (2-byte shift; DotPosition dropping offenses). Replays the parity
  # comparison for every implemented cop. Individual cops may legitimately
  # find nothing here (the synthetic cases above are the non-vacuous ones).
  describe "Ruby stdlib fileutils.rb" do
    fileutils_path = File.join(RbConfig::CONFIG["rubylibdir"], "fileutils.rb")

    it "is present and non-ASCII (precondition for the sweep)" do
      expect(File).to exist(fileutils_path)
      expect(File.read(fileutils_path).ascii_only?).to be(false)
    end

    Shirobai::Cop.constants(false).sort.each do |department|
      mod = Shirobai::Cop.const_get(department)
      next unless mod.is_a?(Module)

      mod.constants(false).sort.each do |name|
        klass = mod.const_get(name)
        next unless klass.is_a?(Class) && klass < RuboCop::Cop::Base

        describe klass.cop_name do
          it "matches stock offenses and autocorrect output" do
            source = File.read(fileutils_path)
            expect_parity(klass.cop_name, source, config)
          end
        end
      end
    end
  end
end
