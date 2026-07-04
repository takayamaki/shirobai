# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/DuplicateMethods`.
      #
      # Stock keeps `@definitions` / `@scopes` on the cop instance, which
      # RuboCop reuses across every file sharing a config — duplicate
      # detection is deliberately cross-file (`Method ... is defined at both
      # first.rb:2 and second.rb:2.`). The Rust side therefore returns the
      # per-file part only: the exact stream of stock `found_method` calls
      # (key, message name, offense range, rescue/ensure scope) in callback
      # order, and this wrapper replays stock's bookkeeping against its own
      # cross-investigation state. The replay is a hash lookup per event;
      # all AST work (scope resolution, `parent_module_name`, anonymous
      # `Class.new` blocks, attr/alias/delegator matchers) happens in Rust
      # on the shared walk.
      #
      # Two event flavors need Ruby-side completion:
      #
      # - `scope_line >= 0`: the key gets an `"@#{smart_path}:#{line}"`
      #   suffix (stock's `source_location`-based anonymous-block scope id;
      #   Rust does not know the buffer name).
      # - `sexp_start >= 0`: stock's `lookup_constant` failed and (through
      #   `each_ancestor`'s block-form return value of `self`) the key
      #   embeds the parser-gem s-expression of the whole defs node. The
      #   wrapper finds that node in `processed_source.ast` and
      #   interpolates it with stock's own `Node#to_s`, staying
      #   byte-identical for arbitrary bodies.
      #
      # No autocorrect (stock has none).
      class DuplicateMethods < RuboCop::Cop::Base
        MSG = "Method `%<method>s` is defined at both %<defined>s and %<current>s."

        def self.cop_name = "Lint/DuplicateMethods"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # `[active_support_extensions_enabled]` (`AllCops`).
        def self.bundle_args(config)
          [!!config.active_support_extensions_enabled?]
        end

        def initialize(config = nil, options = nil)
          super
          @definitions = {}
          @scopes = { 1 => [], 2 => [] } # 1 = :rescue, 2 = :ensure
        end

        def on_new_investigation
          events = resolved_events
          return if events.empty?

          buffer = processed_source.buffer
          path = smart_path(buffer.name)
          off = SourceOffsets.for(bundle_eligible? ? processed_source.raw_source : buffer.source)

          events.each do |name, key, sexp_start, _sexp_end, scope_line, off_start, off_end, line, scope|
            if sexp_start >= 0
              node = defs_node_at(off[sexp_start])
              next unless node

              name = "#{node}.#{name}"
              key = "#{key}#{name}"
            end
            key = "#{key}@#{path}:#{scope_line}" if scope_line >= 0
            current = "#{path}:#{line}"

            if @definitions.key?(key)
              if scope != 0 && !@scopes[scope].include?(key)
                @definitions[key] = current
                @scopes[scope] << key
              else
                range = Parser::Source::Range.new(buffer, off[off_start], off[off_end])
                message = format(MSG, method: name, defined: @definitions[key], current: current)
                add_offense(range, message: message)
              end
            else
              @definitions[key] = current
            end
          end
        end

        private

        def resolved_events
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :duplicate_methods)
          else
            Shirobai.check_duplicate_methods(
              processed_source.buffer.source,
              !!config.active_support_extensions_enabled?
            )
          end
        end

        # The parser-gem defs node starting at char offset `begin_pos`
        # (begin offsets are unique per node start, so no further
        # disambiguation is needed).
        def defs_node_at(begin_pos)
          processed_source.ast&.each_node(:defs)&.find do |n|
            n.source_range.begin_pos == begin_pos
          end
        end

        # Eligible only when parser-gem's buffer source is byte-identical
        # to the raw source the bundle scans (CRLF / BOM / `__END__`
        # truncation break that). Memoized per investigation; the flag is
        # reset each `on_new_investigation` via `begin_investigation`'s
        # fresh `@processed_source`, so memoize on the source identity.
        def bundle_eligible?
          src = processed_source
          return @bundle_eligible if defined?(@bundle_eligible_for) && @bundle_eligible_for.equal?(src)

          @bundle_eligible_for = src
          @bundle_eligible = src.buffer.source == src.raw_source
        end
      end
    end
  end
end
