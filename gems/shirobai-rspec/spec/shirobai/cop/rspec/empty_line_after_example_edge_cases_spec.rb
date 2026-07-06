# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/EmptyLineAfterExample`.
#
# Quirks probed against stock rubocop-rspec 3.10.2 that the vendor spec does
# not fully pin (all asserted as differential offense + `-A` parity):
#
# - a plain `begin a; b end` (kwbegin, no rescue/ensure) is NOT `begin_type?`,
#   so its statements are never flagged; a `begin ... rescue`/`ensure` DOES
#   wrap its main body in `:begin`, and rescue/ensure bodies are `:begin` too.
# - a non-directive trailing comment keeps the offense on the node's end line
#   (blank inserted BEFORE the comment); an enabled `# rubocop:enable` moves
#   the offense onto the directive line (blank AFTER it).
# - a comment (or a trailing-comment code line) followed by a blank line
#   suppresses the offense entirely (`line_with_comment?` walk).
# - a heredoc spilling below a one-line brace example fixes the offense on the
#   heredoc terminator line.
# - numbered-parameter examples never reach `on_block`, so they are not
#   examples.
RSpec.describe Shirobai::Cop::RSpec::EmptyLineAfterExample do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::EmptyLineAfterExample }
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def expect_parity(source, offenses: true)
    expect_lint_parity(stock_class, described_class, source, config, expect_offenses: offenses)
    expect_autocorrect_parity(stock_class, described_class, source, config)
  end

  describe "begin/rescue/ensure sequencing" do
    it "does not flag a plain begin..end (kwbegin is not begin_type)" do
      expect_parity(<<~RUBY, offenses: false)
        RSpec.describe Foo do
          begin
            it 'a' do
              x
            end
            it 'b' do
              y
            end
          end
        end
      RUBY
    end

    it "flags the main body of a begin with a rescue clause" do
      expect_parity(<<~RUBY)
        RSpec.describe Foo do
          begin
            it 'a' do
              x
            end
            it 'b' do
              y
            end
          rescue
            z
          end
        end
      RUBY
    end

    it "flags examples inside a rescue body" do
      expect_parity(<<~RUBY)
        RSpec.describe Foo do
          work
        rescue
          it 'a' do
            x
          end
          it 'b' do
            y
          end
        end
      RUBY
    end

    it "flags examples inside an ensure body" do
      expect_parity(<<~RUBY)
        RSpec.describe Foo do
          work
        ensure
          it 'a' do
            x
          end
          it 'b' do
            y
          end
        end
      RUBY
    end
  end

  describe "trailing comments and directives" do
    it "keeps the offense on the node end line before a plain comment" do
      expect_parity(<<~RUBY)
        RSpec.describe Foo do
          it 'a' do
            x
          end
          # a comment
          it 'b' do
            y
          end
        end
      RUBY
    end

    it "moves the offense onto an enabled rubocop:enable directive line" do
      expect_parity(<<~RUBY)
        RSpec.describe Foo do
          it 'a' do
            x
          end
          # rubocop:enable RSpec/Foo
          it 'b' do
            y
          end
        end
      RUBY
    end

    it "suppresses when a comment is followed by a blank line" do
      expect_parity(<<~RUBY, offenses: false)
        RSpec.describe Foo do
          it 'a' do
            x
          end
          # trailing

          it 'b' do
            y
          end
        end
      RUBY
    end

    it "suppresses when a trailing-comment code line is followed by a blank" do
      expect_parity(<<~RUBY, offenses: false)
        RSpec.describe Foo do
          it 'a' do
            x
          end
          y = 1 # tc

          it 'b' do
            z
          end
        end
      RUBY
    end
  end

  describe "heredocs and one-liners" do
    it "fixes the offense on the heredoc terminator line" do
      expect_parity(<<~RUBY)
        RSpec.describe Foo do
          it(:x) { do_thing(<<~ARGS) }
            a
          ARGS
          it 'b' do
            y
          end
        end
      RUBY
    end

    it "does not treat a numbered-parameter example as an example" do
      expect_parity(<<~RUBY, offenses: false)
        RSpec.describe Foo do
          it('a') { _1 }
          it('b') { _1 }
        end
      RUBY
    end

    it "flags when AllowConsecutiveOneLiners is disabled" do
      cfg = config_with(config, "RSpec/EmptyLineAfterExample", "AllowConsecutiveOneLiners" => false)
      source = <<~RUBY
        RSpec.describe Foo do
          it { one }
          it { two }
        end
      RUBY
      expect_lint_parity(stock_class, described_class, source, cfg)
      expect_autocorrect_parity(stock_class, described_class, source, cfg)
    end
  end

  def config_with(base, badge, overrides)
    hash = base.to_h.dup
    hash[badge] = (hash[badge] || {}).merge(overrides)
    RuboCop::Config.new(hash, base.loaded_path)
  end
end
