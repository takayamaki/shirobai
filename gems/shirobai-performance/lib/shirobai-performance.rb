# frozen_string_literal: true

# Load order matters, and this file owns it so users don't have to:
#
# 1. `shirobai` first. The core gem loads the native extension, replaces
#    the core cops, and defines `Shirobai::Dispatch` — the registration
#    point for this gem's packed-config segment.
# 2. `rubocop-performance` second. Since 1.27 (with RuboCop 1.89) it
#    registers its cops lazily; requiring it here (the gemspec pins the
#    exact version) still makes the replacement order independent of
#    `.rubocop.yml` require order.
# 3. Wrapper cop classes last. Defining each class auto-enlists it
#    (`RuboCop::Cop::Base.inherited`); `Shirobai::Inject.activate!` at the
#    bottom then claims the badges — it loads every replaced stock
#    counterpart first (consuming its lazy registration, so it can never
#    be enlisted again later) and re-enlists the wrappers, which
#    `Registry#clear_enrollment_queue`'s last-write-wins hands the badge.
#
# Requiring rubocop-performance here does NOT merge its config/default.yml
# into RuboCop's default configuration — that is the plugin system's job.
# Users still declare `plugins: [rubocop-performance]` in `.rubocop.yml`
# (or legacy `require:`, which RuboCop auto-promotes to a plugin with a
# deprecation warning) and add `require: [shirobai-performance]`.
require "shirobai"
require "rubocop-performance"

require_relative "shirobai/performance/version"
require_relative "shirobai/cop/performance/detect"
require_relative "shirobai/cop/performance/string_include"
require_relative "shirobai/cop/performance/end_with"
require_relative "shirobai/cop/performance/start_with"
require_relative "shirobai/cop/performance/times_map"

# Wake up the Performance origin in the shared bundle: from now on every
# packed config carries this origin's segment with `enabled=1` plus the
# department's cop settings (segment layout:
# crates/shirobai-core/src/rules/bundle.rs BundleConfig). Without this gem
# the core packs the dormant segment and the Rust side skips the
# Performance rules entirely.
Shirobai::Dispatch.register_plugin_packer(:performance) do |config|
  detect = Shirobai::Cop::Performance::Detect.bundle_args(config)
  end_with = Shirobai::Cop::Performance::EndWith.bundle_args(config)
  start_with = Shirobai::Cop::Performance::StartWith.bundle_args(config)
  [[1, end_with[0], start_with[0]], [[detect[0]]]]
end

# Claim the badges and re-run the autocorrect-incompatibility alignment now
# that this gem's stock department and wrappers are loaded: the core run
# happened before they existed, so lists like
# `Rails/SafeNavigation -> Style::RedundantSelf` or
# `RSpec/AlignLeftLetBrace -> Layout::ExtraSpacing` still name dismissed
# stock classes until this call (see Shirobai::Inject).
Shirobai::Inject.activate!
