# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/MisplacedMagicComment`.
      #
      # Stock walks EVERY comment of the file through `magic_comment_shaped?`
      # and `MagicComment.parse` (three regexp formats), and its
      # `check_top_block_comment` asks `first_code_token`, which materializes
      # the parser-gem token stream (`processed_source.sorted_tokens`, the
      # "toucher" cost) on every file with a `frozen_string_literal` comment —
      # nearly every file in a modern codebase. A second full-comment walk
      # (`check_magic_comment_above_shebang`'s `find`) runs on every file too.
      #
      # So Rust supplies ONLY the candidates: the shebang comment stock's `find`
      # would return, and every magic-comment-shaped comment that mentions
      # `coding` or `frozen` (a superset of the `encoding` /
      # `frozen_string_literal` forms stock acts on) as
      # `(index, line, after_code)`, where `after_code` is stock's
      # `comment.source_range.begin_pos > first_code_token.begin_pos` computed
      # on byte offsets (the shared leading-comment front scan; `false` with no
      # code token). This wrapper runs stock's `check_comment` verbatim on the
      # candidates — `MagicComment.parse` still decides — with `first_code_token`
      # replaced by the flag, and everything else (`known_encoding?`, the
      # effective encoding line, `preceded_only_by_magic_comments?`, messages,
      # `move_comment`) is stock's own code. Offense ranges are the comment
      # objects themselves, so no offset crosses the Rust boundary and no
      # `SourceOffsets` conversion is needed.
      class MisplacedMagicComment < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG_ENCODING = "The `encoding` magic comment is ignored unless placed on the first " \
                       "line (or below a shebang on the first line)."
        MSG_AFTER_CODE = "The `%<directive>s` magic comment is ignored after any code."
        MSG_ABOVE_SHEBANG = "A magic comment above a shebang renders the shebang ineffective."

        def self.cop_name = "Lint/MisplacedMagicComment"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less on the Rust side (the candidate scan needs no config).
        # Kept for the 4+1 single-source-of-config convention.
        def self.bundle_args(_config)
          []
        end

        # Stock's `on_new_investigation`, with the two all-comments walks
        # replaced by the Rust candidates.
        def on_new_investigation
          return if processed_source.buffer.source.empty?

          shebang, candidates = resolved_scan
          comments = processed_source.comments
          check_magic_comment_above_shebang(locate_comment(comments, *shebang)) if shebang
          candidates.each do |index, line, after_code|
            comment = locate_comment(comments, index, line)
            check_comment(comment, after_code) if comment
          end
        end

        private

        def resolved_scan
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :misplaced_magic_comment)
          else
            Shirobai.check_misplaced_magic_comment(processed_source.buffer.source)
          end
        end

        # The Rust side reports `index` into the parse's comment list, which is
        # the list `processed_source.comments` is built from; the line
        # double-checks the index and falls back to `comment_at_line` should
        # the two comment lists ever disagree.
        def locate_comment(comments, index, line)
          comment = comments[index]
          return comment if comment&.loc&.line == line

          processed_source.comment_at_line(line)
        end

        # Stock's `check_comment`, verbatim except that the
        # `frozen_string_literal` branch receives the Rust `after_code` flag.
        def check_comment(comment, after_code)
          return unless magic_comment_shaped?(comment.text)

          magic_comment = RuboCop::MagicComment.parse(comment.text)
          return unless magic_comment.valid?

          if magic_comment.encoding_specified?
            return unless known_encoding?(magic_comment.encoding)

            check_encoding_comment(comment)
          elsif magic_comment.frozen_string_literal_specified?
            check_top_block_comment(comment, "frozen_string_literal", after_code)
          end
        end

        # Stock's `magic_comment_shaped?`, verbatim.
        def magic_comment_shaped?(text)
          /\A#(?![^#]*#)/.match?(text)
        end

        # Stock's `known_encoding?`, verbatim.
        def known_encoding?(name)
          Encoding.find(name)
          true
        rescue ArgumentError
          false
        end

        # Stock's `shebang?`, verbatim.
        def shebang?
          processed_source.lines.first.to_s.start_with?("#!")
        end

        # Stock's `effective_encoding_line`, verbatim.
        def effective_encoding_line
          shebang? ? 2 : 1
        end

        # Stock's `check_encoding_comment`, verbatim.
        def check_encoding_comment(comment)
          line = comment.source_range.line
          return if line == effective_encoding_line && comment_starts_line?(comment)
          # Leave `# frozen_string_literal: true` + `# encoding: x` runs at the
          # very top to Lint/OrderedMagicComments, which already reorders them.
          return if preceded_only_by_magic_comments?(comment)

          add_offense(comment, message: MSG_ENCODING) do |corrector|
            move_comment(corrector, comment, effective_encoding_line)
          end
        end

        # Stock's `check_top_block_comment`, with
        # `return unless first_code_token` and
        # `return unless comment.source_range.begin_pos > first_code_token.begin_pos`
        # folded into the Rust `after_code` flag (no token stream needed).
        def check_top_block_comment(comment, directive, after_code)
          return unless after_code

          message = format(MSG_AFTER_CODE, directive: directive)
          add_offense(comment, message: message) do |corrector|
            move_comment(corrector, comment, effective_encoding_line)
          end
        end

        # Stock's `check_magic_comment_above_shebang`, with the `find` over
        # every comment replaced by the Rust-located comment (nil when the
        # file has no `#!` comment at column 0 below line 1).
        def check_magic_comment_above_shebang(shebang_comment)
          return unless shebang_comment
          # Only flag when what precedes the shebang is magic comments -
          # a `#!` comment deep inside a file is not an intended shebang.
          return unless preceded_only_by_magic_comments?(shebang_comment)

          add_offense(shebang_comment, message: MSG_ABOVE_SHEBANG)
        end

        # Stock's `preceded_only_by_magic_comments?`, verbatim.
        def preceded_only_by_magic_comments?(comment)
          (1...comment.source_range.line).all? do |line_number|
            text = processed_source.lines[line_number - 1]
            (line_number == 1 && text.start_with?("#!")) || RuboCop::MagicComment.parse(text).valid?
          end
        end

        # Stock's `comment_starts_line?`, verbatim.
        def comment_starts_line?(comment)
          processed_source.lines[comment.source_range.line - 1].lstrip.start_with?("#")
        end

        # Stock's `move_comment`, verbatim.
        def move_comment(corrector, comment, target_line)
          removal_range = if comment_starts_line?(comment)
                            range_by_whole_lines(comment.source_range, include_final_newline: true)
                          else
                            range_with_surrounding_space(comment.source_range, side: :left)
                          end
          corrector.remove(removal_range)
          target_range = processed_source.buffer.line_range(target_line)
          if comment.source_range.line < target_line
            corrector.insert_after(target_range, "\n#{comment.text}")
          else
            corrector.insert_before(target_range, "#{comment.text}\n")
          end
        end
      end
    end
  end
end
