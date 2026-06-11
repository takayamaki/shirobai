# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/RedundantSelf`.
      #
      # Rust walks the AST, tracks local-variable scopes and reports the
      # `self` receiver of redundant `self.foo` sends, returning the byte range
      # of `self` plus the `.` operator so Ruby can remove both. Offenses come
      # from the per-file bundled run (`Shirobai::Dispatch`); the allow-list is
      # a load-time constant, so this cop is always bundle-eligible.
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

        # Packed args for the bundled run: `[kernel_methods]`.
        def self.bundle_args(_config)
          [KERNEL_METHODS]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :redundant_self)
          offenses.each do |self_start, self_end, dot_start, dot_end|
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
