# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/Focus` (rubocop-rspec 3.10.2).
      #
      # Rust supplies candidate SEND ranges (focused aliases + focusable
      # selectors carrying `:focus` / `focus: true`); this wrapper relocates
      # each parser send node and runs stock's `on_send` plus autocorrect
      # VERBATIM. The chained / inside-def guards, the exact matchers, and the
      # two correction modes (remove metadata / rename selector, with the
      # non-correctable bare-alias case) match byte for byte.
      class Focus < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        include RuboCop::RSpec::Language
        include Shirobai::Cop::RSpec::SendCandidateSupport
        include RuboCop::Cop::RangeHelp

        MSG = "Focused spec found."

        def self.cop_name = "RSpec/Focus"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        # @!method focusable_selector?(node)
        def_node_matcher :focusable_selector?, <<~PATTERN
          {
            #ExampleGroups.regular
            #ExampleGroups.skipped
            #Examples.regular
            #Examples.skipped
            #Examples.pending
            #SharedGroups.all
          }
        PATTERN

        # @!method metadata(node)
        def_node_matcher :metadata, <<~PATTERN
          {(send #rspec? #focusable_selector? <$(sym :focus) ...>)
           (send #rspec? #focusable_selector? ... (hash <$(pair (sym :focus) true) ...>))}
        PATTERN

        # @!method focused_block?(node)
        def_node_matcher :focused_block?, <<~PATTERN
          (send #rspec? {#ExampleGroups.focused #Examples.focused} ...)
        PATTERN

        private

        def candidate_slot = :rspec_focus

        def fallback_candidates
          Shirobai.check_rspec_focus(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end

        # --- stock's methods, copied verbatim (rubocop-rspec 3.10.2);
        # `on_send` renamed to `investigate_send` so it is not a node callback.

        def investigate_send(node)
          return if node.chained? || node.each_ancestor(:any_def).any?

          if focused_block?(node)
            on_focused_block(node)
          else
            metadata(node) do |focus|
              on_metadata(focus)
            end
          end
        end

        def on_focused_block(node)
          add_offense(node) do |corrector|
            correct_send(corrector, node)
          end
        end

        def on_metadata(node)
          add_offense(node) do |corrector|
            corrector.remove(with_surrounding(node))
          end
        end

        def with_surrounding(focus)
          range_with_space =
            range_with_surrounding_space(focus.source_range, side: :left)

          range_with_surrounding_comma(range_with_space, :left)
        end

        def correct_send(corrector, focus)
          range = focus.loc.selector
          unfocused = focus.method_name.to_s.sub(/^f/, "")
          unless Examples.regular(unfocused) || ExampleGroups.regular(unfocused)
            return
          end

          corrector.replace(range,
                            range.source.sub(focus.method_name.to_s, unfocused))
        end
      end
    end
  end
end
