# frozen_string_literal: true

module Shirobai
  module Cop
    module Metrics
      # Drop-in Rust reimplementation of `Metrics/BlockLength`.
      #
      # Rust parses the source, walks blocks, counts body lines (with comment and
      # `CountAsOne` handling) and excludes class constructors. With no
      # `AllowedPatterns` configured (the default) the `AllowedMethods`
      # exclusion also runs in Rust and only the offending blocks come back;
      # otherwise Ruby applies the `AllowedMethods` / `AllowedPatterns` filters
      # as before. Ruby registers the offenses.
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
          # Fast path: without AllowedPatterns (the default), the AllowedMethods
          # exclusion runs in Rust and only offending blocks come back. The
          # filters are independent per-block predicates, but we still gate on
          # the patterns being empty so any pattern config keeps the exact
          # legacy path.
          fast = allowed_patterns.empty?
          candidates = Shirobai.check_block_length(
            source, max_length, count_comments?, count_as_one,
            fast ? allowed_method_strings : [], fast
          )
          candidates.each do |start, fin, head_end, length, method_name, receiver|
            unless fast
              next if allowed_method?(method_name) || matches_allowed_pattern?(method_name)
              next if method_receiver_excluded?(receiver, method_name)
            end

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
          @count_as_one ||= Array(cop_config["CountAsOne"]).map(&:to_s)
        end

        # The String entries of AllowedMethods, for the Rust-side exclusion.
        # Non-String entries are skipped by stock's `method_receiver_excluded?`
        # (`next unless config.is_a?(String)`) and can never equal a String
        # method name in `allowed_method?`, so dropping them is faithful.
        def allowed_method_strings
          @allowed_method_strings ||= allowed_methods.grep(String)
        end

        # Mirror the lazy `RuboCop::Warning` the calculator raises for an unknown
        # `CountAsOne` type once a block is actually counted. The unknown set is
        # config-derived, so compute it once per instance.
        def validate_count_as_one!
          @unknown_count_as_one ||= count_as_one - FOLDABLE_TYPES
          return if @unknown_count_as_one.empty?

          raise RuboCop::Warning,
                "Unknown foldable type: #{@unknown_count_as_one.first.to_sym.inspect}. " \
                "Valid foldable types are: #{FOLDABLE_TYPES.join(', ')}."
        end
      end
    end
  end
end
