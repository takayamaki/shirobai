# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/SubjectStub`.
#
# Quirks probed against stock rubocop-rspec 3.10.2 that the vendor spec
# does not pin:
#
# - **Top-level spine**: `TopLevelGroup#top_level_nodes` unwraps `class` /
#   `module` / the implicit statement sequence (parser `begin`) ONLY. An
#   explicit `begin...end` (parser `kwbegin`) and `class << self` are NOT
#   unwrapped, so groups inside them are never top-level groups.
# - **numblock top-level groups count**: `spec_group?` is an `any_block`
#   matcher, so a numblock group at the top level IS processed --
#   `expect(subject)` / `is_expected` inside it fires. But its named
#   subjects are keyed by `each_ancestor(:block)` (plain blocks only), so a
#   name defined directly in a numblock group never becomes active.
# - **Descent opacity**: `find_subject_expectations` descends only
#   `send`/`def`/`block`/`begin` children. Expectations under a nested
#   numblock, or under an `lvasgn` (e.g. a lambda assigned to a variable),
#   are invisible; a `csend`-receiver block (`x&.tap do ... end`) is a
#   parser `block` and IS descended.
# - **Exact matcher arity**: `message_expectation?` binds exactly one
#   matcher argument; `.to(matcher, extra)` never matches, while
#   `.to(matcher) { blk }` does (the pattern matches the inner send).
# - **Bare-name targets only**: `allow(foo(1))` (an argument-carrying call)
#   is not a `(send nil? %)` target.
# - **String-named subjects are invisible**: `subject('s')` matches neither
#   `(sym $_)` nor the bare form, so `allow(s)` is not a subject stub.
RSpec.describe Shirobai::Cop::RSpec::SubjectStub do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::SubjectStub }

  def config_with(overrides = {})
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    ss = (hash["RSpec/SubjectStub"] || {}).dup
    hash["RSpec/SubjectStub"] = ss.merge(overrides)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  def expect_parity(source, config: config_with, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config,
                       expect_offenses: expect_offenses)
  end

  describe "top-level spine" do
    it "flags groups wrapped in a top-level class or module" do
      expect_parity(<<~RUBY)
        class Wrapper
          describe Foo do
            subject(:foo) { described_class.new }
            it { allow(foo).to receive(:bar) }
          end
        end
      RUBY
      expect_parity(<<~RUBY)
        module Wrapper
          module Inner
            describe Foo do
              subject(:foo) { described_class.new }
              it { allow(foo).to receive(:bar) }
            end
          end
        end
      RUBY
    end

    it "does not flag groups wrapped in an explicit begin (kwbegin)" do
      expect_parity(<<~RUBY, expect_offenses: false)
        begin
          describe Foo do
            subject(:foo) { described_class.new }
            it { allow(foo).to receive(:bar) }
          end
        end
      RUBY
    end

    it "does not flag groups wrapped in a singleton class" do
      expect_parity(<<~RUBY, expect_offenses: false)
        class << self
          describe Foo do
            subject(:foo) { described_class.new }
            it { allow(foo).to receive(:bar) }
          end
        end
      RUBY
    end

    it "does not flag groups nested inside an arbitrary DSL block" do
      expect_parity(<<~RUBY, expect_offenses: false)
        weird_dsl do
          describe Foo do
            subject(:foo) { described_class.new }
            it { allow(foo).to receive(:bar) }
          end
        end
      RUBY
    end
  end

  describe "numblock groups" do
    it "flags subject/is_expected stubs inside a numblock top-level group" do
      expect_parity(<<~RUBY)
        RSpec.describe('x') {
          _1
          specify do
            expect(subject).to receive(:bar)
          end
        }
      RUBY
      expect_parity(<<~RUBY)
        RSpec.describe('x') {
          _1
          is_expected.to receive(:bar)
        }
      RUBY
    end

    it "flags a named stub in a plain context nested in a numblock group" do
      expect_parity(<<~RUBY)
        RSpec.describe('x') {
          _1
          context 'c' do
            subject(:foo) { y }
            specify { allow(foo).to receive(:bar) }
          end
        }
      RUBY
    end

    it "does not flag a named stub whose subject is keyed under the numblock" do
      # find_all_explicit keys by each_ancestor(:block) -- the numblock group
      # is not a :block, so :foo lands under nil and never activates.
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe('x') {
          _1
          subject(:foo) { y }
          specify do
            allow(foo).to receive(:bar)
          end
        }
      RUBY
    end

    it "does not flag expectations under a numblock nested in a plain group" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe 'x' do
          subject(:foo) { y }
          context('c') {
            _1
            allow(foo).to receive(:bar)
          }
        end
      RUBY
    end
  end

  describe "descent opacity" do
    it "does not flag expectations inside a lambda assigned to a variable" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe 'x' do
          subject(:foo) { y }
          f = -> { allow(foo).to receive(:bar) }
        end
      RUBY
    end

    it "flags expectations inside a csend-receiver block" do
      expect_parity(<<~RUBY)
        RSpec.describe 'x' do
          subject(:foo) { y }
          helper&.tap do
            allow(foo).to receive(:bar)
          end
        end
      RUBY
    end
  end

  describe "matcher and target shapes" do
    it "does not flag a target call carrying arguments" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          subject(:foo) { x }
          it { allow(foo(1)).to receive(:bar) }
        end
      RUBY
    end

    it "does not flag a runner with two arguments" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          subject(:foo) { x }
          it { expect(foo).to receive(:bar), extra }
        end
      RUBY
    end

    it "flags a runner with a matcher argument and a block" do
      expect_parity(<<~RUBY)
        describe Foo do
          subject(:foo) { x }
          it { expect(foo).to(receive(:bar)) { baz } }
        end
      RUBY
    end

    it "does not flag expect_any_instance_of" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          subject(:foo) { x }
          it { expect_any_instance_of(Foo).to receive(:bar) }
        end
      RUBY
    end

    it "does not flag a string-named subject stubbed by name" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          subject('s') { x }
          it { allow(s).to receive(:bar) }
        end
      RUBY
    end
  end

  describe "definition order and scoping" do
    it "flags a stub textually before its subject definition" do
      expect_parity(<<~RUBY)
        describe Foo do
          before { allow(foo).to receive(:bar) }
          subject(:foo) { x }
        end
      RUBY
    end

    it "does not flag sibling-group subjects (superset filtered on replay)" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe Foo do
          describe '#bar' do
            subject(:bar) { :bar }
            it { }
          end

          describe '#baz' do
            specify { allow(bar).to receive(:bar) }
          end
        end
      RUBY
    end

    it "does not flag subjects redefined with let (superset filtered on replay)" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe Foo do
          subject(:foo) { described_class.new }

          context '#bar' do
            subject(:bar) { foo.bar }
            let(:foo) { described_class.new }
            before { allow(foo).to receive(:active?) }
          end
        end
      RUBY
    end
  end

  describe "heredoc interiors" do
    it "flags a stub in a group that also contains a heredoc" do
      expect_parity(<<~RUBY)
        describe Foo do
          subject(:foo) { x }
          it 'a' do
            body = <<~TEXT
              text
            TEXT
            allow(foo).to receive(:bar).and_return(body)
          end
        end
      RUBY
    end
  end

  describe "CRLF sources (bundle-ineligible fallback)" do
    it "matches stock offense positions on a CRLF source" do
      expect_parity(
        "describe Foo do\r\n  subject(:foo) { x }\r\n" \
        "  it { allow(foo).to receive(:bar) }\r\nend\r\n"
      )
    end
  end
end
