# frozen_string_literal: true

# Load order matters, and this file owns it so users don't have to:
#
# 1. `shirobai` first. The core gem loads the native extension, replaces
#    the core cops, and defines `Shirobai::Dispatch` — the registration
#    point for this gem's packed-config segment.
# 2. `rubocop-rspec` second. Stock RSpec cop classes must be enlisted in
#    RuboCop's registry BEFORE the wrappers below:
#    `Registry#clear_enrollment_queue` resolves same-badge collisions by
#    last-write-wins, so whoever is defined later owns the badge.
#    Requiring it here (the gemspec pins the exact version) makes the
#    replacement order independent of `.rubocop.yml` require order.
# 3. Wrapper cop classes last. Defining each class auto-enlists it
#    (`RuboCop::Cop::Base.inherited`) and replaces the stock cop under
#    the same badge.
#
# Requiring rubocop-rspec here does NOT merge its config/default.yml into
# RuboCop's default configuration — that is the plugin system's job. Users
# still declare `plugins: [rubocop-rspec]` in `.rubocop.yml` (or legacy
# `require:`, which RuboCop auto-promotes to a plugin with a deprecation
# warning) and add `require: [shirobai-rspec]`.
require "shirobai"
require "rubocop-rspec"

require_relative "shirobai/rspec/version"
require_relative "shirobai/rspec/node_locator"
require_relative "shirobai/cop/rspec/metadata_support"
require_relative "shirobai/cop/rspec/send_candidate_support"
require_relative "shirobai/cop/rspec/empty_line_separation_support"
require_relative "shirobai/cop/rspec/variable_name"
require_relative "shirobai/cop/rspec/let_setup"
require_relative "shirobai/cop/rspec/variable_definition"
require_relative "shirobai/cop/rspec/multiple_memoized_helpers"
require_relative "shirobai/cop/rspec/repeated_description"
require_relative "shirobai/cop/rspec/repeated_example"
require_relative "shirobai/cop/rspec/named_subject"
require_relative "shirobai/cop/rspec/focus"
require_relative "shirobai/cop/rspec/pending_without_reason"
require_relative "shirobai/cop/rspec/described_class"
require_relative "shirobai/cop/rspec/metadata_style"
require_relative "shirobai/cop/rspec/duplicated_metadata"
require_relative "shirobai/cop/rspec/empty_metadata"
require_relative "shirobai/cop/rspec/sort_metadata"
require_relative "shirobai/cop/rspec/empty_example_group"
require_relative "shirobai/cop/rspec/empty_line_after_example"
require_relative "shirobai/cop/rspec/empty_line_after_example_group"
require_relative "shirobai/cop/rspec/empty_line_after_final_let"
require_relative "shirobai/cop/rspec/empty_line_after_hook"
require_relative "shirobai/cop/rspec/empty_line_after_subject"
require_relative "shirobai/cop/rspec/scattered_setup"
require_relative "shirobai/cop/rspec/dialect"
require_relative "shirobai/cop/rspec/multiple_subjects"
require_relative "shirobai/cop/rspec/shared_examples"

