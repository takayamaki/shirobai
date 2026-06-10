# frozen_string_literal: true

module Shirobai
  module Cop
    module Metrics
      # Drop-in Rust reimplementation of `Metrics/BlockLength`.
      #
      # Rust parses the source, walks blocks, counts body lines (with comment and
      # `CountAsOne` handling) and excludes class constructors. Ruby applies the
      # cheap, config-driven `AllowedMethods` / `AllowedPatterns` filters and
      # registers offenses.
      class BlockLength < RuboCop::Cop::Base
        include RuboCop::Cop::AllowedMethods
        include RuboCop::Cop::AllowedPattern
        extend RuboCop::ExcludeLimit

        LABEL = "Block"
        MSG = "%<label>s has too many lines. [%<length>d/%<max>d]"

        exclude_limit "Max"

        def self.cop_name = "Metrics/BlockLength"
        def self.badge = RuboCop::Cop::Badge.parse("Metrics/BlockLength")

        def on_new_investigation
          source = processed_source.raw_source
          Shirobai.check_block_length(source, max_length, count_comments?).each do |start, fin, length, _method_name, _receiver|
            range = Parser::Source::Range.new(processed_source.buffer, start, fin)
            add_offense(range, message: format(MSG, label: LABEL, length: length, max: max_length)) do
              self.max = length
            end
          end
        end

        private

        def max_length
          cop_config["Max"]
        end

        def count_comments?
          cop_config["CountComments"]
        end
      end
    end
  end
end
