# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/EndOfLine`.
      #
      # Stock's `on_new_investigation` scans `raw_source.each_line` and reports
      # the first line whose terminator is wrong for the enforced style. Its only
      # token access is `last_line` (`processed_source.tokens.last.line`), which
      # materializes the parser-gem token stream on EVERY file — the "toucher"
      # cost. Rust supplies `last_line` (the end line of the last top-level
      # statement, from the shared parse); the wrapper runs stock's exact scan
      # body with that value injected, so detection and the offense range are
      # stock's own code, byte-identical by construction. The cop has no
      # autocorrect.
      #
      # `last_line` is line-based, so it is identical whether computed from
      # `raw_source` or the CR-stripped `buffer.source`; the bundle path (which
      # scans `raw_source`) and the standalone fallback (`buffer.source`) both
      # produce the same value. No `SourceOffsets` conversion is needed (the
      # wrapper builds ranges with stock's own `source_range`).
      class EndOfLine < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::RangeHelp

        MSG_DETECTED = "Carriage return character detected."
        MSG_MISSING = "Carriage return character missing."

        def self.cop_name = "Layout/EndOfLine"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less on the Rust side (`last_line` needs no config). Kept for the
        # 4+1 single-source-of-config convention.
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          last_line = resolved_last_line

          processed_source.raw_source.each_line.with_index do |line, index|
            break if index >= last_line

            msg = offense_message(line)
            next unless msg
            next if unimportant_missing_cr?(index, last_line, line)

            range = source_range(processed_source.buffer, index + 1, 0, line.length)

            add_offense(range, message: msg)
            # Usually there will be carriage return characters on all or none
            # of the lines in a file, so we report only one offense.
            break
          end
        end

        # Stock's `unimportant_missing_cr?`, verbatim.
        def unimportant_missing_cr?(index, last_line, line)
          style == :crlf && index == last_line - 1 && !line.end_with?("\n")
        end

        # Stock's `offense_message`, verbatim (except the fully-qualified
        # `RuboCop::Platform`).
        def offense_message(line)
          effective_style = if style == :native
                              RuboCop::Platform.windows? ? :crlf : :lf
                            else
                              style
                            end
          case effective_style
          when :lf then MSG_DETECTED if line.end_with?("\r", "\r\n")
          else MSG_MISSING unless line.end_with?("\r\n")
          end
        end

        private

        def resolved_last_line
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :end_of_line)
          else
            Shirobai.check_end_of_line(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
