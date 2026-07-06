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
      Shirobai::Cop::Rails::ApplicationJob
    ].freeze

    class << self
      # The rails origin's `[nums, lists]` segment for `config`. Constant
      # once the gem is loaded: `[[1], []]` (enabled, no lists).
      #
      # There is no dormant case on shirobai's side. Two per-cop concerns
      # that DO vary — the `Rails/ApplicationRecord` `Exclude: db/**/*.rb`
      # and the `TargetRailsVersion` (`requires_gem 'railties', '>= 5.0'`)
      # gate on Record / Mailer / Job — are NOT expressed in this segment.
      # They live in each wrapper: RuboCop resolves Include/Exclude and the
      # gem-version gate through the wrapper's own cop config exactly as it
      # does for the stock cop, so a file the wrapper does not run on simply
      # drops the Rust-computed offenses for that cop while the other three
      # cops' slots (from the same shared walk) are still consumed. Without
      # `plugins: [rubocop-rails]` the stock cops usually resolve to
      # `Enabled: false`, but `NewCops: enable` turns them on without their
      # default.yml Exclude/metadata — a user misconfiguration outside
      # shirobai's control; shirobai's own segment is unaffected either way.
      def segment(_config)
        [[1], []]
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
