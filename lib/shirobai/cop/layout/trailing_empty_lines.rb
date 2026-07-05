# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/TrailingEmptyLines`.
      #
      # The cop touches no AST: stock's `on_new_investigation` looks only at the
      # source buffer's trailing whitespace. Rust performs the whole byte scan and
      # returns at most one record — the reported caret range, the autocorrect
      # range and its replacement text, and the `blank_lines` count that selects
      # the message. Ruby reports the offense and applies the single
      # `corrector.replace`, exactly like stock.
      #
      # `EnforcedStyle` (`final_newline` wants 0 trailing blank lines,
      # `final_blank_line` wants 1) is validated through the genuine
      # `ConfigurableEnforcedStyle#style` accessor before the bundle config is
      # derived, so an unrecognized style raises the same error stock would.
      #
      # Stock reads `processed_source.buffer.source`, which the parser normalizes
      # (CRLF `\r\n` -> `\n`, BOM stripping). The bundle, like every cop, runs on
      # `raw_source`; when the parser normalized the source those two differ and
      # the trailing-whitespace byte positions would not line up. So this cop is
      # bundle-eligible only when the buffer source is byte-identical to the raw
      # source (the common case — no CRLF/BOM); otherwise it falls back to a
      # standalone scan of `buffer.source` with offsets converted against the same
      # string. Either way the result is purely config-driven and stateless, so
      # the autocorrect re-passes (fresh `ProcessedSource`) recompute correctly.
      class TrailingEmptyLines < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        STYLES = {
          "final_newline" => 0,
          "final_blank_line" => 1
        }.freeze

        def self.cop_name = "Layout/TrailingEmptyLines"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/TrailingEmptyLines")

        # Packed config nums: `[style]`. An unrecognized `EnforcedStyle` defaults
        # to `final_newline` here; the genuine error is raised by the `style`
        # accessor in `on_new_investigation` before this is consulted.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [[STYLES.fetch(cop_config["EnforcedStyle"] || "final_newline", 0)]]
        end

        def on_new_investigation
          # Validate `EnforcedStyle` through the genuine accessor first: stock
          # raises for an unrecognized style, and this must fire before we derive
          # the bundle config (which would otherwise mask it).
          style

          buffer = processed_source.buffer
          # Offsets are byte positions into the string the check scanned. On the
          # bundle path that is `raw_source` (byte-identical to `buffer.source`
          # there); converting against `raw_source` shares the single-slot
          # `SourceOffsets` cache with the other bundled cops. On the CRLF/BOM
          # fallback the check scanned `buffer.source`, so convert against that.
          off = SourceOffsets.for(bundle_eligible? ? processed_source.raw_source : buffer.source)

          resolved_result.each do |report_start, report_end, ac_start, ac_end, replacement, blank_lines|
            report_range = Parser::Source::Range.new(buffer, off[report_start], off[report_end])
            autocorrect_range = Parser::Source::Range.new(buffer, off[ac_start], off[ac_end])

            add_offense(report_range, message: message(blank_lines)) do |corrector|
              corrector.replace(autocorrect_range, replacement)
            end
          end
        end

        private

        # Mirrors stock's `message`. `wanted_blank_lines` is 0 for `final_newline`
        # and 1 for `final_blank_line`.
        def message(blank_lines)
          wanted_blank_lines = style == :final_newline ? 0 : 1
          case blank_lines
          when -1
            "Final newline missing."
          when 0
            "Trailing blank line missing."
          else
            instead_of = wanted_blank_lines.zero? ? "" : "instead of #{wanted_blank_lines} "
            format("%<current>d trailing blank lines %<prefer>sdetected.",
                   current: blank_lines, prefer: instead_of)
          end
        end

        def resolved_result
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :trailing_empty_lines)
          else
            nums = self.class.bundle_args(config).first
            Shirobai.check_trailing_empty_lines(processed_source.buffer.source, nums)
          end
        end
      end
    end
  end
end
