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
        include Shirobai::Cop::BundleEligible
        # Project-index helpers for the 1.89 cross-file detection
        # (`definitions_in_other_files` etc.) and the
        # `external_dependency_checksum` ResultCache invalidation.
        include RuboCop::Cop::ProjectIndexHelp
        MSG = "Method `%<method>s` is defined at both %<defined>s and %<current>s."

        # Stock 1.89: method names the cop registers that can be looked up in
        # the project index.
        INDEXABLE_METHOD_NAME =
          /\A(?<owner>[A-Z]\w*(?:::[A-Z]\w*)*)(?<separator>[#.])(?<name>[^#.]+)\z/

        def self.cop_name = "Lint/DuplicateMethods"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # `[active_support_extensions_enabled, delegating_methods]`
        # (`AllCops` + the 1.89 `DelegatingMethods` list).
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            !!config.active_support_extensions_enabled?,
            Array(cop_config.fetch("DelegatingMethods", ["delegate"])).map(&:to_s)
          ]
        end

        def initialize(config = nil, options = nil)
          super
          @definitions = {}
          @scopes = { 1 => [], 2 => [] } # 1 = :rescue, 2 = :ensure
        end

        def on_new_investigation
          # The self-alias trick (1.89) and Active Support's redefinition
          # markers (1.90) declare an intentional redefinition only within
          # the file that uses them.
          @intentionally_redefined = Set.new
          events = resolved_events
          return if events.empty?

          buffer = processed_source.buffer
          path = smart_path(buffer.name)
          off = SourceOffsets.for(bundle_eligible? ? processed_source.raw_source : buffer.source)

          # `flags` bits: 0-1 rescue/ensure scope / 4 intentional-redefinition
          # event / 8 the key carries a scope id / 16 inside a def.
          events.each do |flags, name, key, sexp_start, _sexp_end, scope_line, scope_begin,
                          off_start, off_end, line|
            if flags.anybits?(4) # track_intentional_redefinition
              @intentionally_redefined << name
              next
            end
            scope = flags & 3
            if sexp_start >= 0
              node = defs_node_at(off[sexp_start])
              next unless node

              name = "#{node}.#{name}"
              key = "#{key}#{name}"
            end
            # 1.89 `anon_block_identity`: begin_pos joins the id so blocks
            # sharing a line stay distinct.
            key = "#{key}@#{path}:#{scope_line}:#{off[scope_begin]}" if scope_line >= 0
            current = "#{path}:#{line}"

            if @definitions.key?(key)
              defined_display, defined_path = @definitions[key]
              if scope != 0 && !@scopes[scope].include?(key)
                @definitions[key] = [current, buffer.name]
                @scopes[scope] << key
              elsif defined_path != buffer.name &&
                    (@intentionally_redefined.include?(name) || allowed_cross_file_path?(defined_path))
                # `intentional_cross_file_redefinition?` (the self-alias trick
                # / Active Support markers) or `allowed_cross_file_redefinition?`
                # (`AllowedCrossFilePaths`, 1.90): a redefinition of a method
                # defined in another file that stock does not report.
                @definitions[key] = [current, buffer.name]
              else
                range = Parser::Source::Range.new(buffer, off[off_start], off[off_end])
                message = format(MSG, method: name, defined: defined_display, current: current)
                add_offense(range, message: message)
              end
            else
              @definitions[key] = [current, buffer.name]
              # Stock 1.89 `check_cross_file_duplicate`: first sighting, no
              # scope id, no rescue/ensure scope, not inside a def, and the
              # runner handed us an index.
              if project_index && scope.zero? && !flags.anybits?(8) && !flags.anybits?(16)
                check_cross_file_duplicate(name, current, off[off_start], off[off_end])
              end
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
              *self.class.bundle_args(config)
            )
          end
        end

        # --- 1.89 cross-file detection, verbatim from stock (the index
        # helpers come from the ProjectIndexHelp mixin) ---

        def check_cross_file_duplicate(method_name, current, begin_pos, end_pos)
          return if @intentionally_redefined.include?(method_name)
          return unless (prior = cross_file_prior_definition(method_name))

          range = Parser::Source::Range.new(processed_source.buffer, begin_pos, end_pos)
          message = format(MSG, method: method_name,
                                defined: index_source_location(prior),
                                current: current)
          add_offense(range, message: message)
        end

        def cross_file_prior_definition(method_name)
          return unless (match = INDEXABLE_METHOD_NAME.match(method_name))

          definitions = definitions_in_other_files(
            indexed_definitions(match[:owner], match[:separator], match[:name])
          ).reject { |definition| allowed_cross_file_path?(definition.location.to_file_path) }
          return if definitions.empty? || cross_file_self_alias_trick?(definitions)
          return if cross_file_redefinition_marker?(definitions, match[:name])

          first_indexed_definition(definitions)
        end

        # Cross-file duplicates whose other definition site matches one of the
        # `AllowedCrossFilePaths` patterns are skipped (1.90). Patterns are
        # matched like `Exclude` patterns, against the path relative to the
        # directory of the config file that configures the cop; absolute
        # patterns are matched against the absolute path.
        def allowed_cross_file_path?(path)
          patterns = cop_config.fetch("AllowedCrossFilePaths", [])
          return false if patterns.empty?

          relative = config.path_relative_to_config(path)
          patterns.any? { |pattern| match_path?(pattern, relative) || match_path?(pattern, path) }
        end

        def indexed_definitions(owner, separator, name)
          namespace = separator == "." ? "#{owner}::<#{owner.split('::').last}>" : owner

          if name.match?(/\A\w+=\z/)
            # Rubydex indexes `attr_writer :foo` under `foo` rather than
            # `foo=`, so writer definitions come from both declarations.
            indexed_declaration_definitions(namespace, name) +
              indexed_declaration_definitions(namespace, name.delete_suffix("="))
                .select { |definition| writer_attr_definition?(definition) }
          else
            indexed_declaration_definitions(namespace, name).grep_v(Rubydex::AttrWriterDefinition)
          end
        end

        def indexed_declaration_definitions(namespace, name)
          project_index["#{namespace}##{name}()"]&.definitions.to_a
        end

        def writer_attr_definition?(definition)
          definition.is_a?(Rubydex::AttrWriterDefinition) ||
            definition.is_a?(Rubydex::AttrAccessorDefinition)
        end

        def cross_file_self_alias_trick?(definitions)
          aliases, others = definitions.partition do |definition|
            definition.is_a?(Rubydex::MethodAliasDefinition)
          end
          alias_paths = aliases.map { |definition| definition.location.to_file_path }

          others.any? { |definition| alias_paths.include?(definition.location.to_file_path) }
        end

        # An Active Support redefinition marker alongside one of the
        # definitions in another file marks the redefinition there as
        # intentional, like the self-alias trick (1.90). The index does not
        # record the marker's method-name argument, so the other file's
        # source is searched for a marker naming the method.
        def cross_file_redefinition_marker?(definitions, name)
          return false unless active_support_extensions_enabled?

          definitions.map { |definition| definition.location.to_file_path }.uniq.any? do |path|
            redefinition_marker_in_file?(path, name)
          end
        end

        def redefinition_marker_in_file?(path, name)
          return false unless File.readable?(path)

          escaped = Regexp.escape(name)
          marker = /\b(?:silence_redefinition_of_method|redefine_method)\s*\(?\s*
                    (?::#{escaped}|(["'])#{escaped}\1)(?![\w=!?])/x
          File.read(path).scrub.match?(marker)
        end

        def first_indexed_definition(definitions)
          definitions.find { |definition| !definition.is_a?(Rubydex::MethodAliasDefinition) }
        end

        def index_source_location(definition)
          location = definition.location
          "#{smart_path(location.to_file_path)}:#{location.to_display.start_line}"
        end

        # The parser-gem defs node starting at char offset `begin_pos`
        # (begin offsets are unique per node start, so no further
        # disambiguation is needed).
        def defs_node_at(begin_pos)
          processed_source.ast&.each_node(:defs)&.find do |n|
            n.source_range.begin_pos == begin_pos
          end
        end
      end
    end
  end
end
