# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/EmptyLineAfterHook`.
#
# Quirks probed against stock rubocop-rspec 3.10.2:
# - hooks fire in ANY block form: plain, numbered-parameter (`before { _1 }`)
#   and `it`-parameter — the cop aliases `on_numblock`/`on_itblock`.
# - `AllowConsecutiveOneLiners` (default true) skips a single-line hook whose
#   next sibling is also a single-line hook; the last hook before a non-hook is
#   still flagged.
# - disabling the option flags the whole chain.
RSpec.describe Shirobai::Cop::RSpec::EmptyLineAfterHook do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::EmptyLineAfterHook }
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def expect_parity(source, cfg: config, offenses: true)
    expect_lint_parity(stock_class, described_class, source, cfg, expect_offenses: offenses)
    expect_autocorrect_parity(stock_class, described_class, source, cfg)
  end

  it "allows a consecutive one-liner hook chain but flags the last before an example" do
    expect_parity(<<~RUBY)
      RSpec.describe Foo do
        before { a }
        after { b }
        it { c }
      end
    RUBY
  end

  it "flags a numbered-parameter hook" do
    expect_parity(<<~RUBY)
      RSpec.describe Foo do
        before { _1 }
        it 'x' do
          y
        end
      end
    RUBY
  end

  it "flags a multi-line hook" do
    expect_parity(<<~RUBY)
      RSpec.describe Foo do
        around do |test|
          test.run
        end
        it { c }
      end
    RUBY
  end

  it "flags the chain when AllowConsecutiveOneLiners is disabled" do
    cfg = config_with(config, "RSpec/EmptyLineAfterHook", "AllowConsecutiveOneLiners" => false)
    expect_parity(<<~RUBY, cfg: cfg)
      RSpec.describe Foo do
        before { a }
        after { b }

        it { c }
      end
    RUBY
  end

  def config_with(base, badge, overrides)
    hash = base.to_h.dup
    hash[badge] = (hash[badge] || {}).merge(overrides)
    RuboCop::Config.new(hash, base.loaded_path)
  end
end
