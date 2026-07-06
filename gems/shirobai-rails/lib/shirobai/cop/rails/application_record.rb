# frozen_string_literal: true

module Shirobai
  module Cop
    module Rails
      # Drop-in Rust reimplementation of `Rails/ApplicationRecord`
      # (rubocop-rails 2.35.5).
      #
      # Rust replicates stock's `EnforceSuperclass` mixin: `class X <
      # ActiveRecord::Base` (unless `X`'s terminal name is `ApplicationRecord`)
      # and `Class.new(ActiveRecord::Base)` (unless it is the direct value of
      # an `ApplicationRecord =` constant write, covering the `do..end` / `{}`
      # block forms). The offense range is the superclass / argument const node
      # (leading `::` included); the wrapper replaces it with the bare
      # `ApplicationRecord` name — byte for byte with stock's autocorrect.
      #
      # `TargetRailsVersion`: like stock this cop is gated on
      # `requires_gem('railties', '>= 5.0')`, so it stays silent on Rails <
      # 5.0 or without railties in the target bundle. The `Exclude:
      # db/**/*.rb` from the merged default.yml is resolved by RuboCop through
      # this wrapper's badge, exactly as for the stock cop.
      class ApplicationRecord < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector
        extend RuboCop::Cop::TargetRailsVersion

        minimum_target_rails_version 5.0

        MSG = "Models should subclass `ApplicationRecord`."
        SUPERCLASS = "ApplicationRecord"

        def self.cop_name = "Rails/ApplicationRecord"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # No behavioral config: the segment is a wake-up flag only, so
        # `bundle_args` contributes nothing.
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
            Dispatch.offenses_for(processed_source, config, :rails_application_record)
          else
            Shirobai.check_rails_application_record(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
