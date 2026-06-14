# frozen_string_literal: true

module Shirobai
  module Cop
    module Metrics
      # Drop-in Rust reimplementation of `Metrics/MethodLength`.
      #
      # Rust parses the source, finds every method definition (`def` / `def self.`
      # / `define_method` blocks, incl. numbered/`it` blocks), measures the body
      # length with the shared `CodeLength` calculator (comment and `CountAsOne`
      # handling shared with `Metrics/BlockLength`) and returns those exceeding
      # `Max`. `AllowedMethods` / `AllowedPatterns` filtering stays on the Ruby
      # side, which has the exact symbol/regexp semantics; Rust marks each
      # candidate `filterable` (false for a `define_method` whose name argument
      # is not a basic literal, which stock never filters).
      class MethodLength < RuboCop::Cop::Base
        include RuboCop::Cop::AllowedMethods
        include RuboCop::Cop::AllowedPattern
        extend RuboCop::ExcludeLimit

        LABEL = "Method"
        MSG = "%<label>s has too many lines. [%<length>d/%<max>d]"

        exclude_limit "Max"

        def self.cop_name = "Metrics/MethodLength"
        def self.badge = RuboCop::Cop::Badge.parse("Metrics/MethodLength")

        # Packed args for the bundled run: `[max, count_comments, count_as_one]`.
        # `Max` defaults to 10 (default.yml) so a config that does not mention
        # this cop still packs cleanly; the computed slice is discarded in that
        # case.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            cop_config["Max"] || 10,
            !!cop_config["CountComments"],
            Array(cop_config["CountAsOne"]).map(&:to_s)
          ]
        end

        def on_new_investigation
          candidates = Dispatch.offenses_for(processed_source, config, :method_length)
          off = SourceOffsets.for(processed_source.raw_source)
          candidates.each do |start, fin, head_end, length, name, filterable|
            next if filterable && (allowed_method?(name) || matches_allowed_pattern?(name))

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
      end
    end
  end
end
