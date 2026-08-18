# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/AmbiguousBlockAssociation`.
      #
      # Detection and autocorrect both happen in Rust; Ruby turns the byte
      # offsets handed back into offenses and a `remove + insert_before +
      # insert_after` corrector trio (matching stock's `wrap_in_parentheses`
      # byte-for-byte).
      #
      # `AllowedMethods` is taken from the cop's own config (verbatim string
      # entries; `Regexp` entries are filtered out and force the standalone
      # fallback). `AllowedPatterns` (regexp) is matched in Ruby: the wrapper
      # walks every block-bearing call candidate's INNER sender source and
      # collects the strings that match any pattern, then hands the list to
      # the standalone Rust entry as `allowed_inner_sources` so the Rust path
      # can drop them by exact-bytes lookup. Without `AllowedPatterns` the
      # bundle path is taken (no regexp work).
      class AmbiguousBlockAssociation < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Parenthesize the param `%<param>s` to make sure that the " \
              "block will be associated with the `%<method>s` method " \
              "call."
        MSG_DO_END_BLOCK = "`%<inner_method>s` is called without a block because the " \
                           "`do` block binds to `%<outer_method>s`. " \
                           "Use braces or extract to a variable."

        def self.cop_name = "Lint/AmbiguousBlockAssociation"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Returns `[allowed_methods]`. `allowed_methods` mirrors the stock
        # `AllowedMethods` mixin: regexp entries filtered out (any regexp
        # forces the wrapper into the standalone path via `bundle_eligible?`).
        def self.bundle_args(config)
          cfg = config.for_badge(badge)
          allowed = Array(cfg.fetch("AllowedMethods", []))
          names = allowed.reject { |e| e.is_a?(Regexp) }.map(&:to_s)
          [names]
        end

        # Bundle path is OK only when `AllowedMethods` has no regexp AND
        # `AllowedPatterns` is empty (regexp matching against inner-sender
        # source is done in Ruby; the bundle path does not carry the
        # pre-applied list).
        def bundle_eligible?
          cfg = cop_config
          allowed = Array(cfg.fetch("AllowedMethods", []))
          patterns = Array(cfg.fetch("AllowedPatterns", []))
          allowed.none? { |e| e.is_a?(Regexp) } && patterns.empty?
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          fetch_offenses.each do |kind, start_offset, end_offset, param_start, param_end,
                                  inner_send_start, inner_send_end,
                                  ac_open_start, ac_open_end, ac_close_pos|
            range = Parser::Source::Range.new(buffer, off[start_offset], off[end_offset])
            param_range = Parser::Source::Range.new(buffer, off[param_start], off[param_end])
            inner_range = Parser::Source::Range.new(buffer, off[inner_send_start], off[inner_send_end])
            if kind == 1
              # `do...end` binding (stock `on_block`, 1.89): offense on the
              # inner call, message names both methods, no autocorrect.
              message = format(MSG_DO_END_BLOCK,
                               inner_method: param_range.source,
                               outer_method: inner_range.source)
              add_offense(range, message: message)
            else
              message = format(MSG, param: param_range.source, method: inner_range.source)
              add_offense(range, message: message) do |corrector|
                open_range = Parser::Source::Range.new(buffer, off[ac_open_start], off[ac_open_end])
                close_range = Parser::Source::Range.new(buffer, off[ac_close_pos], off[ac_close_pos])
                corrector.remove(open_range)
                corrector.insert_before(open_range, "(")
                corrector.insert_after(close_range, ")")
              end
            end
          end
        end

        private

        def fetch_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :ambiguous_block_association)
          else
            allowed = self.class.bundle_args(config)[0]
            allowed_inner_sources = build_allowed_inner_sources
            Shirobai.check_ambiguous_block_association(
              processed_source.raw_source, allowed, allowed_inner_sources
            )
          end
        end

        # Build the pre-applied `allowed_inner_sources` list for
        # `AllowedPatterns`: walk every CallNode that COULD be flagged (outer
        # call whose last argument is a block-bearing inner call) and collect
        # the inner-sender's source if any configured regexp matches it.
        # Strings inside `AllowedPatterns` are compiled into `Regexp` by stock
        # AllowedPattern mixin convention; we accept both `Regexp` and
        # `String` entries here.
        def build_allowed_inner_sources
          patterns = Array(cop_config.fetch("AllowedPatterns", []))
          return [] if patterns.empty?

          regexps = patterns.map { |p| p.is_a?(Regexp) ? p : Regexp.new(p) }
          # The wrapper does not have direct AST access; instead, query the
          # standalone Rust path with an empty allow-list and read the
          # `inner_send` source bytes off every candidate offense. Any
          # matching pattern adds that source to the skip list. The second
          # call with the filtered list then drops them in Rust.
          raw = Shirobai.check_ambiguous_block_association(processed_source.raw_source, [], [])
          src = processed_source.raw_source
          buffer = processed_source.buffer
          off = SourceOffsets.for(src)
          collected = raw.filter_map do |kind, s, e, _ps, _pe, iss, ise, _aos, _aoe, _acp|
            # kind 0 matches against the inner block sender's source; kind 1
            # (do...end binding) against the inner call's own source, which is
            # its offense range.
            t_start, t_end = kind == 1 ? [s, e] : [iss, ise]
            range = Parser::Source::Range.new(buffer, off[t_start], off[t_end])
            text = range.source
            regexps.any? { |r| text.match?(r) } ? text : nil
          end
          collected.uniq
        end
      end
    end
  end
end
