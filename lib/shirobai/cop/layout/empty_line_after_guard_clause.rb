# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/EmptyLineAfterGuardClause`.
      #
      # Rust walks the AST and emits a candidate offense for every `if`/`unless`
      # whose `if_branch` is a guard clause (`raise`/`fail`/`return`/`break`/
      # `next`, with `and`/`or` peeled to the rhs) UNLESS one of stock's
      # `correct_style?` gates fires:
      #
      # - `node.parent` is `nil`, `rescue`, or `ensure`,
      # - `node.right_sibling` is `nil` or sits under an `if` with `else`,
      # - the right sibling is itself a guard-bearing `if`/`unless`.
      #
      # Each candidate carries (a) the offense range stock passes to
      # `add_offense` (the heredoc closer, the `end` keyword for a multi-line
      # if/unless, or the whole node for the modifier form), and (b) the byte
      # range stock's `range_by_whole_lines` spans plus the 1-based
      # `last_line` driving the `next_line_empty?` check.
      #
      # The wrapper finishes the directive-comment check
      # (`next_line_empty_or_allowed_directive_comment?`): a blank line OR a
      # `# rubocop:enable` / `# :nocov:` / `# simplecov:disable` /
      # `# simplecov:enable` comment whose own next line is blank suppresses
      # the offense.  The autocorrect inserts `"\n"` after the guard's
      # whole-line range, or after the directive comment line when present —
      # byte-for-byte stock corrector behaviour.
      class EmptyLineAfterGuardClause < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "Add empty line after guard clause."
        SIMPLECOV_COMMENT_PATTERN = /\A#\s*(?::nocov:|simplecov\s*:\s*(?:disable|enable)\b)/

        def self.cop_name = "Layout/EmptyLineAfterGuardClause"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          [] # config-less
        end

        def on_new_investigation
          buffer = processed_source.buffer
          candidates = Dispatch.offenses_for(processed_source, config, :empty_line_after_guard_clause)
          return if candidates.empty?

          off = SourceOffsets.for(processed_source.raw_source)
          candidates.each do |offense_start, offense_end, anchor_first_byte, anchor_last_line|
            # `next_line_empty_or_allowed_directive_comment?(anchor_last_line)`:
            # blank below, OR (directive comment immediately below AND blank
            # below that).
            next_line = anchor_last_line + 1
            next if next_line_blank?(next_line)

            directive_target_line = nil
            if next_line_allowed_directive_comment?(next_line)
              # Comment line is below the guard.  Stock then checks the line
              # below the comment: blank suppresses.
              after_directive = next_line + 1
              next if next_line_blank?(after_directive)
              directive_target_line = next_line
            end

            range = Parser::Source::Range.new(buffer, off[offense_start], off[offense_end])
            add_offense(range, message: MSG) do |corrector|
              autocorrect(corrector, buffer, off, anchor_first_byte, anchor_last_line, directive_target_line)
            end
          end
        end

        private

        # `processed_source[line - 1].blank?`: `nil.blank?` is true (line past
        # the end of source = blank), so a missing line suppresses the
        # offense.
        def next_line_blank?(line1)
          content = processed_source[line1 - 1]
          content.nil? || content.strip.empty?
        end

        # `processed_source.comment_at_line(line)` returns a Parser::Source::Comment
        # whose text we feed through `DirectiveComment.new(...).enabled?` and
        # the SimpleCov pattern.
        def next_line_allowed_directive_comment?(line1)
          comment = processed_source.comment_at_line(line1)
          return false unless comment

          RuboCop::DirectiveComment.new(comment).enabled? ||
            SIMPLECOV_COMMENT_PATTERN.match?(comment.text)
        end

        def autocorrect(corrector, buffer, off, anchor_first_byte, anchor_last_line, directive_line)
          if directive_line
            # Insert after the comment line on `directive_line`.
            comment = processed_source.comment_at_line(directive_line)
            anchor = comment.source_range
            corrector.insert_after(anchor, "\n")
            return
          end
          # `range_by_whole_lines(...)` — start of anchor's first line to end
          # of anchor's last-line content (no `\n`).
          last_line_content = processed_source[anchor_last_line - 1] || ""
          line_byte_start = buffer.line_range(anchor_last_line).begin_pos
          line_byte_end = line_byte_start + last_line_content.length
          first_char = off[anchor_first_byte]
          # `line_range` returns char range already; we already converted
          # `anchor_first_byte` and used `line_range` char-aware end.
          range = Parser::Source::Range.new(buffer, first_char, line_byte_end)
          corrector.insert_after(range, "\n")
        end
      end
    end
  end
end
