# frozen_string_literal: true

module Shirobai
  module Inject
    # Badge replacement rides on `Registry#clear_enrollment_queue` being
    # last-write-wins: whoever is enlisted later owns the badge. Up to
    # RuboCop 1.88 every stock cop was loaded (and enlisted) at
    # `require "rubocop"`, so wrappers defined afterwards always won.
    #
    # RuboCop 1.89 registers stock cops lazily: the registry maps each badge
    # to a constant-name String and the class only loads when the constant is
    # resolved (autoload). Loading enlists the stock class through
    # `Base.inherited`, so a replaced cop whose stock file loads AFTER the
    # wrapper — `Registry#load_all_lazy_cops`, a plugin gem referencing the
    # constant (rubocop-rails prepends onto `Style/RedundantSelf`), or our own
    # `stock_counterpart` — would steal the badge back on the next flush.
    #
    # `claim_badges!` closes every such path at require time: resolve each
    # replaced cop's stock constant first (consuming the autoload, so the
    # stock class can never be enlisted again later), then re-enlist the
    # wrapper so the queue flushes with the wrapper last. Unreplaced cops
    # keep their lazy registration — shirobai must not force the whole
    # registry to load.
    def self.activate!
      claim_badges!
      align_autocorrect_incompatibilities!
    end

    def self.claim_badges!
      registry = RuboCop::Cop::Registry.global
      wrapper_cops.each do |wrapper|
        next unless stock_counterpart(wrapper) # consume the stock autoload

        registry.enlist(wrapper)
      end
    end

    # All wrapper classes defined so far, from the core departments and any
    # loaded plugin gem's department (RSpec / Rails / Performance).
    def self.wrapper_cops
      Shirobai::Cop.constants(false).flat_map do |department|
        mod = Shirobai::Cop.const_get(department)
        next [] unless mod.is_a?(Module) && !mod.is_a?(Class)

        mod.constants(false).filter_map do |cop_name|
          klass = mod.const_get(cop_name)
          klass if klass.is_a?(Class) && klass < RuboCop::Cop::Base
        end
      end
    end

    # `Team#each_corrector` skips a cop's corrector for the round when an
    # earlier-merged cop's `autocorrect_incompatible_with` includes the cop's
    # CLASS. Badge replacement breaks that identity check in both directions:
    # a wrapper copying stock's list still names the dismissed stock class,
    # and a stock cop that stays stock (Style/SymbolProc) names a class whose
    # registry slot a wrapper took over. Either way `skips.include?(cop.class)`
    # misses, the skip never fires, and the `-a` round applies corrections
    # stock would have dropped — the corrected trees then drift (fluentd
    # types.rb / log.rb: SpaceInsideBlockBraces lists BlockDelimiters).
    #
    # Rewrite every active cop's list through the stock-to-shirobai map:
    # wrappers get stock's list (the drop-in truth) translated, and remaining
    # stock cops get their own list translated. Runs after all core wrapper
    # classes are defined (each auto-enlists via `Base.inherited`), again at
    # each shirobai plugin gem's require (their stock departments — e.g.
    # `Rails/SafeNavigation` listing `Style::RedundantSelf` — enlist between
    # aligner runs), and lazily when the registry grew since the last run
    # (third-party plugins like rubocop-capybara load during config
    # resolution, after every shirobai require; see `align_if_registry_grew!`
    # and `Dispatch.bundle_token`). Idempotent: an already-translated list
    # maps to itself and is skipped.
    #
    # The rewritten methods return a FRESH copy per call, like stock's
    # per-call array literals: rubocop-performance and rubocop-capybara
    # prepend singleton modules that do `super.push(...)`, so a shared (or
    # frozen) array would accumulate duplicates across calls (or raise
    # FrozenError).
    #
    # The wrapper pass runs unconditionally (the stock counterparts are
    # loaded by `claim_badges!`). The non-wrapper pass only covers
    # `extra_cops` — enumerating the whole registry would force every lazy
    # stock cop to load, so `align_for` passes the enabled set of the config
    # actually being run, which is exactly the set `Team#each_corrector` can
    # consult. Idempotent: an already-translated list maps to itself and is
    # skipped.
    def self.align_autocorrect_incompatibilities!(extra_cops = [])
      replacements = {}
      wrapper_cops.each do |cop|
        stock = stock_counterpart(cop)
        replacements[stock] = cop if stock
      end

      replacements.each do |stock, wrapper|
        list = stock.autocorrect_incompatible_with
                    .map { |klass| replacements.fetch(klass, klass) }.freeze
        wrapper.define_singleton_method(:autocorrect_incompatible_with) { list.dup }
      end
      extra_cops.each do |cop|
        next if replacements.value?(cop)

        list = cop.autocorrect_incompatible_with
        mapped = list.map { |klass| replacements.fetch(klass, klass) }
        next if mapped == list

        mapped.freeze
        cop.define_singleton_method(:autocorrect_incompatible_with) { mapped.dup }
      end
    end

    # Per-config re-alignment for cops that load AFTER every shirobai
    # require: rubocop resolves `plugins:` / `require:` gems while loading
    # the corpus config, and those gems (rubocop-capybara et al.) may list a
    # replaced class or enlist new cops. Called from `Dispatch.bundle_token`
    # on each new config, which happens after config resolution and before
    # any correction round. Memoized per (config identity, registry badge
    # count): a plugin load only ever ADDS badges, so a size change re-runs
    # the alignment mid-run. `Registry#length` only counts badges — it never
    # loads a lazy cop — and `Registry#enabled` loads exactly the cops the
    # run's team is built from.
    def self.align_for(config)
      registry = RuboCop::Cop::Registry.global
      size = registry.length
      @aligned ||= {}.compare_by_identity
      return if @aligned[config] == size

      @aligned[config] = size
      align_autocorrect_incompatibilities!(registry.enabled(config))
    end

    # The stock class whose badge `klass` took over, or nil when the stock
    # constant does not exist (a shirobai-only cop).
    def self.stock_counterpart(klass)
      department, name = klass.cop_name.split("/")
      RuboCop::Cop.const_get(department, false).const_get(name, false)
    rescue NameError
      nil
    end
  end
