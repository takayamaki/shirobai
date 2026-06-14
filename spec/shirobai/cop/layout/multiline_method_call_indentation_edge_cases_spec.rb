# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/MultilineMethodCallIndentation`.
#
# The vendor spec exercises the three `EnforcedStyle`s on plain `a.b\n .c`
# chains, but does NOT pin the "chain link carries a brace block" case that
# Parser-AST `:block` wraps cleanly and that prism's flat `CallNode.location`
# does not. Two stock quirks live here:
#
#   1. `single_line_block_receiver?` — when a chain link is `.foo { ... }` on
#      one line, stock treats the next link as continuing the chain and aligns
#      with the bracy link's `.foo`. In Parser AST the `:block` node's
#      `single_line?` covers only `dot ~ block-end`. In prism the chain
#      link is still a `CallNode` whose `location` spans the WHOLE receiver
#      chain (multi-line as soon as any earlier link broke a line) — measuring
#      "single line" off the full call location wrongly disqualifies it and
#      the cop then over-reports against the first chain link.
#
#   2. `left_hand_side` walks `lhs.parent` while it is `call_type?` with a
#      dot. In Parser AST a call carrying a block is wrapped in a `:block`
#      (not `call_type?`), so the walk stops at the call itself. In prism the
#      same call has the block as a *child*, so a naive parent walk climbs
#      through to the enclosing send and `indentation(lhs)` collapses to the
#      outer expression's column — producing a `Use 2 (not 4) spaces` ghost
#      offense on the brace-block link of `.a.to receive(:x).with(...) { ... }`.
#
# Both quirks were corpus-only before this spec (Mastodon
# `color_extractor.rb` chain link and `bulk_import_service_spec.rb` `.with`
# block); pinned here as differential regressions against the 1.87-pinned
# stock.
RSpec.describe Shirobai::Cop::Layout::MultilineMethodCallIndentation do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::MultilineMethodCallIndentation,
    Shirobai::Cop::Layout::MultilineMethodCallIndentation
  ]

  let(:aligned_config) { RuboCop::ConfigLoader.default_configuration }

  it "treats a brace-block chain link as a continuation (Mastodon color_extractor)" do
    # Minimised from `lib/paperclip/color_extractor.rb` 223-227. Every link
    # after `.with_index { ... }` is aligned with the brace-block link's `.`,
    # so stock produces zero offenses. shirobai used to climb to the bottom
    # `frequencies.map` and emit three `Align .reject/.map/.slice with .map`
    # ghosts.
    src = <<~RUBY
      def palette_from_im_histogram(result, quantity)
        frequencies.map.with_index { |f, i| [f / total_frequencies, hex_values[i]] }
          .sort_by { |r| -r[0] }
          .reject { |r| r[1].size == 8 && r[1].end_with?('00') }
          .map { |r| ColorDiff::Color::RGB.new(*r[1][0..5].scan(/../).map { |c| c.to_i(16) }) }
          .slice(0, quantity)
      end
    RUBY
    expect_lint_parity(*klasses, src, aligned_config, expect_offenses: false)
    expect_autocorrect_parity(*klasses, src, aligned_config)
  end

  it "treats a brace-block link with a multi-line receiver chain as aligned" do
    # The continuation chain itself has multiple links carrying brace blocks.
    # Each `.foo { ... }` is single-line and shirobai must measure that off
    # the call's `dot ~ block-end`, not the full receiver-chain location.
    src = <<~RUBY
      result = xs.map { |x| x }
        .reject { |x| x.nil? }
        .sort_by { |x| x.size }
    RUBY
    expect_lint_parity(*klasses, src, aligned_config, expect_offenses: false)
  end

  it "does not climb past a brace-block link in left_hand_side (RSpec .with)" do
    # Minimised from `spec/services/bulk_import_service_spec.rb` 344-346.
    # `.to` has no parens, its arg is `receive(:call).with(...) { ... }`. The
    # `.with` link begins its line and on prism it has the block as a CHILD
    # (not as a Parser-AST `:block` parent), so a naive `lhs.parent` walk
    # climbs to the outer `.to`/`.allow(...)` and collapses
    # `indentation(lhs)` to 0 — producing a ghost `Use 2 (not 4) spaces`.
    src = <<~RUBY
      allow(resolve_account_service_double)
        .to receive(:call)
          .with('user@foo.bar', any_args) { Fabricate(:account) }
    RUBY
    expect_lint_parity(*klasses, src, aligned_config, expect_offenses: false)
  end

  it "still reports a brace-block chain link that is genuinely misaligned" do
    # Negative control: when the continuation links are not actually aligned
    # with the brace-block link, stock DOES report — shirobai must too, with
    # an identical message. Guards against over-suppressing the new case.
    src = <<~RUBY
      def f
        xs.map { |x| x }
            .reject { |x| x.nil? }
      end
    RUBY
    stock = expect_lint_parity(*klasses, src, aligned_config)
    expect(stock.first[2]).to include("Align `.reject` with `.map`")
  end

  it "does not anchor a stabby lambda body's chain to the lambda's assignment" do
    # Minimised from Discourse `app/models/remote_theme.rb` 383-387. Stock's
    # `disqualified_rhs?` breaks `part_of_assignment_rhs` at the surrounding
    # `:block` (a Parser-AST `:block` wraps every stabby lambda's body), so the
    # `.theme_fields` continuation aligns against `.theme_fields` itself —
    # delta 0, no offense. Prism makes the lambda a separate `LambdaNode`, so
    # without a `Block` frame entry for it the assignment-rhs walk climbs
    # through to the enclosing `transaction_block =` and anchors `.x` chains
    # to the lambda's source range (column 20: `->(*) do`), producing two
    # `Align `.theme_fields` with `->(*) do`` ghosts.
    src = <<~RUBY
      transaction_block = ->(*) do
        theme
          .theme_fields
          .where(id: 1)
      end
    RUBY
    expect_lint_parity(*klasses, src, aligned_config, expect_offenses: false)
  end

  it "treats a stabby lambda with parameter list as a chain anchor barrier" do
    # Multiple stabby lambdas with diverse parameter lists (Discourse
    # `discourse-ai/lib/sentiment/emotion_filter_order.rb` 8 ff,
    # `discourse-solved/lib/discourse_solved/register_filters.rb` 6 ff). The
    # body's `.with/.joins/.order` chain must align against its own first
    # dot-link, not the lambda's `->(scope, ...) do` source.
    src = <<~RUBY
      callback = ->(scope, order_direction, _guardian) do
        scope
          .with(topic_emotion: 1)
          .joins("foo")
          .order("bar")
      end
    RUBY
    expect_lint_parity(*klasses, src, aligned_config, expect_offenses: false)
  end

  it "finds a descendant block buried inside argument lists (Discourse Notifications chain)" do
    # Minimised from `discourse-reactions/plugin.rb` 321 ff. Stock's
    # `handle_descendant_block` walks `node.each_descendant(:any_block).first`
    # — a FULL DFS over the call's subtree, so a `Proc.new do ... end` buried
    # inside an argument hash counts as a descendant block. When such a block
    # is found and the receiver is a call_type, the chain link aligns against
    # the receiver's `.method`. shirobai used to only walk the receiver chain
    # for blocks, missing the argument case, then fell through to the
    # assignment-rhs path and anchored `.set_mutations` to the constant path
    # base (`Notifications::DeletePreviousNotifications`).
    src = <<~RUBY
      reacted_by_two_users =
        Notifications::DeletePreviousNotifications
          .new(
            type: 1,
            previous_query_blk:
              Proc.new do |notifications, data|
                notifications.where(id: data[:previous_notification_id])
              end,
          )
          .set_mutations(
            set_data_blk:
              Proc.new do |notification|
                existing = 1
              end,
          )
    RUBY
    # Stock reports only the `.new` link (which is genuinely misaligned vs.
    # `Notifications::DeletePreviousNotifications`); `.set_mutations` aligns
    # against `.new` via the descendant-block path and is silent.
    stock = expect_lint_parity(*klasses, src, aligned_config)
    expect(stock.size).to eq(1)
    expect(stock.first[2]).to include("Align `.new` with")
  end

  it "respects only the FIRST descendant block's single-line/multi-line status" do
    # Minimised from `discourse-reactions/plugin.rb` 268 ff. The hash on `.new`
    # leads with `threshold: -> { ... }` — a single-line lambda. Stock's
    # `each_descendant(:any_block).first&.multiline?` therefore returns nil
    # (the leading lambda is single-line) regardless of any later multi-line
    # `Proc.new do ... end`, so the chain falls back to syntactic alignment
    # against the constant path base — `.set_mutations` SHOULD report
    # `Align with Notifications::ConsolidateNotifications`. A naive "any
    # multiline descendant block" check would silence the offense.
    src = <<~RUBY
      foo =
        Notifications::ConsolidateNotifications
          .new(
            from: 1,
            threshold: -> { 5 },
            unconsolidated_query_blk:
              Proc.new do |notifications, data|
                notifications.where("foo = ?", data[:x])
              end,
          )
          .set_mutations(
            set_data_blk:
              Proc.new do |notification|
                notification
              end,
          )
    RUBY
    stock = expect_lint_parity(*klasses, src, aligned_config)
    # `.new` and `.set_mutations` should both report against the constant base.
    expect(stock.size).to eq(2)
    expect(stock.map { |o| o[2] }).to include(
      a_string_including("Align `.new` with `Notifications::ConsolidateNotifications`"),
      a_string_including("Align `.set_mutations` with `Notifications::ConsolidateNotifications`")
    )
  end
end
