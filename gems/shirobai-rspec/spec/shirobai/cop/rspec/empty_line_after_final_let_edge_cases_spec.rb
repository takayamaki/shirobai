# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/EmptyLineAfterFinalLet`.
#
# Quirks probed against stock rubocop-rspec 3.10.2:
# - the LAST `let?` among the group's direct body children is the concept; the
#   `let(:x, &blk)` SEND form (block-pass, no block) counts as a `let?`.
# - a group whose final child is the (single or trailing) let has no offense.
# - the final let when it is not the overall-last child IS flagged even with a
#   non-let statement after it.
# - `let!` and `include_context` blocks are covered too.
RSpec.describe Shirobai::Cop::RSpec::EmptyLineAfterFinalLet do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::EmptyLineAfterFinalLet }
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def expect_parity(source, offenses: true)
    expect_lint_parity(stock_class, described_class, source, config, expect_offenses: offenses)
    expect_autocorrect_parity(stock_class, described_class, source, config)
  end

  it "flags the last of several lets" do
    expect_parity(<<~RUBY)
      RSpec.describe Foo do
        let(:a) { 1 }
        let(:b) { 2 }
        it 'x' do
          y
        end
      end
    RUBY
  end

  it "flags a send-form let(:x, &blk) as the final let" do
    expect_parity(<<~RUBY)
      RSpec.describe Foo do
        let(:a) { 1 }
        let(:b, &blk)
        it 'x' do
          y
        end
      end
    RUBY
  end

  it "does not flag a let that is the body's last child" do
    expect_parity(<<~RUBY, offenses: false)
      RSpec.describe Foo do
        it 'x' do
          y
        end
        let(:a) { 1 }
      end
    RUBY
  end

  it "does not flag a single let" do
    expect_parity(<<~RUBY, offenses: false)
      RSpec.describe Foo do
        let(:a) { 1 }
      end
    RUBY
  end

  it "flags the final let inside an include_context block" do
    expect_parity(<<~RUBY)
      RSpec.describe Foo do
        include_context 'shared' do
          let(:a) { 1 }
          it 'x' do
            y
          end
        end
      end
    RUBY
  end
end
