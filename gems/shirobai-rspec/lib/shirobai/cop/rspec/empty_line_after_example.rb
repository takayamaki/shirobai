# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/EmptyLineAfterExample`
      # (rubocop-rspec 3.10.2).
      #
      # The Rust rule flags every plain-block example (`it`/`specify`/`its`/...)
      # that has a following sibling in a `:begin` sequence, minus the allowed
      # consecutive-one-liner shape (`AllowConsecutiveOneLiners`, default true).
      # The shared support module replays stock's comment/blank/directive walk
      # and autocorrect. Probed quirks live in
      # empty_line_after_example_edge_cases_spec.rb.
      class EmptyLineAfterExample < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector
        include Shirobai::Cop::BundleEligible
        include Shirobai::Cop::RSpec::EmptyLineSeparationSupport

        MSG = "Add an empty line after `%<example>s`."

        def self.cop_name = "RSpec/EmptyLineAfterExample"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # `AllowConsecutiveOneLiners` (default true) -> segment num.
        def self.bundle_args(config)
          [config.for_badge(badge).fetch("AllowConsecutiveOneLiners", true) ? 1 : 0]
        end

        def on_new_investigation
          emit_empty_line_offenses(resolved_offenses) do |method|
            format(MSG, example: method)
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            offenses = Dispatch.offenses_for(processed_source, config, :rspec_empty_line_after_example)
            return offenses unless offenses.nil?
          end
          Shirobai.check_rspec_empty_line_after_example(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end
      end
    end
  end
end
