# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard, mirror image of the `Style/StringLiterals`
# edge-case spec.
#
# This cop's offense test is `inside_interpolation?(node) && wrong_quotes?`.
# Stock's `inside_interpolation?` recognises **only** `:dstr` / `:dsym` /
# `:regexp` as interpolation carriers; `:xstr` (backticks) is intentionally
# absent. So a `" "` inside ``` `... #{files.join(" ")} ...` ``` is NOT inside
# interpolation by this cop's contract — it must be left to
# `Style/StringLiterals`, not claimed here.
#
# The shirobai parity bug treated any `Embedded*` ancestor as interpolation,
# regardless of what enclosed it, so this cop over-claimed two Discourse
# offenses. The fix routes the interpolation check through stock's
# drop_while-then-any scan.
RSpec.describe Shirobai::Cop::Style::StringLiteralsInInterpolation do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Style::StringLiteralsInInterpolation,
    Shirobai::Cop::Style::StringLiteralsInInterpolation
  ]

  # The Discourse `documentation.rake:17` shape: stock leaves this cop silent
  # (the inner `" "` is xstr-interior, owned by `Style/StringLiterals`).
  # shirobai must match — both produce zero from this cop.
  it "does not claim string literals inside #{'#{...}'} inside backticks" do
    source = <<~RUBY
      def f(config, files, destination)
        `yarn --silent -c \#{config} \#{files.join(" ")} -d \#{destination}`
      end
    RUBY
    expect_lint_parity(*klasses, source, config, expect_offenses: false)
    expect_autocorrect_parity(*klasses, source, config)
  end

  # The `jive_api.rb:504` shape: bare `JSON.parse \`#{command.join(" ")}\``.
  # Same xstr-interior, same expected silence.
  it "does not claim string literals inside bare backtick-only interpolation" do
    source = "JSON.parse `\#{command.join(\" \")}`\n"
    expect_lint_parity(*klasses, source, config, expect_offenses: false)
    expect_autocorrect_parity(*klasses, source, config)
  end

  # Positive side: a `"PASS"` inside `#{...}` inside a `dstr` IS this cop's
  # responsibility — the dstr ancestor makes the `:begin` qualify under stock's
  # `drop_while`-then-`any?` scan. Both stock and shirobai must report it.
  it "claims string literals inside #{'#{...}'} inside a dstr" do
    source = %("Tests \#{success ? "PASS" : "FAIL"}"\n)
    stock = expect_lint_parity(*klasses, source, config)
    # PASS + FAIL = 2 offenses.
    expect(stock.size).to eq(2)
    expect_autocorrect_parity(*klasses, source, config)
  end

  # Positive side, regexp interpolation: `#{...}` inside `/.../` also qualifies
  # (this cop uniquely overrides `on_regexp` with an empty body so it doesn't
  # ignore regexp children). Stock and shirobai must agree.
  it "claims string literals inside #{'#{...}'} inside a regexp" do
    source = %(/Tests \#{success ? "PASS" : "FAIL"}/\n)
    stock = expect_lint_parity(*klasses, source, config)
    expect(stock.size).to eq(2)
    expect_autocorrect_parity(*klasses, source, config)
  end

  # A source with a non-UTF-8 magic comment: stock's `str_content` keeps the
  # source encoding, so `to_string_literal` (1.92.3: `inspect` for an invalid
  # value) and `inspect` must see the same string, not a UTF-8 copy.
  describe "sources in another encoding" do
    def style_config(style)
      RuboCop::ConfigLoader.merge_with_default(
        RuboCop::Config.new({ "Style/StringLiteralsInInterpolation" => { "EnforcedStyle" => style } }, "(test)"),
        "(test)"
      )
    end

    {
      "binary" => "# encoding: ascii-8bit\nx = \"#{'#'}{\"a\xFFb\"}\"\ny = \"#{'#'}{\"\xC3\xA9\"}\"\nz = \"#{'#'}{'\xC3\xA9'}\"\n",
      "EUC-JP" => "# encoding: euc-jp\nx = \"#{'#'}{\"\xA4\xA2\"}\"\ny = \"#{'#'}{'\xA4\xA2'}\"\n"
    }.each do |label, text|
      %w[single_quotes double_quotes].each do |style|
        it "matches stock in a #{label} source (#{style})" do
          src = text.b.force_encoding(Encoding::UTF_8)
          expect_lint_parity(*klasses, src, style_config(style))
          expect_autocorrect_parity(*klasses, src, style_config(style))
        end
      end
    end
  end
end
