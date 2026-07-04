# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/IfUnlessModifier`.
#
# Every case was probed against stock on a real machine first; the vendor
# spec does not exercise them. Differential style: the same snippet runs
# through stock and shirobai (lint mode and/or the autocorrect loop) and the
# results must match exactly.
#
# Stock quirks pinned here:
# - The first-line comment is carried into the modifier form (and counted in
#   the length) UNLESS it matches `comment_disables_cop?`; a disable
#   directive for ANOTHER cop still counts as a plain comment.
# - `defined?` with a method-call argument always skips the node; an lvar
#   argument skips unless a parser LEFT SIBLING assigns it. parser gives
#   `x ||= ...` an inner zero-value `lvasgn` child, so the value of an
#   or-assign DOES see `x` as assigned.
# - A named capture (`=~` with a regexp literal) suppresses only the
#   to-modifier direction; the too-long direction still fires.
# - `another_statement_on_same_line?` stops at the first parser `begin`;
#   a plain `begin; ...; end` keeps statements as direct kwbegin children,
#   so a same-line sibling inside it does NOT suppress the offense.
# - The heredoc rewrite moves only the LAST argument's heredoc; csend and
#   assignment bodies take the plain rewrite (its output is stock's,
#   byte for byte, even when it looks broken).
# - Multi-pass autocorrect: stock stores ignored NODES and compares their
#   stale ranges numerically on later passes; `(a if b) if c` converges to
#   both forms corrected because the stale outer range fails to contain the
#   pass-2 inner node by one byte.
# - Lengths are CHARACTER counts (plus tab adjustment); the offense flips
#   exactly at the char boundary on non-ASCII lines.
RSpec.describe Shirobai::Cop::Style::IfUnlessModifier do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Style::IfUnlessModifier,
    Shirobai::Cop::Style::IfUnlessModifier
  ]

  long = "a" * 100

  describe "first-line comments (to-modifier direction)" do
    it "moves a trailing comment into the modifier form" do
      expect_autocorrect_parity(*klasses, "if a # trailing comment\n  b\nend\n", config)
    end

    it "keeps a disable directive for ANOTHER cop as a plain comment" do
      expect_autocorrect_parity(
        *klasses, "if a # rubocop:disable Metrics/AbcSize\n  b\nend\n", config
      )
    end

    it "drops a disable-all directive from the form (offense is :disabled)" do
      expect_lint_parity(*klasses, "if a # rubocop:disable all\n  b\nend\n", config)
    end

    it "accepts when the comment pushes the modifier form over Max" do
      src = "if some_condition # #{"c" * 110}\n  do_stuff\nend\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, config)).to be_empty
    end

    it "accepts a comment on the if line combined with code after end" do
      src = "foo(if a # com\n  b\nend)\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "accepts a comment on the end line" do
      expect_lint_parity(*klasses, "if a\n  b\nend # comment\n", config, expect_offenses: false)
    end
  end

  describe "defined? in the condition" do
    it "skips a method-call argument" do
      src = "if defined?(foo)\n  bar\nend\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "skips an lvar argument without a prior sibling assignment" do
      src = "if defined?(foo)\n  foo = 1\nend\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "fires for an lvar argument with a prior sibling assignment" do
      expect_autocorrect_parity(*klasses, "foo = 2\nif defined?(foo)\n  bar\nend\n", config)
    end

    it "fires for an ivar argument" do
      expect_autocorrect_parity(*klasses, "if defined?(@foo)\n  bar\nend\n", config)
    end

    it "fires inside an or-assign of the same lvar (parser's inner lvasgn)" do
      expect_autocorrect_parity(*klasses, "x ||= if defined?(x)\n  1\nend\n", config)
    end

    it "skips a method-call argument nested under &&" do
      src = "if defined?(foo) && x\n  bar\nend\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end
  end

  describe "condition lvasgn shapes" do
    it "skips a shorthand op-assign on a local in the condition" do
      src = "if (i += 1) > m\n  raise\nend\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "skips an or-assign on a local in the condition" do
      src = "if (i ||= 1)\n  raise\nend\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "skips a masgn in the condition" do
      src = "if (a, b = foo)\n  bar\nend\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "fires for a NESTED named capture (no parser lvasgn child)" do
      expect_autocorrect_parity(*klasses, "if x && /(?<y>.)/ =~ s\n  foo\nend\n", config)
    end
  end

  describe "named captures" do
    it "suppresses the to-modifier direction" do
      src = "if /(?<x>.)/ =~ s\n  x\nend\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "still fires the too-long direction" do
      expect_autocorrect_parity(
        *klasses, "do_something_here(x) if /(?<name>.)/ =~ #{long}_string\n", config
      )
    end
  end

  describe "another statement on the same line" do
    it "suppresses the too-long offense for a semicolon sibling" do
      src = "do_something(arg) if #{long}_condition; other_stuff\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "does NOT see siblings that are direct kwbegin children" do
      expect_autocorrect_parity(
        *klasses, "begin; do_something(arg) if #{long}_condition; other_stuff; end\n", config
      )
    end

    it "does NOT see the rescue clause of an implicit begin" do
      expect_autocorrect_parity(
        *klasses, "def m; foo(x) if #{long}_cond; rescue; end\n", config
      )
    end
  end

  describe "heredoc rewrites (too-long direction)" do
    it "moves the last argument's heredoc into the block form" do
      expect_autocorrect_parity(
        *klasses,
        "do_something_with(<<~TEXT) if #{long}_condition\n  body line\nTEXT\n",
        config
      )
    end

    it "moves only the LAST of two heredocs" do
      expect_autocorrect_parity(
        *klasses,
        "do_something_with(<<~A, <<~B) if #{long}_condition\n  a body\nA\n  b body\nB\n",
        config
      )
    end

    it "takes the plain rewrite for an assignment body (stock's own output)" do
      expect_autocorrect_parity(
        *klasses,
        "x = <<~TEXT if #{long}_condition\n  body line\nTEXT\n",
        config
      )
    end

    it "takes the plain rewrite for a csend body" do
      expect_autocorrect_parity(
        *klasses,
        "foo&.do_something_with(<<~TEXT) if #{long}_condition\n  body line\nTEXT\n",
        config
      )
    end
  end

  describe "multi-pass autocorrect" do
    it "converges nested modifier forms like stock (stale ignored ranges)" do
      expect_autocorrect_parity(
        *klasses,
        "(do_something_longer_here(arg) if #{"b" * 80}_cond) if outer_condition_here\n",
        config
      )
    end

    it "corrects a multiline if that is the body of a modifier if" do
      expect_autocorrect_parity(*klasses, "if a\n  b\nend if c\n", config)
    end
  end

  describe "body shapes" do
    it "renders a value-omission call with parentheses" do
      expect_autocorrect_parity(*klasses, "if cond\n  foo(x:)\nend\n", config)
    end

    it "renders a parenless value-omission call with parentheses" do
      expect_autocorrect_parity(*klasses, "if cond\n  foo x:, y: 1\nend\n", config)
    end

    it "accepts a parenthesized sole-statement body (parser begin)" do
      src = "if a\n  (b)\nend\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "converts a kwbegin body" do
      expect_autocorrect_parity(*klasses, "if a\n  begin; b; end\nend\n", config)
    end

    it "converts a non-endless one-line def body" do
      expect_autocorrect_parity(*klasses, "if a\n  def m; end\nend\n", config)
    end

    it "accepts an endless def body" do
      src = "if a\n  def m = 1\nend\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "fires with a blank line inside (3 nonempty lines)" do
      expect_autocorrect_parity(*klasses, "if a\n\n  b\nend\n", config)
    end

    it "keeps a multiline condition verbatim in the modifier form" do
      expect_autocorrect_parity(*klasses, "if aa &&\n   bb then cc\nend\n", config)
    end

    it "converts an if whose condition holds a parenthesized modifier if" do
      expect_autocorrect_parity(*klasses, "if (b if c)\n  d\nend\n", config)
    end

    it "keeps trailing whitespace after end out of the replacement" do
      expect_autocorrect_parity(*klasses, "if a\n  b\nend   \n", config)
    end
  end

  describe "tabs and character counting" do
    it "uses the char column for the block-form indentation under tabs" do
      expect_autocorrect_parity(
        *klasses, "\t\tdo_something(arg) if #{"a" * 95}_condition\n", config
      )
    end

    it "counts characters, not bytes (below the limit)" do
      src = "brief_call(arg) if #{"あ" * 50}_condition\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "counts characters, not bytes (above the limit)" do
      expect_autocorrect_parity(*klasses, "brief_call(arg) if #{"あ" * 100}_condition\n", config)
    end
  end

  describe "Layout/LineLength disabled" do
    let(:config) do
      RuboCop::ConfigLoader.merge_with_default(
        RuboCop::Config.new({ "Layout/LineLength" => { "Enabled" => false } }, "(test)"),
        "(test)"
      )
    end

    it "converts a multiline if regardless of length" do
      expect_autocorrect_parity(
        *klasses,
        "if #{long}_condition_name\n  do_something_quite_long(argument_one, argument_two)\nend\n",
        config
      )
    end

    it "never fires the too-long direction" do
      src = "do_something(arg) if #{long}_condition_name\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end
  end
end
