# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/RequireParentheses`.
      #
      # All detection (predicate-method send with an `&&`/`||` argument, or a
      # ternary first-argument whose condition is `&&`/`||`) happens in Rust;
      # Ruby only turns the byte offsets into offenses. No autocorrect, matching
      # stock. Config-less, so this cop is always bundle-eligible.
      class RequireParentheses < RuboCop::Cop::Base
        MSG = "Use parentheses in the method call to avoid confusion about precedence."

        def self.cop_name = "Lint/RequireParentheses"
        def self.badge = RuboCop::Cop::Badge.parse("Lint/RequireParentheses")

        # No packed args — the rule is config-less. Present so the bundle
        # `packed_config` builder treats this cop uniformly with its peers
        # (currently a no-op; kept for parity with the other wrappers).
        def self.bundle_args(_config) = []

        def on_new_investigation
          offenses = Dispatch.offenses_for(processed_source, config, :require_parentheses)
          off = SourceOffsets.for(processed_source.raw_source)
          buffer = processed_source.buffer
          offenses.each do |start_offset, end_offset|
            range = Parser::Source::Range.new(buffer, off[start_offset], off[end_offset])
            add_offense(range)
          end
        end
      end
    end
  end
end
