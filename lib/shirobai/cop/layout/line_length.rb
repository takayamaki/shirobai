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
      # the upstream mixins so the offense ranges match exactly. Candidates and
      # breakables come from the per-file bundled run (`Shirobai::Dispatch`);
      # the regex exemptions run after the ext call either way, so this cop is
      # always bundle-eligible.
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

        # Packed args for the bundled run: `[max, tab_width, split_strings]`.
        # `tab_width` replicates `LineLengthHelp#tab_indentation_width`
        # (`Layout/IndentationStyle` width, falling back to
        # `Alignment#configured_indentation_width`). `Max` defaults to 120
        # (default.yml) so a config that does not mention this cop still packs
        # cleanly; the computed slice is discarded in that case.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          tab_width = config.for_cop("Layout/IndentationStyle")["IndentationWidth"] ||
                      cop_config["IndentationWidth"] ||
                      config.for_cop("Layout/IndentationWidth")["Width"] || 2
          [cop_config["Max"] || 120, tab_width, !!cop_config["SplitStrings"]]
        end

        # `LineLengthHelp#uri_regexp` memoizes per cop instance, and the real
        # CLI builds a fresh cop per file — so stock rebuilds the same Regexp
        # (an expensive `URI` parser call) for every file with a candidate
        # line. The regexp depends only on the cop's `URISchemes`, so share it
        # per cop-config object instead.
        def self.uri_regexp_for(cop_config)
          @uri_regexp_cache ||= {}.compare_by_identity
          @uri_regexp_cache[cop_config] ||= begin
            parser = defined?(URI::RFC2396_PARSER) ? URI::RFC2396_PARSER : URI::DEFAULT_PARSER
            parser.make_regexp(cop_config["URISchemes"])
          end
        end

        def on_new_investigation
          candidates = Dispatch.offenses_for(processed_source, config, :line_length)

          # Breakable (autocorrection) data must be installed even in lint mode:
          # with AutoCorrect defaulting to 'always', the corrector block runs and
          # a non-empty corrector makes the offense `:uncorrected` (correctable),
          # which stock reports as "[Correctable]" / counts as auto-correctable.
          # Skipping it would flip the offense to `:unsupported` and diverge from
          # stock's lint output. The bundle restricts the walk to candidate lines
          # (length > Max) on the Rust side — the sole lines that can become
          # offenses and consume a breakable range — which is identical to
          # computing it for all lines.
          install_breakables(Dispatch.offenses_for(processed_source, config, :line_length_breakables))

          candidates.each do |candidate|
            line_index, length, _line_start, _line_end, _indent_diff, heredoc_delimiters = candidate
            line = processed_source.lines[line_index]
            check_candidate(line, line_index, length, heredoc_delimiters)
          end
        end

        private

        def uri_regexp
          @uri_regexp ||= self.class.uri_regexp_for(cop_config)
        end

        # Install the per-line autocorrection data (insertion byte offset and,
        # for `SplitStrings`, the string delimiter). Mirrors upstream's
        # `breakable_range_by_line_index` / `breakable_string_delimiters`.
        def install_breakables(breakables)
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          @breakable_range_by_line_index = {}
          @breakable_string_delimiters = {}
          breakables.each do |entry|
            line_index, insert_offset, delimiter = entry
            insert = off[insert_offset]
            @breakable_range_by_line_index[line_index] =
              Parser::Source::Range.new(buffer, insert, insert + 1)
            @breakable_string_delimiters[line_index] = delimiter unless delimiter.empty?
          end
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

        # Memoized per instance: `max` is read several times per candidate line
        # (check_candidate / register_offense / excess_range / highlight_start)
        # and `cop_config` never changes within a run. The ExcludeLimit-generated
        # `max=` writer used in the corrector block appends to a tmp file for
        # `--auto-gen-config` aggregation and never feeds back into this reader.
        def max
          @max ||= cop_config["Max"]
        end
        alias max_line_length max

        def allowed_heredoc
          cop_config["AllowHeredoc"]
        end
      end
    end
  end
end
