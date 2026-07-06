# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/EmptyLineAfterExampleGroup`.
#
# Quirks probed against stock rubocop-rspec 3.10.2:
# - fires on shared groups (`shared_examples`/`shared_context`) as well as
#   example groups (`spec_group?` = EG|SG).
# - two top-level groups with no separating blank flag the first (program-level
#   `:begin`).
# - groups in an `if`/`else` branch body are sequenced as `:begin` too.
RSpec.describe Shirobai::Cop::RSpec::EmptyLineAfterExampleGroup do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::EmptyLineAfterExampleGroup }
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def expect_parity(source, offenses: true)
    expect_lint_parity(stock_class, described_class, source, config, expect_offenses: offenses)
    expect_autocorrect_parity(stock_class, described_class, source, config)
  end

  it "fires on a shared group followed by a describe" do
    expect_parity(<<~RUBY)
      RSpec.describe Foo do
        shared_examples 'x' do
          it { a }
        end
        describe '#bar' do
          it { b }
        end
      end
    RUBY
  end

  it "flags the first of two top-level groups" do
    expect_parity(<<~RUBY)
      RSpec.describe Foo do
        it { a }
      end
      RSpec.describe Bar do
        it { b }
      end
    RUBY
  end

  it "sequences groups in an else branch" do
    expect_parity(<<~RUBY)
      if RUBY_VERSION < '2.3'
        describe 'old' do
        end
      else
        describe 'first check' do
        end
        describe 'second check' do
        end
      end
    RUBY
  end
end
