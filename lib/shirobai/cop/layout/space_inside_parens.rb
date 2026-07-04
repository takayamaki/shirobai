# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceInsideParens`.
      #
      # Stock iterates `sorted_tokens` pairwise; Rust reconstructs the pair
      # facts around every unmasked `(` / `)` byte (strings, comments, char
      # literals and `__END__` data are the only non-token paren bytes) plus
      # one AST-collected token fact: the `tLPAREN_ARG` positions (the
      # space-separated first-argument paren of a parenless call, which is
      # not `left_parens?` and never fires the left-side checks).
      #
      # Each offense is `[start, end, code]`: code 0 removes the range
      # ("Space inside parentheses detected."), code 1 inserts a space before
      # it ("No space inside parentheses detected.").
      class SpaceInsideParens < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MESSAGES = [
          "Space inside parentheses detected.",
          "No space inside parentheses detected."
        ].freeze

        STYLES = { "no_space" => 0, "space" => 1, "compact" => 2 }.freeze

        def self.cop_name = "Layout/SpaceInsideParens"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[[EnforcedStyle], []]`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [[STYLES.fetch(cop_config["EnforcedStyle"] || "no_space")], []]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start, fin, code|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MESSAGES.fetch(code)) do |corrector|
              if code.zero?
                corrector.remove(range)
              else
                corrector.insert_before(range, " ")
              end
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_inside_parens)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_space_inside_parens(processed_source.buffer.source, nums[0])
          end
        end

        # Eligible only when the parser-normalized buffer source is
        # byte-identical to the raw source the bundle scans (CRLF/BOM
        # fallback scans `buffer.source`). Memoized per investigation.
        def bundle_eligible?
          return @bundle_eligible unless @bundle_eligible.nil?

          @bundle_eligible = processed_source.buffer.source == processed_source.raw_source
        end
      end
    end
  end
end
