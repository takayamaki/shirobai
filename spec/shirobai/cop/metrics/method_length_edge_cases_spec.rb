# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: an implicit `begin` with a trailing `else` /
# `ensure` must not count the closing `end` line.
#
# Stock's `code_length(@node)` measures the source of the def's body node,
# which is a `rescue` (parser-gem). The body source spans from the first
# statement through the last line of the closing clause — the `end` keyword
# line is excluded because the `def`'s `end` belongs to the def, not its body.
# In prism the def's body is a `BeginNode` (implicit begin), whose `else_clause`
# `ElseNode` location extends through the enclosing `def`'s `end` keyword when
# no `ensure` follows. Without capping at `ElseNode.end_keyword_loc`, every
# `def m; ...; rescue; ...; else; ...; end` counts +1 line — silent +1
# divergence wherever stock would be at the threshold (Discourse's
# `admin/backups_controller.rb#restore` triggered an extra `[11/10]`).
#
# Vendor spec coverage: vendor's MethodLength spec uses `def m; rescue; end`
# (no else) and rescue/ensure combinations on the same line; the
# `rescue...else...end` shape that exposes the extra `end` line is absent, so
# this regression would be silent without an explicit fixture.
RSpec.describe Shirobai::Cop::Metrics::MethodLength do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Metrics::MethodLength,
    Shirobai::Cop::Metrics::MethodLength
  ]

  # `Max: 3` so a tight body is enough to trip the cop.
  let(:config) do
    RuboCop::Config.new(
      "Metrics/MethodLength" => {
        "Max" => 3,
        "Enabled" => true,
        "CountComments" => false,
        "CountAsOne" => [],
        "AllowedMethods" => [],
        "AllowedPatterns" => [],
        "Exclude" => []
      }
    )
  end

  # `rescue ... else ... end`: the closing `end` line must NOT be counted.
  # Body lines = a, b, c, d, rescue, e, else, f -> 6 (rescue/else themselves
  # are non-blank lines, `end` is excluded). At Max=3 both stock and shirobai
  # must agree on the `[6/3]` length.
  rescue_else = <<~RUBY
    def m
      a
      b
      c
      d
    rescue Foo
      e
    else
      f
    end
  RUBY

  # The shape that is BORDERLINE: stock body source is exactly 4 lines, so at
  # `Max: 4` neither side should emit. shirobai's old bug counted the `end`
  # line and tripped `[5/4]`.
  borderline_rescue_else = <<~RUBY
    def m
      a
    rescue Foo
      b
    else
      c
    end
  RUBY

  # `rescue ... else ... ensure ... end`: else is followed by ensure (which
  # itself stops before `end`), but capping else at its `end_keyword_loc`
  # (which here is the `ensure` keyword) is also the right behaviour.
  rescue_else_ensure = <<~RUBY
    def m
      a
      b
      c
    rescue Foo
      d
    else
      e
    ensure
      f
    end
  RUBY

  it "matches stock on `rescue ... else ... end` (does not count the closing `end`)" do
    expect_lint_parity(*klasses, rescue_else, config)
  end

  it "stays silent at the borderline body length under `rescue ... else ... end`" do
    borderline_config = RuboCop::Config.new(
      "Metrics/MethodLength" => {
        "Max" => 4,
        "Enabled" => true,
        "CountComments" => false,
        "CountAsOne" => [],
        "AllowedMethods" => [],
        "AllowedPatterns" => [],
        "Exclude" => []
      }
    )
    # 4 body lines = stock would have been silent, shirobai's old bug emitted
    # `[5/4]`. Both must be silent now.
    expect_lint_parity(*klasses, borderline_rescue_else, borderline_config, expect_offenses: false)
  end

  it "matches stock on `rescue ... else ... ensure ... end`" do
    expect_lint_parity(*klasses, rescue_else_ensure, config)
  end
end
