# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/LineLength` (detection only).
      #
      # The per-line scan over every line of the file happens in Rust, which
      # returns only the lines that exceed `Max` (plus the heredoc delimiter for
      # lines inside a heredoc body). Ruby then applies the regex-based
      # exemptions (`AllowedPatterns`, `AllowURI`, `AllowQualifiedName`, cop
      # directives, RBS annotations) that rely on Ruby's `URI`/`Regexp`, reusing
      # the upstream mixins so the offense ranges match exactly.
      class LineLength < RuboCop::Cop::Base
        include RuboCop::Cop::CheckLineBreakable
        include RuboCop::Cop::AllowedPattern
        include RuboCop::Cop::RangeHelp
        include RuboCop::Cop::LineLengthHelp
        extend RuboCop::Cop::AutoCorrector

        exclude_limit "Max"

        MSG = "Line is too long. [%<length>d/%<max>d]"

        def self.cop_name = "Layout/LineLength"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/LineLength")

        def on_new_investigation
          source = processed_source.raw_source
          candidates = Shirobai.check_line_length(source, max, tab_indentation_width || 0)

          # Breakable (autocorrection) data is only ever consumed for lines that
          # become offenses, and only candidate lines (length > Max) can. So we
          # restrict the expensive break-point computation to candidate lines —
          # the result for those lines is identical to computing it for all.
          candidate_line_indexes = candidates.map { |candidate| candidate[0] }
          compute_breakables(source, candidate_line_indexes)

          candidates.each do |candidate|
            line_index, length, _line_start, _line_end, _indent_diff, heredoc_delimiters = candidate
            line = processed_source.lines[line_index]
            check_candidate(line, line_index, length, heredoc_delimiters)
          end
        end

        private

        # Build the per-line autocorrection data (insertion byte offset and,
        # for `SplitStrings`, the string delimiter) for the given candidate
        # lines. Mirrors upstream's `breakable_range_by_line_index` /
        # `breakable_string_delimiters`.
        def compute_breakables(source, candidate_line_indexes)
          buffer = processed_source.buffer
          @breakable_range_by_line_index = {}
          @breakable_string_delimiters = {}
          breakables = Shirobai.check_line_length_breakables(
            source, max, !!allow_string_split?, candidate_line_indexes
          )
          breakables.each do |entry|
            line_index, insert_offset, delimiter = entry
            @breakable_range_by_line_index[line_index] =
              Parser::Source::Range.new(buffer, insert_offset, insert_offset + 1)
            @breakable_string_delimiters[line_index] = delimiter unless delimiter.empty?
          end
        end

        def allow_string_split?
          cop_config["SplitStrings"]
        end

        def check_candidate(line, line_index, length, heredoc_delimiters)
          return if allowed_candidate?(line, line_index, heredoc_delimiters)

          if allow_rbs_inline_annotation? && rbs_inline_annotation_on_source_line?(line_index)
            return
          end

          if allow_cop_directives? && directive_on_source_line?(line_index)
            return check_directive_line(line, line_index)
          end

          if allow_uri? || allow_qualified_name?
            return check_line_for_exemptions(line, line_index)
          end

          register_offense(excess_range(nil, line, line_index), line, line_index, length: length)
        end

        def allowed_candidate?(line, line_index, heredoc_delimiters)
          matches_allowed_pattern?(line) ||
            shebang?(line, line_index) ||
            permitted_heredoc?(heredoc_delimiters)
        end

        def permitted_heredoc?(heredoc_delimiters)
          return false if heredoc_delimiters.empty?
          return false unless allowed_heredoc

          allowed_heredoc == true ||
            heredoc_delimiters.any? { |delimiter| allowed_heredoc.include?(delimiter) }
        end

        def shebang?(line, line_index)
          line_index.zero? && line.start_with?("#!")
        end

        def register_offense(loc, line, line_index, length: line_length(line))
          message = format(MSG, length: length, max: max)
          breakable_range = @breakable_range_by_line_index[line_index]

          add_offense(loc, message: message) do |corrector|
            self.max = line_length(line)

            insertion = if (delimiter = @breakable_string_delimiters[line_index])
                          [delimiter, " \\\n", delimiter].join
                        else
                          "\n"
                        end

            corrector.insert_before(breakable_range, insertion) unless breakable_range.nil?
          end
        end

        def excess_range(uri_range, line, line_index)
          excessive_position = if uri_range && uri_range.begin < max
                                 uri_range.end
                               else
                                 highlight_start(line)
                               end

          source_range(processed_source.buffer, line_index + 1,
                       excessive_position...(line_length(line)))
        end

        def highlight_start(line)
          [max - indentation_difference(line), 0].max
        end

        def check_directive_line(line, line_index)
          length_without_directive = line_length_without_directive(line)
          return if length_without_directive <= max

          range = max..(length_without_directive - 1)
          register_offense(
            source_range(processed_source.buffer, line_index + 1, range),
            line,
            line_index,
            length: length_without_directive
          )
        end

        def check_line_for_exemptions(line, line_index)
          uri_range            = range_if_applicable(line, :uri)
          qualified_name_range = range_if_applicable(line, :qualified_name)

          return if allowed_combination?(line, uri_range, qualified_name_range)

          range = uri_range || qualified_name_range
          register_offense(excess_range(range, line, line_index), line, line_index)
        end

        def range_if_applicable(line, type)
          return unless type == :uri ? allow_uri? : allow_qualified_name?

          find_excessive_range(line, type)
        end

        def allowed_combination?(line, uri_range, qualified_name_range)
          if uri_range && qualified_name_range
            allowed_position?(line, uri_range) && allowed_position?(line, qualified_name_range)
          elsif uri_range
            allowed_position?(line, uri_range)
          elsif qualified_name_range
            allowed_position?(line, qualified_name_range)
          else
            false
          end
        end

        def max
          cop_config["Max"]
        end
        alias max_line_length max

        def allowed_heredoc
          cop_config["AllowHeredoc"]
        end
      end
    end
  end
end
