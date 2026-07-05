# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/FrozenStringLiteralComment`.
      #
      # Rust reproduces the whole cop from the file's leading bytes: the
      # `leading_comment_lines` frozen-string-literal facts (`exists?` /
      # `specified?` / `enabled?`), the comment-token search that fixes the
      # offense location for `never` / `always_true`, and the
      # `last_special_comment` insertion point (shebang at token 0, then the
      # `Style/Encoding` UTF-8 pattern on the next token). It returns at most
      # one packed offense; the wrapper rebuilds the range and corrector with
      # stock's own helpers (`source_range`, `range_with_surrounding_space`,
      # `line_range`), so detection and autocorrect are byte-identical.
      #
      # Why replace it: this is one of the last enabled cops that forces
      # RuboCop's Ruby side to build the (lazily converted, expensive)
      # parser-gem token list. Everything this cop reads lives in the leading
      # bytes, so the Rust scan needs no parser tokens at all — removing the
      # whole token-list cost. The cop only supports Ruby >= 2.3, gated here
      # exactly like the stock cop (`minimum_target_ruby_version 2.3`).
      class FrozenStringLiteralComment < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector
        extend RuboCop::Cop::TargetRubyVersion

        minimum_target_ruby_version 2.3

        MSG_MISSING_TRUE = "Missing magic comment `# frozen_string_literal: true`."
        MSG_MISSING = "Missing frozen string literal comment."
        MSG_UNNECESSARY = "Unnecessary frozen string literal comment."
        MSG_DISABLED = "Frozen string literal comment must be set to `true`."

        FROZEN_STRING_LITERAL_ENABLED = "# frozen_string_literal: true"
        FROZEN_STRING_LITERAL_EMACS = "# -*- frozen_string_literal: true -*-"

        # Offense kinds (mirror `frozen_string_literal_comment.rs`).
        KIND_MISSING = 0
        KIND_MISSING_TRUE = 1
        KIND_UNNECESSARY = 2
        KIND_DISABLED = 3

        STYLES = {
          "always" => 0,
          "never" => 1,
          "always_true" => 2
        }.freeze

        def self.cop_name = "Style/FrozenStringLiteralComment"
        def self.badge = RuboCop::Cop::Badge.parse("Style/FrozenStringLiteralComment")

        # Packed config nums: `[style]`. An unrecognized `EnforcedStyle`
        # defaults to `always` here; the genuine error is raised by the `style`
        # accessor in `on_new_investigation`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [[STYLES.fetch(cop_config["EnforcedStyle"] || "always", 0)]]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          src = buffer.source
          off = SourceOffsets.for(src)

          resolved_result.each do |kind, start, fin, line, insert_line, is_emacs|
            case kind
            when KIND_MISSING, KIND_MISSING_TRUE
              message = kind == KIND_MISSING ? MSG_MISSING : MSG_MISSING_TRUE
              add_offense(source_range(buffer, 0, 0), message: message) do |corrector|
                insert_comment(corrector, insert_line)
              end
            when KIND_UNNECESSARY
              range = Parser::Source::Range.new(buffer, off[start], off[fin])
              add_offense(range, message: MSG_UNNECESSARY) do |corrector|
                corrector.remove(range_with_surrounding_space(range, side: :right))
              end
            when KIND_DISABLED
              range = Parser::Source::Range.new(buffer, off[start], off[fin])
              add_offense(range, message: MSG_DISABLED) do |corrector|
                replacement = is_emacs.zero? ? FROZEN_STRING_LITERAL_ENABLED : FROZEN_STRING_LITERAL_EMACS
                corrector.replace(processed_source.buffer.line_range(line), replacement)
              end
            end
          end
        end

        private

        def resolved_result
          # Validate `EnforcedStyle` through the genuine accessor: stock raises
          # `RuntimeError` for an unrecognized style before deriving config.
          style

          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :frozen_string_literal_comment)
          else
            nums = self.class.bundle_args(config).first
            Shirobai.check_frozen_string_literal_comment(processed_source.buffer.source, nums.first)
          end
        end

        # Mirrors stock's `insert_comment`: after the last special comment's
        # line (`insert_line >= 1`), or prepended at the file start (0).
        def insert_comment(corrector, insert_line)
          if insert_line.zero?
            corrector.insert_before(processed_source.buffer.source_range, "#{FROZEN_STRING_LITERAL_ENABLED}\n")
          else
            corrector.insert_after(processed_source.buffer.line_range(insert_line), "\n#{FROZEN_STRING_LITERAL_ENABLED}")
          end
        end
      end
    end
  end
end
