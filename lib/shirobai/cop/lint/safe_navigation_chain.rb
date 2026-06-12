# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/SafeNavigationChain`.
      #
      # Rust detects ordinary method calls chained after a safe-navigation call
      # and computes the autocorrection (insert `&.`, with parenthesization where
      # needed). Ruby supplies the `nil_methods` allow-list and applies the
      # corrections. Offenses come from the per-file bundled run
      # (`Shirobai::Dispatch`); the allow-list is purely config-driven, so this
      # cop is always bundle-eligible.
      class SafeNavigationChain < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Do not chain ordinary method call after safe navigation operator."

        def self.cop_name = "Lint/SafeNavigationChain"
        def self.badge = RuboCop::Cop::Badge.parse("Lint/SafeNavigationChain")

        # Packed args for the bundled run: `[nil_methods]`, replicating
        # `RuboCop::Cop::NilMethods#nil_methods` (nil's own methods + stdlib
        # additions + the `AllowedMethods` config) stringified for Rust.
        def self.bundle_args(config)
          allowed = Cop.allowed_methods_config(config.for_badge(badge))
          [(nil.methods + [:to_d] + allowed.map(&:to_sym)).map(&:to_s)]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :safe_navigation_chain)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |start, fin, replacement, wrap_start, wrap_end|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range) do |corrector|
              # Empty replacement means the offense has no safe correction
              # (e.g. the else branch of a ternary on the same receiver).
              next if replacement.empty?

              corrector.replace(range, replacement)
              if wrap_end > wrap_start
                corrector.wrap(Parser::Source::Range.new(buffer, off[wrap_start], off[wrap_end]), "(", ")")
              end
            end
          end
        end
      end
    end
  end
end
