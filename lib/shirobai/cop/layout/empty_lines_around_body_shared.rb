# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Shared offense-reporting logic for the `EmptyLinesAroundBody` mixin
      # family (method/class/module/block/begin bodies and the
      # exception-handling-keywords variant).
      #
      # Rust replicates the mixin's line arithmetic (`check_beginning` /
      # `check_ending` / `check_deferred_empty_line` and the keyword loop) in
      # one shared-walk rule and returns, per cop, the offense range (the
      # first character of the offense line, exactly stock's
      # `source_range(buffer, line, 0)`), the message and which
      # `EmptyLineCorrector` arm applies (remove the range / insert a `"\n"`
      # before it). Stock's `add_offense` range-dedup (e.g. the begin and end
      # checks of `class Foo\n\nend` hitting the same blank line) is inherited
      # for free by emitting in stock's order. The including cop selects its
      # result slot via `SLOT`; the three configurable cops pack their
      # `EnforcedStyle` through `bundle_args`, so the family is bundle-eligible
      # on every config; CRLF/BOM files take the standalone fallback.
      module EmptyLinesAroundBodyShared
        include Shirobai::Cop::BundleEligible

        # Index of each member in the standalone `check_empty_lines_around_body`
        # result (slot order: method, class, module, block, begin, keywords).
        STANDALONE_INDEX = {
          empty_lines_around_method_body: 0,
          empty_lines_around_class_body: 1,
          empty_lines_around_module_body: 2,
          empty_lines_around_block_body: 3,
          empty_lines_around_begin_body: 4,
          empty_lines_around_exception_handling_keywords: 5
        }.freeze

        # `SupportedStyles` order of `Layout/EmptyLinesAroundClassBody`,
        # mirrored by the Rust style constants.
        STYLES = {
          "no_empty_lines" => 0,
          "empty_lines" => 1,
          "empty_lines_except_namespace" => 2,
          "empty_lines_special" => 3,
          "beginning_only" => 4,
          "ending_only" => 5
        }.freeze

        def self.style_num(config, badge)
          STYLES.fetch(config.for_badge(badge)["EnforcedStyle"] || "no_empty_lines")
        end

        def on_new_investigation
          buffer = processed_source.buffer

          # Rust byte offsets are converted against the source they were
          # computed on (`raw_source` for the bundled run, which must equal
          # `buffer.source`); on the CRLF/BOM fallback the standalone path
          # scans `buffer.source` instead so every offset Rust returns lines
          # up with the parser-gem buffer.
          if bundle_eligible?
            source = processed_source.raw_source
            offenses = Dispatch.offenses_for(processed_source, config, self.class::SLOT)
          else
            source = buffer.source
            offenses = standalone_offenses(source)
          end
          off = SourceOffsets.for(source)
          offenses.each do |start, fin, insert, message, block_body_deferred|
            next if block_body_deferred && left_to_block_body_cop?

            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            # `check_line` (1.92, rubocop#15697): with the `empty_lines` style a
            # missing blank line is not reported when the line that would get
            # it starts with a leading `.` / `&.`. Per offense, like stock.
            next if insert && message.start_with?("Empty line missing at") && leading_dot_line?(range.line)

            add_offense(range, message: message) do |corrector|
              # `EmptyLineCorrector.correct` (1.92.2, rubocop#16138): the
              # `empty_lines` arm replaces the newline before the offense line
              # with two, instead of inserting one before it. Same bytes, but
              # the edit list (`Offense#corrections`) sees the difference.
              if insert
                corrector.replace(range.begin.adjust(begin_pos: -1), "\n\n")
              else
                corrector.remove(range)
              end
            end
          end
        end

        private

        # The standalone run for the CRLF/BOM fallback: the whole family at
        # once (same Rust rule as the shared walk), this cop's slot picked out.
        def standalone_offenses(source)
          Shirobai.check_empty_lines_around_body(
            source,
            EmptyLinesAroundClassBody.bundle_args(config)[0],
            EmptyLinesAroundModuleBody.bundle_args(config)[0],
            EmptyLinesAroundBlockBody.bundle_args(config)[0]
          )[STANDALONE_INDEX.fetch(self.class::SLOT)]
        end

        # The config half of EHK's `left_to_block_body_cop?` (1.92,
        # rubocop#15814); Rust already checked the block parent and the blank
        # lines up to the block's closing line. Runs per flagged offense only.
        def left_to_block_body_cop?
          config.for_enabled_cop("Layout/EmptyLinesAroundBlockBody")["EnforcedStyle"] == "empty_lines"
        end

        # Verbatim from stock's `EmptyLinesAroundBody#leading_dot_line?`.
        def leading_dot_line?(line)
          first_token = processed_source.sorted_tokens.find { |token| token.line == line }

          RuboCop::Cop::Layout::EmptyLinesAroundBody::LEADING_DOT_TYPES.include?(first_token&.type)
        end
      end
    end
  end
end
