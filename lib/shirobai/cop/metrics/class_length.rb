# frozen_string_literal: true

module Shirobai
  module Cop
    module Metrics
      # Drop-in Rust reimplementation of `Metrics/ClassLength`.
      #
      # Rust parses the source, finds every class definition (`class` nodes,
      # `class << self` outside a class, and `Class.new` / `Struct.new` blocks
      # assigned to a constant), measures each with the shared `CodeLength`
      # calculator's classlike / body paths and returns those exceeding `Max`.
      # There is no autocorrect; the wrapper only builds ranges and messages.
      class ClassLength < RuboCop::Cop::Base
        extend RuboCop::ExcludeLimit

        LABEL = "Class"
        MSG = "%<label>s has too many lines. [%<length>d/%<max>d]"
        FOLDABLE_TYPES = %w[array hash heredoc method_call].freeze

        exclude_limit "Max"

        def self.cop_name = "Metrics/ClassLength"
        def self.badge = RuboCop::Cop::Badge.parse("Metrics/ClassLength")

        # Packed args for the bundled run: `[max, count_comments, count_as_one]`.
        # `Max` defaults to 100 (default.yml) so a config that does not mention
        # this cop still packs cleanly; the computed slice is discarded in that
        # case.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            cop_config["Max"] || 100,
            !!cop_config["CountComments"],
            Array(cop_config["CountAsOne"]).map(&:to_s)
          ]
        end

        def on_new_investigation
          candidates = Dispatch.offenses_for(processed_source, config, :class_length)
          off = SourceOffsets.for(processed_source.raw_source)
          candidates.each do |start, fin, head_end, length, sclass|
            validate_count_as_one!

            # Stock's LSP location reads `node.loc.begin` for a
            # `class << self`, which `Parser::Source::Map::Definition` does
            # not define — stock raises (a swallowed cop error) and reports
            # nothing, so report nothing here too.
            next if sclass && RuboCop::LSP.enabled?

            stop = RuboCop::LSP.enabled? ? head_end : fin
            range = Parser::Source::Range.new(processed_source.buffer, off[start], off[stop])
            add_offense(range, message: format(MSG, label: LABEL, length: length, max: max_length)) do
              self.max = length
            end
          end
        end

        private

        # Config-derived and stable for the life of the instance; shares the
        # derivation with the bundled run (single source of truth).
        def bundle_args
          @bundle_args ||= self.class.bundle_args(config)
        end

        def max_length
          bundle_args[0]
        end

        def count_as_one
          bundle_args[2]
        end

        # Mirror the lazy `RuboCop::Warning` the calculator raises for an unknown
        # `CountAsOne` type once a class is actually counted. The unknown set is
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
