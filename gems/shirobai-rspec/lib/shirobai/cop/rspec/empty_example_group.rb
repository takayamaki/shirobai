# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/EmptyExampleGroup`
      # (rubocop-rspec 3.10.2).
      #
      # Architecture B (relocate-and-dispatch): Rust identifies candidate
      # example-group blocks on the shared walk; this wrapper locates the
      # parser-gem block node via `NodeLocator` and runs stock's detection
      # logic VERBATIM. The mutually recursive `examples?` matcher is deep
      # parser-AST structural matching that cannot be reproduced bytewise.
      # The autocorrect is simple (remove whole lines via `range_by_whole_lines`).
      #
      # Stock entry method `on_block` is renamed to `process_candidate` so
      # the Commissioner never dispatches a per-node `on_block`; only
      # `on_new_investigation` is a real callback.
      class EmptyExampleGroup < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        include RuboCop::RSpec::Language
        include RuboCop::Cop::RSpec::InsideExample
        include RuboCop::Cop::RangeHelp
        include Shirobai::Cop::BundleEligible

        MSG = "Empty example group detected."

        def self.cop_name = "RSpec/EmptyExampleGroup"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less (the segment's role lists cover everything).
        def self.bundle_args(_config)
          []
        end

        # --- stock node-pattern matchers, copied verbatim (rubocop-rspec 3.10.2) ---

        # @!method example_group_body(node)
        def_node_matcher :example_group_body, <<~PATTERN
          (block (send #rspec? #ExampleGroups.all ...) args $_)
        PATTERN

        # @!method example_or_group_or_include?(node)
        def_node_matcher :example_or_group_or_include?, <<~PATTERN
          {
            (block
              (send #rspec? {#Examples.all #ExampleGroups.all #Includes.all} ...)
            ...)
            (send nil? {#Examples.all #Includes.all} ...)
          }
        PATTERN

        # @!method examples_inside_block?(node)
        def_node_matcher :examples_inside_block?, <<~PATTERN
          (block !(send nil? #Hooks.all ...) _ #examples?)
        PATTERN

        # @!method examples_directly_or_in_block?(node)
        def_node_matcher :examples_directly_or_in_block?, <<~PATTERN
          {
            #example_or_group_or_include?
            #examples_inside_block?
          }
        PATTERN

        # @!method examples?(node)
        def_node_matcher :examples?, <<~PATTERN
          {
            #examples_directly_or_in_block?
            #examples_in_branches?
            (begin <#examples_directly_or_in_block? ...>)
            (begin <#examples_in_branches? ...>)
          }
        PATTERN

        def on_new_investigation
          RuboCop::RSpec::Language.config = config["RSpec"]["Language"]
          ranges = resolved_candidates
          return if ranges.nil? || ranges.empty?

          source = bundle_eligible? ? processed_source.raw_source : processed_source.buffer.source
          off = SourceOffsets.for(source)
          keys = ranges.map { |(start, fin)| [off[start], off[fin]] }
          located = Shirobai::RSpec::NodeLocator.locate(processed_source, keys)

          keys.each do |key|
            node = located[key]
            process_candidate(node) if node&.block_type?
          end
        end

        private

        def resolved_candidates
          if bundle_eligible?
            candidates = Dispatch.offenses_for(processed_source, config, :rspec_empty_example_group)
            return candidates unless candidates.nil?
          end
          Shirobai.check_rspec_empty_example_group(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end

        # Stock `on_block`, renamed so the Commissioner never dispatches it.
        def process_candidate(node)
          return if node.each_ancestor(:any_def).any?
          return if inside_example?(node)

          example_group_body(node) do |body|
            next unless offensive?(body)

            add_offense(node.send_node) do |corrector|
              corrector.remove(removed_range(node))
            end
          end
        end

        # --- stock private methods, copied verbatim (rubocop-rspec 3.10.2) ---

        def offensive?(body)
          return true unless body
          return false if conditionals_with_examples?(body)

          if body.type?(:if, :case)
            !examples_in_branches?(body)
          else
            !examples?(body)
          end
        end

        def conditionals_with_examples?(body)
          return false unless body.type?(:begin, :case)

          body.each_descendant(:if, :case).any? do |condition_node|
            examples_in_branches?(condition_node)
          end
        end

        def examples_in_branches?(condition_node)
          return false unless condition_node
          return false unless condition_node.type?(:if, :case)

          condition_node.branches.any? { |branch| examples?(branch) }
        end

        def removed_range(node)
          range_by_whole_lines(
            node.source_range,
            include_final_newline: true
          )
        end
      end
    end
  end
end
