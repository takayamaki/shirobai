# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/EmptyLineAfterExampleGroup`
      # (rubocop-rspec 3.10.2).
      #
      # Flags every plain-block spec group (`describe`/`context`/... AND
      # `shared_examples`/`shared_context` with an RSpec receiver) that has a
      # following sibling in a `:begin` sequence. Config-less.
      class EmptyLineAfterExampleGroup < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector
        include Shirobai::Cop::BundleEligible
        include Shirobai::Cop::RSpec::EmptyLineSeparationSupport

        MSG = "Add an empty line after `%<example_group>s`."

        def self.cop_name = "RSpec/EmptyLineAfterExampleGroup"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          emit_empty_line_offenses(resolved_offenses) do |method|
            format(MSG, example_group: method)
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            offenses = Dispatch.offenses_for(processed_source, config, :rspec_empty_line_after_example_group)
            return offenses unless offenses.nil?
          end
          Shirobai.check_rspec_empty_line_after_example_group(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end
      end
    end
  end
end
