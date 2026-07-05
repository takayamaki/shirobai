# frozen_string_literal: true

module Shirobai
  module Cop
    module Performance
      # Drop-in Rust reimplementation of `Performance/StartWith`
      # (rubocop-performance 1.26.1).
      #
      # Mirror image of `Shirobai::Cop::Performance::EndWith`: the Rust
      # side gates on `literal_at_start?` (`\A`, or `^` when
      # `SafeMultiline` is false) and the wrapper drops the anchor with the
      # stock `drop_start_metacharacter` before rebuilding the replacement
      # with RuboCop's own string helpers.
      class StartWith < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::RegexpMetacharacter
        extend RuboCop::Cop::AutoCorrector

        MSG = "Use `String#start_with?` instead of a regex match anchored " \
              "to the beginning of the string."

        def self.cop_name = "Performance/StartWith"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[safe_multiline]` (0/1).
        def self.bundle_args(config)
          [config.for_badge(badge).fetch("SafeMultiline", true) ? 1 : 0]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start, fin, recv_start, recv_end, dot, content|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range) do |corrector|
              receiver_source =
                Parser::Source::Range.new(buffer, off[recv_start], off[recv_end]).source
              literal = to_string_literal(
                interpret_string_escapes(drop_start_metacharacter(content))
              )
              corrector.replace(range, "#{receiver_source}#{dot}start_with?(#{literal})")
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :perf_start_with)
          else
            Shirobai.check_perf_start_with(
              processed_source.buffer.source, self.class.bundle_args(config)[0] == 1
            )
          end
        end
      end
    end
  end
end
