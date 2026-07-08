# frozen_string_literal: true

# Load order matters, and this file owns it so users don't have to:
#
# 1. `shirobai` first. The core gem loads the native extension, replaces
#    the core cops, and defines `Shirobai::Dispatch` — the registration
#    point for this gem's packed-config segment.
# 2. `rubocop-rails` second. Stock Rails cop classes must be enlisted in
#    RuboCop's registry BEFORE the wrappers below:
#    `Registry#clear_enrollment_queue` resolves same-badge collisions by
#    last-write-wins, so whoever is defined later owns the badge.
#    Requiring it here (the gemspec pins the exact version) makes the
#    replacement order independent of `.rubocop.yml` require order.
# 3. Wrapper cop classes last. Defining each class auto-enlists it
#    (`RuboCop::Cop::Base.inherited`) and replaces the stock cop under
#    the same badge.
#
# Requiring rubocop-rails here does NOT merge its config/default.yml into
# RuboCop's default configuration — that is the plugin system's job. Users
# still declare `plugins: [rubocop-rails]` in `.rubocop.yml` (or legacy
# `require:`, which RuboCop auto-promotes to a plugin with a deprecation
# warning) and add `require: [shirobai-rails]`.
require "shirobai"
require "rubocop-rails"

require_relative "shirobai/rails/version"
require_relative "shirobai/cop/rails/application_record"
require_relative "shirobai/cop/rails/application_controller"
require_relative "shirobai/cop/rails/application_mailer"
require_relative "shirobai/cop/rails/application_job"
require_relative "shirobai/cop/rails/unknown_env"
require_relative "shirobai/cop/rails/dynamic_find_by"
require_relative "shirobai/cop/rails/pluck"
require_relative "shirobai/cop/rails/candidate_support"
require_relative "shirobai/cop/rails/http_positional_arguments"
require_relative "shirobai/cop/rails/deprecated_active_model_errors_methods"
require_relative "shirobai/cop/rails/index_by"
require_relative "shirobai/cop/rails/index_with"

module Shirobai
  # Glue for the shirobai-rails plugin gem: the packed-config segment (just a
  # wake-up flag — the Application* cluster carries no behavioral config).
  #
  # Unlike shirobai-rspec there is NO per-file gate: rubocop-rails cops run on
  # every Ruby file (no department Include like RSpec's `**/*_spec.rb`), so
  # once this gem is loaded the rails origin is always awake. The core packs
  # this origin's segment into every config from now on.
  module Rails
    # Wrapper cop classes, appended as cops land. Kept for parity with the
    # sibling plugins and for plumbing specs.
    COP_CLASSES = [
      Shirobai::Cop::Rails::ApplicationRecord,
      Shirobai::Cop::Rails::ApplicationController,
      Shirobai::Cop::Rails::ApplicationMailer,
      Shirobai::Cop::Rails::ApplicationJob,
      Shirobai::Cop::Rails::UnknownEnv,
      Shirobai::Cop::Rails::DynamicFindBy,
      Shirobai::Cop::Rails::Pluck,
      Shirobai::Cop::Rails::HttpPositionalArguments,
      Shirobai::Cop::Rails::DeprecatedActiveModelErrorsMethods,
      Shirobai::Cop::Rails::IndexBy,
      Shirobai::Cop::Rails::IndexWith
    ].freeze

    class << self
      # The rails origin's `[nums, lists]` segment for `config`. The wake-up
      # flag is always `1` once the gem is loaded; the rest is each config-
      # bearing cop's own `bundle_args` (the single source of its config).
      #
      # Segment layout (crates/shirobai-core/src/rules/rails_config.rs):
      #   nums  = [enabled, unknown_env_supports_local]
      #   lists = [unknown_env_environments,
      #            dynamic_find_by_allowed_methods,
      #            dynamic_find_by_allowed_receivers,
      #            dynamic_find_by_whitelist]
      #
      # Per-cop gating that DOES vary (the `Rails/ApplicationRecord`
      # `Exclude: db/**/*.rb`, the `TargetRailsVersion` gates, and each cop's
      # `Enabled`) is NOT in this segment: RuboCop resolves it through each
      # wrapper's own cop config exactly as for the stock cop, so a file a
      # wrapper does not run on simply drops that cop's Rust-computed slot
      # while the other cops' slots (from the same shared walk) are consumed.
      def segment(config)
        ue = Shirobai::Cop::Rails::UnknownEnv.bundle_args(config)
        dfb = Shirobai::Cop::Rails::DynamicFindBy.bundle_args(config)
        nums = [1, *ue[0]]
        lists = [*ue[1], *dfb[1]]
        [nums, lists]
      end
    end
  end
end

# Wake up the rails origin in the shared bundle: from now on every packed
# config carries this origin's segment with `enabled=1` (segment layout:
# crates/shirobai-core/src/rules/bundle.rs BundleConfig). No gate — the
# Application* cops run on every Ruby file. Without this gem the core packs
# the dormant segment and the Rust side skips the Rails rules entirely.
Shirobai::Dispatch.register_plugin_packer(:rails) { |config| Shirobai::Rails.segment(config) }
