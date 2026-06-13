# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/EmptyLineBetweenDefs`.
      #
      # Rust walks the AST, reproduces the stock cop's `on_begin` over every
      # parser-gem `begin` group (a `StatementsNode` with >= 2 children, plus the
      # `kwbegin` / rescue handler bodies), runs `check_defs` over each adjacent
      # pair of definition candidates (method / class / module / `DefLikeMacros`
      # macro), and returns, per offense, the second member's `def_location`
      # range, the formatted message and the autocorrect: an `insert` flag plus
      # the `newline_pos` and the line count `n`. Ruby applies the correction
      # with the same two `RangeHelp#range_between` arms stock uses
      # (`insert_after` a `"\n" * n`, or `remove` the surplus newlines).
      #
      # The offenses come from the per-file bundled run (`Shirobai::Dispatch`);
      # the autocorrect re-passes re-investigate a fresh `ProcessedSource`, which
      # recomputes the bundle from scratch, so this cop keeps no cross-pass state
      # and is always bundle eligible.
      class EmptyLineBetweenDefs < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Layout/EmptyLineBetweenDefs"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: nums
        # `[method_defs, class_defs, module_defs, allow_adjacent_one_line_defs,
        # minimum_empty_lines, maximum_empty_lines]` and the `DefLikeMacros` list.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          numbers = Array(cop_config["NumberOfEmptyLines"] || 1)
          [
            [
              cop_config["EmptyLineBetweenMethodDefs"] ? 1 : 0,
              cop_config["EmptyLineBetweenClassDefs"] ? 1 : 0,
              cop_config["EmptyLineBetweenModuleDefs"] ? 1 : 0,
              cop_config["AllowAdjacentOneLineDefs"] ? 1 : 0,
              numbers.first,
              numbers.last
            ],
            Array(cop_config.fetch("DefLikeMacros", [])).map(&:to_s)
          ]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          offenses_for_source.each do |start, fin, message, insert, pos, n|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: message) do |corrector|
              # `Layout/EmptyLineBetweenDefs#autocorrect`. `pos` is the byte
              # `newline_pos`; convert through `SourceOffsets` so the
              # `range_between` indices are character offsets like the stock
              # corrector's.
              cpos = off[pos]
              if insert
                where_to_insert = range_between(cpos, cpos + 1)
                corrector.insert_after(where_to_insert, "\n" * n)
              else
                corrector.remove(range_between(cpos, cpos + n))
              end
            end
          end
        end

        private

        def offenses_for_source
          Dispatch.offenses_for(processed_source, config, :empty_line_between_defs)
        end
      end
    end
  end
end
