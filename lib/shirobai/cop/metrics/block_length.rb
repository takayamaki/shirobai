# frozen_string_literal: true

module Shirobai
  module Cop
    module Metrics
      # Drop-in Rust reimplementation of `Metrics/BlockLength`.
      #
      # Rust parses the source, walks blocks, counts body lines (with comment and
      # `CountAsOne` handling) and excludes class constructors. With no
      # `AllowedPatterns` configured (the default) the `AllowedMethods`
      # exclusion also runs in Rust, only the offending blocks come back, and
      # the offenses are pulled from the per-file bundled run
      # (`Shirobai::Dispatch`); otherwise the legacy standalone call is made and
      # Ruby applies the `AllowedMethods` / `AllowedPatterns` filters as before.
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

        # Packed args for the bundled (fast-path) run:
        # `[max, count_comments, count_as_one, allowed_method_strings]`.
        # `Max` defaults to 25 (default.yml) so a config that does not mention
        # this cop (vendor specs of the other bundled cops) still packs cleanly;
        # the computed slice is discarded in that case. The String filtering of
        # `AllowedMethods` mirrors the fast path: non-String entries are skipped
        # by stock's `method_receiver_excluded?` (`next unless
        # config.is_a?(String)`) and can never equal a String method name in
        # `allowed_method?`, so dropping them is faithful.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            cop_config["Max"] || 25,
            !!cop_config["CountComments"],
            Array(cop_config["CountAsOne"]).map(&:to_s),
            Cop.allowed_methods_config(cop_config).grep(String)
          ]
        end

        def on_new_investigation
          # Fast path: without AllowedPatterns (the default), the AllowedMethods
          # exclusion runs in Rust and only offending blocks come back. The
          # filters are independent per-block predicates, but we still gate on
          # the patterns being empty so any pattern config keeps the exact
          # legacy path.
          fast = bundle_eligible?
          candidates =
            if fast
              Dispatch.offenses_for(processed_source, config, :block_length)
            else
              Shirobai.check_block_length(
                processed_source.raw_source, max_length, count_comments?, count_as_one, [], false
              )
            end
          off = SourceOffsets.for(processed_source.raw_source)
          candidates.each do |start, fin, head_end, length, method_name, receiver|
            unless fast
              next if allowed_method?(method_name) || matches_allowed_pattern?(method_name)
              next if method_receiver_excluded?(receiver, method_name)
            end

            validate_count_as_one!

            stop = RuboCop::LSP.enabled? ? head_end : fin
            range = Parser::Source::Range.new(processed_source.buffer, off[start], off[stop])
            add_offense(range, message: format(MSG, label: LABEL, length: length, max: max_length)) do
              self.max = length
            end
          end
        end

        private

        # The bundle always applies the Rust-side fast path, which is only
        # faithful while no `AllowedPatterns` are configured.
        def bundle_eligible?
          allowed_patterns.empty?
        end

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

        # Config-derived and stable for the life of the instance; shares the
        # derivation with the bundled run (single source of truth).
        def bundle_args
          @bundle_args ||= self.class.bundle_args(config)
        end

        def max_length
          bundle_args[0]
        end

        def count_comments?
          bundle_args[1]
        end

        def count_as_one
          bundle_args[2]
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
