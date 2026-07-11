# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/ArgumentsForwarding`.
      #
      # Rust does the whole `on_def` walk: for every method definition it
      # snapshots each descendant call / super / yield, collects the local
      # variable references in the body, and (when the def node closes)
      # classifies each call against the def's rest / kwrest / block params.
      # It returns, in stock's exact `add_offense` order, one record per
      # offense: the highlight range, the message code, and the precise
      # corrector op stream (`add_parentheses` reproduced byte for byte,
      # including stock's double `)` / `((` when two different-range offenses
      # each add parentheses to a paren-less def / call). RuboCop dedups
      # offenses by range through a Set, so a repeated range's corrector is
      # dropped here exactly like in stock.
      #
      # The cop is `TargetRubyVersion`-gated (`minimum_target_ruby_version
      # 2.7`); shirobai always parses with prism Latest, but the version only
      # gates the cop's LOGIC, which is passed through `bundle_args` from
      # `config.target_ruby_version` (pure config, parser-independent). The
      # cross-cop `Naming/BlockForwarding EnforcedStyle == 'explicit'` read and
      # the two boolean options plus the three redundant-name lists are packed
      # the same way.
      class ArgumentsForwarding < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector
        extend RuboCop::Cop::TargetRubyVersion

        minimum_target_ruby_version 2.7

        MESSAGES = [
          "Use shorthand syntax `...` for arguments forwarding.",
          "Use anonymous positional arguments forwarding (`*`).",
          "Use anonymous keyword arguments forwarding (`**`).",
          "Use anonymous block arguments forwarding (`&`)."
        ].freeze

        def self.cop_name = "Style/ArgumentsForwarding"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Naming::BlockForwarding, RuboCop::Cop::Style::MethodDefParentheses]
        end

        # `[[target_ruby, allow_only_rest, use_anon, explicit_block],
        #   [redundant_rest, redundant_kwrest, redundant_block]]`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          block_forwarding = config.for_enabled_cop("Naming/BlockForwarding")
          nums = [
            (config.target_ruby_version * 10).round,
            cop_config.fetch("AllowOnlyRestArgument", true) ? 1 : 0,
            cop_config.fetch("UseAnonymousForwarding", false) ? 1 : 0,
            block_forwarding["EnforcedStyle"] == "explicit" ? 1 : 0
          ]
          lists = [
            Array(cop_config.fetch("RedundantRestArgumentNames", [])),
            Array(cop_config.fetch("RedundantKeywordRestArgumentNames", [])),
            Array(cop_config.fetch("RedundantBlockArgumentNames", []))
          ]
          [nums, lists]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(bundle_eligible? ? processed_source.raw_source : buffer.source)

          resolved_offenses.each do |start, fin, message_idx, ops|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MESSAGES[message_idx]) do |corrector|
              apply_ops(corrector, buffer, off, ops)
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :arguments_forwarding)
          else
            nums, lists = self.class.bundle_args(config)
            Shirobai.check_arguments_forwarding(
              processed_source.buffer.source,
              nums[0], nums[1] == 1, nums[2] == 1, nums[3] == 1,
              lists[0], lists[1], lists[2]
            )
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
            end
          end
        end
      end
    end
  end
end
