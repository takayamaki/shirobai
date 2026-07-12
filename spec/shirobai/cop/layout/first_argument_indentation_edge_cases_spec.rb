# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/FirstArgumentIndentation`.
#
# The vendor spec exercises the `EnforcedStyle`s and the `ArgumentAlignment`
# interplay, but does NOT pin this stock filter that fell out of the Discourse
# parity diff:
#
#   `should_check?` is gated on `!bare_operator?(node)`, and `bare_operator?`
#   is `operator_method? && !dot?`. rubocop-ast's `OPERATOR_METHODS` set
#   includes `[]` and `[]=`, so a braceless index read (`x[:sym]`, written
#   without a `.` between the receiver and the bracket selector) is a bare
#   operator and stock silently skips it. The setter form (`x[:sym] = v`) is
#   also covered by the `!setter_method?` guard, but the read form relies
#   entirely on the operator filter.
#
#   shirobai used to special-case `[]` and `[]=` *out* of its operator
#   classifier, so multi-line index reads with a misaligned argument line
#   produced a false positive that stock never reports. Real CLI diff on
#   Discourse: 9 ghosts across `app/models/color_scheme.rb` (modifier-`if`
#   guarding a `&.map { ... } if params[\n  :sym\n]` body) and
#   `app/services/upcoming_changes/action/track_status_changes.rb`
#   (`.event_data[\n  "new_value"\n]` at the tail of a `.`-chain).
#
# Pinned here as differential regressions against the 1.87-pinned stock.
RSpec.describe Shirobai::Cop::Layout::FirstArgumentIndentation do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::FirstArgumentIndentation,
    Shirobai::Cop::Layout::FirstArgumentIndentation
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "skips a multi-line braceless index read used as a modifier-if argument (color_scheme.rb:436)" do
    # Minimised from `app/models/color_scheme.rb`: the trailing `if` takes
    # `params[\n  :base_scheme_id\n]` as its condition. The `[` opens a
    # `:[]` send (bare operator) whose first argument is `:base_scheme_id`,
    # mis-indented relative to the `if params[` line. stock filters the call
    # out via `bare_operator?` and never reports.
    src = <<~RUBY
      colors =
        BUILT_IN_SCHEMES[scheme_name.to_sym]&.map { |name, hex| { name: name, hex: hex } } if params[
        :base_scheme_id
      ]
    RUBY
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end

  it "skips a multi-line braceless index read at the tail of a `.`-chain (track_status_changes.rb:81)" do
    # Minimised from `app/services/upcoming_changes/action/track_status_changes.rb`:
    # the deepest call is `.event_data[\n  "new_value"\n]`. The bracket is a
    # `:[]` send (bare operator) so stock skips it. The dot-chain receiver
    # before the `[` does not turn it into a `dot?` call — `node.dot?` is
    # about the selector position, and `x.y[...]` keeps `:[]` itself
    # bracketed, not dotted.
    src = <<~RUBY
      def previous_status_for(change_name)
        previous_status_events
          .select { |event| event.upcoming_change_name == change_name.to_s }
          .last
          .event_data[
          "new_value"
        ]
      end
    RUBY
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end

  it "skips even when the indexed argument is wildly off (the operator filter wins)" do
    # Negative confirmation: even a column-0 first arg is silent, because the
    # `bare_operator?` filter fires before `check_alignment` ever runs.
    src = <<~RUBY
      x = foo[
      :wildly_unindented
      ]
    RUBY
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end

  it "still reports a misaligned first argument on a regular multi-line call (positive control)" do
    # Negative control: a non-operator method call still triggers the check.
    # Guards against the fix swallowing every operator-shaped send.
    src = <<~RUBY
      run(
      :foo,
        bar: 3
      )
    RUBY
    stock = expect_lint_parity(*klasses, src, config)
    expect(stock.size).to be >= 1
  end

  it "treats a lone block-pass argument as the first argument" do
    # parser-gem's `send.arguments` includes the block-pass argument, so a
    # call whose only argument is `&blk` still has a first argument to check.
    # prism keeps it in `CallNode#block()`, outside `arguments()` — shirobai
    # used to bail out and miss the offense. Same prism/parser mapping family
    # as the `Layout/ArgumentAlignment` bare-`&` gap found in the redmine
    # `-a` byte audit.
    src = <<~RUBY
      foo(
          &blk
      )
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "treats a lone block-pass argument to super as the first argument" do
    src = <<~RUBY
      def run(&blk)
        super(
            &blk
        )
      end
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "does not drag multi-line percent-string interiors when realigning" do
    # Same taboo as Layout/ArgumentAlignment: the correction target must be
    # the parser NODE so `inside_string_ranges` protects the `%()` body.
    src = <<~RUBY
      foo(
      %(line1
      line2 and more))
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "keeps the positional argument as the first when a block-pass follows" do
    src = <<~RUBY
      foo(
          bar,
          &blk
      )
    RUBY
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end
end