end

require_relative "source_offsets"
require_relative "node_locator"
require_relative "cop/base"
require_relative "dispatch"
require_relative "cop/lint/debugger"
require_relative "cop/lint/duplicate_magic_comment"
require_relative "cop/lint/ordered_magic_comments"
require_relative "cop/lint/duplicate_methods"
require_relative "cop/metrics/block_length"
require_relative "cop/metrics/method_length"
require_relative "cop/metrics/block_nesting"
require_relative "cop/metrics/class_length"
require_relative "cop/metrics/module_length"
require_relative "cop/metrics/cyclomatic_complexity"
require_relative "cop/metrics/perceived_complexity"
require_relative "cop/metrics/abc_size"
require_relative "cop/naming/variable_number"
require_relative "cop/naming/method_name"
require_relative "cop/naming/predicate_prefix"
require_relative "cop/lint/ambiguous_block_association"
require_relative "cop/lint/parentheses_as_grouped_expression"
require_relative "cop/lint/require_parentheses"
require_relative "cop/lint/safe_navigation_chain"
require_relative "cop/lint/self_assignment"
require_relative "cop/lint/unreachable_code"
require_relative "cop/lint/useless_access_modifier"
require_relative "cop/lint/void"
require_relative "cop/layout/argument_alignment"
require_relative "cop/layout/array_alignment"
require_relative "cop/layout/assignment_indentation"
require_relative "cop/layout/closing_parenthesis_indentation"
require_relative "cop/layout/multiline_operation_indentation"
require_relative "cop/layout/multiline_method_call_indentation"
require_relative "cop/layout/multiline_method_call_brace_layout"
require_relative "cop/layout/access_modifier_indentation"
require_relative "cop/layout/empty_line_after_guard_clause"
require_relative "cop/layout/empty_line_after_magic_comment"
require_relative "cop/layout/empty_comment"
require_relative "cop/layout/empty_lines"
require_relative "cop/layout/leading_empty_lines"
require_relative "cop/layout/initial_indentation"
require_relative "cop/layout/end_of_line"
require_relative "cop/layout/line_continuation_spacing"
require_relative "cop/layout/space_inside_string_interpolation"
require_relative "cop/style/empty_literal"
require_relative "cop/style/magic_comment_format"
require_relative "cop/naming/ascii_identifiers"
require_relative "cop/style/mutable_constant"
require_relative "cop/layout/rescue_ensure_alignment"
require_relative "cop/layout/dot_position"
require_relative "cop/layout/empty_line_between_defs"
require_relative "cop/layout/empty_lines_around_arguments"
require_relative "cop/layout/end_alignment"
require_relative "cop/layout/def_end_alignment"
require_relative "cop/layout/block_alignment"
require_relative "cop/layout/else_alignment"
require_relative "cop/layout/empty_lines_around_method_body"
require_relative "cop/layout/empty_lines_around_class_body"
require_relative "cop/layout/empty_lines_around_module_body"
require_relative "cop/layout/empty_lines_around_block_body"
require_relative "cop/layout/empty_lines_around_begin_body"
require_relative "cop/layout/empty_lines_around_exception_handling_keywords"
require_relative "cop/layout/extra_spacing"
require_relative "cop/layout/first_argument_indentation"
require_relative "cop/layout/first_array_element_indentation"
require_relative "cop/layout/first_hash_element_indentation"
require_relative "cop/layout/hash_alignment"
require_relative "cop/layout/indentation_consistency"
require_relative "cop/layout/indentation_width"
require_relative "cop/layout/line_length"
require_relative "cop/layout/trailing_empty_lines"
require_relative "cop/layout/space_after_colon"
require_relative "cop/layout/space_after_comma"
require_relative "cop/layout/space_after_semicolon"
require_relative "cop/layout/space_around_equals_in_parameter_default"
require_relative "cop/layout/space_around_method_call_operator"
require_relative "cop/layout/space_around_keyword"
require_relative "cop/layout/space_around_operators"
require_relative "cop/layout/space_before_block_braces"
require_relative "cop/layout/space_before_comma"
require_relative "cop/layout/space_before_first_arg"
require_relative "cop/layout/space_before_comment"
require_relative "cop/layout/space_before_semicolon"
require_relative "cop/layout/space_inside_array_literal_brackets"
require_relative "cop/layout/space_inside_parens"
require_relative "cop/layout/space_inside_reference_brackets"
require_relative "cop/layout/space_inside_block_braces"
require_relative "cop/layout/space_inside_hash_literal_braces"
require_relative "cop/style/block_delimiters"
require_relative "cop/style/file_null"
require_relative "cop/style/frozen_string_literal_comment"
require_relative "cop/style/hash_each_methods"
require_relative "cop/style/hash_syntax"
require_relative "cop/style/hash_transform_keys"
require_relative "cop/style/string_literals"
require_relative "cop/style/string_literals_in_interpolation"
require_relative "cop/style/trailing_comma_in_arguments"
require_relative "cop/style/trailing_comma_in_hash_literal"
require_relative "cop/style/trailing_comma_in_array_literal"
require_relative "cop/style/line_end_concatenation"
require_relative "cop/style/nested_parenthesized_calls"
require_relative "cop/style/percent_literal_delimiters"
require_relative "cop/style/redundant_freeze"
require_relative "cop/style/redundant_self"
require_relative "cop/style/redundant_self_assignment"
require_relative "cop/style/colon_method_call"
require_relative "cop/style/stabby_lambda_parentheses"
require_relative "cop/style/if_unless_modifier"
require_relative "cop/style/semicolon"
require_relative "cop/style/arguments_forwarding"

Shirobai::Inject.activate!
