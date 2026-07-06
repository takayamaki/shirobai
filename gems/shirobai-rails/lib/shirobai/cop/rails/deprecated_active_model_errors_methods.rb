# frozen_string_literal: true

require "set"

module Shirobai
  module Cop
    module Rails
      # Drop-in Rust-backed reimplementation of
      # `Rails/DeprecatedActiveModelErrorsMethods` (rubocop-rails 2.35.5),
      # Architecture B.
      #
      # Rust supplies candidate SEND ranges (the five errors-chain shapes:
      # `errors[...]`/`errors.messages[...]`/`errors.details[...]` manipulation
      # and assignment, and `errors.{keys,values,to_h,to_xml}`); this wrapper
      # relocates each parser send node and runs stock's `on_send` (renamed
      # `investigate_send`) plus autocorrect VERBATIM. The receiver-model
      # heuristic (`model_file?`), the Rails <= 6.0 gate on the incompatible
      # methods, `skip_autocorrect?` (uncorrectable `details << ...`) and the
      # receiver-walk offense range all run on the parser AST, so offenses and
      # `-A` bytes match stock exactly.
      class DeprecatedActiveModelErrorsMethods < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        include Shirobai::Cop::Rails::CandidateSupport
        extend RuboCop::Cop::AutoCorrector

        MSG = 'Avoid manipulating ActiveModel errors as hash directly.'
        AUTOCORRECTABLE_METHODS = %i[<< clear keys].freeze
        INCOMPATIBLE_METHODS = %i[keys values to_h to_xml].freeze

        MANIPULATIVE_METHODS = Set[
          *%i[
            << append clear collect! compact! concat
            delete delete_at delete_if drop drop_while fill filter! keep_if
            flatten! insert map! pop prepend push reject! replace reverse!
            rotate! select! shift shuffle! slice! sort! sort_by! uniq! unshift
          ]
        ].freeze

        def self.cop_name = "Rails/DeprecatedActiveModelErrorsMethods"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        def_node_matcher :receiver_matcher_outside_model, '{send ivar lvar}'
        def_node_matcher :receiver_matcher_inside_model, '{nil? send ivar lvar}'

        def_node_matcher :any_manipulation?, <<~PATTERN
          {
            #root_manipulation?
            #root_assignment?
            #errors_deprecated?
            #messages_details_manipulation?
            #messages_details_assignment?
          }
        PATTERN

        def_node_matcher :root_manipulation?, <<~PATTERN
          (send
            (send
              (send #receiver_matcher :errors) :[] ...)
            MANIPULATIVE_METHODS
            ...
          )
        PATTERN

        def_node_matcher :root_assignment?, <<~PATTERN
          (send
            (send #receiver_matcher :errors)
            :[]=
            ...)
        PATTERN

        def_node_matcher :errors_deprecated?, <<~PATTERN
          (send
            (send #receiver_matcher :errors)
            {:keys :values :to_h :to_xml})
        PATTERN

        def_node_matcher :messages_details_manipulation?, <<~PATTERN
          (send
            (send
              (send
                (send #receiver_matcher :errors)
                {:messages :details})
                :[]
                ...)
              MANIPULATIVE_METHODS
            ...)
        PATTERN

        def_node_matcher :messages_details_assignment?, <<~PATTERN
          (send
            (send
              (send #receiver_matcher :errors)
              {:messages :details})
            :[]=
            ...)
        PATTERN

        private

        def candidate_slot = :rails_deprecated_active_model_errors_methods

        def fallback_candidates
          Shirobai.check_rails_deprecated_active_model_errors_methods(processed_source.buffer.source)
        end

        # --- stock's methods, copied verbatim (rubocop-rails 2.35.5);
        # `on_send` renamed to `investigate_send` so it is not a node callback.

        def investigate_send(node)
          any_manipulation?(node) do
            next if target_rails_version <= 6.0 && INCOMPATIBLE_METHODS.include?(node.method_name)

            add_offense(node) do |corrector|
              next if skip_autocorrect?(node)

              autocorrect(corrector, node)
            end
          end
        end

        def skip_autocorrect?(node)
          return true unless AUTOCORRECTABLE_METHODS.include?(node.method_name)
          return false unless (receiver = node.receiver.receiver)

          receiver.send_type? && receiver.method?(:details) && node.method?(:<<)
        end

        def autocorrect(corrector, node)
          receiver = node.receiver

          range = offense_range(node, receiver)
          replacement = replacement(node, receiver)

          corrector.replace(range, replacement)
        end

        def offense_range(node, receiver)
          receiver = receiver.receiver while receiver.send_type? && !receiver.method?(:errors) && receiver.receiver
          range_between(receiver.source_range.end_pos, node.source_range.end_pos)
        end

        def replacement(node, receiver)
          return '.attribute_names' if node.method?(:keys)

          key = receiver.first_argument.source

          case node.method_name
          when :<<
            value = node.first_argument.source

            ".add(#{key}, #{value})"
          when :clear
            ".delete(#{key})"
          end
        end

        def receiver_matcher(node)
          model_file? ? receiver_matcher_inside_model(node) : receiver_matcher_outside_model(node)
        end

        def model_file?
          processed_source.file_path.include?('/models/')
        end
      end
    end
  end
end
