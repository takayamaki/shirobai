# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/ColonMethodCall`.
      #
      # Detection and autocorrect both happen in Rust; Ruby turns the byte
      # offsets of the `::` token into an offense range plus a `corrector.replace(range, '.')`
      # call (matching stock's `add_offense(node.loc.dot) { |c| c.replace(node.loc.dot, '.') }`).
      #
      # The cop carries no config (stock has neither `EnforcedStyle` nor an
      # allowed-methods list), so `bundle_args` returns an empty vector and the
      # bundle path is always taken.
      class ColonMethodCall < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Do not use `::` for method calls."

        # Stock declares `autocorrect_incompatible_with [RedundantSelf]`. We
        # mirror that so the corrector dispatcher serialises us against
        # `Style/RedundantSelf` in the same pass, matching stock's behaviour
        # when both cops fire in one autocorrect cycle.
        def self.autocorrect_incompatible_with
          [Shirobai::Cop::Style::RedundantSelf]
        end

        def self.cop_name = "Style/ColonMethodCall"
        def self.badge = RuboCop::Cop::Badge.parse("Style/ColonMethodCall")

        # Config-less cop. Returns an empty array so `Dispatch.packed_config`
        # can splat it without touching `nums` or `lists`.
        def self.bundle_args(_config)
          []
        end

        def bundle_eligible?
          true
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          fetch_offenses.each do |dot_start, dot_end|
            range = Parser::Source::Range.new(buffer, off[dot_start], off[dot_end])
            add_offense(range) do |corrector|
              corrector.replace(range, ".")
            end
          end
        end

        private

        def fetch_offenses
          Dispatch.offenses_for(processed_source, config, :colon_method_call)
        end
      end
    end
  end
end
