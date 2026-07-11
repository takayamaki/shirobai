# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/MagicCommentFormat`.
      #
      # Stock's only parser-gem token access is `leading_comment_lines`, which
      # asks `processed_source.tokens.find { |t| !t.comment? }` for the first
      # non-comment token — materializing the whole parser-gem token stream on
      # every file (the "toucher" cost). Everything else runs on the parser-gem
      # *comment* objects (`each_comment_in_lines`, backed by
      # `processed_source.comments` from the parse — not the token stream) and
      # stock's own `CommentRange` regex extraction, offense predicates,
      # messages and corrections.
      #
      # So Rust supplies ONLY the leading-line boundary (the first non-comment
      # token line, without tokens); this wrapper rebuilds
      # `leading_comment_lines` from it and runs stock's `magic_comments` and the
      # rest verbatim. `CommentRange` is reused directly from the stock cop, and
      # the offense/message/correction helpers are copied verbatim, so detection,
      # messages, and autocorrect are stock's own code — byte-identical by
      # construction, including non-ASCII offsets (offense ranges come from
      # `CommentRange`'s `loc.expression` char offsets, never through Rust).
      class MagicCommentFormat < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        SNAKE_SEPARATOR = "_"
        KEBAB_SEPARATOR = "-"
        MSG = "Prefer %<style>s case for magic comments."
        MSG_VALUE = "Prefer %<case>s for magic comment values."

        # Reuse stock's `CommentRange` (the `DIRECTIVE_REGEXP` / `VALUE_REGEXP`
        # scan that turns a comment into directive/value source ranges) verbatim.
        CommentRange = RuboCop::Cop::Style::MagicCommentFormat::CommentRange

        def self.cop_name = "Style/MagicCommentFormat"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less on the Rust side (the leading-line boundary needs no
        # config; every EnforcedStyle / Capitalization axis is handled here in
        # Ruby by stock's own methods). Kept for the 4+1 single-source-of-config
        # convention.
        def self.bundle_args(_config)
          []
        end

        # Stock's `on_new_investigation`, verbatim.
        def on_new_investigation
          return unless processed_source.ast

          magic_comments.each do |comment|
            issues = find_issues(comment)
            register_offenses(issues) if issues.any?
          end
        end

        private

        # Stock's `magic_comments`, verbatim except the fully-qualified
        # `RuboCop::MagicComment`. `leading_comment_lines` (below) is the only
        # override.
        def magic_comments
          processed_source.each_comment_in_lines(leading_comment_lines)
                          .select { |comment| RuboCop::MagicComment.parse(comment.text).valid? }
                          .map { |comment| CommentRange.new(comment) }
        end

        # Stock's `leading_comment_lines` without `processed_source.tokens`: Rust
        # returns the first non-comment token's 1-based line (or `0` for none).
        def leading_comment_lines
          line = resolved_first_token_line
          return (0...line) unless line.zero?

          # Stock returns the endless `0..` when the file has no non-comment
          # token. That branch is unreachable while `processed_source.ast` is
          # non-nil (the `on_new_investigation` guard), but keep the range finite
          # so `each_comment_in_lines` can never loop forever: every comment sits
          # on a line <= `lines.size`, so this covers exactly the same comments.
          (0..processed_source.lines.size)
        end

        def resolved_first_token_line
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :magic_comment_format)
          else
            Shirobai.check_magic_comment_format(processed_source.buffer.source)
          end
        end

        # Stock's `find_issues`, verbatim.
        def find_issues(comment)
          issues = { directives: [], values: [] }

          comment.directives.each do |directive|
            issues[:directives] << directive if directive_offends?(directive)
          end

          comment.values.each do |value| # rubocop:disable Style/HashEachMethods
            issues[:values] << value if wrong_capitalization?(value.source, value_capitalization)
          end

          issues
        end

        # Stock's `directive_offends?`, verbatim.
        def directive_offends?(directive)
          incorrect_separator?(directive.source) ||
            wrong_capitalization?(directive.source, directive_capitalization)
        end

        # Stock's `register_offenses`, verbatim.
        def register_offenses(issues)
          fix_directives(issues[:directives])
          fix_values(issues[:values])
        end

        # Stock's `fix_directives`, verbatim.
        def fix_directives(issues)
          return if issues.empty?

          msg = format(MSG, style: expected_style)

          issues.each do |directive|
            add_offense(directive, message: msg) do |corrector|
              replacement = replace_separator(replace_capitalization(directive.source,
                                                                     directive_capitalization))
              corrector.replace(directive, replacement)
            end
          end
        end

        # Stock's `fix_values`, verbatim.
        def fix_values(issues)
          return if issues.empty?

          msg = format(MSG_VALUE, case: value_capitalization)

          issues.each do |value|
            add_offense(value, message: msg) do |corrector|
              corrector.replace(value, replace_capitalization(value.source, value_capitalization))
            end
          end
        end

        # Stock's `expected_style`, verbatim.
        def expected_style
          [directive_capitalization, style].compact.join(" ").gsub(/_?case\b/, "")
        end

        # Stock's `wrong_separator`, verbatim.
        def wrong_separator
          style == :snake_case ? KEBAB_SEPARATOR : SNAKE_SEPARATOR
        end

        # Stock's `correct_separator`, verbatim.
        def correct_separator
          style == :snake_case ? SNAKE_SEPARATOR : KEBAB_SEPARATOR
        end

        # Stock's `incorrect_separator?`, verbatim.
        def incorrect_separator?(text)
          text[wrong_separator]
        end

        # Stock's `wrong_capitalization?`, verbatim.
        def wrong_capitalization?(text, expected_case)
          return false unless expected_case

          case expected_case
          when :lowercase
            text != text.downcase
          when :uppercase
            text != text.upcase
          end
        end

        # Stock's `replace_separator`, verbatim.
        def replace_separator(text)
          text.tr(wrong_separator, correct_separator)
        end

        # Stock's `replace_capitalization`, verbatim.
        def replace_capitalization(text, style)
          return text unless style

          case style
          when :lowercase
            text.downcase
          when :uppercase
            text.upcase
          end
        end

        # Stock's `directive_capitalization`, verbatim.
        def directive_capitalization
          cop_config["DirectiveCapitalization"]&.to_sym.tap do |style|
            unless valid_capitalization?(style)
              raise "Unknown `DirectiveCapitalization` #{style} selected!"
            end
          end
        end

        # Stock's `value_capitalization`, verbatim.
        def value_capitalization
          cop_config["ValueCapitalization"]&.to_sym.tap do |style|
            unless valid_capitalization?(style)
              raise "Unknown `ValueCapitalization` #{style} selected!"
            end
          end
        end

        # Stock's `valid_capitalization?`, verbatim.
        def valid_capitalization?(style)
          return true unless style

          supported_capitalizations.include?(style)
        end

        # Stock's `supported_capitalizations`, verbatim.
        def supported_capitalizations
          @supported_capitalizations ||= cop_config["SupportedCapitalizations"].map(&:to_sym).freeze
        end
      end
    end
  end
end
