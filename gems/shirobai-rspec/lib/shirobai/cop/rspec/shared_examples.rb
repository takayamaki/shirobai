# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/SharedExamples`
      # (rubocop-rspec 3.10.2).
      #
      # Rust supplies candidate SEND ranges: shared-example / include_* calls
      # (`#SharedGroups.all` with an rspec receiver, or `#Includes.all` with a
      # nil receiver) whose FIRST argument is a plain `str` or `sym` title. The
      # anchor set is style-INDEPENDENT (both a symbol-under-`string` and a
      # string-under-`symbol` are potential offenders), so no config crosses the
      # FFI. This wrapper relocates each parser send node and runs stock's
      # `on_send` plus autocorrect VERBATIM: `EnforcedStyle` (read here via
      # ConfigurableEnforcedStyle) decides `offense?`, the message, and the
      # titleize / symbolize replacement, all on the real parser node, so the
      # offense range and correction match byte for byte.
      class SharedExamples < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::RSpec::Language
        include Shirobai::Cop::RSpec::SendCandidateSupport

        def self.cop_name = "RSpec/SharedExamples"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        # @!method shared_examples(node)
        def_node_matcher :shared_examples, <<~PATTERN
          {
            (send #rspec? #SharedGroups.all $_ ...)
            (send nil? #Includes.all $_ ...)
          }
        PATTERN

        private

        def candidate_slot = :rspec_shared_examples

        def fallback_candidates
          Shirobai.check_rspec_shared_examples(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end

        # --- stock's `on_send`, copied verbatim (rubocop-rspec 3.10.2);
        # renamed to `investigate_send` so the Commissioner never dispatches a
        # per-node `on_send`.
        def investigate_send(node)
          shared_examples(node) do |ast_node|
            next unless offense?(ast_node)

            checker = new_checker(ast_node)
            add_offense(ast_node, message: checker.message) do |corrector|
              corrector.replace(ast_node, checker.preferred_style)
            end
          end
        end

        def offense?(ast_node)
          if style == :symbol
            ast_node.str_type?
          else # string
            ast_node.sym_type?
          end
        end

        def new_checker(ast_node)
          if style == :symbol
            SymbolChecker.new(ast_node)
          else # string
            StringChecker.new(ast_node)
          end
        end

        # :nodoc:
        class SymbolChecker
          MSG = 'Prefer %<prefer>s over `%<current>s` ' \
                "to symbolize shared examples."

          attr_reader :node

          def initialize(node)
            @node = node
          end

          def message
            format(MSG, prefer: preferred_style, current: node.value.inspect)
          end

          def preferred_style
            ":#{node.value.to_s.downcase.tr(' ', '_')}"
          end
        end

        # :nodoc:
        class StringChecker
          MSG = 'Prefer %<prefer>s over `%<current>s` ' \
                "to titleize shared examples."

          attr_reader :node

          def initialize(node)
            @node = node
          end

          def message
            format(MSG, prefer: preferred_style, current: node.value.inspect)
          end

          def preferred_style
            "'#{node.value.to_s.tr('_', ' ')}'"
          end
        end
      end
    end
  end
end
