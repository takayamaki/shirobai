# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/IndentationWidth`.
#
# The autocorrect realignment node for a body that carries rescue/ensure
# handlers is parser-gem's `:rescue` / `:ensure` node. That node ends at the
# last handler expression — the enclosing `end` keyword is NOT part of it:
#
#   - `:ensure` ends at its body's last statement, or at the `ensure`
#     keyword when the ensure body is empty.
#   - `:rescue` with an `else` ends at the else body's last statement, or at
#     the `else` keyword when the else body is empty.
#   - otherwise `:rescue` ends at the last resbody: its last statement, or
#     `then` / `=> ref` / the last exception class / the `rescue` keyword as
#     the clause shrinks.
#
# prism's implicit `BeginNode` location instead runs through the enclosing
# `end`. shirobai used to hand that full span to `AlignmentCorrector`, which
# then also shifted the handler-free tail lines (comments, blank-with-spaces
# lines, the `end` line itself) that stock leaves alone. The first bad shift
# feeds the next `-a` iteration and the trees drift apart (redmine
# `issues_controller.rb` / `codeset_util.rb` / `setting.rb` and 6 more).
RSpec.describe Shirobai::Cop::Layout::IndentationWidth do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::IndentationWidth,
    Shirobai::Cop::Layout::IndentationWidth
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "realigns only through the last resbody in a block body rescue (redmine issues_controller)" do
    # Minimised from the `-a` intermediate state of redmine
    # app/controllers/issues_controller.rb 466-472 (after Style/RedundantBegin
    # removed the `begin`). The resbody is empty, so the parser rescue node
    # ends at `Bar`; the comment line and the `end` line stay put.
    src = <<~RUBY
      foo.each do |x|
          x.reload.destroy
        rescue Bar # raised by #reload
          # nothing to do
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "realigns through the last resbody statement in a block body rescue" do
    src = <<~RUBY
      foo.each do |x|
          x.destroy
        rescue Bar => e
          log e
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "realigns through the else body in a block body rescue with else" do
    src = <<~RUBY
      foo.each do |x|
          x.destroy
        rescue Bar
          log
        else
          done
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "realigns through the ensure body in a block body ensure" do
    src = <<~RUBY
      foo.each do |x|
          x.destroy
        ensure
          unlock
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "realigns through the ensure keyword when the ensure body is empty" do
    src = <<~RUBY
      foo.each do |x|
          x.destroy
        ensure
          # nothing
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "realigns only the handler span in a def body rescue" do
    src = <<~RUBY
      def foo
          bar
        rescue Baz
          # nothing
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "leaves a comment line between the last resbody and `end` alone in an explicit begin" do
    # The explicit `begin ... end` path used to stretch the realignment range
    # to the line just before `end`, dragging trailing comment lines with it.
    src = <<~RUBY
      begin
          foo
        rescue Bar
          baz
        # trailing comment
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "realigns through `then` when an empty resbody uses the then keyword" do
    src = <<~RUBY
      foo.each do |x|
          x.destroy
        rescue Bar then
          # nothing
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "realigns through the last exception class when a bare resbody has no body" do
    src = <<~RUBY
      foo.each do |x|
          x.destroy
        rescue Bar, Baz
          # nothing
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "realigns through the last clause of a multi-rescue chain" do
    src = <<~RUBY
      foo.each do |x|
          x.destroy
        rescue Bar
          log
        rescue Baz
          # nothing
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end
end
