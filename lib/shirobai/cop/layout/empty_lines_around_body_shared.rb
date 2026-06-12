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
      # `EnforcedStyle` through `bundle_args`, so the family is always
      # bundle-eligible.
      module EmptyLinesAroundBodyShared
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

          offenses = Dispatch.offenses_for(processed_source, config, self.class::SLOT)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |start, fin, insert, message|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: message) do |corrector|
              # EmptyLineCorrector.correct
              if insert
                corrector.insert_before(range, "\n")
              else
                corrector.remove(range)
              end
            end
          end
        end
      end
    end
  end
end
