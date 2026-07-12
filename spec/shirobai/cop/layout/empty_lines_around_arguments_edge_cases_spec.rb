# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/EmptyLinesAroundArguments`.
#
# parser-gem's `send.arguments` includes the block-pass argument
# (`&blk`, bare `&`) as the last argument, so stock scans for an empty line
# above it like any other argument — and a call whose ONLY argument is a
# block-pass still has arguments. prism keeps the block argument in
# `CallNode#block()`, outside `arguments()`; shirobai used to drop it. Same
# prism/parser mapping family as the `Layout/ArgumentAlignment` bare-`&` gap
# found in the redmine `-a` byte audit.
RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundArguments do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::EmptyLinesAroundArguments,
    Shirobai::Cop::Layout::EmptyLinesAroundArguments
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "flags an empty line above a lone block-pass argument" do
    src = <<~RUBY
      foo(

        &blk
      )
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "flags an empty line above a trailing block-pass argument" do
    src = <<~RUBY
      foo(
        bar,

        &blk
      )
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "accepts a block-pass argument without surrounding empty lines" do
    src = <<~RUBY
      foo(
        bar,
        &blk
      )
    RUBY
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect_autocorrect_parity(*klasses, src, config)
  end
end
