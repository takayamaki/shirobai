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
        FOLDABLE_TYPES = %w[array hash heredoc method_call].freeze

        exclude_limit "Max"

        def self.cop_name = "Metrics/BlockLength"
        def self.badge = RuboCop::Cop::Badge.parse("Metrics/BlockLength")

        def on_new_investigation
          source = processed_source.raw_source
          Shirobai.check_block_length(source, max_length, count_comments?, count_as_one).each do |start, fin, head_end, length, method_name, receiver|
            next if allowed_method?(method_name) || matches_allowed_pattern?(method_name)
            next if method_receiver_excluded?(receiver, method_name)

            validate_count_as_one!

            stop = RuboCop::LSP.enabled? ? head_end : fin
            range = Parser::Source::Range.new(processed_source.buffer, start, stop)
            add_offense(range, message: format(MSG, label: LABEL, length: length, max: max_length)) do
              self.max = length
            end
          end
        end

        private

        # Port of RuboCop's `method_receiver_excluded?`, operating on the method
        # name and raw receiver source that Rust hands back instead of a node.
        def method_receiver_excluded?(node_receiver, node_method)
          node_receiver = node_receiver.empty? ? nil : node_receiver.gsub(/\s+/, "")

          allowed_methods.any? do |config|
            next unless config.is_a?(String)

            receiver, method = config.split(".")

            unless method
              method = receiver
              receiver = node_receiver
            end

            method == node_method && receiver == node_receiver
          end
        end

        def max_length
          cop_config["Max"]
        end

        def count_comments?
          cop_config["CountComments"]
        end

        def count_as_one
          Array(cop_config["CountAsOne"]).map(&:to_s)
        end

        # Mirror the lazy `RuboCop::Warning` the calculator raises for an unknown
        # `CountAsOne` type once a block is actually counted.
        def validate_count_as_one!
          unknown = count_as_one - FOLDABLE_TYPES
          return if unknown.empty?

          raise RuboCop::Warning,
                "Unknown foldable type: #{unknown.first.to_sym.inspect}. " \
                "Valid foldable types are: #{FOLDABLE_TYPES.join(', ')}."
        end
      end
    end
  end
end
