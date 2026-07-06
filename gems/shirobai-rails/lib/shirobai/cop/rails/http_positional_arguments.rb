# frozen_string_literal: true

module Shirobai
  module Cop
    module Rails
      # Drop-in Rust-backed reimplementation of `Rails/HttpPositionalArguments`
      # (rubocop-rails 2.35.5), Architecture B.
      #
      # Rust supplies candidate SEND ranges (bare-receiver HTTP verb calls with
      # an action plus a data argument); this wrapper relocates each parser
      # send node and runs stock's `on_send` (renamed `investigate_send`) plus
      # autocorrect VERBATIM. The routing-block / rack-test guards, the
      # `http_request?` / `needs_conversion?` matchers (kwsplat / forwarded /
      # already-keyword hashes) and the full-node source rebuild all run on the
      # parser AST, so offenses and `-A` bytes match stock exactly.
      #
      # `TargetRailsVersion`: like stock, gated on
      # `requires_gem('railties', '>= 5.0')` — silent on Rails < 5.0 or without
      # railties in the target bundle. The `Include: **/spec/**, **/test/**`
      # from the merged default.yml resolves through this wrapper's badge,
      # exactly as for the stock cop.
      class HttpPositionalArguments < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        include Shirobai::Cop::Rails::CandidateSupport
        extend RuboCop::Cop::AutoCorrector
        extend RuboCop::Cop::TargetRailsVersion

        MSG = 'Use keyword arguments instead of positional arguments for http call: `%<verb>s`.'
        KEYWORD_ARGS = %i[method params session body flash xhr as headers env to].freeze
        ROUTING_METHODS = %i[draw routes].freeze
        RESTRICT_ON_SEND = %i[get post put patch delete head].freeze

        minimum_target_rails_version 5.0

        def self.cop_name = "Rails/HttpPositionalArguments"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # No behavioral config: the candidate list is a wake-up flag only.
        def self.bundle_args(_config)
          []
        end

        def_node_matcher :http_request?, <<~PATTERN
          (send nil? {#{RESTRICT_ON_SEND.map(&:inspect).join(' ')}} !nil? $_ ...)
        PATTERN

        def_node_matcher :kwsplat_hash?, <<~PATTERN
          (hash (kwsplat _))
        PATTERN

        def_node_matcher :forwarded_kwrestarg?, <<~PATTERN
          (hash (forwarded-kwrestarg))
        PATTERN

        def_node_matcher :include_rack_test_methods?, <<~PATTERN
          (send nil? :include
            (const
              (const
                (const {nil? cbase} :Rack) :Test) :Methods))
        PATTERN

        private

        def candidate_slot = :rails_http_positional_arguments

        def fallback_candidates
          Shirobai.check_rails_http_positional_arguments(processed_source.buffer.source)
        end

        # --- stock's methods, copied verbatim (rubocop-rails 2.35.5);
        # `on_send` renamed to `investigate_send` so it is not a node callback.

        def investigate_send(node)
          return if in_routing_block?(node) || use_rack_test_methods?

          http_request?(node) do |data|
            return unless needs_conversion?(data)

            message = format(MSG, verb: node.method_name)

            add_offense(highlight_range(node), message: message) do |corrector|
              corrector.replace(node, correction(node))
            end
          end
        end

        def in_routing_block?(node)
          !!node.each_ancestor(:block).detect { |block| ROUTING_METHODS.include?(block.method_name) }
        end

        def use_rack_test_methods?
          processed_source.ast.each_descendant(:send).any? do |node|
            include_rack_test_methods?(node)
          end
        end

        # rubocop:disable Metrics/CyclomaticComplexity
        def needs_conversion?(data)
          return false if data.forwarded_args_type? || forwarded_kwrestarg?(data)
          return true unless data.hash_type?
          return false if kwsplat_hash?(data)

          data.each_pair.none? do |pair|
            special_keyword_arg?(pair.key) || (format_arg?(pair.key) && data.pairs.one?)
          end
        end
        # rubocop:enable Metrics/CyclomaticComplexity

        def special_keyword_arg?(node)
          node.sym_type? && KEYWORD_ARGS.include?(node.value)
        end

        def format_arg?(node)
          node.sym_type? && node.value == :format
        end

        def highlight_range(node)
          _http_path, *data = *node.arguments

          range_between(data.first.source_range.begin_pos, data.last.source_range.end_pos)
        end

        def convert_hash_data(data, type)
          return '' if data.hash_type? && data.empty?

          hash_data = if data.hash_type?
                        format('{ %<data>s }', data: data.pairs.map(&:source).join(', '))
                      else
                        # user supplies an object,
                        # no need to surround with braces
                        data.source
                      end

          format(', %<type>s: %<hash_data>s', type: type, hash_data: hash_data)
        end

        def correction(node)
          http_path, *data = *node.arguments

          controller_action = http_path.source
          params = convert_hash_data(data.first, 'params')
          session = convert_hash_data(data.last, 'session') if data.size > 1

          format(correction_template(node), name: node.method_name,
                                            action: controller_action,
                                            params: params,
                                            session: session)
        end

        def correction_template(node)
          if parentheses?(node)
            '%<name>s(%<action>s%<params>s%<session>s)'
          else
            '%<name>s %<action>s%<params>s%<session>s'
          end
        end
      end
    end
  end
end
