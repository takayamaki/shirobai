# frozen_string_literal: true

module Shirobai
  module Cop
    module Metrics
      # Drop-in Rust reimplementation of `Metrics/AbcSize`.
      #
      # Rust returns the per-method `<assignment, branch, condition>` counts for
      # every method whose squared vector exceeds the configured `Max.floor`
      # (`bundle_args` packs `Max.floor`; a non-natural `Max` packs `-1` so every
      # method is returned and the exact `score > Max` filter below stays the
      # behavioral source of truth). The float score, the vector string and the
      # message are derived here so floats never cross the FFI boundary.
      class AbcSize < RuboCop::Cop::Base
        include RuboCop::Cop::AllowedMethods
        include RuboCop::Cop::AllowedPattern
        extend RuboCop::ExcludeLimit

        MSG = "Assignment Branch Condition size for `%<method>s` is too high. " \
              "[%<abc_vector>s %<complexity>.4g/%<max>.4g]"

        exclude_limit "Max"

        def self.cop_name = "Metrics/AbcSize"
        def self.badge = RuboCop::Cop::Badge.parse("Metrics/AbcSize")

        # Packed args for the bundled run: `[max_floor, discount_repeated]`.
        # `max_floor` is the cop's `Max.floor` (conservative prefilter: a float
        # `Max` floors down, an integer one is exact); a non-natural `Max`
        # disables the prefilter with `-1`. `discount_repeated` is
        # `!CountRepeatedAttributes` (default `CountRepeatedAttributes: true`).
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          max = cop_config["Max"]
          max_floor = max.is_a?(Numeric) && max >= 0 ? max.floor : -1
          discount = cop_config["CountRepeatedAttributes"] == false
          [max_floor, discount ? 1 : 0]
        end

        def on_new_investigation
          max = cop_config["Max"]

          analysis = Dispatch.offenses_for(processed_source, config, :abc_size)
          off = SourceOffsets.for(processed_source.raw_source)
          analysis.each do |start, fin, head_end, name, assignment, branch, condition|
            next if allowed_method?(name) || matches_allowed_pattern?(name)

            complexity = Math.sqrt((assignment**2) + (branch**2) + (condition**2)).round(2)
            next unless complexity > max

            abc_vector = "<#{assignment}, #{branch}, #{condition}>"
            stop = RuboCop::LSP.enabled? ? head_end : fin
            range = Parser::Source::Range.new(processed_source.buffer, off[start], off[stop])
            message = format(MSG, method: name, complexity: complexity, abc_vector: abc_vector, max: max)
            add_offense(range, message: message) { self.max = complexity.ceil }
          end
        end
      end
    end
  end
end
