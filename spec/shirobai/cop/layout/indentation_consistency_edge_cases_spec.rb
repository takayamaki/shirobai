# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/IndentationConsistency`.
#
# An `ensure` body is a parser-gem `(begin)` statement group like any other
# body, so stock checks its statements against each other. In prism the
# `EnsureNode` hangs off `BeginNode`'s concretely-typed `ensure_clause`
# field, which the generated walker visits directly — the shared walk's
# branch hooks never fire for it (same family as `RescueNode`), so shirobai
# never checked ensure bodies at all. No corpus file has a misindented
# ensure body at rest; under `-a` the Style/Semicolon split leaves
# ` i.before_shutdown` one-space lines that only stock re-indents
# (fluentd test_output.rb / test_server.rb).
RSpec.describe Shirobai::Cop::Layout::IndentationConsistency do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::IndentationConsistency,
    Shirobai::Cop::Layout::IndentationConsistency
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "flags inconsistent statements in a block's ensure body (fluentd test_output)" do
    # The `-a` intermediate after Style/Semicolon splits
    # `i.stop; i.before_shutdown; ...` — every statement after the first
    # keeps the single space that followed its semicolon.
    src = <<~RUBY
      test 'foo' do
        yield
      ensure
        i.stop
       i.before_shutdown
       i.shutdown
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "flags inconsistent statements in a def's ensure body" do
    src = <<~RUBY
      def foo
        yield
      ensure
        a
          b
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "flags inconsistent statements in an explicit begin's ensure body" do
    src = <<~RUBY
      begin
        yield
      ensure
        a
          b
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "accepts a consistent ensure body" do
    src = <<~RUBY
      def foo
        yield
      ensure
        a
        b
      end
    RUBY
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect_autocorrect_parity(*klasses, src, config)
  end
end
