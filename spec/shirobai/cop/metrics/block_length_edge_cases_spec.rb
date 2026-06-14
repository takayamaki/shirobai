# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: `super do..end` is a `block` in parser-gem.
#
# Parser-gem represents `super do |x| ... end` and `super(arg) do |x| ... end`
# as a `(block (zsuper) ...)` / `(block (super ...) ...)` whose `method_name`
# is `:super`, so stock's `on_block` runs `check_code_length` on it. Prism
# carries the block off the `SuperNode` / `ForwardingSuperNode` instead, so the
# wrapper that only hooks `CallNode`s silently misses long `super do..end`
# bodies. Discourse's `plugins/chat/lib/discourse_dev/category_channel.rb`
# hits this with a 32-line `super do |channel| ... end` (Max=25).
#
# Vendor spec coverage: BlockLength's vendor spec uses `proc { ... }` /
# `method(...) { ... }` only and never a `super` block, so this regression
# would be silent without an explicit fixture.
RSpec.describe Shirobai::Cop::Metrics::BlockLength do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Metrics::BlockLength,
    Shirobai::Cop::Metrics::BlockLength
  ]

  # `Max: 3` so a tight body is enough to trip the cop and we do not have to
  # copy 26 lines of fixture.
  let(:config) do
    RuboCop::Config.new(
      "Metrics/BlockLength" => {
        "Max" => 3,
        "Enabled" => true,
        "CountComments" => false,
        "CountAsOne" => [],
        "AllowedMethods" => [],
        "AllowedPatterns" => [],
        "Exclude" => [],
        "IgnoredMethods" => []
      }
    )
  end

  # `super do..end` with no arguments (parser-gem `(block (zsuper) ...)`,
  # prism `ForwardingSuperNode` carrying a `BlockNode`).
  zsuper_block = "def m\n  super do |c|\n    a\n    b\n    c\n    d\n  end\nend\n"

  # `super(arg) do..end` with arguments (parser-gem `(block (super ...) ...)`,
  # prism `SuperNode` carrying a `BlockNode`).
  super_block = "def m\n  super(arg) do |c|\n    a\n    b\n    c\n    d\n  end\nend\n"

  # Short `super do..end` (within Max=3): both must emit nothing.
  short_zsuper = "def m\n  super do |c|\n    a\n  end\nend\n"

  it "reports a long `super do..end` (no args)" do
    expect_lint_parity(*klasses, zsuper_block, config)
  end

  it "reports a long `super(arg) do..end`" do
    expect_lint_parity(*klasses, super_block, config)
  end

  it "stays silent on a short `super do..end`" do
    expect_lint_parity(*klasses, short_zsuper, config, expect_offenses: false)
  end

  it "honours `AllowedMethods: [super]`" do
    cfg = RuboCop::Config.new(
      "Metrics/BlockLength" => {
        "Max" => 3,
        "Enabled" => true,
        "CountComments" => false,
        "CountAsOne" => [],
        "AllowedMethods" => ["super"],
        "AllowedPatterns" => [],
        "Exclude" => [],
        "IgnoredMethods" => []
      }
    )
    expect_lint_parity(*klasses, zsuper_block, cfg, expect_offenses: false)
    expect_lint_parity(*klasses, super_block, cfg, expect_offenses: false)
  end
end
