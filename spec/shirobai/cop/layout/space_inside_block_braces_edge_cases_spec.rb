# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: lambda literals are parser `block` nodes too.
#
# The parser gem models `-> {}`, `lambda {}` and `proc {}` as `block` nodes, so
# SpaceInsideBlockBraces must check their braces. prism instead splits them into
# a separate `LambdaNode` (for `->`) distinct from `BlockNode`; a wrapper that
# only visits `BlockNode` would silently UNDER-detect lambda braces. This only
# surfaced on the corpus (the vendor + non_ascii fixtures use a plain
# `foo.each {puts x}` block), and the cargo `lambda_blocks` test guards the Rust
# layer but not the Ruby drop-in compat.
#
# These cases pin that stock and shirobai agree on offenses AND on the
# autocorrected (space-inserted) source for `->`, `lambda` and `proc`, plus a
# brace-less arrow `->{x}` (no leading inner-brace space because `{` follows the
# `>` directly). A plain block is included as the control.
RSpec.describe Shirobai::Cop::Layout::SpaceInsideBlockBraces do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Layout::SpaceInsideBlockBraces,
    Shirobai::Cop::Layout::SpaceInsideBlockBraces
  ]

  {
    "arrow lambda with params (`->(x) {x}`)" => "->(x) {x}\n",
    "bare arrow lambda (`->{x}`)" => "->{x}\n",
    "lambda keyword block (`lambda {x}`)" => "lambda {x}\n",
    "proc keyword block (`proc {x}`)" => "proc {x}\n",
    "plain block control (`foo.each {x}`)" => "foo.each {x}\n"
  }.each do |label, source|
    it "matches stock offenses and autocorrect for #{label}" do
      expect_lint_parity(*klasses, source, config)
      expect_autocorrect_parity(*klasses, source, config)
    end
  end

  # A bare `super { }` block is a prism `ForwardingSuperNode` whose `block` is a
  # concretely-typed field. The generated walker calls `visit_block_node` on it
  # directly, so the block never reaches the shared-walk enter hook (the same
  # family as the `RescueNode` trap). A wrapper that only sees blocks through the
  # generic hook UNDER-detects these braces. `super(...) { }` is a `SuperNode`
  # with a normal child block and was never affected, so it is the control here.
  # These pin that stock and shirobai agree on offenses AND autocorrect across
  # the style / empty-style / space-before-params axes.
  #
  # A config with the given `Layout/SpaceInsideBlockBraces` styles on top of
  # defaults. `Config#to_h` returns the default configuration's INTERNAL hash,
  # so it must be duped before a key is reassigned — mutating it in place leaks
  # the styles into every later spec that reads the (identity-memoized) default.
  def super_config(style, empty_style, sbbp)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["Layout/SpaceInsideBlockBraces"] =
      hash["Layout/SpaceInsideBlockBraces"].merge(
        "EnforcedStyle" => style,
        "EnforcedStyleForEmptyBraces" => empty_style,
        "SpaceBeforeBlockParameters" => sbbp
      )
    RuboCop::Config.new(hash, default.loaded_path)
  end

  context "bare `super { }` (ForwardingSuperNode) block braces" do
    forms = {
      "no inner space (`super {x}`)" => "super {x}\n",
      "brace touching super (`super{x}`)" => "super{x}\n",
      "left space only (`super {x }`)" => "super {x }\n",
      "block param pipe (`super {|n| n}`)" => "super {|n| n}\n",
      "empty adjacent braces (`super {}`)" => "super {}\n",
      "empty spaced braces (`super { }`)" => "super { }\n",
      "spaced inner (`super { x }`)" => "super { x }\n",
      "do/end super block (ignored)" => "super do x end\n"
    }

    [
      %w[space no_space], %w[space space],
      %w[no_space no_space], %w[no_space space]
    ].each do |style, empty|
      [true, false].each do |sbbp|
        cfg_label = "style=#{style} empty=#{empty} sbbp=#{sbbp}"
        forms.each do |label, source|
          it "matches stock for #{label} under #{cfg_label}" do
            cfg = super_config(style, empty, sbbp)
            expect_lint_parity(*klasses, source, cfg, expect_offenses: false)
            expect_autocorrect_parity(*klasses, source, cfg)
          end
        end
      end
    end
  end

  context "`super(...) { }` (SuperNode) block braces — control, never regressed" do
    it "still matches stock offenses and autocorrect" do
      expect_lint_parity(*klasses, "super() {x}\n", config)
      expect_autocorrect_parity(*klasses, "super() {x}\n", config)
      expect_lint_parity(*klasses, "super(a) {x}\n", config)
      expect_autocorrect_parity(*klasses, "super(a) {x}\n", config)
    end
  end
end
