# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/TrailingCommaInArrayLiteral`, pinning
# stock 1.88.0 quirks probed during the port that the vendor spec does not
# exercise:
#
# - `square_brackets?`: only `[`-opened literals count. `%w[…]` percent arrays
#   (opening token `%w[`, not `[`) and implicit no-bracket arrays
#   (`a = 1, 2` / `return 1, 2`) never fire, whatever the style.
# - Stock's `heredoc?` only treats `send_type?` (`:send`). A SAFE-NAVIGATION
#   call wrapping a heredoc element is `(csend …)` and is NOT heredoc-flagged,
#   so the comma regex crosses the newline into the heredoc body and flags
#   (and removes!) a comma that is heredoc text. The plain `.` version is
#   heredoc-flagged and stays clean.
# - A splat (`*a`) last element gets put/avoid like any element.
# - Nested literals each offend on their own trailing comma.
RSpec.describe Shirobai::Cop::Style::TrailingCommaInArrayLiteral do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Style::TrailingCommaInArrayLiteral,
    Shirobai::Cop::Style::TrailingCommaInArrayLiteral
  ]

  def config_for(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        {
          "Style/TrailingCommaInArrayLiteral" => {
            "EnforcedStyleForMultiline" => style
          }
        }, "(test)"
      ),
      "(test)"
    )
  end

  it "never fires on percent arrays regardless of bracket shape" do
    %w[no_comma comma consistent_comma diff_comma].each do |style|
      expect_lint_parity(*klasses, "x = %w[\n  a\n  b\n]\n", config_for(style),
                         expect_offenses: false)
      expect_lint_parity(*klasses, "x = %i(\n  a\n  b\n)\n", config_for(style),
                         expect_offenses: false)
    end
  end

  it "never fires on implicit (no-bracket) arrays" do
    expect_lint_parity(*klasses, "a = 1,\n  2\n", config_for("consistent_comma"),
                       expect_offenses: false)
    expect_lint_parity(*klasses, "def f\n  return 1,\n    2\nend\n",
                       config_for("consistent_comma"), expect_offenses: false)
  end

  it "flags the heredoc-body comma of a safe-navigation heredoc element (stock quirk)" do
    source = "x = [\n  a&.b(<<~X)\n, inside\n  X\n]\n"
    expect_lint_parity(*klasses, source, config_for("no_comma"))
    expect_autocorrect_parity(*klasses, source, config_for("no_comma"))
  end

  it "does not flag the heredoc-body comma of a plain-call heredoc element" do
    source = "x = [\n  a.b(<<~X)\n, inside\n  X\n]\n"
    expect_lint_parity(*klasses, source, config_for("no_comma"), expect_offenses: false)
  end

  it "puts and avoids a comma after a splat last element" do
    expect_lint_parity(*klasses, "x = [\n  *a\n]\n", config_for("consistent_comma"))
    expect_autocorrect_parity(*klasses, "x = [\n  *a\n]\n", config_for("consistent_comma"))
    expect_lint_parity(*klasses, "x = [\n  *a,\n]\n", config_for("no_comma"))
    expect_autocorrect_parity(*klasses, "x = [\n  *a,\n]\n", config_for("no_comma"))
    expect_lint_parity(*klasses, "x = [\n  *a,\n]\n", config_for("consistent_comma"),
                       expect_offenses: false)
  end

  it "treats a single element with the closing bracket on its line as allowed multiline" do
    expect_lint_parity(*klasses, "x = [{\n  a: 1\n}]\n", config_for("consistent_comma"),
                       expect_offenses: false)
  end

  it "handles a bare heredoc as the last element" do
    expect_lint_parity(*klasses, "x = [\n  1,\n  <<~EOS,\n    t\n  EOS\n]\n",
                       config_for("no_comma"))
    expect_autocorrect_parity(*klasses, "x = [\n  1,\n  <<~EOS,\n    t\n  EOS\n]\n",
                              config_for("no_comma"))
    expect_lint_parity(*klasses, "x = [\n  1,\n  <<~EOS\n    t\n  EOS\n]\n",
                       config_for("consistent_comma"))
    expect_autocorrect_parity(*klasses, "x = [\n  1,\n  <<~EOS\n    t\n  EOS\n]\n",
                              config_for("consistent_comma"))
  end

  it "flags each nested literal's trailing comma" do
    source = "x = [[1, 2,], [3],]\n"
    expect_lint_parity(*klasses, source, config_for("no_comma"))
    expect_autocorrect_parity(*klasses, source, config_for("no_comma"))
  end
end
