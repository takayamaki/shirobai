# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/LineLength` autocorrect on a
# single-line method call that carries a MULTI-LINE block.
#
# Prism attaches the block to the `CallNode`, so the call's own location spans
# through the block's `end`. The parser gem models the same code as
# `(block (send ...) ...)` and stock's `already_on_multiple_lines?` asks
# `multiline?` on the SEND node only — which stops before the block. A
# single-line send with a multi-line block is therefore still breakable in
# stock (the corrector inserts a newline inside the argument list), and the
# Rust rule must measure the send part, not the whole block expression.
# Found as a corpus divergence on fluentd (`op.on(...) {|s| ... }`) and
# redmine (`with_settings :a => ..., :b => 1 do ... end`).
RSpec.describe Shirobai::Cop::Layout::LineLength do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::LineLength,
    Shirobai::Cop::Layout::LineLength
  ]

  let(:default_config) { RuboCop::ConfigLoader.default_configuration }
  let(:max40_config) do
    config_with(default_config, "Layout/LineLength", "Max" => 40)
  end

  it "breaks a parenthesized call with a multi-line brace block" do
    src = <<~RUBY
      op.on('-s', "--setup DIR", "install it") {|s|
        s
      }
    RUBY
    expect_lint_parity(*klasses, src, max40_config)
    expect(expect_autocorrect_parity(*klasses, src, max40_config))
      .to eq(<<~RUBY)
        op.on('-s', "--setup DIR",#{trailing_space}
        "install it") {|s|
          s
        }
      RUBY
  end

  it "breaks a parenthesized call with a multi-line do block" do
    src = <<~RUBY
      foo.bar(alpha_one, beta_two_beta, gamma_three) do |x|
        x
      end
    RUBY
    expect_lint_parity(*klasses, src, max40_config)
    expect(expect_autocorrect_parity(*klasses, src, max40_config))
      .to eq(<<~RUBY)
        foo.bar(alpha_one, beta_two_beta,#{trailing_space}
        gamma_three) do |x|
          x
        end
      RUBY
  end

  it "breaks an unparenthesized call with a multi-line do block" do
    # Unparenthesized call: the first element is never moved, so the break
    # lands before the SECOND keyword pair.
    src = <<~RUBY
      with_settings :aa => ['bb', 'cc'], :dd => 1 do
        x
      end
    RUBY
    expect_lint_parity(*klasses, src, max40_config)
    expect(expect_autocorrect_parity(*klasses, src, max40_config))
      .to eq(<<~RUBY)
        with_settings :aa => ['bb', 'cc'],#{trailing_space}
        :dd => 1 do
          x
        end
      RUBY
  end

  it "still declines when the send part itself is multi-line" do
    # Args already span two lines: `already_on_multiple_lines?` is true on the
    # SEND node itself, so neither arm may break the long second line.
    src = <<~RUBY
      foo.bar(alpha_one, beta_two_beta_beta_x,
              gamma_three_gamma_long_name_yy) do |x|
        x
      end
    RUBY
    expect_lint_parity(*klasses, src, max40_config)
    expect(expect_autocorrect_parity(*klasses, src, max40_config)).to eq(src)
  end

  def trailing_space
    " "
  end

  def config_with(base, cop_name, overrides)
    hash = base.to_h.dup
    hash[cop_name] = (hash[cop_name] || {}).merge(overrides)
    RuboCop::Config.new(hash, base.loaded_path)
  end
end
