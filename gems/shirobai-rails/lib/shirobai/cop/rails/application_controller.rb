# frozen_string_literal: true

module Shirobai
  module Cop
    module Rails
      # Drop-in Rust reimplementation of `Rails/ApplicationController`
      # (rubocop-rails 2.35.5). See `ApplicationRecord` for the shared
      # `EnforceSuperclass` semantics and offense/autocorrect ranges.
      #
      # Unlike Record / Mailer / Job this cop has NO `TargetRailsVersion`
      # gate — stock does not declare one, so it runs on every Rails version.
      class ApplicationController < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector

        MSG = "Controllers should subclass `ApplicationController`."
        SUPERCLASS = "ApplicationController"

        def self.cop_name = "Rails/ApplicationController"
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
            Dispatch.offenses_for(processed_source, config, :rails_application_controller)
          else
            Shirobai.check_rails_application_controller(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
