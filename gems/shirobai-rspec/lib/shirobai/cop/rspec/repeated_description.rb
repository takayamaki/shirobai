# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/RepeatedDescription`
      # (rubocop-rspec 3.10.2).
      #
      # Byte-level signature comparison is impossible here: stock groups
      # examples by parser-node STRUCTURAL equality (`it 'a'` and `it "a"` are
      # equal; two `dstr` doc strings with the same interpolation are equal).
      # So the split is: Rust collects each example group's `examples` on the
      # shared walk (exact stock scope semantics) and puts the example BLOCK
      # node ranges of every group with >= 2 examples on the wire; the wrapper
      # relocates those parser nodes, wraps them in the stock
      # `RuboCop::RSpec::Example`, and runs stock's grouping VERBATIM. That
      # gives parity by construction for the equality-sensitive part.
      #
      # `repeated_descriptions` (non-`its`) groups by `[metadata, doc_string]`
      # and reports each `definition` (the send node); `repeated_its` groups by
      # `[doc_string, example]` and reports the whole node. Both arms keep only
      # groups where the signature is truthy (`Array#any?`) and larger than one.
      class RepeatedDescription < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible

        MSG = "Don't repeat descriptions within an example group."

        def self.cop_name = "RSpec/RepeatedDescription"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less (the segment's role lists cover everything).
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          groups = resolved_groups
          return if groups.empty?

          source = bundle_eligible? ? processed_source.raw_source : processed_source.buffer.source
          off = SourceOffsets.for(source)
          located = locate_blocks(groups, off)

          groups.each do |ranges|
            examples = examples_for(ranges, off, located)
            next if examples.length < 2

            repeated_descriptions(examples).each { |description| add_offense(description) }
            repeated_its(examples).each { |its| add_offense(its) }
          end
        end

        private

        # Bundle path only when raw_source and the parser buffer agree byte for
        # byte (CRLF/BOM break that — see Shirobai::Cop::BundleEligible); a
        # gated-off file (nil from Dispatch) also falls back. The fallback scans
        # `buffer.source` so every offset lines up with parser-gem's index.
        def resolved_groups
          if bundle_eligible?
            groups = Dispatch.offenses_for(processed_source, config, :rspec_repeated_description)
            return groups unless groups.nil?
          end
          Shirobai.check_rspec_repeated_description(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end

        # Relocate the example block nodes and wrap them in the stock Example.
        # A locate miss (a prism/parser range disagreement we have not met yet)
        # drops that example defensively rather than crashing.
        def examples_for(ranges, off, located)
          ranges.filter_map do |(start, fin)|
            node = located[[off[start], off[fin]]]
            node && RuboCop::RSpec::Example.new(node)
          end
        end

        # One AST descent for every group's example block ranges (converted to
        # char offsets). Returns the `[begin, end] => node` map.
        def locate_blocks(groups, off)
          char_ranges = groups.flat_map do |ranges|
            ranges.map { |(start, fin)| [off[start], off[fin]] }
          end
          return {} if char_ranges.empty?

          Shirobai::RSpec::NodeLocator.locate(processed_source, char_ranges)
        end

        # --- stock's private methods, copied verbatim (rubocop-rspec 3.10.2),
        # adapted to take the pre-collected Example list instead of a group node.

        def repeated_descriptions(examples)
          grouped_examples =
            examples
              .reject { |n| n.definition.method?(:its) }
              .group_by { |example| example_signature(example) }

          grouped_examples
            .select { |signatures, group| signatures.any? && group.size > 1 }
            .values
            .flatten
            .map(&:definition)
        end

        def repeated_its(examples)
          grouped_its =
            examples
              .select { |n| n.definition.method?(:its) }
              .group_by { |example| its_signature(example) }

          grouped_its
            .select { |signatures, group| signatures.any? && group.size > 1 }
            .values
            .flatten
            .map(&:to_node)
        end

        def example_signature(example)
          [example.metadata, example.doc_string]
        end

        def its_signature(example)
          [example.doc_string, example]
        end
      end
    end
  end
end
