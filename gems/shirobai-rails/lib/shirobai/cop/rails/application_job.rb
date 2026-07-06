# frozen_string_literal: true

module Shirobai
  module Cop
    module Rails
      # Drop-in Rust reimplementation of `Rails/ApplicationJob`
      # (rubocop-rails 2.35.5). See `ApplicationRecord` for the shared
      # `EnforceSuperclass` semantics and offense/autocorrect ranges. Gated on
      # `requires_gem('railties', '>= 5.0')` like stock.
      #
      # (Stock defines an explicit `autocorrect(node)` returning
      # `corrector.replace(node, SUPERCLASS)`; the other three cops reach the
      # same replacement through the `AutoCorrector` block. Both collapse to
      # "replace the offense range with the superclass name", which is what
      # the shared wrapper corrector does here.)
      class ApplicationJob < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector
        extend RuboCop::Cop::TargetRailsVersion

        minimum_target_rails_version 5.0

        MSG = "Jobs should subclass `ApplicationJob`."
        SUPERCLASS = "ApplicationJob"

        def self.cop_name = "Rails/ApplicationJob"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start_offset, end_offset|
            range = Parser::Source::Range.new(buffer, off[start_offset], off[end_offset])
            add_offense(range, message: MSG) do |corrector|
              corrector.replace(range, self.class::SUPERCLASS)
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :rails_application_job)
          else
            Shirobai.check_rails_application_job(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
