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
#
# Also guards two more shapes found on mastodon:
# - `rescue A, B, C => e` — parser wraps the exception list in an implicit
#   array node that CheckLineBreakable can break; prism has no such node.
# - a single-line `{ ... }` block on a multi-line receiver chain —
#   BlockNode#single_line? only compares the `{` and `}` lines, and the
#   block claim OVERWRITES earlier claims on the same line.
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

  it "breaks a rescue clause with multiple exception classes" do
    # parser wraps `rescue A, B, C` exception lists in an implicit array node
    # and CheckLineBreakable treats it as a breakable collection. Prism keeps
    # the exceptions as a bare NodeList on RescueNode, so the rule has to
    # synthesize the collection. Found on mastodon remotable.rb.
    src = <<~RUBY
      begin
        foo
      rescue Aaa::BbbError, Ccc::DddError, Eee::FffError => e
        bar
      end
    RUBY
    expect_lint_parity(*klasses, src, max40_config)
    expect(expect_autocorrect_parity(*klasses, src, max40_config))
      .to eq(<<~RUBY)
        begin
          foo
        rescue Aaa::BbbError, Ccc::DddError,#{trailing_space}
        Eee::FffError => e
          bar
        end
      RUBY
  end

  it "breaks a rescue clause whose first entry is a splat" do
    # mastodon fetch_resource_service shape: `rescue *ERRORS, A, B => e`.
    # The splat is just the first element of the implicit array.
    src = <<~RUBY
      begin
        foo
      rescue *SOME_CONNECTION_ERRORS, Aaa::BbbError, Ccc::DddError => e
        bar
      end
    RUBY
    expect_lint_parity(*klasses, src, max40_config)
    expect(expect_autocorrect_parity(*klasses, src, max40_config))
      .to eq(<<~RUBY)
        begin
          foo
        rescue *SOME_CONNECTION_ERRORS,#{trailing_space}
        Aaa::BbbError, Ccc::DddError => e
          bar
        end
      RUBY
  end

  it "claims a single-line brace block hanging off a multi-line receiver" do
    # BlockNode#single_line? compares only the `{` and `}` lines — the
    # receiver may span any number of lines. The claim goes to the block
    # EXPRESSION's first line (the receiver line), while the insertion point
    # sits after the `|args|` on a later line. The block path has no
    # line-with-comment guard, so the trailing comment does not suppress it.
    # Found on mastodon feed_manager.rb.
    #
    # `fresh_cop_per_pass`: the claimed line (the receiver line) stays over
    # Max after the correction, so pass 2 re-registers its offense; a REUSED
    # stock cop instance would then crash on its stale breakable map (stock
    # never resets it). The real CLI runs a fresh cop per pass.
    src = <<~RUBY
      long_variable_name_one ||
        ((abc[:key][1] || []) + [xyz])          # trailing comment
          .any? { |target_id| abc[:blocking][target_id] } ||
        other_value
    RUBY
    expect_lint_parity(*klasses, src, max40_config)
    expect(expect_autocorrect_parity(*klasses, src, max40_config, fresh_cop_per_pass: true))
      .to eq(<<~RUBY)
        long_variable_name_one ||
          ((abc[:key][1] || []) + [xyz])          # trailing comment
            .any? { |target_id|
         abc[:blocking][target_id] } ||
          other_value
      RUBY
  end

  it "lets a block claim overwrite an earlier semicolon claim" do
    # Stock claims semicolons in on_new_investigation, then the node walk's
    # block handler ASSIGNS (not write-once) — a block on the same line wins.
    src = "aaa = 1; bbb.ccc_ddd(eee).fff { |g| g * 27 }\n"
    expect_lint_parity(*klasses, src, max40_config)
    expect(expect_autocorrect_parity(*klasses, src, max40_config))
      .to eq("aaa = 1; bbb.ccc_ddd(eee).fff { |g|\n g * 27 }\n")
  end

  it "still declines a rescue clause whose exception list is multi-line" do
    src = <<~RUBY
      begin
        foo
      rescue Aaa::BbbError,
             Ccc::DddError, Eee::FffError, Ggg::HhhError => e
        bar
      end
    RUBY
    expect_lint_parity(*klasses, src, max40_config)
    expect(expect_autocorrect_parity(*klasses, src, max40_config)).to eq(src)
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
