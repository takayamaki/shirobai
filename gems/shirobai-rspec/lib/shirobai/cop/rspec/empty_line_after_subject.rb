# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/EmptyLineAfterSubject`
      # (rubocop-rspec 3.10.2).
      #
      # Flags every plain-block `subject`/`subject!` that is inside an example
      # group (its outermost enclosing top-level statement is a spec group) and
      # has a following sibling in a `:begin` sequence. Config-less.
      class EmptyLineAfterSubject < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector
        include Shirobai::Cop::BundleEligible
        include Shirobai::Cop::RSpec::EmptyLineSeparationSupport

        MSG = "Add an empty line after `%<subject>s`."

        def self.cop_name = "RSpec/EmptyLineAfterSubject"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          emit_empty_line_offenses(resolved_offenses) do |method|
            format(MSG, subject: method)
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            offenses = Dispatch.offenses_for(processed_source, config, :rspec_empty_line_after_subject)
            return offenses unless offenses.nil?
          end
          Shirobai.check_rspec_empty_line_after_subject(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end
      end
    end
  end
end
