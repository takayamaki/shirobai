# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/ArgumentAlignment`.
#
# parser-gem's `send.arguments` includes the block-pass argument
# (`&blk`, and the bare `&` since Ruby 3.1) as the LAST argument, so stock
# aligns it like any other argument. prism keeps the block argument in
# `CallNode#block()`, outside `arguments()` — shirobai used to drop it,
# missing both the offense and the realignment. Found via the redmine `-a`
# byte audit: `bazaar_adapter.rb`'s `shellout(..., &)` keeps a stale indent
# after Style/RedundantAssignment removes the `ret =` prefix, and only stock
# pulled the `&` back into alignment.
RSpec.describe Shirobai::Cop::Layout::ArgumentAlignment do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::ArgumentAlignment,
    Shirobai::Cop::Layout::ArgumentAlignment
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "aligns a misaligned block-pass argument (redmine bazaar_adapter)" do
    src = <<~RUBY
      shellout(
        a + ' ' +
          b.map { |e| shell_quote e.to_s }.join(' '),
          &blk
      )
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "accepts an aligned block-pass argument" do
    src = <<~RUBY
      shellout(
        a + ' ' +
          b.map { |e| shell_quote e.to_s }.join(' '),
        &blk
      )
    RUBY
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "counts the block-pass argument when deciding a call has multiple arguments" do
    # One positional argument + block-pass: parser sees TWO arguments, so the
    # call qualifies for alignment and the block-pass gets aligned.
    src = <<~RUBY
      shellout(first_arg,
               second_line_arg,
                 &blk)
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "aligns a block-pass argument under with_fixed_indentation" do
    fixed = config_with("Layout/ArgumentAlignment",
                        "EnforcedStyle" => "with_fixed_indentation")
    src = <<~RUBY
      shellout(
        first_arg,
          &blk
      )
    RUBY
    expect_lint_parity(*klasses, src, fixed)
    expect_autocorrect_parity(*klasses, src, fixed)
  end

  def config_with(cop_name, overrides)
    base = RuboCop::ConfigLoader.default_configuration
    hash = base.to_h.dup
    hash[cop_name] = (hash[cop_name] || {}).merge(overrides)
    RuboCop::Config.new(hash, base.loaded_path)
  end
end
