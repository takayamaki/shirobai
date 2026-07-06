# frozen_string_literal: true

module Shirobai
  module Cop
    module Rails
      # Drop-in Rust reimplementation of `Rails/DynamicFindBy`
      # (rubocop-rails 2.35.5).
      #
      # Rust replicates the `on_send` / `on_csend` detection (the
      # `/^find_by_(.+?)(!)?$/` name match, the argument count / no-splat /
      # no-hash rules, the `AllowedMethods` / `AllowedReceivers` / `Whitelist`
      # suppressions, and the receiverless-inside-ActiveRecord class check) and
      # the full autocorrect: replace the selector with `find_by` / `find_by!`
      # and insert each `col: ` keyword before its argument. The wrapper owns
      # only the fixed `MSG` (the method name comes from the selector range).
      class DynamicFindBy < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector

        MSG = "Use `%<static_name>s` instead of dynamic `%<method>s`."

        def self.cop_name = "Rails/DynamicFindBy"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Contributes `[nums, lists]` pieces to the rails segment:
        # `[[], [AllowedMethods, AllowedReceivers, Whitelist]]`.
        def self.bundle_args(config)
          cop = config.for_badge(badge)
          [[], [cop["AllowedMethods"] || [], cop["AllowedReceivers"] || [], cop["Whitelist"] || []]]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start_o, end_o, static_name, sel_s, sel_e, inserts|
            range = Parser::Source::Range.new(buffer, off[start_o], off[end_o])
            sel_range = Parser::Source::Range.new(buffer, off[sel_s], off[sel_e])
            message = format(MSG, static_name: static_name, method: sel_range.source)
            add_offense(range, message: message) do |corrector|
              corrector.replace(sel_range, static_name)
              inserts.each do |arg_start, keyword|
                pos = off[arg_start]
                corrector.insert_before(Parser::Source::Range.new(buffer, pos, pos), keyword)
              end
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :rails_dynamic_find_by)
          else
            Shirobai.check_rails_dynamic_find_by(
              processed_source.buffer.source,
              cop_config["AllowedMethods"] || [], cop_config["AllowedReceivers"] || [],
              cop_config["Whitelist"] || []
            )
          end
        end
      end
    end
  end
end
