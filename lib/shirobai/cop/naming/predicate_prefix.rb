# frozen_string_literal: true

module Shirobai
  module Cop
    module Naming
      # Drop-in Rust reimplementation of `Naming/PredicatePrefix`.
      #
      # Rust walks the definition sites (`def`/`defs` and the configured
      # `MethodDefinitionMacros` calls with a symbol first argument), keeps the
      # rare sites whose name literally starts with a configured `NamePrefix`
      # entry (the same cheap gate the stock cop applies before its regex
      # check) and reports them together with whether a
      # `sig { returns(T::Boolean) }` block immediately precedes the
      # definition. Ruby then replays the stock per-prefix filtering verbatim
      # on those candidates — the `/^#{prefix}[^0-9]/` interpolation,
      # `expected_name` equality, assignment names, `AllowedMethods` and
      # `UseSorbetSigs` — so every config option keeps its stock semantics
      # (including regex-interpolated prefixes). With every filter applied
      # after the ext call, this cop is always bundle-eligible.
      class PredicatePrefix < RuboCop::Cop::Base
        include RuboCop::Cop::AllowedMethods

        def self.cop_name = "Naming/PredicatePrefix"
        def self.badge = RuboCop::Cop::Badge.parse("Naming/PredicatePrefix")

        # Packed args for the bundled run: `[name_prefixes, macros]` (two of
        # the bundle's string lists). Both may be absent when the config does
        # not mention this cop (the slice is then discarded); default to empty
        # lists (no candidates).
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            Array(cop_config["NamePrefix"]).map(&:to_s),
            Array(cop_config["MethodDefinitionMacros"]).map(&:to_s)
          ]
        end

        def on_new_investigation
          candidates = Dispatch.offenses_for(processed_source, config, :predicate_prefix)

          off = SourceOffsets.for(processed_source.raw_source)
          candidates.each do |start, fin, method_name, is_def, sorbet_boolean_sig|
            predicate_prefixes.each do |prefix|
              next if allowed_method_name?(method_name, prefix)
              # The Sorbet exemption only applies to `def`/`defs` sites; the
              # stock `on_send` (macro) path never consults the sig.
              next if is_def && use_sorbet_sigs? && !sorbet_boolean_sig

              range = Parser::Source::Range.new(processed_source.buffer, off[start], off[fin])
              add_offense(range, message: message(method_name, expected_name(method_name, prefix)))
            end
          end
        end

        def validate_config
          forbidden_prefixes.each do |forbidden_prefix|
            next if predicate_prefixes.include?(forbidden_prefix)

            raise RuboCop::ValidationError, <<~MSG.chomp
              The `Naming/PredicatePrefix` cop is misconfigured. Prefix #{forbidden_prefix} must be included in NamePrefix because it is included in ForbiddenPrefixes.
            MSG
          end
        end

        private

        # Verbatim from the stock cop (minus the `start_with?` gate's role as
        # candidate filter, which Rust already applied with identical
        # semantics): the regex interpolation, the corrected-name equality and
        # the `AllowedMethods` lookup all run on the original Ruby side.
        def allowed_method_name?(method_name, prefix)
          !(method_name.start_with?(prefix) && # cheap check to avoid allocating Regexp
              method_name.match?(/^#{prefix}[^0-9]/)) ||
            method_name == expected_name(method_name, prefix) ||
            method_name.end_with?("=") ||
            allowed_method?(method_name)
        end

        def expected_name(method_name, prefix)
          new_name = if forbidden_prefixes.include?(prefix)
                       method_name.sub(prefix, "")
                     else
                       method_name.dup
                     end
          new_name << "?" unless method_name.end_with?("?")
          new_name
        end

        def message(method_name, new_name)
          "Rename `#{method_name}` to `#{new_name}`."
        end

        def forbidden_prefixes
          cop_config["ForbiddenPrefixes"]
        end

        def predicate_prefixes
          cop_config["NamePrefix"]
        end

        def use_sorbet_sigs?
          cop_config["UseSorbetSigs"]
        end
      end
    end
  end
end
