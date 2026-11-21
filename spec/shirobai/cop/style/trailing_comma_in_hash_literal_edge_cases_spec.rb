# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/TrailingCommaInHashLiteral`, pinning
# stock 1.88.0 quirks probed during the port that the vendor spec does not
# exercise:
#
# - A braceless keyword hash never reaches this cop (stock: no `loc.end`;
#   prism: `KeywordHashNode`, a different node type from `HashNode`).
# - Stock's `heredoc?` only treats `send_type?` (`:send`). A SAFE-NAVIGATION
#   call wrapping a heredoc value is `(csend …)` and is NOT heredoc-flagged,
#   so the comma regex crosses the newline into the heredoc body and flags
#   (and removes!) a comma that is heredoc text. The plain `.` version is
#   heredoc-flagged and stays clean.
# - `allowed_multiline_argument?`: a single-pair hash whose closing `}` does
#   not begin its line is not "multiline", so `consistent_comma` wants
#   nothing on the outer hash (while an inner hash may still offend).
# - `put_comma` on a kwsplat (`**a`) last element works like any pair.
RSpec.describe Shirobai::Cop::Style::TrailingCommaInHashLiteral do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Style::TrailingCommaInHashLiteral,
    Shirobai::Cop::Style::TrailingCommaInHashLiteral
  ]

  def config_for(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        {
          "Style/TrailingCommaInHashLiteral" => {
            "EnforcedStyleForMultiline" => style
          }
        }, "(test)"
      ),
      "(test)"
    )
  end

  it "never fires on a braceless keyword hash argument" do
    %w[no_comma comma consistent_comma diff_comma].each do |style|
      expect_lint_parity(*klasses, "m(a: 1, b: 2,)\n", config_for(style), expect_offenses: false)
      expect_lint_parity(*klasses, "m(a: 1,\n  b: 2)\n", config_for(style), expect_offenses: false)
    end
  end

  it "flags the heredoc-body comma of a safe-navigation heredoc value (stock quirk)" do
    source = "h = {\n  a: b&.c(<<~X)\n, inside\n  X\n}\n"
    expect_lint_parity(*klasses, source, config_for("no_comma"))
    expect_autocorrect_parity(*klasses, source, config_for("no_comma"))
  end

  it "does not flag the heredoc-body comma of a plain-call heredoc value" do
    source = "h = {\n  a: b.c(<<~X)\n, inside\n  X\n}\n"
    expect_lint_parity(*klasses, source, config_for("no_comma"), expect_offenses: false)
  end

  it "treats a single pair with the closing brace on the value line as allowed multiline" do
    # The outer hash is allowed (its `}` does not begin its line); the inner
    # hash's `}` does begin its line, so only the inner pair gets a comma.
    source = "h = { a: {\n  b: 1\n} }\n"
    expect_lint_parity(*klasses, source, config_for("consistent_comma"))
    expect_autocorrect_parity(*klasses, source, config_for("consistent_comma"))
  end

  it "puts and avoids a comma after a kwsplat last element" do
    expect_lint_parity(*klasses, "h = {\n  **a\n}\n", config_for("consistent_comma"))
    expect_autocorrect_parity(*klasses, "h = {\n  **a\n}\n", config_for("consistent_comma"))
    expect_lint_parity(*klasses, "h = {\n  **a,\n}\n", config_for("no_comma"))
    expect_autocorrect_parity(*klasses, "h = {\n  **a,\n}\n", config_for("no_comma"))
    expect_lint_parity(*klasses, "h = {\n  **a,\n}\n", config_for("consistent_comma"),
                       expect_offenses: false)
  end

  it "puts the caret on the last line of a multiline last value" do
    source = "h = {\n  a: [\n    1\n  ]\n}\n"
    expect_lint_parity(*klasses, source, config_for("consistent_comma"))
    expect_autocorrect_parity(*klasses, source, config_for("consistent_comma"))
  end

  # 1.92.3 (rubocop#16209): `put_comma` inserts the comma after the whole last
  # item, so parentheses `Lint/AmbiguousOperatorPrecedence` wraps around a part
  # of it (starting on an earlier line) end up before the comma. One shared
  # Team round shows the merge order; a single cop cannot.
  it "puts the comma after parentheses another cop adds in the same round (1.92.3)" do
    cfg = RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        {
          "Style/TrailingCommaInHashLiteral" => { "EnforcedStyleForMultiline" => "comma" },
          "Lint/AmbiguousOperatorPrecedence" => { "Enabled" => true }
        },
        "(test)"
      ),
      "(test)"
    )
    src = "{\n  a: 1,\n  b: x + y *\n    z\n}\n{\n  \"é\" => 1, k: [1,\n    2] + c *\n    d\n}\n"
    other = RuboCop::Cop::Lint::AmbiguousOperatorPrecedence
    stock = one_team_round([RuboCop::Cop::Style::TrailingCommaInHashLiteral, other], src, cfg)
    expect(stock).to include("z),")
    expect(one_team_round([Shirobai::Cop::Style::TrailingCommaInHashLiteral, other], src, cfg)).to eq(stock)
    expect(one_team_round([other, Shirobai::Cop::Style::TrailingCommaInHashLiteral], src, cfg))
      .to eq(one_team_round([other, RuboCop::Cop::Style::TrailingCommaInHashLiteral], src, cfg))
  end
end
