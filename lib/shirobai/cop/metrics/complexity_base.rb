# frozen_string_literal: true

require_relative "complexity_shared"

module Shirobai
  module Cop
    module Metrics
      # Shared offense-reporting logic for the two complexity cops. Each Rust
      # analysis entry carries both the cyclomatic and perceived score; the
      # including cop selects its metric via `#metric_score`. The analysis comes
      # from the per-file bundled run (`Shirobai::Dispatch`); the prefilter
      # thresholds (`ComplexityShared.bundle_args`) tolerate any `Max` value, so
      # both cops are always bundle-eligible.
      module ComplexityBase
        def on_new_investigation
          max = cop_config["Max"]

          analysis = Dispatch.offenses_for(processed_source, config, :complexity)
          off = SourceOffsets.for(processed_source.raw_source)
          analysis.each do |start, fin, head_end, name, cyclomatic, perceived|
            next if allowed_method?(name) || matches_allowed_pattern?(name)

            complexity = metric_score(cyclomatic, perceived)
            next unless complexity > max

            stop = RuboCop::LSP.enabled? ? head_end : fin
            range = Parser::Source::Range.new(processed_source.buffer, off[start], off[stop])
            message = format(self.class::MSG, method: name, complexity: complexity, max: max)
            add_offense(range, message: message) { self.max = complexity }
          end
        end
      end
    end
  end
end
