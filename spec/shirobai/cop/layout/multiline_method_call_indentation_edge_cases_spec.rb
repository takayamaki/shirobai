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
end
