# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/Dialect` (rubocop-rspec 3.10.2).
      #
      # Rust supplies candidate SEND ranges for calls whose method name is a
      # configured `PreferredMethods` key (with an rspec receiver). The key set
      # rides the rspec segment's 17th role list, so an unconfigured or disabled
      # cop emits ZERO candidates and the shared walk's Dialect classifier stays
      # dormant (its always-on cost is a single "is the key set empty?" check).
      #
      # This wrapper relocates each parser send node and runs stock's `on_send`
      # plus autocorrect VERBATIM: `rspec_method?` (receiver + `ALL.all`) and
      # `preferred_methods` (the override-cancellation logic in the stock
      # `MethodPreference` mixin) are re-checked here, so the offense range,
      # message, and selector rewrite match byte for byte. The Rust key set is a
      # safe SUPERSET of the effective offenders (it carries the raw
      # `PreferredMethods` keys; the mixin may cancel some), so relocation never
      # misses an offense and the verbatim guard drops any extra candidate.
      class Dialect < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        include RuboCop::RSpec::Language
        include RuboCop::Cop::MethodPreference
        include Shirobai::Cop::RSpec::SendCandidateSupport

        MSG = "Prefer `%<prefer>s` over `%<current>s`."

        def self.cop_name = "RSpec/Dialect"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        # The rspec segment's 17th list: the configured `PreferredMethods` keys
        # (the method names the cop rewrites). Rust emits Dialect candidates only
        # for these names, so an empty map keeps the classifier dormant.
        def self.bundle_lists(config)
          preferred = config.for_badge(badge).fetch("PreferredMethods", {})
          preferred.is_a?(Hash) ? preferred.keys.map(&:to_s) : []
        end

        # @!method rspec_method?(node)
        def_node_matcher :rspec_method?, "(send #rspec? #ALL.all ...)"

        private

        def candidate_slot = :rspec_dialect

        def fallback_candidates
          Shirobai.check_rspec_dialect(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end

        # --- stock's `on_send`, copied verbatim (rubocop-rspec 3.10.2);
        # renamed to `investigate_send` so the Commissioner never dispatches a
        # per-node `on_send`.
        def investigate_send(node)
          return unless rspec_method?(node)
          return unless preferred_methods[node.method_name]

          msg = format(MSG, prefer: preferred_method(node.method_name),
                            current: node.method_name)

          add_offense(node, message: msg) do |corrector|
            current = node.loc.selector
            preferred = preferred_method(current.source)

            corrector.replace(current, preferred)
          end
        end
      end
    end
  end
end
