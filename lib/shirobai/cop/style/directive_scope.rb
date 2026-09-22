# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/DirectiveScope`.
      #
      # Stock walks EVERY comment of the file: `DirectiveComment.new` (the
      # `DIRECTIVE_COMMENT_REGEXP` match) followed by
      # `comment_config.comment_only_line?`, whose first call materializes the
      # parser-gem token stream (`processed_source.tokens`, the "toucher" cost)
      # on every file that has a comment. Only a comment matching the directive
      # regexp (`#\s*rubocop\s*:\s*(disable|enable|push|...)`) can reach
      # `check_pair` / `check_push_pop` / `check_enable_pair`; for any other
      # comment the loop body is a no-op.
      #
      # So Rust supplies ONLY the candidate list: the `(index, line)` of every
      # comment whose text contains `rubocop` (a strict superset of the regexp
      # matches). This wrapper runs stock's loop body verbatim on those comments
      # and nothing else — `DirectiveComment`, `comment_config`'s disabled
      # ranges and statement scopes, messages and corrections are all stock's
      # own code, so detection and autocorrect are byte-identical by
      # construction (offense ranges are the comment objects themselves; no
      # offset ever crosses the Rust boundary, so no `SourceOffsets` conversion
      # is needed). A file with no `rubocop` comment never touches tokens.
      class DirectiveScope < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG_PAIR = "Use `%<mode>s-next` instead of a `%<mode>s`/`enable` pair " \
                   "around a single statement."
        MSG_ENABLE_PAIR = "Use `enable-next` instead of an `enable`/`disable` pair " \
                          "around a single statement."
        MSG_PUSH_POP = "Use `%<replacement>s` instead of `push`/`pop` around a single statement."

        def self.cop_name = "Style/DirectiveScope"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less on the Rust side (the candidate scan needs no config).
        # Kept for the 4+1 single-source-of-config convention.
        def self.bundle_args(_config)
          []
        end

        # Stock's `on_new_investigation`, with the all-comments walk replaced
        # by the Rust candidate list (see `each_candidate_directive`).
        def on_new_investigation
          each_candidate_directive do |directive|
            next unless comment_config.comment_only_line?(directive.line_number)

            if plain_disable?(directive)
              check_pair(directive)
            elsif signed_push?(directive)
              check_push_pop(directive)
            elsif plain_enable?(directive)
              check_enable_pair(directive)
            end
          end
        end

        private

        # Yields a `DirectiveComment` for every comment containing `rubocop`,
        # in document order. The Rust side reports `(index, line)` into the
        # parse's comment list, which is the list `processed_source.comments`
        # is built from; the line double-checks the index and falls back to
        # `comment_at_line` should the two comment lists ever disagree.
        def each_candidate_directive
          comments = processed_source.comments
          resolved_candidates.each do |index, line|
            comment = comments[index]
            comment = processed_source.comment_at_line(line) unless comment&.loc&.line == line
            next unless comment

            yield RuboCop::DirectiveComment.new(comment)
          end
        end

        def resolved_candidates
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :directive_scope)
          else
            Shirobai.check_directive_scope(processed_source.buffer.source)
          end
        end

        # Stock's `comment_config`, verbatim.
        def comment_config
          processed_source.comment_config
        end

        # Stock's `plain_disable?`, verbatim.
        def plain_disable?(directive)
          directive.disabled? && !directive.disable_next?
        end

        # Stock's `plain_enable?`, verbatim.
        def plain_enable?(directive)
          directive.enabled? && !directive.enable_next? && !directive.all_cops?
        end

        # Stock's `signed_push?`, verbatim.
        def signed_push?(directive)
          directive.push? && !directive.signed_args.empty?
        end

        # Stock's `check_pair`, verbatim.
        def check_pair(directive)
          enable = single_statement_closing_directive(directive)
          return unless enable&.enabled?
          return unless enable.raw_cop_names.sort == directive.raw_cop_names.sort

          message = format(MSG_PAIR, mode: directive.mode)
          add_offense(directive.comment, message: message) do |corrector|
            convert_pair(corrector, directive, enable)
          end
        end

        # Stock's `convert_pair`, verbatim.
        def convert_pair(corrector, directive, enable)
          replacement = directive.comment.text.sub(/\b#{directive.mode}\b/,
                                                   "#{directive.mode}-next")
          corrector.replace(directive.comment, replacement)
          remove_line(corrector, enable.comment)
        end

        # Stock's `check_push_pop`, verbatim.
        def check_push_pop(directive)
          pop_line = balancing_pop_line(directive)
          return unless pop_line && comment_config.comment_only_line?(pop_line)
          return unless wraps_single_statement?(directive, pop_line)

          pop_comment = processed_source.comment_at_line(pop_line)
          replacement = push_replacement(directive)
          message = format(MSG_PUSH_POP, replacement: replacement_mode(directive))
          add_offense(directive.comment, message: message) do |corrector|
            corrector.replace(directive.comment, replacement)
            remove_line(corrector, pop_comment)
          end
        end

        # Stock's `balancing_pop_line`, verbatim.
        def balancing_pop_line(push_directive)
          depth = 0
          each_directive_after(push_directive) do |directive|
            if directive.push?
              depth += 1
            elsif directive.pop?
              return directive.line_number if depth.zero?

              depth -= 1
            end
          end
          nil
        end

        # Stock's `each_directive_after`, walking the Rust candidates instead
        # of every comment: a directive with `cop_names`, `push?` or `pop?`
        # matched the directive regexp, so it contains `rubocop` and is a
        # candidate.
        def each_directive_after(reference)
          each_candidate_directive do |directive|
            next unless directive.cop_names || directive.push? || directive.pop?

            yield directive if directive.line_number > reference.line_number
          end
        end

        # Stock's `replacement_mode`, verbatim.
        def replacement_mode(directive)
          ops = directive.signed_args.keys.sort
          if ops == ["-"]
            "disable-next"
          elsif ops == ["+"]
            "enable-next"
          else
            "next"
          end
        end

        # Stock's `push_replacement`, verbatim.
        def push_replacement(directive)
          text = case replacement_mode(directive)
                 when "disable-next"
                   "# rubocop:disable-next #{directive.signed_args['-'].join(', ')}"
                 when "enable-next"
                   "# rubocop:enable-next #{directive.signed_args['+'].join(', ')}"
                 else
                   "# rubocop:next #{directive.cops}"
                 end
          reason = directive.reason
          reason ? "#{text} -- #{reason}" : text
        end

        # Stock's `check_enable_pair`, verbatim.
        def check_enable_pair(directive)
          closing = enable_pair_closing(directive)
          return unless closing

          add_offense(directive.comment, message: MSG_ENABLE_PAIR) do |corrector|
            corrector.replace(directive.comment,
                              directive.comment.text.sub(/\benable\b/, "enable-next"))
            remove_line(corrector, closing.comment)
          end
        end

        # Stock's `enable_pair_closing`, verbatim.
        def enable_pair_closing(directive)
          scope = comment_config.statement_scope_after(directive.line_number)
          return nil unless scope && scope.begin == directive.line_number + 1

          closing = re_disable_below(directive, scope.end + 1)
          closing if closing && closed_open_disables?(directive, closing)
        end

        # Stock's `re_disable_below`, verbatim.
        def re_disable_below(directive, line)
          return nil unless comment_config.comment_only_line?(line)

          comment = processed_source.comment_at_line(line)
          return nil unless comment

          closing = RuboCop::DirectiveComment.new(comment)
          return nil unless closing.disabled? && !closing.disable_next?
          return nil unless closing.raw_cop_names.sort == directive.raw_cop_names.sort

          closing
        end

        # Stock's `closed_open_disables?`, verbatim.
        def closed_open_disables?(directive, closing)
          directive.cop_names.all? do |cop|
            ranges = comment_config.cop_disabled_line_ranges[qualified_name(cop)]
            ranges && closed_at?(ranges, directive.line_number) && reopened_by?(ranges, closing)
          end
        end

        # Stock's `closed_at?`, verbatim.
        def closed_at?(ranges, line)
          ranges.any? { |range| range.end == line }
        end

        # Stock's `reopened_by?`, verbatim.
        def reopened_by?(ranges, closing)
          ranges.any? do |range|
            range.respond_to?(:directive) && range.directive.comment.equal?(closing.comment)
          end
        end

        # Stock's `qualified_name`, verbatim.
        def qualified_name(cop_name)
          RuboCop::Cop::Registry.qualified_cop_name(cop_name.strip, processed_source.file_path,
                                                    correct_namespace: false)
        end

        # Stock's `single_statement_closing_directive`, verbatim.
        def single_statement_closing_directive(directive)
          closing_line = single_closing_line(directive)
          return nil unless closing_line

          closing_comment = processed_source.comment_at_line(closing_line)
          return nil unless closing_comment
          return nil unless wraps_single_statement?(directive, closing_line)

          RuboCop::DirectiveComment.new(closing_comment)
        end

        # Stock's `single_closing_line`, verbatim.
        def single_closing_line(directive)
          ranges = ranges_opened_by(directive)
          return nil if ranges.empty?

          ends = ranges.map(&:end).uniq
          return nil unless ends.size == 1 && ends.first.to_f.finite?

          ends.first
        end

        # Stock's `ranges_opened_by`, verbatim.
        def ranges_opened_by(directive)
          comment_config.cop_disabled_line_ranges.each_value.flat_map do |ranges|
            ranges.select do |range|
              range.respond_to?(:directive) && range.directive.comment.equal?(directive.comment)
            end
          end
        end

        # Stock's `wraps_single_statement?`, verbatim.
        def wraps_single_statement?(directive, closing_line)
          scope = comment_config.statement_scope_after(directive.line_number)

          !scope.nil? && scope.begin == directive.line_number + 1 && scope.end + 1 == closing_line
        end

        # Stock's `remove_line`, verbatim.
        def remove_line(corrector, comment)
          corrector.remove(range_by_whole_lines(comment.source_range, include_final_newline: true))
        end
      end
    end
  end
end
