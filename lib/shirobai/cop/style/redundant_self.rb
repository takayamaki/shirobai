# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/RedundantSelf`.
      #
      # Rust walks the AST, tracks local-variable scopes and reports the
      # `self` receiver of redundant `self.foo` sends, returning the byte range
      # of `self` plus the `.` operator so Ruby can remove both.
      class RedundantSelf < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Redundant `self` detected."

        # Same timing as stock (`KERNEL_METHODS = Kernel.methods(false)`):
        # snapshot once at load, not per investigation.
        KERNEL_METHODS = Kernel.methods(false).map(&:to_s).freeze

        def self.cop_name = "Style/RedundantSelf"
        def self.badge = RuboCop::Cop::Badge.parse("Style/RedundantSelf")

        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Style::ColonMethodCall, RuboCop::Cop::Layout::DotPosition]
        end

        def on_new_investigation
          source = processed_source.raw_source
          buffer = processed_source.buffer

          Shirobai.check_redundant_self(source, KERNEL_METHODS).each do |self_start, self_end, dot_start, dot_end|
            range = Parser::Source::Range.new(buffer, self_start, self_end)
            add_offense(range) do |corrector|
              corrector.remove(range)
              corrector.remove(Parser::Source::Range.new(buffer, dot_start, dot_end))
            end
          end
        end
      end
    end
  end
end
