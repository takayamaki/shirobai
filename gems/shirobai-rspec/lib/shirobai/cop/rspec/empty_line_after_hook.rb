# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/EmptyLineAfterHook`
      # (rubocop-rspec 3.10.2).
      #
      # Flags every hook (`before`/`after`/`around`/... in ANY block form —
      # plain, numbered, `it`-param) that has a following sibling in a `:begin`
      # sequence, minus the allowed consecutive-one-liner hook chain
      # (`AllowConsecutiveOneLiners`, default true).
      class EmptyLineAfterHook < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector
        include Shirobai::Cop::BundleEligible
        include Shirobai::Cop::RSpec::EmptyLineSeparationSupport

        MSG = "Add an empty line after `%<hook>s`."

        def self.cop_name = "RSpec/EmptyLineAfterHook"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # `AllowConsecutiveOneLiners` (default true) -> segment num.
        def self.bundle_args(config)
          [config.for_badge(badge).fetch("AllowConsecutiveOneLiners", true) ? 1 : 0]
        end

        def on_new_investigation
          emit_empty_line_offenses(resolved_offenses) do |method|
            format(MSG, hook: method)
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            offenses = Dispatch.offenses_for(processed_source, config, :rspec_empty_line_after_hook)
            return offenses unless offenses.nil?
          end
          Shirobai.check_rspec_empty_line_after_hook(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end
      end
    end
  end
end
