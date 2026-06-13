# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/HashAlignment`.
      #
      # Rust parses the source, walks every multi-line hash literal (replicating
      # the `EnforcedLastArgumentHashStyle` `ignore_node` of a call's last hash
      # argument and the `Layout/ArgumentAlignment: with_fixed_indentation`
      # incompatibility skip), and for each pair computes the column-delta triple
      # `{key, separator, value}` under each configured alignment
      # (`EnforcedHashRocketStyle` / `EnforcedColonStyle`, possibly multi-style).
      # A non-zero delta is an offense; for the colon/rocket flavours the
      # least-offending permitted style wins, and keyword-splat offenses are
      # always reported. Rust returns, per offending pair / kwsplat, the offense
      # range, a message selector, the delta triple and the byte ranges of the
      # key / operator / value (parser geometry). Ruby applies the realignment
      # via `corrector.insert_before` / `corrector.remove`, exactly like stock's
      # `adjust` (including the `key_delta` clamp to `-key.column`). Offenses
      # come from the per-file bundled run (`Shirobai::Dispatch`); the config
      # derivation is purely config-driven, so this cop is always
      # bundle-eligible.
      class HashAlignment < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MESSAGES = [
          "Align the keys of a hash literal if they span more than one line.",
          "Align the separators of a hash literal if they span more than one line.",
          "Align the keys and values of a hash literal if they span more than one line.",
          "Align keyword splats with the rest of the hash if it spans more than one line."
        ].freeze

        STYLE_CODES = { "key" => "key", "separator" => "separator", "table" => "table" }.freeze
        LAST_ARG_CODES = {
          "always_inspect" => 0,
          "always_ignore" => 1,
          "ignore_explicit" => 2,
          "ignore_implicit" => 3
        }.freeze

        def self.cop_name = "Layout/HashAlignment"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/HashAlignment")

        # Packed args for the bundled run: `[rocket_styles, colon_styles,
        # last_argument_style_code, enforce_fixed]`. The style lists carry the
        # `EnforcedHashRocketStyle` / `EnforcedColonStyle` values in config order
        # (deduplicated downstream, matching stock's `formats.uniq`). The
        # enforce flag replicates `enforce_first_argument_with_fixed_indentation?`
        # (`Layout/ArgumentAlignment` `with_fixed_indentation`), driving the
        # `autocorrect_incompatible_with_other_cops?` skip.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          arg_alignment_config = config.for_enabled_cop("Layout/ArgumentAlignment")
          [
            normalize_styles(cop_config["EnforcedHashRocketStyle"]),
            normalize_styles(cop_config["EnforcedColonStyle"]),
            LAST_ARG_CODES.fetch(cop_config["EnforcedLastArgumentHashStyle"], 0),
            arg_alignment_config["EnforcedStyle"] == "with_fixed_indentation"
          ]
        end

        def self.normalize_styles(value)
          formats = value.is_a?(String) ? [value] : Array(value)
          formats = ["key"] if formats.empty?
          formats.map { |f| STYLE_CODES.fetch(f, "key") }
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :hash_alignment)
          off = SourceOffsets.for(processed_source.raw_source)
          # Stock registers a hash's offenses inside one `on_hash` callback. If
          # one offense's corrector raises `Parser::ClobberingError` (adjacent
          # key/separator/value adjustments that collide), it aborts THAT
          # callback — keeping the offenses already added but dropping the
          # clobbering one and the rest of that hash — while other hashes
          # (separate callbacks) are unaffected. We add all offenses in one
          # `on_new_investigation`, so we reproduce that confinement explicitly:
          # the Rust `group` id marks each source hash, and a clobber skips only
          # the remaining offenses of its own group.
          aborted_group = nil
          offenses.each do |group, start, fin, message_idx, has_value, deltas, key, op, value|
            next if group == aborted_group

            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            begin
              add_offense(range, message: MESSAGES[message_idx]) do |corrector|
                if has_value
                  correct_key_value(corrector, buffer, off, deltas, key, op, value)
                else
                  # `correct_no_value`: adjust the whole node by the key delta.
                  adjust(corrector, deltas[0], range)
                end
              end
            rescue Parser::ClobberingError
              aborted_group = group
            end
          end
        end

        private

        # Mirrors stock's `correct_key_value`: adjust key, separator and value
        # by their deltas, clamping the key delta to `-key.column`.
        def correct_key_value(corrector, buffer, off, deltas, key, op, value)
          key_delta, separator_delta, value_delta = deltas
          key_start, _key_end, key_column = key

          key_delta = -key_column if key_delta < -key_column

          key_range = Parser::Source::Range.new(buffer, off[key_start], off[key[1]])
          op_range = Parser::Source::Range.new(buffer, off[op[0]], off[op[1]])
          value_range = Parser::Source::Range.new(buffer, off[value[0]], off[value[1]])

          adjust(corrector, key_delta, key_range)
          adjust(corrector, separator_delta, op_range)
          adjust(corrector, value_delta, value_range)
        end

        # Mirrors stock's `adjust`: insert spaces for a positive delta, remove
        # `delta.abs` characters before the range for a negative one.
        def adjust(corrector, delta, range)
          if delta.positive?
            corrector.insert_before(range, " " * delta)
          elsif delta.negative?
            corrector.remove(range_between(range.begin_pos - delta.abs, range.begin_pos))
          end
        end
      end
    end
  end
end