module Shirobai
  # Glue for the shirobai-rspec plugin gem: the packed-config segment
  # (RSpec/Language role lists + per-cop settings) and the per-file gate.
  module RSpec
    # Wrapper cop classes, appended as cops land. The per-file gate below is
    # the union of their `relevant_file?` — by construction it can never be
    # narrower than what any wrapper will ask for.
    COP_CLASSES = [
      Shirobai::Cop::RSpec::VariableName,
      Shirobai::Cop::RSpec::LetSetup,
      Shirobai::Cop::RSpec::VariableDefinition,
      Shirobai::Cop::RSpec::MultipleMemoizedHelpers,
      Shirobai::Cop::RSpec::RepeatedDescription,
      Shirobai::Cop::RSpec::RepeatedExample,
      Shirobai::Cop::RSpec::NamedSubject,
      Shirobai::Cop::RSpec::Focus,
      Shirobai::Cop::RSpec::PendingWithoutReason,
      Shirobai::Cop::RSpec::DescribedClass,
      Shirobai::Cop::RSpec::MetadataStyle,
      Shirobai::Cop::RSpec::DuplicatedMetadata,
      Shirobai::Cop::RSpec::EmptyMetadata,
      Shirobai::Cop::RSpec::SortMetadata,
      Shirobai::Cop::RSpec::EmptyExampleGroup,
      Shirobai::Cop::RSpec::EmptyLineAfterExample,
      Shirobai::Cop::RSpec::EmptyLineAfterExampleGroup,
      Shirobai::Cop::RSpec::EmptyLineAfterFinalLet,
      Shirobai::Cop::RSpec::EmptyLineAfterHook,
      Shirobai::Cop::RSpec::EmptyLineAfterSubject,
      Shirobai::Cop::RSpec::ScatteredSetup,
      Shirobai::Cop::RSpec::Dialect,
      Shirobai::Cop::RSpec::MultipleSubjects,
      Shirobai::Cop::RSpec::SharedExamples
    ].freeze

    # `config['RSpec']['Language']` sub-role paths in the fixed wire order of
    # the rspec segment's lists (see BundleConfig in
    # crates/shirobai-core/src/rules/bundle.rs and
    # crates/shirobai-core/src/rules/rspec_language.rs).
    ROLE_PATHS = [
      %w[ExampleGroups Regular], %w[ExampleGroups Focused],
      %w[ExampleGroups Skipped],
      %w[Examples Regular], %w[Examples Focused], %w[Examples Skipped],
      %w[Examples Pending],
      %w[Expectations], %w[Helpers], %w[Hooks], %w[ErrorMatchers],
      %w[Includes Examples], %w[Includes Context],
      %w[SharedGroups Examples], %w[SharedGroups Context],
      %w[Subjects]
    ].freeze

    class << self
      # The rspec origin's `[nums, lists]` segment for `config`, memoized by
      # config identity (also the standalone fallback path's argument
      # source). Reads the RESOLVED `RSpec/Language` hash — RuboCop's config
      # layer has already applied `inherit_mode: merge` — and flattens it;
      # no merging happens here.
      #
      # Contract for malformed configs: when the department or Language hash
      # is missing (e.g. rubocop-rspec required as a library without
      # `plugins:`, so its default.yml was never merged), the segment is
      # dormant. Note that a dormant segment only covers shirobai's side:
      # without `plugins:` the stock RSpec cops usually resolve to
      # `Enabled: false`, but `NewCops: enable` turns them on without their
      # default.yml Includes, and the non-replaced stock cops then fire on
      # non-spec files. That is a user misconfiguration outside shirobai's
      # control; shirobai's own segment stays dormant either way.
      # Non-Array role values and non-String list
      # entries are dropped — a Symbol in a role list never matches in
      # stock either (`Array#include?` compares against `element.to_s`).
      def segment(config)
        @segments ||= {}.compare_by_identity
        @segments[config] ||= compute_segment(config)
      end

      # Per-file gate for `Shirobai::Dispatch`: does any wrapper cop run on
      # this file? Wrapper cops are departmental (RSpec `Include:
      # **/*_spec.rb` / `**/spec/**/*` + factory Excludes resolve into every
      # cop's config), so most files of a mixed codebase keep the rspec
      # origin dormant and never build its rules.
      def relevant_file?(config, path)
        gate_cops(config).any? { |cop| cop.relevant_file?(path) }
      end

      private

      def compute_segment(config)
        dept = config["RSpec"]
        lang = dept.is_a?(Hash) ? dept["Language"] : nil
        return Shirobai::Dispatch::DORMANT_SEGMENTS.fetch(:rspec) unless lang.is_a?(Hash)

        lists = ROLE_PATHS.map do |path|
          value = lang.dig(*path)
          value.is_a?(Array) ? value.grep(String) : []
        end
        # 17th list: RSpec/Dialect PreferredMethods keys (empty unless
        # configured — keeps the shared walk's Dialect classifier dormant).
        lists << Shirobai::Cop::RSpec::Dialect.bundle_lists(config)
        vn = Shirobai::Cop::RSpec::VariableName.bundle_args(config)
        vd = Shirobai::Cop::RSpec::VariableDefinition.bundle_args(config)
        mmh = Shirobai::Cop::RSpec::MultipleMemoizedHelpers.bundle_args(config)
        ns = Shirobai::Cop::RSpec::NamedSubject.bundle_args(config)
        ex = Shirobai::Cop::RSpec::EmptyLineAfterExample.bundle_args(config)
        hook = Shirobai::Cop::RSpec::EmptyLineAfterHook.bundle_args(config)
        [[1, vn[0], vd[0], mmh[0], mmh[1], ns[0], ns[1], ex[0], hook[0]], lists]
      end

      # One wrapper instance per class per config, only for gate use.
      # `relevant_file?` resolves Include/Exclude through the instance's own
      # cop config — exactly the check `Team#roundup_relevant_cops` performs
      # before running the wrapper, so gate and wrapper cannot disagree.
      def gate_cops(config)
        @gate_cops ||= {}.compare_by_identity
        @gate_cops[config] ||= COP_CLASSES.map { |klass| klass.new(config) }
      end
    end
  end
end

# Wake up the rspec origin in the shared bundle: from now on every packed
# config carries this origin's segment (role lists + cop settings, layout:
# crates/shirobai-core/src/rules/bundle.rs BundleConfig) — except on files
# the gate turns down, which keep the dormant segment. Without this gem the
# core always packs the dormant segment and the Rust side skips the RSpec
# rules entirely.
Shirobai::Dispatch.register_plugin_packer(
  :rspec,
  gate: lambda do |config, processed_source|
    Shirobai::RSpec.relevant_file?(config, processed_source.file_path)
  end
) { |config| Shirobai::RSpec.segment(config) }
