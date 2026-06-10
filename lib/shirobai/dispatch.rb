# frozen_string_literal: true

module Shirobai
  # Per-investigation coordinator: computes the offenses for a group of cops in
  # one shared Rust AST walk, memoized by source. The first participating cop on
  # a file triggers the bundled run; the rest read their slice from the cache.
  #
  # The cache key is the `raw_source` identity, so the autocorrect loop (which
  # re-investigates a freshly built `ProcessedSource`) naturally recomputes.
  module Dispatch
    STYLE_MAPS = {
      "Layout/MultilineOperationIndentation" => { aligned: 0, indented: 1 },
      "Layout/MultilineMethodCallIndentation" =>
        { aligned: 0, indented: 1, indented_relative_to_receiver: 2 }
    }.freeze

    class << self
      # Returns the offenses (raw Rust tuples) for `cop_name` on this source.
      def multiline(processed_source, config, cop_name)
        src = processed_source.raw_source
        unless defined?(@cached_source) && @cached_source.equal?(src) && @cached_config.equal?(config)
          @cached_source = src
          @cached_config = config
          @cached_result = compute_multiline(src, config)
        end
        @cached_result.fetch(cop_name)
      end

      private

      def compute_multiline(src, config)
        base = config.for_cop("Layout/IndentationWidth")["Width"] || 2
        op_cfg = flatten(config, "Layout/MultilineOperationIndentation", base)
        mc_cfg = flatten(config, "Layout/MultilineMethodCallIndentation", base)
        op_off, mc_off = Shirobai.check_multiline_bundle(src, op_cfg, mc_cfg)
        {
          "Layout/MultilineOperationIndentation" => op_off,
          "Layout/MultilineMethodCallIndentation" => mc_off
        }
      end

      # [style_u8, configured_indentation_width, base_indentation_width]
      #
      # `EnforcedStyle` is absent only when a spec configures the *other* cop in
      # the bundle (whose offenses are then discarded); default to the first
      # supported style (`0`) in that case.
      def flatten(config, cop_name, base)
        cop_config = config.for_cop(cop_name)
        style = STYLE_MAPS.fetch(cop_name)[cop_config["EnforcedStyle"]&.to_sym] || 0
        indent = cop_config["IndentationWidth"] || base
        [style, indent, base]
      end
    end
  end
end
