# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/ClosingParenthesisIndentation`.
#
# The vendor spec covers `on_send` / `on_def` / `on_begin` (parenthesised
# expressions), but it does NOT exercise the `on_begin` path that parser-gem
# materialises around a string / regexp / symbol INTERPOLATION `#{...}`. That
# `:begin` node carries `loc.begin == "#{"` and `loc.end == "}"`, so stock's
# `check` treats the closing `}` as a hanging closing paren (the offense message
# even hard-codes `Indent `)``). Prism splits interpolation into its own
# `EmbeddedStatementsNode`, separate from `ParenthesesNode`, so the dispatcher
# has to opt in to both — Redmine `redcloth3.rb:775` slipped past a previously
# parens-only implementation and surfaced as a -1 in the real-CLI corpus diff.
RSpec.describe Shirobai::Cop::Layout::ClosingParenthesisIndentation do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::ClosingParenthesisIndentation,
    Shirobai::Cop::Layout::ClosingParenthesisIndentation
  ]

  let(:default_config) { RuboCop::ConfigLoader.default_configuration }

  it "checks the closing `}` of a regexp interpolation (Redmine redcloth3 form)" do
    # Direct reduction of redcloth3.rb:773-775 — the `#{` is at column 24 on
    # line 1, the inner statement is at column 8 on line 2, so the expected
    # column for `}` is 8 - 2 = 6 (line_break_after_left_paren branch).
    src = <<~RUBY
      MARKDOWN_RULE_RE = /^(\#{
              ['*', '-', '_'].collect { |ch| ' ?(' + Regexp::quote(ch) + ' ?){3,}' }.join('|')
          })$/
    RUBY
    offenses = expect_lint_parity(*klasses, src, default_config)
    expect(offenses.first[2]).to include("Indent `)` to column 6 (not 4)")
    expect(expect_autocorrect_parity(*klasses, src, default_config))
      .to eq(<<~RUBY)
        MARKDOWN_RULE_RE = /^(\#{
                ['*', '-', '_'].collect { |ch| ' ?(' + Regexp::quote(ch) + ' ?){3,}' }.join('|')
              })$/
      RUBY
  end

  it "checks the closing `}` of a plain string interpolation" do
    # Same `on_begin` branch outside a regexp: parser materialises `:begin`
    # around `#{...}` in a `"..."` literal too. Inner statement at column 4,
    # `IndentationWidth` 2, so expected column for `}` is 4 - 2 = 2.
    src = "a = \"x\#{\n    obj.thing(arg)\n}\"\n"
    offenses = expect_lint_parity(*klasses, src, default_config)
    expect(offenses.first[2]).to include("Indent `)` to column 2 (not 0)")
  end

  it "checks the closing `}` of a symbol interpolation" do
    # Dynamic symbol literal: `:"foo#{...}"` interpolation is the same `:begin`.
    # Inner statement at column 4, so expected column for `}` is 4 - 2 = 2.
    src = ":\"sym\#{\n    inner.call(arg)\n}\"\n"
    offenses = expect_lint_parity(*klasses, src, default_config)
    expect(offenses.first[2]).to include("Indent `)` to column 2 (not 0)")
  end

  it "accepts a correctly-indented interpolation `}`" do
    # Negative case: when `}` already matches the outdent rule, stock is silent.
    src = <<~RUBY
      MARKDOWN_RULE_RE = /^(\#{
              ['*', '-', '_'].collect { |ch| ' ?(' + Regexp::quote(ch) + ' ?){3,}' }.join('|')
            })$/
    RUBY
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  it "leaves a same-line interpolation untouched" do
    # `}` is not on its own line -> `begins_its_line?` is false for both.
    src = "x = \"a\#{foo(arg)}b\"\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end
end
