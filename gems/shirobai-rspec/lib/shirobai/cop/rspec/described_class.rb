# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/DescribedClass`
      # (rubocop-rspec 3.10.2).
      #
      # Rust identifies candidate `describe(Const)` blocks on the shared walk.
      # This wrapper locates the parser block node and runs stock's full
      # detection + autocorrect VERBATIM: `find_usage` recursion with
      # scope-change guards, `collapse_namespace`, `described_constant`,
      # and all config axes (EnforcedStyle, SkipBlocks, OnlyStaticConstants).
      #
      # Probed quirks live as differential specs in
      # described_class_edge_cases_spec.rb.
      class DescribedClass < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::RSpec::Namespace
        include RuboCop::RSpec::Language
        include Shirobai::Cop::BundleEligible

        DESCRIBED_CLASS = 'described_class'
        MSG             = 'Use `%<replacement>s` instead of `%<src>s`.'

        def self.cop_name = "RSpec/DescribedClass"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # No bundle_args needed -- the Rust side does not pack config for
        # this cop (all config axes are handled in the Ruby wrapper).
        def self.bundle_args(_config)
          []
        end

        # @!method common_instance_exec_closure?(node)
        def_node_matcher :common_instance_exec_closure?, <<~PATTERN
          (block
            {
              (send (const nil? {:Class :Module :Struct}) :new ...)
              (send (const nil? :Data) :define ...)
              (send _ {:class_eval :module_eval :instance_eval} ...)
              (send _ {:class_exec :module_exec :instance_exec} ...)
            }
            ...
          )
        PATTERN

        # @!method rspec_block?(node)
        def_node_matcher :rspec_block?,
                         '(any_block (send #rspec? #ALL.all ...) ...)'

        # @!method scope_changing_syntax?(node)
        def_node_matcher :scope_changing_syntax?, '{def class module}'

        # @!method described_constant(node)
        def_node_matcher :described_constant, <<~PATTERN
          (block (send _ :describe $(const ...) ...) (args) $_)
        PATTERN

        # @!method contains_described_class?(node)
        def_node_search :contains_described_class?,
                        '(send nil? :described_class)'

        def on_new_investigation
          RuboCop::RSpec::Language.config = config["RSpec"]["Language"]
          ranges = resolved_candidates
          return if ranges.nil? || ranges.empty?

          source = bundle_eligible? ? processed_source.raw_source : processed_source.buffer.source
          off = SourceOffsets.for(source)
          keys = ranges.map { |(start, fin)| [off[start], off[fin]] }
          located = Shirobai::RSpec::NodeLocator.locate(processed_source, keys)

          keys.each do |key|
            node = located[key]
            process_candidate(node) if node&.block_type?
          end
        end

        private

        # Stock's on_block logic, copied verbatim.
        def process_candidate(node)
          @described_class, body = described_constant(node)
          return unless body

          find_usage(body) do |match|
            msg = message(match.const_name)
            add_offense(match, message: msg) do |corrector|
              autocorrect(corrector, match)
            end
          end
        end

        def autocorrect(corrector, match)
          replacement = if style == :described_class
                          DESCRIBED_CLASS
                        else
                          @described_class.const_name
                        end

          corrector.replace(match, replacement)
        end

        def find_usage(node, &block)
          yield(node) if offensive?(node)
          return if scope_change?(node) || allowed?(node)

          node.each_child_node do |child|
            find_usage(child, &block)
          end
        end

        def allowed?(node)
          node.const_type? && only_static_constants?
        end

        def message(offense)
          if style == :described_class
            format(MSG, replacement: DESCRIBED_CLASS, src: offense)
          else
            format(MSG, replacement: @described_class.const_name,
                        src: DESCRIBED_CLASS)
          end
        end

        def scope_change?(node)
          scope_changing_syntax?(node) ||
            common_instance_exec_closure?(node) ||
            skippable_block?(node)
        end

        def skippable_block?(node)
          return false unless cop_config['SkipBlocks']

          node.any_block_type? && !rspec_block?(node)
        end

        def only_static_constants?
          cop_config.fetch('OnlyStaticConstants', true)
        end

        def offensive?(node)
          if style == :described_class
            offensive_described_class?(node)
          else
            node.send_type? && node.method?(:described_class)
          end
        end

        def offensive_described_class?(node)
          return false unless node.const_type?

          # E.g. `described_class::CONSTANT`
          return false if contains_described_class?(node)

          nearest_described_class, = node.each_ancestor(:block)
            .map { |ancestor| described_constant(ancestor) }.find(&:itself)

          return false if nearest_described_class.equal?(node)

          full_const_name(nearest_described_class) == full_const_name(node)
        end

        def full_const_name(node)
          symbolized_namespace = namespace(node).map(&:to_sym)
          collapse_namespace(symbolized_namespace, const_name(node))
        end

        # @param namespace [Array<Symbol>]
        # @param const [Array<Symbol>]
        # @return [Array<Symbol>]
        def collapse_namespace(namespace, const)
          return const if namespace.empty? || const.first.nil?

          start = [0, (namespace.length - const.length)].max
          max = namespace.length
          intersection = (start..max).find do |shift|
            namespace[shift, max - shift] == const[0, max - shift]
          end
          [*namespace[0, intersection], *const]
        end

        # @param node [RuboCop::AST::Node]
        # @return [Array<Symbol>]
        def const_name(node)
          namespace = node.namespace
          name = node.short_name
          if !namespace
            [name]
          elsif namespace.const_type?
            [*const_name(namespace), name]
          elsif %i[lvar cbase send].include?(namespace.type)
            [nil, name]
          end
        end

        # --- Resolution ---

        def resolved_candidates
          if bundle_eligible?
            ranges = Dispatch.offenses_for(processed_source, config, :rspec_described_class)
            return ranges unless ranges.nil?
          end
          Shirobai.check_rspec_described_class(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end
      end
    end
  end
end
