# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/UselessAccessModifier`.
      #
      # Rust walks the AST once and replicates the union of stock's entry
      # points: the `check_node` handlers on class/module/sclass bodies and
      # eval-style / `included` blocks, the `check_scope` recursion that
      # tracks `cur_vis`/`unused` through transparent children (conditionals,
      # `begin` bodies, plain blocks, hash values…) while skipping method
      # definitions and `defs`, and the unconditional top-level `on_begin`
      # flagging — including the `macro?` scope chain that gates
      # `bare_access_modifier?` and the `private_class_method`-with-arguments
      # state reset. Offense ranges (the modifier send) and modifier names
      # come from Rust; Ruby formats stock's message and derives the
      # whole-line removal with the stock `range_by_whole_lines` helper, so
      # the autocorrect is byte-identical by construction. Offenses come from
      # the per-file bundled run (`Shirobai::Dispatch`); the config is two
      # string lists (`ContextCreatingMethods` / `MethodCreatingMethods`)
      # plus the `ActiveSupportExtensionsEnabled` flag, so this cop is always
      # bundle-eligible.
      class UselessAccessModifier < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "Useless `%<current>s` access modifier."

        def self.cop_name = "Lint/UselessAccessModifier"
        def self.badge = RuboCop::Cop::Badge.parse("Lint/UselessAccessModifier")

        # Packed args for the bundled run:
        # `[context_creating_methods, method_creating_methods, active_support_extensions_enabled]`.
        def self.bundle_args(config)
          cop_cfg = config.for_badge(badge)
          [
            Array(cop_cfg["ContextCreatingMethods"]).map(&:to_s),
            Array(cop_cfg["MethodCreatingMethods"]).map(&:to_s),
            !!config.active_support_extensions_enabled?
          ]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :useless_access_modifier)
          return if offenses.empty?

          # Rust reports prism BYTE offsets, but `Parser::Source::Range`
          # indexes the buffer by CHARACTERS; on a non-ASCII file every
          # offense after a multibyte character would land shifted (verified
          # against stock on Ruby's own `fileutils.rb`). `ascii_only?` is a
          # cached coderange lookup, and the byteslice conversion only runs
          # for the rare offenses inside non-ASCII files.
          src = processed_source.raw_source
          ascii = src.ascii_only?
          offenses.each do |start, fin, current|
            unless ascii
              start = src.byteslice(0, start).length
              fin = src.byteslice(0, fin).length
            end
            range = Parser::Source::Range.new(buffer, start, fin)
            add_offense(range, message: format(MSG, current: current)) do |corrector|
              corrector.remove(range_by_whole_lines(range, include_final_newline: true))
            end
          end
        end
      end
    end
  end
end
