# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/TrailingCommaInArguments`, pinning a
# stock 1.88.0 quirk probed while porting the sibling literal cops:
#
# Stock's `TrailingComma#heredoc?` only treats `send_type?` (`:send`). A
# SAFE-NAVIGATION call wrapping a heredoc argument is `(csend …)` and is NOT
# heredoc-flagged, so the comma regex (`/\A\s*,/` instead of the heredoc-safe
# `/\A[^\S\n]*,/`) crosses the newline into the heredoc body and flags (and
# removes!) a comma that is heredoc text. The plain `.` version is
# heredoc-flagged and stays clean. Prism folds send/csend into one `CallNode`,
# so the port must branch on the safe-navigation flag to match stock.
RSpec.describe Shirobai::Cop::Style::TrailingCommaInArguments do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Style::TrailingCommaInArguments,
    Shirobai::Cop::Style::TrailingCommaInArguments
  ]

  it "flags the heredoc-body comma of a safe-navigation heredoc argument (stock quirk)" do
    source = "m(x, a&.b(<<~X)\n, inside\n  X\n)\n"
    expect_lint_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  it "does not flag the heredoc-body comma of a plain-call heredoc argument" do
    source = "m(x, a.b(<<~X)\n, inside\n  X\n)\n"
    expect_lint_parity(*klasses, source, config, expect_offenses: false)
  end
end
