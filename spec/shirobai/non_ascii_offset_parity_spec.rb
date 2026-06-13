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
    # The `do` token range, the correction ops (delimiter replacements plus a
    # comment relocation with a multibyte comment text) and the cross-pass
    # ignored-range accumulation (the autocorrect loop's second pass must not
    # resurrect the nested block suppressed by the first offense).
    "Style/BlockDelimiters" =>
      "foo {\n  bar do |x| x end\n} # マルチバイト末尾コメント\neach do |y| end\n"
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
