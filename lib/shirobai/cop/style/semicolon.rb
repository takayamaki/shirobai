# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/Semicolon`.
      #
      # Stock has two detection paths. Path (a) (`on_new_investigation`) groups
      # `processed_source.tokens` by line and flags a semicolon in six
      # positional token-index patterns. shirobai builds no parser token
      # stream, so path (a) runs in Rust: a masked byte scan (the same opaque
      # regions as the punctuation cops) plus a few AST-collected opener
      # positions reconstruct those index facts. Rust returns one `[offset,
      # last_token]` pair per flagged `;`.
      #
      # Path (b) (`on_begin`, the token scan for expression separators) stays
      # here, ported verbatim from stock: it walks `processed_source.tokens`
      # for real `;` tokens on lines that carry more than one expression, so a
      # `;` inside a string/regexp literal is skipped and it matches stock byte
      # for byte. The `AllowAsExpressionSeparator` option only turns path (b)
      # off, exactly as in stock.
      #
      # Autocorrect is stock's, run at correction time on the real parser AST /
      # tokens (only reached with `-a`): path (a)'s last-token pattern may wrap
      # an endless range or a hash value omission in parentheses; path (b)
      # replaces the `;` with a newline unless a heredoc opened earlier on the
      # line. Because both paths call `add_offense` in stock's order (path (a)
      # first, then path (b) during the walk), RuboCop's own offense dedup and
      # corrector-conflict resolution reproduce stock's result on the lines
      # both paths touch.
      class Semicolon < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector

        MSG = "Do not use semicolons to terminate expressions."

        def self.cop_name = "Style/Semicolon"
        def self.badge = RuboCop::Cop::Badge.parse("Style/Semicolon")

        # Config-less on the bundle side: path (a) needs no config and path (b)
        # reads `AllowAsExpressionSeparator` directly from `cop_config`.
        def self.bundle_args(_config) = []

        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Style::SingleLineMethods]
        end

        def on_new_investigation
          return if processed_source.blank? || !processed_source.raw_source.include?(";")

          check_for_line_terminator_or_opener
        end

        def on_begin(node)
          return if cop_config["AllowAsExpressionSeparator"]
          return unless node.source.include?(";")

          exprs = node.children

          return if exprs.size < 2

          expressions_per_line(exprs).each do |line, expr_on_line|
            # Every line with more than one expression on it is a
            # potential offense.
            next unless expr_on_line.size > 1

            find_semicolon_positions(line) { |pos| register_semicolon(line, pos, true) }
          end
        end

        private

        # Path (a): the Rust byte scan gives byte offsets; the wrapper maps them
        # to a single-character `;` range (the offense highlight and, for the
        # last-token pattern, the preceding token for autocorrect).
        def check_for_line_terminator_or_opener
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          path_a_offenses(source).each do |byte_offset, last_token|
            char = off[byte_offset]
            range = Parser::Source::Range.new(buffer, char, char + 1)
            token_before = last_token ? token_before_semicolon(range) : nil
            register_semicolon_range(range, false, token_before)
          end
        end

        def path_a_offenses(source)
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :semicolon)
          else
            Shirobai.check_semicolon(source)
          end
        end

        # The parser token right before the `;` on its line. Only used for the
        # last-token pattern, where stock passes `tokens[-2]` of the line group
        # (the token before the trailing `;`). Reached only during autocorrect.
        def token_before_semicolon(range)
          line_tokens = processed_source.tokens.group_by(&:line)[range.line]
          return nil unless line_tokens&.last&.semicolon?

          line_tokens[-2]
        end

        def register_semicolon(line, column, after_expression, token_before_semicolon = nil)
          range = source_range(processed_source.buffer, line, column)
          register_semicolon_range(range, after_expression, token_before_semicolon)
        end

        # rubocop:disable Metrics/AbcSize, Metrics/MethodLength
        def register_semicolon_range(range, after_expression, token_before_semicolon)
          add_offense(range) do |corrector|
            if after_expression
              replace_semicolon_with_line_break(corrector, range)
            else
              node = nil
              # Prevents becoming one range instance with the subsequent line
              # when an endless range without parentheses precedes the `;`.
              # See: https://github.com/rubocop/rubocop/issues/10791
              if token_before_semicolon&.regexp_dots?
                node = find_node(range_nodes, token_before_semicolon)
              elsif token_before_semicolon&.type == :tLABEL
                node = find_node(value_omission_pair_nodes, token_before_semicolon).parent
                space = node.parent.loc.selector.end.join(node.source_range.begin)
                corrector.remove(space)
              end

              corrector.wrap(node, "(", ")") if node
              corrector.remove(range)
            end
          end
        end
        # rubocop:enable Metrics/AbcSize, Metrics/MethodLength

        def replace_semicolon_with_line_break(corrector, range)
          # Replacing the semicolon with a newline would move the rest of the
          # line into the body of a heredoc opened earlier on that line.
          return if heredoc_opened_before_semicolon?(range)

          corrector.replace(range, "\n")
        end

        def heredoc_opened_before_semicolon?(semicolon_range)
          processed_source.ast.each_descendant(:any_str).select(&:heredoc?).any? do |heredoc|
            heredoc.first_line == semicolon_range.line &&
              heredoc.source_range.end_pos <= semicolon_range.begin_pos
          end
        end

        def expressions_per_line(exprs)
          # Create a map matching lines to the number of expressions on them.
          exprs_lines = exprs.map(&:last_line)
          exprs_lines.group_by(&:itself)
        end

        def find_semicolon_positions(line)
          # Scan for all the semicolon tokens on the line. Iterating tokens
          # rather than the raw source skips `;` characters inside
          # string/regexp literals.
          processed_source.tokens.each do |token|
            yield token.column if token.line == line && token.semicolon?
          end
        end

        def find_node(nodes, token_before_semicolon)
          nodes.detect do |node|
            node.source_range.overlaps?(token_before_semicolon.pos)
          end
        end

        def range_nodes
          return @range_nodes if instance_variable_defined?(:@range_nodes)

          ast = processed_source.ast
          @range_nodes = ast.range_type? ? [ast] : []
          @range_nodes.concat(ast.each_descendant(:range).to_a)
        end

        def value_omission_pair_nodes
          if instance_variable_defined?(:@value_omission_pair_nodes)
            return @value_omission_pair_nodes
          end

          ast = processed_source.ast
          @value_omission_pair_nodes = ast.each_descendant(:pair).to_a.select(&:value_omission?)
        end
      end
    end
  end
end
