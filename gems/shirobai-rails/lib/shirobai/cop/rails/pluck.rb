# frozen_string_literal: true

module Shirobai
  module Cop
    module Rails
      # Drop-in Rust reimplementation of `Rails/Pluck`
      # (rubocop-rails 2.35.5).
      #
      # Rust detects `map { |x| x[:key] }` / `collect { |x| x[:key] }`
      # patterns replaceable with `pluck(:key)`, including numblock (`_1`)
      # and itblock (`it`) forms. The ancestor block-with-receiver guard
      # (N+1 query risk) and the regexp key / block-arg-in-key exclusions
      # are all handled Rust-side.
      #
      # Autocorrect: replace from the selector (map/collect) through the
      # block end with `pluck(<key source>)`.
      class Pluck < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector
        extend RuboCop::Cop::TargetRailsVersion

        MSG = "Prefer `%<replacement>s` over `%<current>s`."

        minimum_target_rails_version 5.0

        def self.cop_name = "Rails/Pluck"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # No behavioral config to pack into the segment.
        def self.bundle_args(_config)
          [[], []]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |sel_start, block_end, key_src_start, key_src_end|
            offense_range = Parser::Source::Range.new(buffer, off[sel_start], off[block_end])
            key_source = source.byteslice(key_src_start, key_src_end - key_src_start)
            replacement = "pluck(#{key_source})"
            message = format(MSG, replacement: replacement, current: offense_range.source)
            add_offense(offense_range, message: message) do |corrector|
              corrector.replace(offense_range, replacement)
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :rails_pluck)
          else
            Shirobai.check_rails_pluck(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
