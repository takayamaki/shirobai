# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Shared Ruby side of the RSpec empty-line family
      # (`RSpec/EmptyLineAfter{Example,ExampleGroup,FinalLet,Hook,Subject}`).
      #
      # The Rust rule (`rspec_empty_line.rs`) owns concept classification,
      # `last_child?` and the heredoc-aware `final_end_location(node).line`; it
      # emits, per cop, `[final_end_line, method_name]` for every candidate that
      # clears those gates. This module replays the REST of stock's
      # `EmptyLineSeparation#missing_separating_line` verbatim — the trailing
      # comment walk, the enabled-`# rubocop:enable` directive tracking, the
      # blank-line suppression, the offense location and the `"\n"` autocorrect
      # — over the same `ProcessedSource`, so those parts are byte-for-byte
      # identical to stock.
      module EmptyLineSeparationSupport
        include RuboCop::Cop::RangeHelp

        # `offenses` is the Rust wire shape `[[final_end_line, method_name],
        # ...]`. The block turns a method name into the per-cop message.
        def emit_empty_line_offenses(offenses)
          offenses.each do |final_end_line, method_name|
            add_missing_separating_line(final_end_line, yield(method_name))
          end
        end

        private

        # stock `EmptyLineSeparation#missing_separating_line`.
        def add_missing_separating_line(final_end_line, message)
          line = final_end_line
          enable_directive_line = nil
          while processed_source.line_with_comment?(line + 1)
            line += 1
            comment = processed_source.comment_at_line(line)
            enable_directive_line = line if RuboCop::DirectiveComment.new(comment).enabled?
          end

          return if processed_source[line].blank?

          location = offending_loc(enable_directive_line || final_end_line)
          add_offense(location, message: message) do |corrector|
            corrector.insert_after(location.end, "\n")
          end
        end

        # stock `EmptyLineSeparation#offending_loc`.
        def offending_loc(last_line)
          offending_line = processed_source[last_line - 1]

          content_length = offending_line.lstrip.length
          start = offending_line.length - content_length

          source_range(processed_source.buffer, last_line, start, content_length)
        end
      end
    end
  end
end
