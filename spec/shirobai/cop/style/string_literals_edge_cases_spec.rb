# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: the `xstr`-interior boundary between
# `Style/StringLiterals` and `Style/StringLiteralsInInterpolation`.
#
# Stock's `StringHelp#inside_interpolation?` is `node.ancestors.drop_while {
# |a| !a.begin_type? }.any? { |a| a.type?(:dstr, :dsym, :regexp) }`. Crucially
# `:xstr` is *not* on that recognised type list. So a string literal living
# inside a `#{...}` inside backticks is **NOT** "inside interpolation" by stock's
# contract, and `Style/StringLiterals` claims it (not the In-Interpolation cop).
#
# The shirobai parity bug (Discourse): the interpolation guard used a flat
# `any(|f| f.is_interp)` over the frame stack, which fired for *any*
# `Embedded*` ancestor regardless of what enclosed it. Under that buggy guard a
# `" "` inside ```yarn ... #{files.join(" ")} ...``` was treated as
# interpolation-internal, so `Style/StringLiterals` skipped it and
# `Style/StringLiteralsInInterpolation` claimed it — a same-position drop-in
# violation (cop name + message differ even though the total offense count is
# unchanged). Discourse `lib/tasks/documentation.rake:17` and
# `script/import_scripts/jive_api.rb:504` both trip this.
#
# The fix routes the interpolation check through stock's drop_while-then-any
# scan: an `Embedded*` frame must have a `:dstr` / `:dsym` / `:regexp` ancestor
# strictly outside it. `xstr` (and `InterpolatedXStringNode`) is intentionally
# absent, mirroring stock.
RSpec.describe Shirobai::Cop::Style::StringLiterals do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Style::StringLiterals,
    Shirobai::Cop::Style::StringLiterals
  ]

  # Stock claims the `" "` inside ``` `... #{files.join(" ")} ...` ``` because
  # the enclosing `xstr` is not a `dstr`/`dsym`/`regexp` carrier. Discourse
  # `lib/tasks/documentation.rake:17` shape.
  it "claims string literals inside #{'#{...}'} inside backticks" do
    source = <<~RUBY
      def f(config, files, destination)
        `yarn --silent -c \#{config} \#{files.join(" ")} -d \#{destination}`
      end
    RUBY
    expect_lint_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  # The bare `JSON.parse \`#{command.join(" ")}\`` form from
  # `script/import_scripts/jive_api.rb:504`. Same xstr-interior shape, no
  # surrounding method body.
  it "claims string literals inside bare backtick-only interpolation" do
    source = "JSON.parse `\#{command.join(\" \")}`\n"
    expect_lint_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  # Negative side: a `" "` inside a `#{...}` inside a real `dstr` (double-quoted
  # string) must NOT be claimed by `Style/StringLiterals` — that one belongs to
  # `Style/StringLiteralsInInterpolation`. Both stock and shirobai must produce
  # the *same* (empty for this cop) result, so a regression that resurrected
  # the flat-any guard would surface as a new false positive here.
  it "does not claim string literals inside #{'#{...}'} inside a dstr" do
    source = %("Tests \#{success ? "PASS" : "FAIL"}"\n)
    # stock returns no `Style/StringLiterals` offenses for this fixture (the
    # outer dstr-literal does not offend; the inner "PASS"/"FAIL" are claimed
    # by the In-Interpolation cop, not by this one). Avoid the not-be-empty
    # vacuous guard here — the assertion that matters is `shirobai == stock`.
    expect_lint_parity(*klasses, source, config, expect_offenses: false)
    expect_autocorrect_parity(*klasses, source, config)
  end
end
