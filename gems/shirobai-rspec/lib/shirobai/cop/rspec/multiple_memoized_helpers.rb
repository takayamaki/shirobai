# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/MultipleMemoizedHelpers`
      # (rubocop-rspec 3.10.2).
      #
      # Rust does the scope-tree work on the shared walk: for every plain-block
      # spec group it unions the memoized helpers visible from the group's own
      # frame and every parser-block ancestor frame (stock's `all_helpers`),
      # dedups them by node identity, and maps each `let` (plus `subject` when
      # `AllowSubject: false`) to its `variable_definition?` name. Names that are
      # decidable bytewise (sym value / str value / nil) are counted as
      # `rust_distinct`; `dsym`/`dstr` names cannot be (two interpolations that
      # differ only in whitespace are structurally EQUAL but not byte-equal), so
      # their source ranges are handed here. Rust emits a group only when the
      # safe upper bound `rust_distinct + dsym_dstr_count > Max`.
      #
      # The wrapper locates the `dsym`/`dstr` nodes in the parser AST, dedups
      # them with stock's structural node equality (`Array#uniq`), computes
      # `count = rust_distinct + located_uniq`, and on a real `count > Max`
      # reports the group and records `self.max = count` through the same
      # `exclude_limit 'Max'` DSL stock uses (drives `--auto-gen-config`).
      class MultipleMemoizedHelpers < RuboCop::Cop::Base
        MSG = "Example group has too many memoized helpers [%<count>d/%<max>d]"

        exclude_limit "Max"

        def self.cop_name = "RSpec/MultipleMemoizedHelpers"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[Max, AllowSubject]`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [cop_config.fetch("Max", 5), cop_config.fetch("AllowSubject", true) ? 1 : 0]
        end

        def on_new_investigation
          groups = Dispatch.offenses_for(processed_source, config, :rspec_multiple_memoized_helpers)
          # Gated-off file (see Dispatch#offenses_for): standalone fallback.
          groups ||= Shirobai.check_rspec_multiple_memoized_helpers(
            processed_source.raw_source, *Shirobai::RSpec.segment(config)
          )
          return if groups.empty?

          max = cop_config["Max"]
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          located = locate_dynamic_names(groups, off)

          groups.each do |(gstart, gend, rust_distinct, dyn_ranges)|
            # A locate miss (a prism/parser range disagreement we have not
            # met yet) falls back to the range itself as the identity, so it
            # degrades to per-occurrence distinctness instead of silently
            # merging every missed node into one `nil`.
            nodes = dyn_ranges.map { |(s, e)| located[[off[s], off[e]]] || [off[s], off[e]] }
            count = rust_distinct + nodes.uniq.length
            next if count <= max

            self.max = count
            range = Parser::Source::Range.new(buffer, off[gstart], off[gend])
            add_offense(range, message: format(MSG, count: count, max: max))
          end
        end

        private

        # One AST descent for every group's `dsym`/`dstr` ranges (converted to
        # char offsets). Returns the `[begin, end] => node` map.
        def locate_dynamic_names(groups, off)
          char_ranges = groups.flat_map do |(_gstart, _gend, _rust_distinct, dyn_ranges)|
            dyn_ranges.map { |(s, e)| [off[s], off[e]] }
          end
          return {} if char_ranges.empty?

          Shirobai::RSpec::NodeLocator.locate(processed_source, char_ranges)
        end
      end
    end
  end
end
