# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/EmptyLineAfterFinalLet`
      # (rubocop-rspec 3.10.2).
      #
      # For every plain-block example/shared group or `include_*` block, the
      # Rust rule finds the last `let?` among its direct body children (block
      # form or the `let(:x, &blk)` send form) and flags it when it is not the
      # body's last child. Config-less.
      class EmptyLineAfterFinalLet < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector
        include Shirobai::Cop::BundleEligible
        include Shirobai::Cop::RSpec::EmptyLineSeparationSupport

        MSG = "Add an empty line after the last `%<let>s`."

        def self.cop_name = "RSpec/EmptyLineAfterFinalLet"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          emit_empty_line_offenses(resolved_offenses) do |method|
            format(MSG, let: method)
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            offenses = Dispatch.offenses_for(processed_source, config, :rspec_empty_line_after_final_let)
            return offenses unless offenses.nil?
          end
          Shirobai.check_rspec_empty_line_after_final_let(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end
      end
    end
  end
end
