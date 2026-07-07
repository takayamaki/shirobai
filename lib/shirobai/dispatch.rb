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
    # Origin order of the `Shirobai.check_all` result Array and of the packed
    # config segments: outer index 0 is the core batch, 1 the
    # shirobai-performance plugin, 2 the shirobai-rspec plugin, 3 the
    # shirobai-rails plugin, and so on. Mirrors the origin constants in
    # `crates/shirobai-core/src/rules/bundle.rs` (`ORIGIN_*` / `N_ORIGINS`);
    # adding a plugin means one entry here and one constant there — that pair
    # of one-line edits is the only place plugin batches can conflict.
    ORIGINS = %i[core performance rspec rails].freeze

    # `[origin, rule]` slot pairs into the `Shirobai.check_all` result
    # (`result[origin][rule]`). The rule order within each origin mirrors the
    # order documented on `BundleConfig` and built by `check_all` in
    # `ext/shirobai/src/lib.rs`; each slot carries the same shape as the
    # cop's standalone `Shirobai.check_*` return value. Wrappers never index
    # the result themselves — they go through `offenses_for`.
    SLOTS = {
      debugger: [0, 0].freeze,
      block_length: [0, 1].freeze,
      block_nesting: [0, 2].freeze,
      complexity: [0, 3].freeze,
      variable_number: [0, 4].freeze,
      method_name: [0, 5].freeze,
      safe_navigation_chain: [0, 6].freeze,
      multiline_operation: [0, 7].freeze,
      multiline_method_call: [0, 8].freeze,
      dot_position: [0, 9].freeze,
      line_length: [0, 10].freeze,
      line_length_breakables: [0, 11].freeze,
      line_end_concatenation: [0, 12].freeze,
      argument_alignment: [0, 13].freeze,
      first_argument_indentation: [0, 14].freeze,
      redundant_self: [0, 15].freeze,
      indentation_width: [0, 16].freeze,
      predicate_prefix: [0, 17].freeze,
      closing_parenthesis_indentation: [0, 18].freeze,
      first_array_element_indentation: [0, 19].freeze,
      hash_each_methods: [0, 20].freeze,
      void: [0, 21].freeze,
      useless_access_modifier: [0, 22].freeze,
      empty_lines_around_method_body: [0, 23].freeze,
      empty_lines_around_class_body: [0, 24].freeze,
      empty_lines_around_module_body: [0, 25].freeze,
      empty_lines_around_block_body: [0, 26].freeze,
      empty_lines_around_begin_body: [0, 27].freeze,
      empty_lines_around_exception_handling_keywords: [0, 28].freeze,
      block_delimiters: [0, 29].freeze,
      abc_size: [0, 30].freeze,
      indentation_consistency: [0, 31].freeze,
      empty_line_between_defs: [0, 32].freeze,
      end_alignment: [0, 33].freeze,
      block_alignment: [0, 34].freeze,
      else_alignment: [0, 35].freeze,
      first_hash_element_indentation: [0, 36].freeze,
      hash_alignment: [0, 37].freeze,
      empty_lines_around_arguments: [0, 38].freeze,
      hash_syntax: [0, 39].freeze,
      string_literals: [0, 40].freeze,
      trailing_comma_in_arguments: [0, 41].freeze,
      string_literals_in_interpolation: [0, 42].freeze,
      trailing_empty_lines: [0, 43].freeze,
      space_around_method_call_operator: [0, 44].freeze,
      space_around_keyword: [0, 45].freeze,
      space_inside_block_braces: [0, 46].freeze,
      method_length: [0, 47].freeze,
      def_end_alignment: [0, 48].freeze,
      require_parentheses: [0, 49].freeze,
      self_assignment: [0, 50].freeze,
      nested_parenthesized_calls: [0, 51].freeze,
      parentheses_as_grouped_expression: [0, 52].freeze,
      percent_literal_delimiters: [0, 53].freeze,
      multiline_method_call_brace_layout: [0, 54].freeze,
      access_modifier_indentation: [0, 55].freeze,
      assignment_indentation: [0, 56].freeze,
      redundant_self_assignment: [0, 57].freeze,
      colon_method_call: [0, 58].freeze,
      stabby_lambda_parentheses: [0, 59].freeze,
      unreachable_code: [0, 60].freeze,
      hash_transform_keys: [0, 61].freeze,
      ambiguous_block_association: [0, 62].freeze,
      empty_line_after_guard_clause: [0, 63].freeze,
      empty_comment: [0, 64].freeze,
      empty_line_after_magic_comment: [0, 65].freeze,
      empty_lines: [0, 66].freeze,
      leading_empty_lines: [0, 67].freeze,
      class_length: [0, 68].freeze,
      module_length: [0, 69].freeze,
      trailing_comma_in_hash_literal: [0, 70].freeze,
      trailing_comma_in_array_literal: [0, 71].freeze,
      space_inside_hash_literal_braces: [0, 72].freeze,
      space_inside_array_literal_brackets: [0, 73].freeze,
      space_before_block_braces: [0, 74].freeze,
      if_unless_modifier: [0, 75].freeze,
      space_before_comma: [0, 76].freeze,
      space_after_comma: [0, 77].freeze,
      space_before_semicolon: [0, 78].freeze,
      space_after_semicolon: [0, 79].freeze,
      space_after_colon: [0, 80].freeze,
      space_before_comment: [0, 81].freeze,
      space_inside_parens: [0, 82].freeze,
      space_inside_reference_brackets: [0, 83].freeze,
      space_before_first_arg: [0, 84].freeze,
      duplicate_magic_comment: [0, 85].freeze,
      duplicate_methods: [0, 86].freeze,
      array_alignment: [0, 87].freeze,
      file_null: [0, 88].freeze,
      semicolon: [0, 89].freeze,
      redundant_freeze: [0, 90].freeze,
      frozen_string_literal_comment: [0, 91].freeze,
      arguments_forwarding: [0, 92].freeze,
      # shirobai-performance plugin slots (origin 1). Always present in the
      # wire format; the Rust side leaves them empty unless the plugin gem
      # registered its packed segment (`Dispatch.register_plugin_packer`).
      perf_detect: [1, 0].freeze,
      perf_string_include: [1, 1].freeze,
      perf_end_with: [1, 2].freeze,
      perf_start_with: [1, 3].freeze,
      perf_times_map: [1, 4].freeze,
      # shirobai-rspec plugin slots (origin 2), all filled by the single
      # RSpecDispatcherRule. The rspec origin is additionally gated per
      # file (see `register_plugin_packer`'s `gate`): the RSpec department
      # only runs on spec files, so other files use a dormant-rspec token.
      rspec_variable_name: [2, 0].freeze,
      rspec_let_setup: [2, 1].freeze,
      rspec_variable_definition: [2, 2].freeze,
      rspec_multiple_memoized_helpers: [2, 3].freeze,
      rspec_repeated_description: [2, 4].freeze,
      rspec_repeated_example: [2, 5].freeze,
      rspec_named_subject: [2, 6].freeze,
      # R2 metadata family (rspec origin slots 7-12). Slots 9-12 (the four
      # Metadata-mixin cops) read the same shared metadata-anchor list.
      rspec_focus: [2, 7].freeze,
      rspec_pending_without_reason: [2, 8].freeze,
      rspec_metadata_style: [2, 9].freeze,
      rspec_duplicated_metadata: [2, 10].freeze,
      rspec_empty_metadata: [2, 11].freeze,
      rspec_sort_metadata: [2, 12].freeze,
      # R2 empty-line family (rspec origin slots 13-17), all filled by the
      # single RSpecEmptyLineRule.
      rspec_empty_line_after_example: [2, 13].freeze,
      rspec_empty_line_after_example_group: [2, 14].freeze,
      rspec_empty_line_after_final_let: [2, 15].freeze,
      rspec_empty_line_after_hook: [2, 16].freeze,
      rspec_empty_line_after_subject: [2, 17].freeze,
      rspec_empty_example_group: [2, 18].freeze,
      rspec_described_class: [2, 19].freeze,
      rspec_scattered_setup: [2, 20].freeze,
      # shirobai-rails plugin slots (origin 3), all filled by the single
      # RailsAppVisitor. The rails origin is NOT per-file gated: the
      # Application* cops run on every Ruby file, so it is always awake once
      # the plugin gem is loaded.
      rails_application_record: [3, 0].freeze,
      rails_application_controller: [3, 1].freeze,
      rails_application_mailer: [3, 2].freeze,
      rails_application_job: [3, 3].freeze,
      # send/block-table cluster (rails origin slots 4-5), each its own rule.
      rails_unknown_env: [3, 4].freeze,
      rails_dynamic_find_by: [3, 5].freeze,
      # Rails Architecture-B cops (origin 3 slots 6-7): the Rust side emits
      # candidate send ranges here, and the wrapper relocates the parser node
      # and runs stock detection + autocorrect verbatim (same shape as the
      # rspec send-candidate cops). The 3-point positional sync (this map,
      # the `BundleResult` field order in `crates/.../bundle.rs`, the ext
      # `check_all` push order) moves together.
      rails_http_positional_arguments: [3, 6].freeze,
      rails_deprecated_active_model_errors_methods: [3, 7].freeze,
      rails_pluck: [3, 8].freeze
    }.freeze

    # Dormant packed-config segment per plugin origin: the enable flag (first
    # num) is 0 and the cop settings are placeholders (see the `BundleConfig`
    # docs for each segment's field order). Packed whenever the plugin gem
    # has not registered a packer — or, for a gated origin, whenever the
    # origin's gate says the current file is not relevant; the Rust side
    # then skips that origin's rules and leaves its slots empty.
    DORMANT_SEGMENTS = {
      performance: [[0, 0, 0].freeze, [[].freeze].freeze].freeze,
      # rspec: enable flag + per-cop nums (VariableName style,
      # VariableDefinition style, MMH Max, MMH AllowSubject, NamedSubject
      # style, NamedSubject IgnoreSharedExamples, EmptyLineAfterExample
      # AllowConsecutiveOneLiners, EmptyLineAfterHook
      # AllowConsecutiveOneLiners), then the sixteen
      # RSpec/Language role lists.
      rspec: [[0, 0, 0, 0, 0, 0, 0, 0, 0].freeze, ([[].freeze] * 16).freeze].freeze,
      # rails: wake-up flag + UnknownEnv supports_local; four lists
      # (UnknownEnv Environments + DynamicFindBy AllowedMethods /
      # AllowedReceivers / Whitelist).
      rails: [[0, 0].freeze, [[].freeze, [].freeze, [].freeze, [].freeze].freeze].freeze
    }.freeze

    class << self
      # Registration point for plugin gems: `origin` is the ORIGINS key and
      # the block is a callable `(config) -> [nums, lists]` producing that
      # origin's segment documented on `BundleConfig` (enable flag first).
      # Called at plugin require time, BEFORE any lint run packs a config.
      # Configs packed earlier keep their dormant tokens (token memoization
      # is per Config identity), so requiring a plugin mid-run is not
      # supported.
      #
      # `gate` (optional) makes the origin per-file: a callable
      # `(config, processed_source) -> bool` answering "does any of this
      # origin's cops run on this file?". Department-scoped plugins
      # (rubocop-rspec's `Include: **/*_spec.rb`) pass one so non-relevant
      # files use a token whose segment is dormant and never pay for the
      # origin's rules. The gate must be a superset of the wrapper cops'
      # `relevant_file?` — a wrapper that runs on a gated-off file falls
      # back to its standalone entry point (`offenses_for` returns nil).
      def register_plugin_packer(origin, gate: nil, &packer)
        raise ArgumentError, "unknown origin #{origin.inspect}" unless ORIGINS.include?(origin)

        plugin_packers[origin] = packer
        plugin_gates[origin] = gate if gate
      end

      def plugin_packers
        @plugin_packers ||= {}
      end

      def plugin_gates
        @plugin_gates ||= {}
      end

      # Returns the raw Rust result for `cop_key` on this source, or nil when
      # `cop_key` belongs to a gated origin that is inactive for this file
      # (the safety net: the wrapper then takes its standalone entry point,
      # so a gate/relevant_file? disagreement can only cost speed, never
      # offenses).
      def offenses_for(processed_source, config, cop_key)
        src = processed_source.raw_source
        unless defined?(@cached_source) && @cached_source.equal?(src) && @cached_config.equal?(config)
          inactive = inactive_origins(config, processed_source)
          result = Shirobai.check_all(src, bundle_token(config, inactive))
          @cached_source = src
          @cached_config = config
          @cached_result = result
          @cached_inactive = inactive
        end
        origin, rule = SLOTS.fetch(cop_key)
        return nil if @cached_inactive.include?(ORIGINS.fetch(origin))

        @cached_result.fetch(origin).fetch(rule)
      end

      # The Rust-side token for `config` with the given gated-off origins,
      # registering its packed bundle config on first sight. Memoized per
      # (config object identity, inactive origin set): a lint run shares one
      # `Config` object across all cops in the team, so a run registers
      # O(#distinct configs x #distinct gate outcomes) entries (each spec
      # example registers one; entries are small and never evicted).
      def bundle_token(config, inactive = EMPTY_INACTIVE)
        @bundle_tokens ||= {}.compare_by_identity
        per_config = (@bundle_tokens[config] ||= {})
        per_config[inactive] ||=
          Shirobai.register_bundle_config(*packed_config(config, inactive))
      end

      private

      EMPTY_INACTIVE = [].freeze

      # The gated origins whose gate turns the current file down. Frozen and
      # deterministic (ORIGINS order) so it doubles as the token memo key.
      def inactive_origins(config, processed_source)
        gates = plugin_gates
        return EMPTY_INACTIVE if gates.empty?

        inactive = ORIGINS.select do |origin|
          gate = gates[origin]
          gate && !gate.call(config, processed_source)
        end
        inactive.empty? ? EMPTY_INACTIVE : inactive.freeze
      end

      # Builds the `(nums, lists)` wire format documented on `BundleConfig`
      # (crates/shirobai-core/src/rules/bundle.rs). Every cop's values come
      # from its `bundle_args` class method — the same derivation its instance
      # uses — resolved from `config` alone (cop-own config via
      # `config.for_badge`, exactly like `RuboCop::Cop::Base#cop_config`).
      # Origins in `inactive` pack their dormant segment even when their
      # packer is registered (the per-file gate said no).
      def packed_config(config, inactive = EMPTY_INACTIVE)
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
        ara = Cop::Layout::ArrayAlignment.bundle_args(config)
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
        ea = Cop::Layout::EndAlignment.bundle_args(config)
        ba = Cop::Layout::BlockAlignment.bundle_args(config)
        elsea = Cop::Layout::ElseAlignment.bundle_args(config)
        fhe = Cop::Layout::FirstHashElementIndentation.bundle_args(config)
        ha = Cop::Layout::HashAlignment.bundle_args(config)
        hs = Cop::Style::HashSyntax.bundle_args(config)
        sl = Cop::Style::StringLiterals.bundle_args(config)
        tca = Cop::Style::TrailingCommaInArguments.bundle_args(config)
        tchl = Cop::Style::TrailingCommaInHashLiteral.bundle_args(config)
        tcal = Cop::Style::TrailingCommaInArrayLiteral.bundle_args(config)
        sli = Cop::Style::StringLiteralsInInterpolation.bundle_args(config)
        tel = Cop::Layout::TrailingEmptyLines.bundle_args(config)
        sibb = Cop::Layout::SpaceInsideBlockBraces.bundle_args(config)
        ml = Cop::Metrics::MethodLength.bundle_args(config)
        cl = Cop::Metrics::ClassLength.bundle_args(config)
        mol = Cop::Metrics::ModuleLength.bundle_args(config)
        dea = Cop::Layout::DefEndAlignment.bundle_args(config)
        npc = Cop::Style::NestedParenthesizedCalls.bundle_args(config)
        # Lint::ParenthesesAsGroupedExpression and Lint::UnreachableCode are
        # config-less; their `bundle_args` returns `[]` and contributes nothing
        # to `nums` / `lists`.
        pld = Cop::Style::PercentLiteralDelimiters.bundle_args(config)
        mmcbl = Cop::Layout::MultilineMethodCallBraceLayout.bundle_args(config)
        ami = Cop::Layout::AccessModifierIndentation.bundle_args(config)
        ai = Cop::Layout::AssignmentIndentation.bundle_args(config)
        slp = Cop::Style::StabbyLambdaParentheses.bundle_args(config)
        # Style/HashTransformKeys is config-less; its `bundle_args` returns `[]`
        # and contributes nothing to `nums` / `lists`.
        _ = Cop::Style::HashTransformKeys.bundle_args(config)
        aba = Cop::Lint::AmbiguousBlockAssociation.bundle_args(config)
        # Layout/EmptyLineAfterGuardClause is config-less; its `bundle_args` returns `[]`
        # and contributes nothing to `nums` / `lists`.
        _ = Cop::Layout::EmptyLineAfterGuardClause.bundle_args(config)
        # Layout/EmptyLineAfterMagicComment is config-less too.
        _ = Cop::Layout::EmptyLineAfterMagicComment.bundle_args(config)
        ec = Cop::Layout::EmptyComment.bundle_args(config)
        # Layout/EmptyLines is config-less; its `bundle_args` returns `[]`
        # and contributes nothing to `nums` / `lists`.
        _ = Cop::Layout::EmptyLines.bundle_args(config)
        # Layout/LeadingEmptyLines is config-less; its `bundle_args` returns
        # `[]` and contributes nothing to `nums` / `lists`.
        _ = Cop::Layout::LeadingEmptyLines.bundle_args(config)
        sihlb = Cop::Layout::SpaceInsideHashLiteralBraces.bundle_args(config)
        sialb = Cop::Layout::SpaceInsideArrayLiteralBrackets.bundle_args(config)
        sbbb = Cop::Layout::SpaceBeforeBlockBraces.bundle_args(config)
        ium = Cop::Style::IfUnlessModifier.bundle_args(config)
        sbcm = Cop::Layout::SpaceBeforeComma.bundle_args(config)
        sacm = Cop::Layout::SpaceAfterComma.bundle_args(config)
        sbsm = Cop::Layout::SpaceBeforeSemicolon.bundle_args(config)
        sasm = Cop::Layout::SpaceAfterSemicolon.bundle_args(config)
        # Layout/SpaceAfterColon and Layout/SpaceBeforeComment are config-less;
        # their `bundle_args` returns `[]` and contributes nothing to
        # `nums` / `lists`.
        _ = Cop::Layout::SpaceAfterColon.bundle_args(config)
        _ = Cop::Layout::SpaceBeforeComment.bundle_args(config)
        sipn = Cop::Layout::SpaceInsideParens.bundle_args(config)
        sirb = Cop::Layout::SpaceInsideReferenceBrackets.bundle_args(config)
        sbfa = Cop::Layout::SpaceBeforeFirstArg.bundle_args(config)
        # Lint/DuplicateMagicComment is config-less; its `bundle_args` returns
        # `[]` and contributes nothing to `nums` / `lists`.
        _ = Cop::Lint::DuplicateMagicComment.bundle_args(config)
        dm = Cop::Lint::DuplicateMethods.bundle_args(config)
        # Style/Semicolon is config-less (path (a) needs no config; path (b)'s
        # AllowAsExpressionSeparator is handled in the wrapper); its
        # `bundle_args` returns `[]` and contributes nothing to `nums` / `lists`.
        _ = Cop::Style::Semicolon.bundle_args(config)
        rf = Cop::Style::RedundantFreeze.bundle_args(config)
        fslc = Cop::Style::FrozenStringLiteralComment.bundle_args(config)
        af = Cop::Style::ArgumentsForwarding.bundle_args(config)

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
          *elbd[0],                       # EmptyLineBetweenDefs method/class/module/adjacent/min/max
          ea[0],                          # EndAlignment style
          ba[0],                          # BlockAlignment style
          elsea[0],                       # ElseAlignment style (Layout/EndAlignment EnforcedStyleAlignWith)
          fhe[0], fhe[1], num(fhe[2]), num(fhe[3]), num(fhe[4]), # FirstHashElementIndentation style / indent / enforce / colon-sep / rocket-sep
          ha[2], num(ha[3]), # HashAlignment EnforcedLastArgumentHashStyle code / enforce_fixed
          *hs[0], # HashSyntax style / shorthand / urswsv / prfnaes / ruby31 / ruby22 (6 nums)
          *sl[0], # StringLiterals style / consistent_multiline (2 nums)
          *tca[0], # TrailingCommaInArguments style (1 num)
          *sli[0], # StringLiteralsInInterpolation style (1 num)
          *tel[0], # TrailingEmptyLines style (1 num)
          *sibb[0], # SpaceInsideBlockBraces style / empty style / SpaceBeforeBlockParameters (3 nums)
          ml[0], num(ml[1]), # MethodLength Max / CountComments
          dea[0], # DefEndAlignment style
          mmcbl[0][0], # MultilineMethodCallBraceLayout EnforcedStyle
          ami[0], ami[1], # AccessModifierIndentation style / indentation_width
          ai[0], # AssignmentIndentation IndentationWidth
          slp[0], # StabbyLambdaParentheses style
          ec[0], ec[1], # EmptyComment AllowBorderComment / AllowMarginComment
          cl[0], num(cl[1]), # ClassLength Max / CountComments
          mol[0], num(mol[1]), # ModuleLength Max / CountComments
          *tchl[0], # TrailingCommaInHashLiteral style (1 num)
          *tcal[0], # TrailingCommaInArrayLiteral style (1 num)
          *sihlb[0], # SpaceInsideHashLiteralBraces style / empty no_space (2 nums)
          *sialb[0], # SpaceInsideArrayLiteralBrackets style / empty space (2 nums)
          *ium, # IfUnlessModifier max_line_length (-1 = disabled) / tab_width (2 nums)
          *sbbb[0], # SpaceBeforeBlockBraces style / empty style / bd conflict flag (3 nums)
          *sbcm[0], # SpaceBeforeComma lcurly_space (1 num)
          *sacm[0], # SpaceAfterComma rcurly_no_space (1 num)
          *sbsm[0], # SpaceBeforeSemicolon lcurly_space (1 num)
          *sasm[0], # SpaceAfterSemicolon rcurly_no_space (1 num)
          *sipn[0], # SpaceInsideParens style (1 num)
          *sirb[0], # SpaceInsideReferenceBrackets style / empty space (2 nums)
          *sbfa[0], # SpaceBeforeFirstArg allow_for_alignment (1 num)
          num(dm[0]), # DuplicateMethods ActiveSupportExtensionsEnabled
          *ara, # ArrayAlignment style / indent (2 nums)
          *rf[0], # RedundantFreeze target_ruby_30_plus / string_literals_frozen_by_default (2 nums)
          *fslc[0], # FrozenStringLiteralComment style (1 num)
          *af[0] # ArgumentsForwarding target_ruby / allow_only_rest / use_anon / explicit_block (4 nums)
        ]
        lists = [dbg[0], dbg[1], bl[2], bl[3], vn[2], snc[0], rs[0], pp[0], pp[1], hem[0],
                 uam[0], uam[1], *bd[1], elbd[1], ha[0], ha[1], ml[2], npc[0], pld[0], aba[0],
                 cl[2], mol[2], *af[1]]

        # One sub-array per origin: `nums[origin]` / `lists[origin]`. Core is
        # origin 0; every plugin origin packs its registered segment or the
        # dormant default, so segment offsets never shift when another origin
        # grows.
        packed_nums = [nums]
        packed_lists = [lists]
        ORIGINS.drop(1).each do |origin|
          packer = Dispatch.plugin_packers[origin]
          packer = nil if inactive.include?(origin)
          seg_nums, seg_lists = packer ? packer.call(config) : DORMANT_SEGMENTS.fetch(origin)
          packed_nums << seg_nums
          packed_lists << seg_lists
        end
        [packed_nums, packed_lists]
      end

      def num(value)
        value ? 1 : 0
      end
    end
  end
end
