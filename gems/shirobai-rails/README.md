---
description: shirobai-rails plugin gem — Rails cop wrappers, always-awake origin, no per-file gate
paths:
  - gems/shirobai-rails/**
---

# shirobai-rails

Drop-in Rust replacement for [rubocop-rails](https://github.com/rubocop/rubocop-rails)
cops, built on the shirobai core.

## How it fits together

This gem is a **pure-Ruby thin shell**: wrapper cop classes plus load-order
glue. The Rust rules live in the shirobai core gem's native extension —
one shared cdylib, no extra cargo build for users.

- `rubocop-rails` is pinned exactly (`= 2.35.5`), same policy as core's
  rubocop pin: byte-level behavior copies break on minor updates.
- `shirobai` is pinned to the exact same version as this gem: the bundle
  slot layout and the packed-config segment are an internal wire contract.
- `lib/shirobai-rails.rb` owns the load order (shirobai → rubocop-rails →
  wrappers) and registers the packed-config segment with
  `Shirobai::Dispatch.register_plugin_packer(:rails)`. Without this gem
  the core packs a dormant segment and the Rust side skips every Rails
  rule.

Two things are different from the shirobai-rspec shell:

- **No per-file gate.** rubocop-rails cops run on every Ruby file (no
  department `Include` like RSpec's `**/*_spec.rb`), so the rails origin
  is **always awake** once the gem is loaded. The design constraint that
  follows: candidate classification on the Rust side must stay cheap
  (table-driven const-name checks riding the existing shared walk, no
  extra AST pass), because the cost is paid on every file with no gate to
  hide behind.
- **Mostly-thin config segment.** The first cluster was the four
  Application* cops (`Rails/ApplicationRecord` / `...Controller` /
  `...Mailer` / `...Job`) — fixed class-inheritance checks with no
  behavioral config. The send/block-table cluster added
  `Rails/DynamicFindBy` (`AllowedMethods` / `AllowedReceivers` /
  `Whitelist`) and `Rails/UnknownEnv` (`Environments` + the Rails >= 7.1
  `local` view), so the segment now carries those lists; each cop's
  `bundle_args` is the single source of its own config. Per-cop gating
  that varies (the `Rails/ApplicationRecord` `Exclude: db/**/*.rb`, the
  `TargetRailsVersion` gates, each cop's `Enabled`) is still handled by
  the wrappers, not the segment: RuboCop resolves it through each
  wrapper's own cop config exactly as for the stock cop.

  The second cluster is two **Architecture-B** cops
  (`Rails/HttpPositionalArguments`, `Rails/DeprecatedActiveModelErrorsMethods`):
  the Rust side emits candidate SEND ranges on the same shared walk, and the
  wrapper relocates the parser node (`Shirobai::NodeLocator`) and runs stock's
  `on_send` + autocorrect VERBATIM. This keeps the source-reconstruction
  autocorrect and the file-path / target-version heuristics on the parser AST,
  where they match stock byte for byte, while Rust only narrows which nodes
  are visited. These cops need no segment config either — their gating
  (`requires_gem`, `target_rails_version`, `model_file?`) lives in the
  wrappers.

> **Non-Rails projects.** Because the origin has no gate, adding
> shirobai-rails to a project with no railties in the target bundle still
> wakes the shared-walk rule (the gated Record/Mailer/Job cops stay
> silent via `requires_gem`, and `ApplicationController` simply finds no
> `ActionController::Base` subclasses). There is no reason to install
> shirobai-rails on a non-Rails project; do not.

## Usage

```ruby
# Gemfile
gem "rubocop", "= 1.88.0"
gem "rubocop-rails", "= 2.35.5"
gem "shirobai"
gem "shirobai-rails"
```

```yaml
# .rubocop.yml
plugins:
  - rubocop-rails
require:
  - shirobai-rails
```

`plugins:` merges rubocop-rails's default config (that part stays stock);
the `require:` swaps the implemented cops for the Rust-backed versions.
Requiring `shirobai-rails` also loads `shirobai` core.

## Testing

Specs run with this directory's own bundle (kept separate from the repo
root bundle so the core suite never auto-integrates the plugin config):

```sh
cd gems/shirobai-rails
bundle install
bundle exec rspec
```

The parity oracle for this gem is `benches/parity_diff_rails.sh` at the
repo root (see `benches/README.md`). Like the rspec oracle it writes a
uniform config into the corpus root that `inherit_from`s the pinned
rubocop-rails default.yml (so the cops actually resolve on both sides),
pins railties in the oracle Gemfiles so the `TargetRailsVersion` gate
activates, and self-tests against a synthetic fixture that must fire every
implemented Rails cop on the stock side before the corpus diff counts.

Implemented cop status lives in `docs/cop-status.md` at the repo root.
