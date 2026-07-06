# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for Rails/UnknownEnv. The vendor spec covers the
# predicate / comparison / case matrix and the DidYouMean messages; these pin
# quirks probed against stock rubocop-rails 2.35.5 that the Rust port (which
# emits only `[start, end, name]` and lets the wrapper build the message) could
# regress:
#
# - **cbase Rails**: `::Rails.env.x?` is `rails_env?` too.
# - **comparison operand order**: the string may be on either side of
#   `== / === / !=`; the offense highlights the string node.
# - **case with multiple `when` conditions**: each unknown string condition is
#   a separate offense.
# - **non-string `when`**: ignored.
# - **CRLF**: the wrapper falls back to its standalone entry point.
RSpec.describe "Rails/UnknownEnv edge cases" do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }
  let(:cops) { [RuboCop::Cop::Rails::UnknownEnv, Shirobai::Cop::Rails::UnknownEnv] }

  it "matches stock for cbase `::Rails.env` predicate (message included)" do
    expect_lint_parity(*cops, "::Rails.env.proudction?\n", config)
  end

  it "matches stock for the string-first comparison order" do
    expect_lint_parity(*cops, "'developpment' == Rails.env\n", config)
  end

  it "matches stock for `===` and `!=`" do
    expect_lint_parity(*cops, "Rails.env === 'proudction'\nRails.env != 'proudction'\n", config)
  end

  it "matches stock for a case with multiple when conditions" do
    src = "case Rails.env\nwhen 'development', 'proudction'\n  x\nend\n"
    expect_lint_parity(*cops, src, config)
  end

  it "ignores a non-string when condition" do
    src = "case Rails.env\nwhen proudction\n  x\nend\n"
    expect_lint_parity(*cops, src, config, expect_offenses: false)
  end

  it "accepts known environments" do
    expect_lint_parity(*cops, "Rails.env.production?\nRails.env == 'test'\n", config,
                       expect_offenses: false)
  end

  it "keeps offsets aligned on a CRLF source" do
    expect_lint_parity(*cops, "Rails.env.proudction?\r\n", config)
  end
end
