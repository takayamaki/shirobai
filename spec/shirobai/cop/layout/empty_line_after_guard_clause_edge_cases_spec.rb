# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/EmptyLineAfterGuardClause`.
#
# The vendor spec covers the headline shapes, but several places where a
# prism-based port can silently diverge are not pinned:
#
#   1. A modifier `if` inside a `when` clause body: parser-AST parent is
#      `begin` (when's body becomes implicit begin when multi-stmt), prism
#      doesn't fire the StatementsNode hook for the when's typed visit, so a
#      naive port that only looks at the prism ancestor stack misses the
#      multi-stmt sibling and emits NO offense (regression seen on
#      `fileutils.rb`'s `link` method).  Stock emits an offense on the if's
#      `end` keyword.
#   2. A guard `if cond; return; end` followed by `begin..rescue..end`: the
#      if is multi-line so offense range is the `end` keyword (3 bytes), not
#      the whole if node.
#   3. `next` modifier-form under prism: prism uses the Latest grammar so
#      `next unless x` IS supported even though the vendor spec brackets that
#      case in `Ruby <= 3.2` / `unsupported_on: :prism`.  Stock under prism
#      emits the offense; the port must too.
#   4. Sole-stmt modifier-if inside the rescue's `else` body: parser parent
#      is `rescue` (outer), `next_line_rescue_or_ensure?` is true → no
#      offense.  Prism reaches this via a `BeginNode#else_clause` -> ElseNode
#      -> StatementsNode chain; the port must classify that ElseNode as the
#      rescue-else (not the if-else).
#   5. The `Style/HashTransformKeys` / `Layout/AccessModifierIndentation` -
#      style "stash the prism node in the frame" idiom: the rule keeps
#      `Node<'static>` (byte-copied via `transmute_copy`) on its ancestor
#      stack so it can call prism accessors on the parent at the if visit.
#      A regression where the lifetime erasure read freed memory would
#      surface as a crash on the first non-trivial source.
#
# All cases are differential against the 1.87-pinned vendor.
RSpec.describe Shirobai::Cop::Layout::EmptyLineAfterGuardClause do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::EmptyLineAfterGuardClause,
    Shirobai::Cop::Layout::EmptyLineAfterGuardClause
  ]

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  it "flags a multi-line guard if inside a `when` clause body with sibling" do
    src = <<~RUBY
      case x
      when :a
        if cond
          raise "nope"
        end
        bar
      end
    RUBY
    expect_lint_parity(*klasses, src, cfg)
    expect(expect_autocorrect_parity(*klasses, src, cfg)).to eq(<<~RUBY)
      case x
      when :a
        if cond
          raise "nope"
        end

        bar
      end
    RUBY
  end

  it "accepts a sole-stmt modifier if in a rescue's `else` body" do
    src = <<~RUBY
      def foo
        bar
      rescue Y
        baz
      else
        return if x
      end
    RUBY
    expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
  end

  it "flags `next unless x` followed by a statement (prism uses Latest grammar)" do
    src = <<~RUBY
      def foo
        next unless need_next? # comment
        foobar
      end
    RUBY
    expect_lint_parity(*klasses, src, cfg)
  end

  it "flags a multi-line guard if inside a `case`-`else` body with sibling" do
    src = <<~RUBY
      case x
      when :a
        bar
      else
        if cond
          return
        end
        baz
      end
    RUBY
    expect_lint_parity(*klasses, src, cfg)
  end

  it "matches stock on a modifier guard inside a lambda body with sibling" do
    src = <<~RUBY
      -> do
        return if x
        bar
      end
    RUBY
    expect_lint_parity(*klasses, src, cfg)
  end

  it "matches stock on a guard whose if_branch is `and return`" do
    src = <<~RUBY
      def foo
        render :a and return if cond
        do_something
      end
    RUBY
    expect_lint_parity(*klasses, src, cfg)
  end

  it "accepts `return <<~HEREDOC if cond` whose heredoc body fills the next lines" do
    # Mirrors discourse's `topic_tracking_state.rb` and `lib/discourse.rb`:
    # the heredoc body sits on the lines immediately after the `return`, so
    # stock's `last_heredoc_argument` routes the check to the heredoc closer
    # and finds the following blank line — no offense.  A port that fails to
    # descend into the `return` node's argument misses the heredoc and fires
    # on the if's `node.last_line` (the `return` line) instead.
    src = <<~RUBY
      def foo
        return <<~SQL if cond
            body line
          SQL

        bar
      end
    RUBY
    expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
  end

  it "accepts `next <<~HEREDOC if cond` whose heredoc body fills the next lines" do
    src = <<~RUBY
      items.each do |i|
        next <<~MSG if i.skip?
          body line
        MSG

        bar(i)
      end
    RUBY
    expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
  end

  it "accepts `break <<~HEREDOC if cond` whose heredoc body fills the next lines" do
    src = <<~RUBY
      result = loop do
        break <<~MSG if done?
          body line
        MSG

        step
      end
    RUBY
    expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
  end

  it "skips a guard whose right sibling is itself a multi-line guard `if`" do
    src = <<~RUBY
      def foo
        return if something?
        if something_else?
          raise bar(
            baz
          )
        end
      end
    RUBY
    expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
  end
end
