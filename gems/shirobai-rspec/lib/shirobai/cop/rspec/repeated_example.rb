# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/RepeatedExample`
      # (rubocop-rspec 3.10.2).
      #
      # Same split as `RSpec/RepeatedDescription`: Rust collects each example
      # group's `examples` on the shared walk and puts the example BLOCK node
      # ranges of every group with >= 2 examples on the wire (identical data to
      # RepeatedDescription — each cop owns its slot). The wrapper relocates the
      # parser nodes, wraps them in the stock `RuboCop::RSpec::Example`, includes
      # stock's `RepeatedItems` mixin, and runs `find_repeated_groups` VERBATIM.
      # The signature is `[metadata, implementation]` (plus `definition.arguments`
      # for `its`), compared by parser-node structural equality — so quote-style
      # and paren differences are repeats while a `dstr` and a heredoc body are
      # not, all decided on real nodes rather than bytewise.
      class RepeatedExample < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::RSpec::RepeatedItems

        MSG = "Don't repeat examples within an example group. " \
              "Repeated on line(s) %<lines>s."

        def self.cop_name = "RSpec/RepeatedExample"
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

            find_repeated_examples(examples).each do |repeated_examples|
              add_offenses_for_repeated_group(repeated_examples)
            end
          end
        end

        private

        # Bundle path only when raw_source and the parser buffer agree byte for
        # byte (CRLF/BOM break that — see Shirobai::Cop::BundleEligible); a
        # gated-off file (nil from Dispatch) also falls back. The fallback scans
        # `buffer.source` so every offset lines up with parser-gem's index.
        def resolved_groups
          if bundle_eligible?
            groups = Dispatch.offenses_for(processed_source, config, :rspec_repeated_example)
            return groups unless groups.nil?
          end
          Shirobai.check_rspec_repeated_example(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end

        # Relocate the example block nodes and wrap them in the stock Example.
        # A locate miss drops that example defensively rather than crashing.
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

        def find_repeated_examples(examples)
          find_repeated_groups(
            examples,
            key_proc: ->(example) { build_example_signature(example) }
          )
        end

        def build_example_signature(example)
          signature = [example.metadata, example.implementation]
          if example.definition.method?(:its)
            signature << example.definition.arguments
          end
          signature
        end

        def add_offenses_for_repeated_group(repeated_examples)
          repeated_examples.each do |example|
            other_lines = extract_other_lines(repeated_examples, example)
            add_offense(example.to_node, message: message(other_lines))
          end
        end

        def extract_other_lines(examples_group, current_example)
          current_node = current_example.to_node

          examples_group
            .reject { |ex| ex.to_node.equal?(current_node) }
            .map { |ex| ex.to_node.first_line }
            .uniq
            .sort
        end

        def message(other_lines)
          format(MSG, lines: other_lines.join(", "))
        end
      end
    end
  end
end
