# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceAroundOperators`.
      #
      # A hybrid cop: core detection is AST-driven (Rust walks the AST once,
      # reproducing stock's per-node callbacks and `range_with_surrounding_space`
      # logic), while the `AllowForAlignment` (default true) check for the
      # excess-space arm consults the parser-gem token stream. Rust does both —
      # the AST walk in the bundle's walk-outer phase, the alignment filter over
      # the lexed token list — and returns one record per offense:
      # `[op_start, op_end, ws_start, ws_end, kind, operator, replacement]`.
      #
      # `[op_start, op_end)` is the operator range (the offense highlight). `kind`
      # selects the message (0 = "Space around operator detected", 1 =
      # "Surrounding space missing", 2 = "should be surrounded by a single
      # space"). The autocorrect replaces the `range_with_surrounding_space`
      # range `[ws_start, ws_end)` with `replacement` (Rust computes the
      # replacement string exactly as stock's `autocorrect` would, including the
      # `**` / rational `/` / line-continuation / `ForceEqualSignAlignment`
      # arms).
      #
      # Config inputs: `EnforcedStyleForExponentOperator` /
      # `EnforcedStyleForRationalLiterals` / `AllowForAlignment` (this cop) plus
      # `Layout/HashAlignment`'s `EnforcedHashRocketStyle` (the `table` guard) and
      # `Layout/ExtraSpacing`'s `ForceEqualSignAlignment` (the autocorrect
      # collision avoidance). All are purely config-driven, so the cop is always
      # bundle eligible — except when `raw_source != buffer.source` (CRLF/BOM
      # normalization), where the token byte positions would not line up and it
      # falls back to a standalone lex of `buffer.source`. (CRLF/buffer.source
      # trap, `aee0e8e`.)
      class SpaceAroundOperators < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        # Stock declares this; preserved so the autocorrect ordering against
        # `Style/SelfAssignment` matches.
        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Style::SelfAssignment]
        end

        def self.cop_name = "Layout/SpaceAroundOperators"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed config nums: `[enabled, exponent_style, rational_style,
        # allow_for_alignment, hash_table_style, force_equal_sign_alignment]`.
        #
        # `enabled` is the token-cop gate the bundle reads to decide whether to
        # collect the parser-gem token stream at all (this is a token cop). It is
        # a superset of "this cop runs": a false positive only costs the token
        # pass on a file the cop never reads, never a wrong offense; a false
        # negative would drop offenses, so we only turn it off when `Enabled` is
        # literally `false`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          enabled = cop_config["Enabled"] == false ? 0 : 1
          exponent = cop_config["EnforcedStyleForExponentOperator"] == "space" ? 1 : 0
          rational = cop_config["EnforcedStyleForRationalLiterals"] == "space" ? 1 : 0
          allow = cop_config.fetch("AllowForAlignment", true) ? 1 : 0

          hash_styles = Array(config.for_cop("Layout/HashAlignment")["EnforcedHashRocketStyle"])
          table = hash_styles.include?("table") ? 1 : 0

          force = config.for_cop("Layout/ExtraSpacing")["ForceEqualSignAlignment"] ? 1 : 0
          [[enabled, exponent, rational, allow, table, force]]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(bundle_eligible? ? processed_source.raw_source : buffer.source)

          resolved_result.each do |op_start, op_end, ws_start, ws_end, kind, operator, replacement|
            op_range = Parser::Source::Range.new(buffer, off[op_start], off[op_end])
            ws_range = Parser::Source::Range.new(buffer, off[ws_start], off[ws_end])
            message = message_for(kind, operator)
            add_offense(op_range, message: message) do |corrector|
              corrector.replace(ws_range, replacement)
            end
          end
        end

        private

        MSG_DETECTED = "Space around operator `%<op>s` detected."
        MSG_MISSING = "Surrounding space missing for operator `%<op>s`."
        MSG_SINGLE = "Operator `%<op>s` should be surrounded by a single space."

        def message_for(kind, operator)
          case kind
          when 0 then format(MSG_DETECTED, op: operator)
          when 1 then format(MSG_MISSING, op: operator)
          else format(MSG_SINGLE, op: operator)
          end
        end

        def resolved_result
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_around_operators)
          else
            # nums[0] is the token-cop gate (only the bundle reads it); the
            # standalone fallback runs unconditionally, so skip it and pass the
            # five config nums.
            nums = self.class.bundle_args(config).first
            Shirobai.check_space_around_operators(
              processed_source.buffer.source,
              nums[1], nums[2], nums[3] != 0, nums[4] != 0, nums[5] != 0
            )
          end
        end

        def bundle_eligible?
          return @bundle_eligible unless @bundle_eligible.nil?

          @bundle_eligible = processed_source.buffer.source == processed_source.raw_source
        end
      end
    end
  end
end
