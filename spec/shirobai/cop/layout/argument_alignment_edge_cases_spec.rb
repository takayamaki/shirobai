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

  it "does not drag multi-line percent-string interiors when realigning (fluentd test_config_parser)" do
    # Stock hands AlignmentCorrector the argument NODE, whose
    # `inside_string_ranges` taboo protects the string interior lines.
    # A bare range loses that protection and a positive delta indents the
    # `%()` body lines too.
    src = <<~RUBY
      x = foo(bar,
      %(line1
      line2 and more))
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  def config_with(cop_name, overrides)
    base = RuboCop::ConfigLoader.default_configuration
    hash = base.to_h.dup
    hash[cop_name] = (hash[cop_name] || {}).merge(overrides)
    RuboCop::Config.new(hash, base.loaded_path)
  end
end

# 1.92 (rubocop#16048): a binary operator call (`a << {...}`) is skipped, while
# `[]` and dotted operator calls (`a.<<(...)`, `a.+(...)`) are still checked.
# A braced hash first argument with two or more pairs counts as multiple
# arguments (stock `multiple_arguments?`).
RSpec.describe Shirobai::Cop::Layout::ArgumentAlignment do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::ArgumentAlignment,
    Shirobai::Cop::Layout::ArgumentAlignment
  ]

  source_192 = <<~RUBY
    foo(
          {a: 1, b: 2})
    bar(
          {a: 1})
    foo << {
             a: 1
           } <<
      {
        b: 2,
        c: 3
      }
    foo.<<(
            {
              b: 2,
              c: 3
            }
    )
    x[
          {a: 1, b: 2}]
    a.+(
         {a: 1, b: 2})
    a +
          {a: 1, b: 2}
  RUBY

  %w[with_fixed_indentation with_first_argument].each do |style|
    it "matches stock on operator calls and braced hashes (#{style})" do
      cfg = RuboCop::ConfigLoader.merge_with_default(
        RuboCop::Config.new({ "Layout/ArgumentAlignment" => { "EnforcedStyle" => style } }, "(test)"),
        "(test)"
      )
      expect_lint_parity(*klasses, source_192, cfg, expect_offenses: style == "with_fixed_indentation")
      expect_autocorrect_parity(*klasses, source_192, cfg)
    end
  end
end
