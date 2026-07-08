# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/MultipleSubjects`
      # (rubocop-rspec 3.10.2).
      #
      # Rust does the scope-tree work on the shared walk: for every plain-block
      # example group it collects the `subject?` definitions directly in the
      # group's scope (stock's `ExampleGroup#subjects` /
      # `find_all_in_scope(:subject?)` barrier semantics) and, when there is
      # more than one, emits the BLOCK ranges of all but the last definition
      # (stock's `subjects[0...-1]`). This wrapper relocates each parser block
      # node and runs stock's `add_offense` + autocorrect VERBATIM: the rename
      # (`subject(:x)` -> `let(:x)`), the removal (unnamed `subject`), and the
      # `subject!` non-correctable case all decide on the real parser node, so
      # the offense range and correction match byte for byte.
      class MultipleSubjects < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        include RuboCop::Cop::RangeHelp
        include Shirobai::Cop::BundleEligible

        MSG = "Do not set more than one subject per example group"

        def self.cop_name = "RSpec/MultipleSubjects"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          ranges = resolved_candidates
          return if ranges.nil? || ranges.empty?

          source = bundle_eligible? ? processed_source.raw_source : processed_source.buffer.source
          off = SourceOffsets.for(source)
          keys = ranges.map { |(start, fin)| [off[start], off[fin]] }
          located = Shirobai::RSpec::NodeLocator.locate(processed_source, keys)

          keys.each do |key|
            node = located[key]
            next unless node&.block_type?

            add_offense(node) do |corrector|
              autocorrect(corrector, node)
            end
          end
        end

        private

        def resolved_candidates
          if bundle_eligible?
            ranges = Dispatch.offenses_for(processed_source, config, :rspec_multiple_subjects)
            return ranges unless ranges.nil?
          end
          Shirobai.check_rspec_multiple_subjects(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end

        # --- stock's private autocorrect methods, copied verbatim
        # (rubocop-rspec 3.10.2). ---

        def autocorrect(corrector, subject)
          return unless subject.method_name.equal?(:subject) # Ignore `subject!`

          if named_subject?(subject)
            rename_autocorrect(corrector, subject)
          else
            remove_autocorrect(corrector, subject)
          end
        end

        def named_subject?(node)
          node.send_node.arguments?
        end

        def rename_autocorrect(corrector, node)
          corrector.replace(node.send_node.loc.selector, "let")
        end

        def remove_autocorrect(corrector, node)
          range = range_by_whole_lines(node.source_range,
                                       include_final_newline: true)
          corrector.remove(range)
        end
      end
    end
  end
end
