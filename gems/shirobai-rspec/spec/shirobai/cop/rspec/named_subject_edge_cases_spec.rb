# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/NamedSubject`.
#
# Quirks probed against stock rubocop-rspec 3.10.2 that the vendor spec
# does not pin:
#
# - **Plain-block ancestor only**: stock's `on_block` fires for plain blocks
#   only (no numblock/itblock handler), so a `subject` reference inside a
#   numblock example (`it('a') { subject; _1 }`) is NOT reported.
# - **Literal `subject` send only**: `subject_usage` searches the hard-coded
#   `$(send nil? :subject)`, so `is_expected`, `subject(:x)`, `subject!` and
#   `subject(&b)` never count, while the send inside a `subject { }` definition
#   placed in an example DOES.
# - **Repeated references dedup by range**: nested example blocks find the same
#   reference several times, but `add_offense` dedups by range -> one offense.
# - **`shared_context` is not a shared example**: `IgnoreSharedExamples` only
#   suppresses references inside `SharedGroups.examples`
#   (`shared_examples` / `shared_examples_for`), never `shared_context`.
# - **Outermost example/hook-or-shared decides**: an example ABOVE a shared
#   group is not enclosed by it and is still reported.
# - **named_only nearest resolution**: the innermost enclosing `subject`
#   definition decides, and `subject!` definitions count.
RSpec.describe Shirobai::Cop::RSpec::NamedSubject do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::NamedSubject }

  def config_with(overrides = {})
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    ns = (hash["RSpec/NamedSubject"] || {}).dup
    hash["RSpec/NamedSubject"] = ns.merge(overrides)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  def expect_parity(source, config: config_with, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config,
                       expect_offenses: expect_offenses)
  end

  describe "reference shapes (always style)" do
    it "flags bare subject in examples and hooks" do
      expect_parity(<<~RUBY)
        RSpec.describe User do
          subject { described_class.new }

          it('a') { expect(subject.valid?).to be(true) }
          before(:each) { do_x(subject) }
          around(:each) { |t| do_x(subject) }
        end
      RUBY
    end

    it "does not flag subject inside a numblock example" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe User do
          it('a') { subject.foo; _1 }
        end
      RUBY
    end

    it "does not flag is_expected, named subject, or block-pass subject" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe User do
          it { is_expected.to be_valid }
          it('a') { subject(:x) }
          it('b') { subject(&blk) }
        end
      RUBY
    end

    it "flags the send inside a subject definition placed in an example" do
      expect_parity(<<~RUBY)
        RSpec.describe User do
          it('a') do
            subject { 1 }
          end
        end
      RUBY
    end

    it "reports one offense per reference despite nested example blocks" do
      offenses = expect_parity(<<~RUBY)
        it 'x' do
          it 'y' do
            expect(subject).to be
          end
        end
      RUBY
      expect(offenses.length).to eq(1)
    end

    it "ignores subject outside any example or hook" do
      expect_parity("subject.foo\n", expect_offenses: false)
      expect_parity("def foo\n  it(subject)\nend\n", expect_offenses: false)
    end
  end

  describe "shared example groups" do
    it "suppresses references inside shared_examples by default" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe User do
          shared_examples 'x' do
            it('a') { subject.foo }
          end
        end
      RUBY
    end

    it "reports references inside shared_examples when IgnoreSharedExamples is false" do
      expect_parity(<<~RUBY, config: config_with("IgnoreSharedExamples" => false))
        RSpec.describe User do
          shared_examples 'x' do
            it('a') { subject.foo }
          end
        end
      RUBY
    end

    it "does not treat shared_context as a shared example" do
      expect_parity(<<~RUBY)
        RSpec.describe User do
          shared_context 'x' do
            it('a') { subject.foo }
          end
        end
      RUBY
    end

    it "reports an example that encloses a shared group" do
      expect_parity(<<~RUBY)
        it 'outer' do
          shared_examples 'x' do
            it 'inner' do
              subject.foo
            end
          end
        end
      RUBY
    end
  end

  describe "named_only style" do
    def named_only(overrides = {})
      config_with({ "EnforcedStyle" => "named_only" }.merge(overrides))
    end

    it "ignores references when the nearest subject is unnamed" do
      expect_parity(<<~RUBY, config: named_only, expect_offenses: false)
        RSpec.describe User do
          subject { described_class.new }
          it('a') { expect(subject).to be }
        end
      RUBY
    end

    it "flags references when the nearest subject is named" do
      expect_parity(<<~RUBY, config: named_only)
        RSpec.describe User do
          subject(:u) { described_class.new }
          it('a') { expect(subject).to be }
        end
      RUBY
    end

    it "uses the innermost declaration even when an outer one is named" do
      expect_parity(<<~RUBY, config: named_only, expect_offenses: false)
        RSpec.describe User do
          subject(:u) { described_class.new }
          describe 'age' do
            subject { u.age }
            it('a') { expect(subject.can_drive).to be }
          end
        end
      RUBY
    end

    it "treats an unnamed subject! declaration as unnamed" do
      expect_parity(<<~RUBY, config: named_only, expect_offenses: false)
        RSpec.describe User do
          subject! { described_class.new }
          it('a') { expect(subject).to be }
        end
      RUBY
    end
  end

  describe "CRLF sources (bundle-ineligible fallback)" do
    it "matches stock offense positions on a CRLF source" do
      expect_parity(
        "RSpec.describe User do\r\n  subject { x }\r\n" \
        "  it('a') { expect(subject.foo).to be }\r\nend\r\n"
      )
    end
  end
end
