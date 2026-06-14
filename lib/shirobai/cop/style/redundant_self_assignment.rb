# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/RedundantSelfAssignment`.
      #
      # Detection and autocorrect both happen in Rust; Ruby turns each tuple
      # into an offense and a `corrector.replace` / `corrector.remove` op.
      # `METHODS_RETURNING_SELF` is closed and copied verbatim into the Rust
      # rule, so the cop is config-less (`bundle_args` returns `[]`).
      #
      # Two `kind`s of offenses come back:
      # * `kind = 0` (variable assignment, `var = var.<METHOD!>(...)`): replace
      #   `[range_start, range_end)` with `source[rhs_start..rhs_end]` (drop the
      #   `var = ` prefix, leave the rhs source intact — stock does the same via
      #   `corrector.replace(node, rhs.source)`).
      # * `kind = 1` (setter, `obj.foo = obj.foo.<METHOD!>(...)`): delete
      #   `[range_start, range_end)`, which is the `obj.foo = ` prefix (stock's
      #   `corrector.remove(range_between(node_start, first_arg_start))`).
      class RedundantSelfAssignment < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Redundant self assignment detected. " \
              "Method `%<method_name>s` modifies its receiver in place."

        def self.cop_name = "Style/RedundantSelfAssignment"
        def self.badge = RuboCop::Cop::Badge.parse("Style/RedundantSelfAssignment")

        # Config-less from the Rust side — the destructive method set is a
        # closed list in `crates/shirobai-core/src/rules/redundant_self_assignment.rs`.
        def self.bundle_args(_config) = []

        def on_new_investigation
          offenses = Dispatch.offenses_for(processed_source, config, :redundant_self_assignment)
          return if offenses.empty?

          off = SourceOffsets.for(processed_source.raw_source)
          buffer = processed_source.buffer
          source = processed_source.raw_source

          offenses.each do |op_start, op_end, method_name, kind, range_start, range_end, rhs_start, rhs_end|
            op_range = Parser::Source::Range.new(buffer, off[op_start], off[op_end])
            message = format(MSG, method_name: method_name)
            add_offense(op_range, message: message) do |corrector|
              target = Parser::Source::Range.new(buffer, off[range_start], off[range_end])
              if kind == 0
                # Variable assignment: replace the whole `var = var.foo(args)`
                # node with the rhs source `var.foo(args)`.
                replacement = source.byteslice(rhs_start, rhs_end - rhs_start)
                corrector.replace(target, replacement)
              else
                # Setter: remove the `obj.foo = ` prefix in place.
                corrector.remove(target)
              end
            end
          end
        end
      end
    end
  end
end
