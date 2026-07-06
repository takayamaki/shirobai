# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/PendingWithoutReason`
      # (rubocop-rspec 3.10.2). No autocorrect (stock has none).
      #
      # Rust supplies candidate SEND ranges (skipped/pending example or skipped
      # example-group names, plus any send carrying `:skip` / `:pending`
      # metadata); this wrapper relocates each parser send node and runs stock's
      # `on_send` VERBATIM. All parent-relationship logic (spec_group? /
      # example? / block detection) runs on the real parser AST.
      class PendingWithoutReason < RuboCop::Cop::Base
        include RuboCop::RSpec::Language
        include Shirobai::Cop::RSpec::SendCandidateSupport

        MSG = "Give the reason for pending or skip."

        def self.cop_name = "RSpec/PendingWithoutReason"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        # @!method skipped_in_example?(node)
        def_node_matcher :skipped_in_example?, <<~PATTERN
          {
            (send nil? ${#Examples.skipped #Examples.pending})
            (any_block (send nil? ${#Examples.skipped}) ...)
          }
        PATTERN

        # @!method skipped_by_example_method?(node)
        def_node_matcher :skipped_by_example_method?, <<~PATTERN
          (send nil? ${#Examples.skipped #Examples.pending})
        PATTERN

        # @!method skipped_by_example_method_with_block?(node)
        def_node_matcher :skipped_by_example_method_with_block?, <<~PATTERN
          (any_block (send nil? ${#Examples.skipped #Examples.pending} ...) ...)
        PATTERN

        # @!method metadata_without_reason?(node)
        def_node_matcher :metadata_without_reason?, <<~PATTERN
          (send #rspec?
            {#ExampleGroups.all #Examples.all} ...
            {
              <(sym ${:pending :skip}) ...>
              (hash <(pair (sym ${:pending :skip}) true) ...>)
            }
          )
        PATTERN

        # @!method skipped_by_example_group_method?(node)
        def_node_matcher :skipped_by_example_group_method?, <<~PATTERN
          (send #rspec? ${#ExampleGroups.skipped} ...)
        PATTERN

        private

        def candidate_slot = :rspec_pending_without_reason

        def fallback_candidates
          Shirobai.check_rspec_pending_without_reason(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end

        # --- stock's methods, copied verbatim (rubocop-rspec 3.10.2);
        # `on_send` renamed to `investigate_send` so it is not a node callback.

        def investigate_send(node)
          on_pending_by_metadata(node)
          return unless (parent = parent_node(node))

          if spec_group?(parent) || block_node_example_group?(node)
            on_skipped_by_example_method(node)
            on_skipped_by_example_group_method(node)
          elsif example?(parent)
            on_skipped_by_in_example_method(node)
          end
        end

        def parent_node(node)
          node_or_block = node.block_node || node
          return unless (parent = node_or_block.parent)

          parent.begin_type? && parent.parent ? parent.parent : parent
        end

        def block_node_example_group?(node)
          node.block_node &&
            example_group?(node.block_node) &&
            explicit_rspec?(node.receiver)
        end

        def on_skipped_by_in_example_method(node)
          skipped_in_example?(node) do |pending|
            add_offense(node, message: "Give the reason for #{pending}.")
          end
        end

        def on_pending_by_metadata(node)
          metadata_without_reason?(node) do |pending|
            add_offense(node, message: "Give the reason for #{pending}.")
          end
        end

        def on_skipped_by_example_method(node)
          skipped_by_example_method?(node) do |pending|
            add_offense(node, message: "Give the reason for #{pending}.")
          end

          skipped_by_example_method_with_block?(node.parent) do |pending|
            add_offense(node, message: "Give the reason for #{pending}.")
          end
        end

        def on_skipped_by_example_group_method(node)
          skipped_by_example_group_method?(node) do
            add_offense(node, message: "Give the reason for skip.")
          end
        end
      end
    end
  end
end
