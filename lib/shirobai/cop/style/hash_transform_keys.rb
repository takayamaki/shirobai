# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/HashTransformKeys`.
      #
      # Stock matches four shapes via the `HashTransformMethod` mixin
      # (`each_with_object({})` / `Hash[_.map {...}]` / `_.map{...}.to_h` /
      # `_.to_h{...}`) and emits a sequence of corrector ops that strips
      # `Hash[` / `]` (or the trailing `.to_h`), swaps the selector to
      # `transform_keys`, rewrites the block arguments and replaces the body
      # with just the key transformation. Rust replays the same gating
      # (`hash_receiver?`, `noop_transformation?`, `transformation_uses_both_args?`,
      # `use_transformed_argname?`) on the prism tree and returns the offense
      # span plus an ordered list of `(start, end, replacement)` edits the
      # wrapper plays back verbatim via `corrector.replace`.
      class HashTransformKeys < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Style/HashTransformKeys"
        def self.badge = RuboCop::Cop::Badge.parse("Style/HashTransformKeys")

        # Config-less from the bundle's point of view: no per-cop config
        # influences detection or autocorrect. (The cop's `Enabled` /
        # `Exclude` / etc. are honoured by the Team, not by us.)
        def self.bundle_args(_config) = []

        def on_new_investigation
          buffer = processed_source.buffer
          offenses = Dispatch.offenses_for(processed_source, config, :hash_transform_keys)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |start, fin, message, edits|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: message) do |corrector|
              edits.each do |edit_start, edit_end, replacement|
                edit_range = Parser::Source::Range.new(buffer, off[edit_start], off[edit_end])
                corrector.replace(edit_range, replacement)
              end
            end
          end
        end
      end
    end
  end
end
