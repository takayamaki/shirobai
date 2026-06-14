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
end
