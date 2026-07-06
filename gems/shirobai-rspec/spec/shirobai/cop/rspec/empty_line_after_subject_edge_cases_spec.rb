# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/EmptyLineAfterSubject`.
#
# Quirks probed against stock rubocop-rspec 3.10.2:
# - `inside_example_group?`: the subject fires only when its OUTERMOST enclosing
#   top-level statement is a spec group; a top-level subject, or a subject in a
#   group wrapped by a top-level `class`/`module`, is not inside a group.
# - a heredoc argument spilling below a one-line brace subject fixes the offense
#   on the heredoc terminator line.
RSpec.describe Shirobai::Cop::RSpec::EmptyLineAfterSubject do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::EmptyLineAfterSubject }
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def expect_parity(source, offenses: true)
    expect_lint_parity(stock_class, described_class, source, config, expect_offenses: offenses)
    expect_autocorrect_parity(stock_class, described_class, source, config)
  end

  it "flags a subject followed by a let inside a group" do
    expect_parity(<<~RUBY)
      RSpec.describe Foo do
        subject(:obj) { described_class }
        let(:foo) { bar }
      end
    RUBY
  end

  it "does not flag a top-level subject" do
    expect_parity(<<~RUBY, offenses: false)
      subject(:obj) { described_class }
      let(:foo) { bar }
    RUBY
  end

  it "does not flag a subject in a group wrapped by a top-level class" do
    expect_parity(<<~RUBY, offenses: false)
      class Wrap
        RSpec.describe Foo do
          subject(:obj) { described_class }
          let(:foo) { bar }
        end
      end
    RUBY
  end

  it "fixes the offense on a spilling heredoc terminator line" do
    expect_parity(<<~RUBY)
      RSpec.describe Foo do
        subject(:obj) { described_class.new(<<~ARGS) }
          a
          b
        ARGS
        let(:foo) { bar }
      end
    RUBY
  end
end
