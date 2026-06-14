# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/MultilineMethodCallBraceLayout`.
      #
      # Rust walks the AST once, classifies every parenthesized `send` / `csend`
      # by the open/close line layout per `EnforcedStyle` (`symmetrical`,
      # `new_line`, `same_line`), and returns the close `)` offense range, the
      # message-code (`SAME_LINE` / `NEW_LINE` / `ALWAYS_NEW_LINE` /
      # `ALWAYS_SAME_LINE`), the send node's `source_range.begin_pos` and a
      # `correctable` flag (false when stock's
      # `new_line_needed_before_closing_brace?` fires — comment after the last
      # element AND the call is chained / an argument). Ruby builds the offense,
      # re-resolves the parser-gem node from `processed_source.ast.each_node`
      # (single linear walk per offense — autocorrect-eligible offenses per file
      # are tiny in real corpora) and delegates the autocorrect to stock's
      # `MultilineLiteralBraceCorrector` so the byte-exact behaviour is
      # preserved (heredoc-chain relocation, comment-block reflow, trailing
      # commas).
      class MultilineMethodCallBraceLayout < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        STYLES = { "symmetrical" => 0, "new_line" => 1, "same_line" => 2 }.freeze

        # Re-export the stock message constants so vendor specs that read
        # `described_class::SAME_LINE_MESSAGE` etc. resolve them on the wrapper
        # (the shared examples are run with `described_class` == this class).
        SAME_LINE_MESSAGE = RuboCop::Cop::Layout::MultilineMethodCallBraceLayout::SAME_LINE_MESSAGE
        NEW_LINE_MESSAGE = RuboCop::Cop::Layout::MultilineMethodCallBraceLayout::NEW_LINE_MESSAGE
        ALWAYS_NEW_LINE_MESSAGE = RuboCop::Cop::Layout::MultilineMethodCallBraceLayout::ALWAYS_NEW_LINE_MESSAGE
        ALWAYS_SAME_LINE_MESSAGE = RuboCop::Cop::Layout::MultilineMethodCallBraceLayout::ALWAYS_SAME_LINE_MESSAGE

        MESSAGES = [
          SAME_LINE_MESSAGE,
          NEW_LINE_MESSAGE,
          ALWAYS_NEW_LINE_MESSAGE,
          ALWAYS_SAME_LINE_MESSAGE
        ].freeze

        def self.cop_name = "Layout/MultilineMethodCallBraceLayout"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[[style], []]`. Unknown styles
        # default to symmetrical (0); stock raises `Unknown EnforcedStyle` lazily
        # from `ConfigurableEnforcedStyle#style`, so we surface the same error
        # before the bundle run if the configured value is invalid.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [[STYLES.fetch(cop_config["EnforcedStyle"] || "symmetrical", 0)], []]
        end

        def on_new_investigation
          # Trigger `ConfigurableEnforcedStyle`'s `style` lookup once so an
          # unknown EnforcedStyle raises identically to stock before any
          # offense is emitted.
          _ = style

          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          offenses = offenses_for_source

          offenses.each do |start, fin, msg_code, send_start, send_end, correctable|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            message = MESSAGES.fetch(msg_code)
            if correctable
              add_offense(range, message: message) do |corrector|
                # `(send_start, send_end)` are Rust BYTE offsets — convert to
                # parser-gem CHAR offsets via SourceOffsets to match what
                # `node.source_range.begin_pos/end_pos` actually report.
                node = node_for_send_range(off[send_start], off[send_end])
                next unless node

                RuboCop::Cop::MultilineLiteralBraceCorrector.correct(
                  corrector, node, processed_source
                )
              end
            else
              # Stock emits the offense but returns BEFORE touching `corrector`
              # so `correctable?` stays false. Calling `add_offense` without a
              # block matches.
              add_offense(range, message: message)
            end
          end
        end

        private

        def offenses_for_source
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :multiline_method_call_brace_layout)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_multiline_method_call_brace_layout(
              processed_source.raw_source, nums[0]
            )
          end
        end

        # No per-investigation state and the config is a single u8. Kept for
        # symmetry with the other wrappers.
        def bundle_eligible?
          true
        end

        # Locate the parser-gem `send` / `csend` node whose source range matches
        # `[s, e)` exactly. We need BOTH ends because chained calls
        # (`foo(...).bar`) share `begin_pos` with the inner call — pre-order
        # `each_node` would otherwise return the OUTER `.bar` send, which has
        # no own `loc.begin`/`loc.end` for braces and crashes the corrector.
        # The pair pins us to the inner brace-bearing send the Rust rule chose.
        def node_for_send_range(s, e)
          root = processed_source.ast
          return nil unless root

          root.each_node(:send, :csend).find do |node|
            sr = node.source_range
            sr && sr.begin_pos == s && sr.end_pos == e
          end
        end
      end
    end
  end
end
