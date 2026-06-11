# frozen_string_literal: true

module Shirobai
  module Cop
    module Metrics
      # Drop-in Rust reimplementation of `Metrics/BlockNesting`.
      #
      # Rust walks the AST tracking the running nesting level, applies the
      # `CountBlocks` / `CountModifierForms` flags and returns the reportable
      # offense ranges plus the deepest level that exceeded `Max`. Ruby keeps the
      # `ExcludeLimit` bookkeeping (`self.max =`) and registers the offenses.
      # Offenses come from the per-file bundled run (`Shirobai::Dispatch`); the
      # config derivation is purely config-driven, so this cop is always
      # bundle-eligible.
      class BlockNesting < RuboCop::Cop::Base
        extend RuboCop::ExcludeLimit

        exclude_limit "Max"

        def self.cop_name = "Metrics/BlockNesting"
        def self.badge = RuboCop::Cop::Badge.parse("Metrics/BlockNesting")

        # Packed args for the bundled run:
        # `[max, count_blocks, count_modifier_forms]`. `Max` defaults to 3
        # (default.yml) so a config that does not mention this cop still packs
        # cleanly; the computed slice is discarded in that case.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            cop_config["Max"] || 3,
            !!cop_config.fetch("CountBlocks", false),
            !!cop_config.fetch("CountModifierForms", false)
          ]
        end

        def on_new_investigation
          return if processed_source.blank?

          max = bundle_args[0]
          offenses, deepest = Dispatch.offenses_for(processed_source, config, :block_nesting)
          return if offenses.empty?

          self.max = deepest
          message = "Avoid more than #{max} levels of block nesting."
          offenses.each do |start, fin|
            range = Parser::Source::Range.new(processed_source.buffer, start, fin)
            add_offense(range, message: message)
          end
        end

        private

        # Config-derived and stable for the life of the instance; shares the
        # derivation with the bundled run (single source of truth).
        def bundle_args
          @bundle_args ||= self.class.bundle_args(config)
        end
      end
    end
  end
end
