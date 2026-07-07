# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/ScatteredSetup`
      # (rubocop-rspec 3.10.2).
      #
      # Architecture B: Rust identifies example-group blocks on the shared
      # walk; the Ruby wrapper locates the parser node via NodeLocator and
      # runs stock's detection logic VERBATIM.
      #
      # Detection depends on `RuboCop::RSpec::ExampleGroup.new(node).hooks`
      # which walks the parser AST to enumerate hooks, filters by
      # `inside_class_method?`, `knowable_scope?`, and hook name. AC is
      # body-merge manipulation with heredoc-aware `final_end_location`. The
      # `RepeatedItems` grouping and the `Hook` wrapper's `[name, scope,
      # metadata]` key are parser-AST-shaped.
      class ScatteredSetup < RuboCop::Cop::Base
        include RuboCop::Cop::RSpec::FinalEndLocation
        include RuboCop::Cop::RangeHelp
        include RuboCop::Cop::RSpec::RepeatedItems
        extend RuboCop::Cop::AutoCorrector
        include RuboCop::RSpec::Language
        include Shirobai::Cop::BundleEligible

        MSG = 'Do not define multiple `%<hook_name>s` hooks in the same ' \
              'example group (also defined on %<lines>s).'

        def self.cop_name = "RSpec/ScatteredSetup"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less (the segment's role lists cover everything).
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          super
          RuboCop::RSpec::Language.config = config["RSpec"]["Language"]

          ranges = resolved_ranges
          return if ranges.empty?

          source = bundle_eligible? ? processed_source.raw_source : processed_source.buffer.source
          off = SourceOffsets.for(source)
          located = locate_blocks(ranges, off)

          ranges.each do |(start, fin)|
            node = located[[off[start], off[fin]]]
            next unless node

            process_candidate(node)
          end
        end

        private

        def resolved_ranges
          if bundle_eligible?
            result = Dispatch.offenses_for(processed_source, config, :rspec_scattered_setup)
            return result unless result.nil?
          end
          Shirobai.check_rspec_scattered_setup(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end

        def locate_blocks(ranges, off)
          char_ranges = ranges.map { |(start, fin)| [off[start], off[fin]] }
          return {} if char_ranges.empty?

          Shirobai::RSpec::NodeLocator.locate(processed_source, char_ranges)
        end

        # --- stock's on_block logic, renamed to process_candidate ---

        def process_candidate(node)
          return unless example_group?(node)

          repeated_hooks(node).each do |occurrences|
            occurrences.each do |occurrence|
              msg = message(occurrences, occurrence)
              add_offense(occurrence, message: msg) do |corrector|
                autocorrect(corrector, occurrences.first, occurrence)
              end
            end
          end
        end

        def repeated_hooks(node) # rubocop:disable Metrics/CyclomaticComplexity
          hooks = RuboCop::RSpec::ExampleGroup.new(node).hooks
            .reject(&:inside_class_method?)
            .select { |hook| hook.knowable_scope? && hook.name != :around }

          find_repeated_groups(
            hooks,
            key_proc: ->(hook) { [hook.name, hook.scope, hook.metadata] }
          ).map { |hook_group| hook_group.map(&:to_node) }
        end

        def lines_msg(numbers)
          if numbers.size == 1
            "line #{numbers.first}"
          else
            "lines #{numbers.join(', ')}"
          end
        end

        def message(occurrences, occurrence)
          lines = occurrences.map(&:first_line)
          lines_except_current = lines - [occurrence.first_line]
          format(MSG, hook_name: occurrences.first.method_name,
                      lines: lines_msg(lines_except_current))
        end

        def autocorrect(corrector, first_occurrence, occurrence)
          return if first_occurrence == occurrence || !first_occurrence.body

          # Take heredocs into account
          body = occurrence.body&.source_range&.with(
            end_pos: final_end_location(occurrence).begin_pos
          )

          corrector.insert_after(first_occurrence.body,
                                 "\n#{body&.source}")
          corrector.remove(range_by_whole_lines(occurrence.source_range,
                                                include_final_newline: true))
        end
      end
    end
  end
end
