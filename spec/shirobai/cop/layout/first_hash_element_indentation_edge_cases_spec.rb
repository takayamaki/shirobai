# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/FirstHashElementIndentation`.
#
# The vendor spec exercises `EnforcedStyle`s and the separator handling, but
# does NOT pin this stock early-return discovered via Redmine parity diff:
#
#   When `first_pair` exists AND it sits on the SAME LINE as `{`, stock's
#   `check` returns early — BEFORE `check_right_brace` is called. A `}` on a
#   subsequent line is therefore NEVER reported, regardless of its indent.
#   shirobai used to fall through and `check_right_brace`, so any
#   `STATUS_MAPPING = {10 => :a, # ...\n  ...\n  }` triggered a ghost "Indent
#   the right brace the same as the start of the line where the left brace
#   is." Real CLI diff on Redmine: 9 ghosts in `migrate_from_mantis.rake` and
#   `migrate_from_trac.rake`.
#
# Pinned here as differential regressions against the 1.87-pinned stock.
RSpec.describe Shirobai::Cop::Layout::FirstHashElementIndentation do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::FirstHashElementIndentation,
    Shirobai::Cop::Layout::FirstHashElementIndentation
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "skips `}` check when the first pair is on the `{` line (Redmine STATUS_MAPPING)" do
    # Minimised from `lib/tasks/migrate_from_mantis.rake`. First pair `10 =>
    # :a` is on the `{` line, so stock skips the entire `check` and never
    # reports the closing `}`'s position.
    src = <<~RUBY
      namespace :foo do
        task :bar do
          STATUS_MAPPING = {10 => :a,      # comment
                            20 => :b, # comment
                            30 => :c    # comment
                            }
        end
      end
    RUBY
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end

  it "skips even when `}` is wildly misindented (since the `{`-line short-circuit fires)" do
    # Negative confirmation: even a `}` at column 0 is silent because the
    # `same_line?(first_pair, left_brace)` early return wins.
    src = <<~RUBY
      h = {a: 1,
           b: 2,
           c: 3
      }
    RUBY
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end

  it "still reports a misaligned first pair when `{` is on its own line" do
    # Negative control: when the first pair is NOT on the `{` line, stock's
    # check proceeds and reports both first-pair and `}` divergence. Guards
    # against a too-eager short-circuit.
    src = <<~RUBY
      h = {
       a: 1,
        b: 2
        }
    RUBY
    stock = expect_lint_parity(*klasses, src, config)
    expect(stock.size).to be >= 1
  end
end

# Edge cases for 1.92 (rubocop#16050 / #15857): a first pair that does not
# begin its line (after `**opts,`) is not checked, and with the
# Layout/HashAlignment separator style a multiline key is left out of the
# longest-key offset (and a multiline first key gets no offset). These
# mix rockets, multiline array keys and an interpolated multiline colon
# key, which the vendor spec does not.
RSpec.describe Shirobai::Cop::Layout::FirstHashElementIndentation do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::FirstHashElementIndentation,
    Shirobai::Cop::Layout::FirstHashElementIndentation
  ]

  source_192 = <<~'RUBY'
    a << {
          **opts, 'b' => 1,
      'ccc' => 2
    }
    a << {
      [
        :x
      ] => 1,
         'longer_key' => 2
    }
    a << {
         'k' => 1,
      [:a,
       :b] => 2,
      'kkkk' => 3
    }
    b = {
          foo: 1,
      barbaz: 2,
      "q#{x(
      )}": 3
    }
  RUBY

  [nil, "separator"].each do |style|
    it "matches stock with HashAlignment style #{style.inspect}" do
      hash = style ? { "Layout/HashAlignment" => { "EnforcedColonStyle" => style, "EnforcedHashRocketStyle" => style } } : {}
      cfg = RuboCop::ConfigLoader.merge_with_default(RuboCop::Config.new(hash, "(test)"), "(test)")
      expect_lint_parity(*klasses, source_192, cfg)
      expect_autocorrect_parity(*klasses, source_192, cfg)
    end
  end
end
