# frozen_string_literal: true

module Shirobai
  module Cop
    module Performance
      # Drop-in Rust reimplementation of `Performance/EndWith`
      # (rubocop-performance 1.26.1).
      #
      # Rust replicates the stock pattern union (regexp argument first,
      # then regexp receiver; `&.` only on the argument side) with the
      # `literal_at_end?` gate: content is `LITERAL_REGEX`-only and anchored
      # with `\z` (or `$` when `SafeMultiline` is false). The wrapper
      # rebuilds the replacement exactly like stock — with the
      # rubocop-performance `RegexpMetacharacter` mixin's
      # `drop_end_metacharacter` and RuboCop's own
      # `interpret_string_escapes` / `to_string_literal` — so anchor
      # dropping, escape interpretation and quote selection cannot drift.
      class EndWith < RuboCop::Cop::Base
        include RuboCop::Cop::RegexpMetacharacter
        extend RuboCop::Cop::AutoCorrector

        MSG = "Use `String#end_with?` instead of a regex match anchored " \
              "to the end of the string."

        def self.cop_name = "Performance/EndWith"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[safe_multiline]` (0/1).
        def self.bundle_args(config)
          [config.for_badge(badge).fetch("SafeMultiline", true) ? 1 : 0]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :perf_end_with)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |start, fin, recv_start, recv_end, dot, content|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range) do |corrector|
              receiver_source =
                Parser::Source::Range.new(buffer, off[recv_start], off[recv_end]).source
              literal = to_string_literal(
                interpret_string_escapes(drop_end_metacharacter(content))
              )
              corrector.replace(range, "#{receiver_source}#{dot}end_with?(#{literal})")
            end
          end
        end
      end
    end
  end
end
