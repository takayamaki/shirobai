# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/VariableName`.
#
# Quirks probed against stock rubocop-rspec 3.10.2 that the vendor spec
# does not pin:
#
# - **The gate is the OUTERMOST statement**: `inside_example_group?` finds
#   the node's top-level enclosing statement and asks whether THAT is a
#   `spec_group?`. A group wrapped in a top-level class/module never
#   counts; a class INSIDE a top-level group is transparent.
# - **`spec_group?` is any_block**: a numblock (`{ _1 }`) top-level group
#   counts.
# - **Candidates are send-shaped**: a bare `let(:name)` without a block, a
#   numblock `let(:name) { _1 }`, and `subject(:name)` all match; the
#   receiver must be nil (an `RSpec.let` never matches, `RSpec.describe`
#   only matters for the group gate).
# - **Only plain sym/str names**: dstr/dsym names are skipped entirely.
# - **No operator escape**: 1.88's ConfigurableFormatting has no operator
#   guard, so `let(:+)` IS flagged.
# - **Unicode**: `[[:lower:]]`/`[[:upper:]]` are Unicode properties —
#   `café_name` passes snake_case, `ユーザ` (no case) fails both styles.
# - **Style detection**: valid names mark the current style as detected;
#   offenses mark the alternative (or "unrecognized"), feeding
#   `--auto-gen-config`. Compared via `config_to_allow_offenses`.
RSpec.describe Shirobai::Cop::RSpec::VariableName do
  let(:stock_class) { RuboCop::Cop::RSpec::VariableName }

  def build_config(cop_overrides = {})
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["RSpec/VariableName"] = (hash["RSpec/VariableName"] || {}).merge(cop_overrides)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  def investigate(klass, source, config, ruby_version)
    cop = klass.new(config)
    processed = RuboCop::ProcessedSource.new(source, ruby_version)
    processed.config = config
    processed.registry = RuboCop::Cop::Registry.global
    report = RuboCop::Cop::Commissioner.new([cop]).investigate(processed)
    expect(report.errors).to be_empty
    offenses = report.offenses.map do |o|
      [o.location.begin_pos, o.location.end_pos, o.message, o.status, o.correctable?]
    end.sort
    [offenses, cop]
  end

  # Differential: offenses AND the detected-style bookkeeping
  # (config_to_allow_offenses drives --auto-gen-config) must match stock.
  def expect_parity(source, config: build_config, ruby_version: 3.1, expect_offenses: true)
    stock_offenses, stock_cop = investigate(stock_class, source, config, ruby_version)
    if expect_offenses
      expect(stock_offenses).not_to be_empty, "fixture produced no stock offense; fix the source"
    end
    shirobai_offenses, shirobai_cop = investigate(described_class, source, config, ruby_version)
    expect(shirobai_offenses).to eq(stock_offenses)
    expect(shirobai_cop.config_to_allow_offenses).to eq(stock_cop.config_to_allow_offenses)
    stock_offenses
  end

  describe "the top-level statement gate" do
    it "ignores groups wrapped in a top-level class" do
      expect_parity(<<~RUBY, expect_offenses: false)
        class Foo
          describe 'x' do
            let(:userName) { 1 }
          end
        end
      RUBY
    end

    it "ignores groups wrapped in a top-level module" do
      expect_parity(<<~RUBY, expect_offenses: false)
        module Foo
          describe 'x' do
            let(:userName) { 1 }
          end
        end
      RUBY
    end

    it "flags through a class inside the group" do
      expect_parity(<<~RUBY)
        describe 'x' do
          class Foo
            let(:userName) { 1 }
          end
        end
      RUBY
    end

    it "counts a numblock top-level group (spec_group? is any_block)" do
      expect_parity("RSpec.describe('x') { let(:userName) { _1 } }\n")
    end

    it "counts shared groups but not include_context blocks" do
      expect_parity(<<~RUBY)
        shared_examples 'x' do
          let(:userName) { 1 }
        end
      RUBY
      expect_parity(<<~RUBY, expect_offenses: false)
        include_context 'x' do
          let(:userName) { 1 }
        end
      RUBY
    end

    it "ignores top-level lets and non-rspec receivers" do
      expect_parity("let(:userName) { 1 }\n", expect_offenses: false)
      expect_parity(<<~RUBY, expect_offenses: false)
        Foo.describe 'x' do
          let(:userName) { 1 }
        end
      RUBY
    end
  end

  describe "send-shaped candidates" do
    it "flags a bare let without a block" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let(:userName)
        end
      RUBY
    end

    it "flags a numblock let (the matcher never sees the block)" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let(:userName) { _1 }
        end
      RUBY
    end

    it "skips dstr and dsym names in either style" do
      source = <<~'RUBY'
        describe 'x' do
          let("user#{x}Name") { 1 }
          let(:"user#{x}Name") { 1 }
        end
      RUBY
      expect_parity(source, expect_offenses: false)
      expect_parity(
        source,
        config: build_config("EnforcedStyle" => "camelCase"),
        expect_offenses: false
      )
    end

    it "flags operator symbol names (no operator escape in 1.88)" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let(:+) { 1 }
        end
      RUBY
    end

    it "handles %q() and %() literals like stock" do
      # %q() is a plain str (candidate); %() is a dstr (skipped).
      expect_parity(<<~RUBY)
        describe 'x' do
          let(%q(badName)) { 1 }
          let(%(otherBadName)) { 2 }
        end
      RUBY
    end
  end

  describe "unicode names" do
    it "treats accented lowercase as [[:lower:]]" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let(:café_name) { 1 }
          let(:caféName) { 2 }
        end
      RUBY
    end

    it "treats caseless scripts as neither style" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let(:ユーザ) { 1 }
        end
      RUBY
      expect_parity(<<~RUBY, config: build_config("EnforcedStyle" => "camelCase"))
        describe 'x' do
          let(:ユーザ) { 1 }
        end
      RUBY
    end
  end

  describe "camelCase style" do
    let(:config) { build_config("EnforcedStyle" => "camelCase") }

    it "flags snake_case and spaced names" do
      expect_parity(<<~RUBY, config: config)
        describe 'x' do
          let(:user_name) { 1 }
          let('user name') { 2 }
          let(:_leadingOk) { 3 }
          let(:_) { 4 }
        end
      RUBY
    end
  end

  describe "AllowedPatterns" do
    it "filters offenses and style detection like stock" do
      config = build_config("AllowedPatterns" => ["^userFood"])
      expect_parity(<<~RUBY, config: config)
        describe 'x' do
          let(:userFood_1) { 1 }
          let(:userName) { 2 }
          let(:okay_name) { 3 }
        end
      RUBY
    end

    it "matches against the literal VALUE, not the source" do
      config = build_config("AllowedPatterns" => ["\\AuserFood\\z"])
      expect_parity(<<~'RUBY', config: config, expect_offenses: false)
        describe 'x' do
          let("userFood") { 1 }
        end
      RUBY
    end
  end

  describe "style detection bookkeeping" do
    it "marks the alternative style when every offense fits it" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let(:userName) { 1 }
        end
      RUBY
    end

    it "gives up when an offense fits no style" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let(:'user name') { 1 }
        end
      RUBY
    end

    it "records the current style from valid names" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'x' do
          let(:good_name) { 1 }
        end
      RUBY
    end
  end

  describe "CRLF sources (bundle-ineligible fallback)" do
    # A CRLF source normalizes to LF in the parser buffer while `raw_source`
    # keeps the `\r`s, so shared-walk offsets no longer line up with parser
    # positions. `bundle_eligible?` must route these files to the standalone
    # entry point over `buffer.source`.
    it "matches stock offense positions on a CRLF source" do
      expect_parity("describe 'x' do\r\n  let(:userName) { 1 }\r\nend\r\n")
    end
  end
end
