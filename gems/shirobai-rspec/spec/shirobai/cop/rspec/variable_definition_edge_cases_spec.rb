# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/VariableDefinition`.
#
# This cop has ZERO positives in every measured corpus, so the vendor spec
# plus these differential specs are the only behavioral safety net. Every
# example runs the STOCK cop and the SHIROBAI cop side by side through
# autocorrect-to-convergence (or lint) and asserts identical offenses AND
# byte-identical corrected source.
#
# Quirks probed against stock rubocop-rspec 3.10.2:
#
# - **symbols style** flags plain `str` names only; **strings style** flags
#   `sym` AND `dsym`. A `dstr` (plain string interpolation) name is never
#   flagged under either style.
# - **empty `%()` is a parser-gem `dstr`** (prism folds it into a StringNode,
#   but the matcher follows parser-gem), so it is skipped; a non-empty
#   `%(abc)` is a `str` and is flagged. An empty quote string `''` is a `str`
#   (flagged, corrected to `:""`).
# - **corrections come from the VALUE** for sym/str (escapes interpreted, then
#   `Symbol#inspect` / `String#to_sym.inspect`) and from the SOURCE for dsym
#   (`variable.source[1..]`).
# - **the gate is `RSpec/VariableName`'s**: send-shaped `let`/`subject`
#   (bare, numblock, or `subject(...)`) inside a top-level spec group.
RSpec.describe Shirobai::Cop::RSpec::VariableDefinition do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::VariableDefinition }

  def config_for(style)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["RSpec/VariableDefinition"] =
      (hash["RSpec/VariableDefinition"] || {}).merge("EnforcedStyle" => style)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  def expect_ac(source, style:)
    expect_autocorrect_parity(stock_class, described_class, source, config_for(style))
  end

  def expect_lint(source, style:, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config_for(style),
                       expect_offenses: expect_offenses)
  end

  describe "symbols style (default)" do
    it "flags and corrects str names, skips sym/dsym/dstr" do
      corrected = expect_ac(<<~'RUBY', style: "symbols")
        RSpec.describe Foo do
          let('user') { 1 }
          subject("other") { 2 }
          let(:already) { 3 }
          let(:"a#{x}") { 4 }
          let("plain#{x}") { 5 }
        end
      RUBY
      expect(corrected).to include("let(:user)", 'subject(:other)')
      expect(corrected).to include('let("plain#{x}")')
    end

    it "handles %q()/%(abc) (str) but skips the empty %() (parser dstr)" do
      corrected = expect_ac(<<~RUBY, style: "symbols")
        RSpec.describe Foo do
          let(%q(pq)) { 1 }
          let(%(abc)) { 2 }
          let(%()) { 3 }
        end
      RUBY
      expect(corrected).to include("let(:pq)", "let(:abc)", "let(%())")
    end

    it "corrects from the interpreted value (escapes, spaces, empty, multibyte)" do
      corrected = expect_ac(<<~'RUBY', style: "symbols")
        RSpec.describe Foo do
          let("user name") { 1 }
          let("tab\tname") { 2 }
          let("quote\"name") { 3 }
          let('') { 4 }
          let("ユーザ") { 5 }
        end
      RUBY
      expect(corrected).to include('let(:"user name")', 'let(:"tab\tname")', 'let(:ユーザ)')
      expect(corrected).to include('let(:"")')
    end

    it "flags bare and numblock lets (send-shaped candidates)" do
      corrected = expect_ac(<<~RUBY, style: "symbols")
        RSpec.describe Foo do
          let('bare')
          let('numbered') { _1 }
        end
      RUBY
      expect(corrected).to include("let(:bare)", "let(:numbered)")
    end

    it "does not touch names outside a top-level spec group" do
      expect_lint("let('user') { 1 }\n", style: "symbols", expect_offenses: false)
      expect_lint(<<~RUBY, style: "symbols", expect_offenses: false)
        class Foo
          describe 'x' do
            let('user') { 1 }
          end
        end
      RUBY
    end
  end

  describe "strings style" do
    it "flags and corrects sym and dsym names, skips str/dstr" do
      corrected = expect_ac(<<~'RUBY', style: "strings")
        RSpec.describe Foo do
          let(:user) { 1 }
          let(:"user name") { 2 }
          let(:"a#{x}") { 3 }
          subject!(:sub) { 4 }
          let('already') { 5 }
          let("plain#{x}") { 6 }
        end
      RUBY
      expect(corrected).to include('let("user")', 'let("user name")', 'subject!("sub")')
      # dsym is corrected by slicing the source (keeps the interpolation).
      expect(corrected).to include('let("a#{x}")')
      expect(corrected).to include("let('already')", 'let("plain#{x}")')
    end

    it "corrects sym names from the value (spaces, escapes, multibyte)" do
      corrected = expect_ac(<<~'RUBY', style: "strings")
        RSpec.describe Foo do
          let(:"tab\tname") { 1 }
          let(:ユーザ) { 2 }
        end
      RUBY
      expect(corrected).to include('let("tab\tname")', 'let("ユーザ")')
    end
  end

  describe "CRLF sources (bundle-ineligible fallback)" do
    # A CRLF source normalizes to LF in the parser buffer while `raw_source`
    # keeps the `\r`s, so shared-walk offsets no longer line up with parser
    # positions. `bundle_eligible?` must route these files to the standalone
    # entry point over `buffer.source` — both the offense positions and the
    # autocorrected bytes must stay byte-identical to stock.
    it "matches stock offenses and autocorrect on a CRLF source" do
      src = "describe 'x' do\r\n  let('user') { 1 }\r\nend\r\n"
      expect_lint(src, style: "symbols")
      # Both sides rewrite from the LF-normalized parser buffer, so the
      # corrected output is byte-identical (and LF) on both.
      corrected = expect_ac(src, style: "symbols")
      expect(corrected).to include("let(:user)")
    end
  end
end
