# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/SafeNavigationChain`.
      #
      # Rust detects ordinary method calls chained after a safe-navigation call
      # and computes the autocorrection (insert `&.`, with parenthesization where
      # needed). Ruby supplies the `nil_methods` allow-list and applies the
      # corrections.
      class SafeNavigationChain < RuboCop::Cop::Base
        include RuboCop::Cop::NilMethods
        extend RuboCop::Cop::AutoCorrector

        MSG = "Do not chain ordinary method call after safe navigation operator."

        def self.cop_name = "Lint/SafeNavigationChain"
        def self.badge = RuboCop::Cop::Badge.parse("Lint/SafeNavigationChain")

        def on_new_investigation
          source = processed_source.raw_source
          methods = nil_method_names
          buffer = processed_source.buffer

          Shirobai.check_safe_navigation_chain(source, methods).each do |start, fin, replacement, wrap_start, wrap_end|
            range = Parser::Source::Range.new(buffer, start, fin)
            add_offense(range) do |corrector|
              # Empty replacement means the offense has no safe correction
              # (e.g. the else branch of a ternary on the same receiver).
              next if replacement.empty?

              corrector.replace(range, replacement)
              if wrap_end > wrap_start
                corrector.wrap(Parser::Source::Range.new(buffer, wrap_start, wrap_end), "(", ")")
              end
            end
          end
        end

        private

        # Config-derived and stable for the life of the instance.
        def nil_method_names
          @nil_method_names ||= nil_methods.map(&:to_s)
        end
      end
    end
  end
end
