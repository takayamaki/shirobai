# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/BlockDelimiters`.
      #
      # Rust walks the AST once, replicating stock's callback order: `on_send`
      # registers every block inside unparenthesized call arguments as ignored,
      # and `on_block` (calls, `super`, lambda literals, numbered/`it` blocks)
      # judges the configured style (all four `EnforcedStyle`s, the
      # `require_do_end?` rescue quirk, `BracesRequiredMethods`,
      # `AllowedMethods`, and the parser-parent emulation behind `semantic`).
      # Corrections come back as the exact corrector call sequence
      # (`replace`/`remove`/`insert_before`/`insert_after`/`wrap`), including
      # the comment relocation and the `begin`..`end` wrapping of rescue
      # bodies.
      #
      # The cop instance accumulates the ignored block ranges across
      # autocorrect iterations, mirroring `IgnoredNode`'s `@ignored_nodes`
      # (which is never reset between investigations on one instance): blocks
      # ignored by `on_send` unconditionally, offense blocks only when the
      # offense is enabled — which is why the ignore lands inside the
      # `add_offense` block, exactly like stock's `ignore_node`.
      #
      # The bundled / resolved path assumes every offense is enabled. When
      # Rust reports that a candidate was suppressed solely by another
      # offense's range (`has_conditional` — its real fate depends on
      # `rubocop:disable` directives), or when `AllowedPatterns` need regex
      # matching, the wrapper replays the raw event stream through
      # `add_offense`, which natively reproduces the disable semantics.
      class BlockDelimiters < RuboCop::Cop::Base
        include RuboCop::Cop::AllowedMethods
        include RuboCop::Cop::AllowedPattern
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Style/BlockDelimiters"
        def self.badge = RuboCop::Cop::Badge.parse("Style/BlockDelimiters")

        # `Style/RedundantBegin` rewrites the same `begin` bodies the
        # do-end-to-braces correction wraps.
        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Style::RedundantBegin]
        end

        STYLES = {
          "line_count_based" => 0,
          "semantic" => 1,
          "braces_for_chaining" => 2,
          "always_braces" => 3
        }.freeze

        # Packed args for the bundled run: `[nums, lists]` with
        # `nums = [style, AllowBracesOnProceduralOneLiners]` and
        # `lists = [ProceduralMethods, FunctionalMethods, AllowedMethods,
        # BracesRequiredMethods]`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            [
              STYLES.fetch(cop_config["EnforcedStyle"] || "line_count_based"),
              cop_config["AllowBracesOnProceduralOneLiners"] ? 1 : 0
            ],
            [
              Array(cop_config["ProceduralMethods"]).map(&:to_s),
              Array(cop_config["FunctionalMethods"]).map(&:to_s),
              Shirobai::Cop.allowed_methods_config(cop_config).map(&:to_s),
              Array(cop_config.fetch("BracesRequiredMethods", [])).map(&:to_s)
            ]
          ]
        end

        def on_new_investigation
          @ignored_ranges ||= []
          return replay_events if !allowed_patterns.empty?

          offenses, send_ignores, has_conditional = resolved_result
          # A candidate was suppressed solely by another offense's range: its
          # real fate depends on disable directives, so replay exactly.
          return replay_events if has_conditional

          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          send_ignores.each { |s, e| @ignored_ranges << [s, e] }
          offenses.each do |ts, te, bs, be, message, ops|
            range = Parser::Source::Range.new(buffer, off[ts], off[te])
            add_offense(range, message: message) do |corrector|
              apply_ops(corrector, buffer, off, ops)
              # `ignore_node` runs only when the offense is enabled; the
              # ranges stay BYTE offsets (they go back to Rust).
              @ignored_ranges << [bs, be]
            end
          end
        end

        private

        def resolved_result
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :block_delimiters)
          else
            nums, lists = bundle_args
            Shirobai.check_block_delimiters(
              processed_source.raw_source, nums, lists, @ignored_ranges
            )
          end
        end

        # The bundle computes this cop with no prior ignored ranges, so it
        # only matches the direct call on the first (lint) pass; autocorrect
        # re-passes carry the accumulated ranges through the standalone entry
        # point.
        def bundle_eligible?
          @ignored_ranges.empty?
        end

        # Exact replay of stock's traversal over the Rust event stream:
        # `part_of_ignored_node?` (inclusive containment), AllowedMethods /
        # AllowedPatterns, then `add_offense` with the in-block ignore.
        def replay_events
          nums, lists = bundle_args
          events = Shirobai.check_block_delimiters_events(
            processed_source.raw_source, nums, lists
          )
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          events.each do |ignore, bs, be, ts, te, name, message, ops|
            if ignore
              @ignored_ranges << [bs, be]
              next
            end
            next if @ignored_ranges.any? { |s, e| s <= bs && be <= e }
            next if allowed_method?(name) || matches_allowed_pattern?(name)

            range = Parser::Source::Range.new(buffer, off[ts], off[te])
            add_offense(range, message: message) do |corrector|
              apply_ops(corrector, buffer, off, ops)
              @ignored_ranges << [bs, be]
            end
          end
        end

        def apply_ops(corrector, buffer, off, ops)
          ops.each do |kind, s, e, text|
            range = Parser::Source::Range.new(buffer, off[s], off[e])
            case kind
            when 0 then corrector.replace(range, text)
            when 1 then corrector.remove(range)
            when 2 then corrector.insert_before(range, text)
            when 3 then corrector.insert_after(range, text)
            when 4 then corrector.wrap(range, "begin\n", "\nend")
            end
          end
        end

        # Config-derived and stable for the life of the instance; shares the
        # derivation with the bundled run (single source of truth).
        def bundle_args
          @bundle_args ||= self.class.bundle_args(config)
        end
      end
    end
  end
end
