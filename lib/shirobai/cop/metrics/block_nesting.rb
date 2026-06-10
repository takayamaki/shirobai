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
      class BlockNesting < RuboCop::Cop::Base
        extend RuboCop::ExcludeLimit

        exclude_limit "Max"

        def self.cop_name = "Metrics/BlockNesting"
        def self.badge = RuboCop::Cop::Badge.parse("Metrics/BlockNesting")

        def on_new_investigation
          return if processed_source.blank?

          max = cop_config["Max"]
          offenses, deepest = Shirobai.check_block_nesting(
            processed_source.raw_source, max, count_blocks?, count_modifier_forms?
          )
          return if offenses.empty?

          self.max = deepest
          message = "Avoid more than #{max} levels of block nesting."
          offenses.each do |start, fin|
            range = Parser::Source::Range.new(processed_source.buffer, start, fin)
            add_offense(range, message: message)
          end
        end

        private

        def count_blocks?
          cop_config.fetch("CountBlocks", false)
        end

        def count_modifier_forms?
          cop_config.fetch("CountModifierForms", false)
        end
      end
    end
  end
end
