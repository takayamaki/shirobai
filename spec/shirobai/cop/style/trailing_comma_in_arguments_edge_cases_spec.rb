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
  # rubocop 1.92.2 (rubocop#16161): `put_comma` returns early when the last
  # argument is forwarded args (`...`), as it already did for a block-pass.
  context "with argument forwarding or a block-pass last (rubocop#16161)" do
    %w[comma consistent_comma diff_comma].each do |style|
      context "with EnforcedStyleForMultiline: #{style}" do
        let(:config) do
          RuboCop::ConfigLoader.merge_with_default(
            RuboCop::Config.new(
              { "Style/TrailingCommaInArguments" => { "EnforcedStyleForMultiline" => style } }, "(test)"
            ),
            "(test)"
          )
        end

        it "does not put a comma after `...`" do
          source = "def foo(...)\n  bar(\n    baz,\n    ...\n  )\nend\n"
          expect_lint_parity(*klasses, source, config, expect_offenses: false)
        end

        it "does not put a comma after `...` as the only argument" do
          source = "def foo(...)\n  bar(\n    ...\n  )\nend\n"
          expect_lint_parity(*klasses, source, config, expect_offenses: false)
        end

        it "does not put a comma after `&blk`" do
          source = "def foo(&blk)\n  bar(\n    baz,\n    &blk\n  )\nend\n"
          expect_lint_parity(*klasses, source, config, expect_offenses: false)
        end

        it "still puts a comma after the argument before a do block" do
          source = "def foo(...)\n  bar(\n    baz,\n    qux\n  ) do\n  end\nend\n"
          expect_lint_parity(*klasses, source, config)
          expect_autocorrect_parity(*klasses, source, config)
          expect_correction_edits_parity(*klasses, source, config)
        end
      end
    end

    # A comma after `...` is a syntax error (`unexpected token tCOMMA`), so
    # the avoid-comma arm can never see `...` last. The comma before it is
    # not a trailing comma.
    it "does not flag the comma before `...` under no_comma" do
      source = "def foo(...)\n  bar(baz, ...)\nend\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
    end
  end
end
