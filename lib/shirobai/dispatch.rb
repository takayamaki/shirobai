# frozen_string_literal: true

module Shirobai
  # Per-file coordinator: computes every cop's offenses in ONE bundled ext call
  # (`Shirobai.check_all`), memoized by source. The first participating cop on
  # a file triggers the bundled run; the rest read their slice from the cache.
  #
  # The cache key is the `raw_source` identity (`equal?`, not `==`), so the
  # autocorrect loop (which re-investigates a freshly built `ProcessedSource`)
  # naturally recomputes, and a different file never reuses stale results.
  #
  # A cop whose per-investigation config/state cannot be represented in the
  # bundle (see each wrapper's `bundle_eligible?`) skips this coordinator and
  # calls its standalone entry point directly instead.
  module Dispatch
    # Slot order of the `Shirobai.check_all` result Array. Mirrors the order
    # documented on `BundleConfig` and built by `check_all` in
    # `ext/shirobai/src/lib.rs`; each slot carries the same shape as the cop's
    # standalone `Shirobai.check_*` return value.
    SLOTS = {
      debugger: 0,
      block_length: 1,
      block_nesting: 2,
      complexity: 3,
      variable_number: 4,
      method_name: 5,
      safe_navigation_chain: 6,
      multiline_operation: 7,
      multiline_method_call: 8,
      dot_position: 9,
      line_length: 10,
      line_length_breakables: 11,
      line_end_concatenation: 12,
      argument_alignment: 13,
      first_argument_indentation: 14,
      redundant_self: 15,
      indentation_width: 16,
      predicate_prefix: 17,
      closing_parenthesis_indentation: 18,
      first_array_element_indentation: 19,
      hash_each_methods: 20,
      void: 21,
      useless_access_modifier: 22,
      empty_lines_around_method_body: 23,
      empty_lines_around_class_body: 24,
      empty_lines_around_module_body: 25,
      empty_lines_around_block_body: 26,
      empty_lines_around_begin_body: 27,
      empty_lines_around_exception_handling_keywords: 28,
      block_delimiters: 29,
      abc_size: 30,
      indentation_consistency: 31,
      empty_line_between_defs: 32
    }.freeze

    class << self
      # Returns the raw Rust result for `cop_key` on this source.
      def offenses_for(processed_source, config, cop_key)
        src = processed_source.raw_source
        unless defined?(@cached_source) && @cached_source.equal?(src) && @cached_config.equal?(config)
          result = Shirobai.check_all(src, bundle_token(config))
          @cached_source = src
          @cached_config = config
          @cached_result = result
        end
        @cached_result.fetch(SLOTS.fetch(cop_key))
      end

      # The Rust-side token for `config`, registering its packed bundle config
      # on first sight. Memoized per config object identity: a lint run shares
      # one `Config` object across all cops in the team, so a run registers
      # O(#distinct configs) entries (each spec example registers one; entries
      # are small and never evicted).
      def bundle_token(config)
        @bundle_tokens ||= {}.compare_by_identity
        @bundle_tokens[config] ||= Shirobai.register_bundle_config(*packed_config(config))
      end

      private

      # Builds the `(nums, lists)` wire format documented on `BundleConfig`
      # (crates/shirobai-core/src/rules/bundle.rs). Every cop's values come
      # from its `bundle_args` class method — the same derivation its instance
      # uses — resolved from `config` alone (cop-own config via
      # `config.for_badge`, exactly like `RuboCop::Cop::Base#cop_config`).
      def packed_config(config)
        dbg = Cop::Lint::Debugger.bundle_args(config)
        bl = Cop::Metrics::BlockLength.bundle_args(config)
        bn = Cop::Metrics::BlockNesting.bundle_args(config)
        cx = Cop::Metrics::ComplexityShared.bundle_args(config)
        vn = Cop::Naming::VariableNumber.bundle_args(config)
        mn = Cop::Naming::MethodName.bundle_args(config)
        snc = Cop::Lint::SafeNavigationChain.bundle_args(config)
        dot = Cop::Layout::DotPosition.bundle_args(config)
        ll = Cop::Layout::LineLength.bundle_args(config)
        op = Cop::Layout::MultilineOperationIndentation.bundle_args(config)
        mc = Cop::Layout::MultilineMethodCallIndentation.bundle_args(config)
        aa = Cop::Layout::ArgumentAlignment.bundle_args(config)
        fai = Cop::Layout::FirstArgumentIndentation.bundle_args(config)
        iw = Cop::Layout::IndentationWidth.bundle_args(config)
        rs = Cop::Style::RedundantSelf.bundle_args(config)
        pp = Cop::Naming::PredicatePrefix.bundle_args(config)
        cpi = Cop::Layout::ClosingParenthesisIndentation.bundle_args(config)
        fae = Cop::Layout::FirstArrayElementIndentation.bundle_args(config)
        hem = Cop::Style::HashEachMethods.bundle_args(config)
        vd = Cop::Lint::Void.bundle_args(config)
        uam = Cop::Lint::UselessAccessModifier.bundle_args(config)
        elb_class = Cop::Layout::EmptyLinesAroundClassBody.bundle_args(config)
        elb_module = Cop::Layout::EmptyLinesAroundModuleBody.bundle_args(config)
        elb_block = Cop::Layout::EmptyLinesAroundBlockBody.bundle_args(config)
        bd = Cop::Style::BlockDelimiters.bundle_args(config)
        abc = Cop::Metrics::AbcSize.bundle_args(config)
        ic = Cop::Layout::IndentationConsistency.bundle_args(config)
        elbd = Cop::Layout::EmptyLineBetweenDefs.bundle_args(config)

        nums = [
          bl[0], num(bl[1]), 1, # BlockLength Max / CountComments / filtered (eligibility implies the fast path)
          bn[0], num(bn[1]), num(bn[2]), # BlockNesting Max / CountBlocks / CountModifierForms
          cx[0], cx[1],                  # complexity prefilter maxes
          vn[0], vn[1],                  # VariableNumber style / flags
          mn[0],                         # MethodName style (bundle always computes the filtered flavor)
          dot[0],                        # DotPosition style
          ll[0], ll[1], num(ll[2]),      # LineLength Max / tab width / SplitStrings
          op[0], op[1], op[2],           # MultilineOperationIndentation style / indent / base
          mc[0], mc[1], mc[2],           # MultilineMethodCallIndentation style / indent / base
          aa[0], aa[1], num(aa[2]),      # ArgumentAlignment style / indent / incompatible
          fai[0], fai[1], num(fai[2]),   # FirstArgumentIndentation style / indent / enforce flag
          *iw,                           # IndentationWidth packed config (7 nums)
          cpi[0],                        # ClosingParenthesisIndentation indent width
          fae[0], fae[1], num(fae[2]),   # FirstArrayElementIndentation style / indent / enforce flag
          num(vd[0]),                    # Void CheckForMethodsWithNoSideEffects
          num(uam[2]),                   # UselessAccessModifier ActiveSupportExtensionsEnabled
          elb_class[0], elb_module[0], elb_block[0], # EmptyLinesAround{Class,Module,Block}Body styles
          *bd[0],                        # BlockDelimiters style / procedural-oneliners flag
          abc[0], abc[1],                # AbcSize max_floor / discount_repeated
          ic[0],                         # IndentationConsistency indented_internal_methods
          *elbd[0]                        # EmptyLineBetweenDefs method/class/module/adjacent/min/max
        ]
        lists = [dbg[0], dbg[1], bl[2], bl[3], vn[2], snc[0], rs[0], pp[0], pp[1], hem[0],
                 uam[0], uam[1], *bd[1], elbd[1]]
        [nums, lists]
      end

      def num(value)
        value ? 1 : 0
      end
    end
  end
end
