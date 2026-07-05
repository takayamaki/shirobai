# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/IfUnlessModifier`.
      #
      # Rust walks every `if`/`unless` once and returns, in walk order, the
      # candidates for both directions with everything byte/AST-shaped already
      # decided: shape eligibility, the reconstructed modifier form and both
      # of its lengths (with/without the first-line comment), and the exact
      # corrector ops for the block-form rewrite. Ruby finishes the two
      # regex-dependent decisions (the magnus rule keeps regexes here):
      #
      # - `comment_disables_cop?` on the first-line comment picks the
      #   with/without-comment variant (it changes the modifier-form length
      #   and the replacement text), and
      # - the `Layout/LineLength` exemptions prune the "too long" direction
      #   (`AllowedPatterns`, cop directives, `AllowURI`, per-line disables
      #   via `comment_config`), reusing the stock mixins.
      #
      # The wrapper also replays stock's `ignore_node` bookkeeping with plain
      # char ranges: the store persists across autocorrect passes (stock's
      # `@ignored_nodes` is never reset), and containment is the same numeric
      # begin/end comparison stock does against stale ranges.
      #
      # Bundle-eligible whenever the parser buffer equals the raw source
      # (CRLF/BOM sources fall back to the standalone entry point over
      # `buffer.source` so every offset lines up with parser positions).
      class IfUnlessModifier < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::AllowedPattern
        include RuboCop::Cop::RangeHelp
        include RuboCop::Cop::LineLengthHelp
        extend RuboCop::Cop::AutoCorrector

        MSG_USE_MODIFIER = "Favor modifier `%<keyword>s` usage when having a " \
                           "single-line body. Another good alternative is " \
                           "the usage of control flow `&&`/`||`."
        MSG_USE_NORMAL = "Modifier form of `%<keyword>s` makes the line too long."

        def self.cop_name = "Style/IfUnlessModifier"
        def self.badge = RuboCop::Cop::Badge.parse("Style/IfUnlessModifier")

        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Style::Next, RuboCop::Cop::Style::SoleNestedConditional]
        end

        # Packed args for the bundled run: `[max_line_length, tab_width]`.
        # `max_line_length` mirrors `AutocorrectLogic#max_line_length`
        # (`-1` when `Layout/LineLength` is disabled); `tab_width` mirrors
        # `LineLengthHelp#tab_indentation_width` resolved with THIS cop's
        # `cop_config` (the `configured_indentation_width` fallback).
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          max = if config.cop_enabled?("Layout/LineLength")
                  config.for_cop("Layout/LineLength")["Max"] || 120
                else
                  -1
                end
          tab = config.for_cop("Layout/IndentationStyle")["IndentationWidth"] ||
                cop_config["IndentationWidth"] ||
                config.for_cop("Layout/IndentationWidth")["Width"] || 2
          [max, tab]
        end

        def on_new_investigation
          # Mirrors stock's `@ignored_nodes`: never reset between passes.
          @ignored_ranges ||= []
          candidates = resolved_result
          return if candidates.empty?

          candidates.each do |candidate|
            kind = candidate[0]
            if kind.zero?
              check_use_modifier(candidate)
            else
              check_too_long(candidate)
            end
          end
        end

        private

        def resolved_result
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :if_unless_modifier)
          else
            Shirobai.check_if_unless_modifier(rust_source, self.class.bundle_args(config))
          end
        end

        def rust_source
          bundle_eligible? ? processed_source.raw_source : processed_source.buffer.source
        end

        # Direction 1: multiline if/unless that would fit as a modifier.
        def check_use_modifier(candidate)
          _, kw_s, kw_e, node_s, node_e, flags, c_s, c_e, repl_no, repl_with, = candidate
          if flags.anybits?(4) # first-line comment present
            comment = rust_source.byteslice(c_s, c_e - c_s)
            if comment_disables_cop?(comment)
              fits = flags.anybits?(16)
              replacement = repl_no
            else
              # `first_line_comment(node) && code_after(node)` rejection.
              return if flags.anybits?(8)

              fits = flags.anybits?(32)
              replacement = repl_with
            end
          else
            fits = flags.anybits?(16)
            replacement = repl_no
          end
          return unless fits

          keyword = flags.anybits?(1) ? "unless" : "if"
          add_candidate_offense(kw_s, kw_e, node_s, node_e, flags,
                                format(MSG_USE_MODIFIER, keyword: keyword)) do |corrector, node_range|
            corrector.replace(node_range, replacement)
          end
        end

        # Direction 2: single-line modifier form that is too long. Rust only
        # guarantees the raw length check; the `Layout/LineLength` regex
        # exemptions are applied here, in stock's order.
        def check_too_long(candidate)
          _, kw_s, kw_e, node_s, node_e, flags, _, _, _, _, line_no, ops = candidate
          return unless line_length_enabled_at_line?(line_no)

          line = processed_source.lines[line_no - 1]
          return if matches_allowed_pattern?(line)

          if allow_cop_directives? && directive_on_source_line?(line_no - 1)
            return unless line_length_without_directive(line) > max_line_length
          elsif allow_uri?
            uri_range = find_excessive_range(line, :uri)
            return if uri_range && allowed_position?(line, uri_range)
          end

          keyword = flags.anybits?(1) ? "unless" : "if"
          buffer = processed_source.buffer
          off = SourceOffsets.for(rust_source)
          add_candidate_offense(kw_s, kw_e, node_s, node_e, flags,
                                format(MSG_USE_NORMAL, keyword: keyword)) do |corrector, _node_range|
            ops.each do |op_kind, s, e, text|
              range = Parser::Source::Range.new(buffer, off[s], off[e])
              if op_kind.zero?
                corrector.replace(range, text)
              else
                corrector.remove(range)
              end
            end
          end
        end

        # Shared offense/correction skeleton: stock's corrector block guards
        # (`part_of_ignored_node?`, `another_modifier_if_on_same_line?`) and
        # the `ignore_node` bookkeeping.
        def add_candidate_offense(kw_s, kw_e, node_s, node_e, flags, message)
          buffer = processed_source.buffer
          off = SourceOffsets.for(rust_source)
          kw_range = Parser::Source::Range.new(buffer, off[kw_s], off[kw_e])
          node_b = off[node_s]
          node_e2 = off[node_e]
          add_offense(kw_range, message: message) do |corrector|
            next if part_of_ignored_range?(node_b, node_e2)
            next if flags.anybits?(2) # another_modifier_if_on_same_line?

            node_range = Parser::Source::Range.new(buffer, node_b, node_e2)
            yield(corrector, node_range)
            @ignored_ranges << [node_b, node_e2]
          end
        end

        # `IgnoredNode#part_of_ignored_node?`: numeric containment against the
        # stored (possibly stale, from an earlier pass) ranges.
        def part_of_ignored_range?(node_b, node_e)
          @ignored_ranges.any? { |b, e| b <= node_b && e >= node_e }
        end

        # --- stock private methods reused verbatim ---

        def line_length_enabled_at_line?(line)
          processed_source.comment_config.cop_enabled_at_line?("Layout/LineLength", line)
        end

        def allowed_patterns
          line_length_config = config.for_cop("Layout/LineLength")
          line_length_config["AllowedPatterns"] || line_length_config["IgnoredPatterns"] || []
        end

        def comment_disables_cop?(comment)
          regexp_pattern = "# rubocop : (disable|todo) ([^,],)* (all|#{cop_name})"
          Regexp.new(regexp_pattern.gsub(" ", '\s*')).match?(comment)
        end

        # Share the `URISchemes` regexp across cop instances (stock rebuilds
        # it per file); the derivation is identical to `LineLengthHelp`.
        def uri_regexp
          @uri_regexp ||= Layout::LineLength.uri_regexp_for(config.for_cop("Layout/LineLength"))
        end
      end
    end
  end
end
