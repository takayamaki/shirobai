---
description: shirobai-rspec plugin gem — RSpec cop wrappers, Language segment, per-file gate
paths:
  - gems/shirobai-rspec/**
---

# shirobai-rspec

Drop-in Rust replacement for [rubocop-rspec](https://github.com/rubocop/rubocop-rspec)
cops, built on the shirobai core.

## How it fits together

This gem is a **pure-Ruby thin shell**: wrapper cop classes plus load-order
glue. The Rust rules live in the shirobai core gem's native extension —
one shared cdylib, no extra cargo build for users.

- `rubocop-rspec` is pinned exactly (`= 3.10.2`), same policy as core's
  rubocop pin: byte-level behavior copies break on minor updates.
- `shirobai` is pinned to the exact same version as this gem: the bundle
  slot layout and the packed-config segment are an internal wire contract.
- `lib/shirobai-rspec.rb` owns the load order (shirobai → rubocop-rspec →
  wrappers) and registers the packed-config segment with
  `Shirobai::Dispatch.register_plugin_packer(:rspec)`. Without this gem
  the core packs a dormant segment and the Rust side skips every RSpec
  rule.

Two things are different from the shirobai-performance shell:

- **RSpec/Language.** rubocop-rspec resolves its whole DSL through
  configurable name lists (`RSpec/Language` in the resolved config). The
  segment carries the sixteen role lists; the Rust side folds them into
  one `name -> role mask` table per config and classifies each node with
  a single hash probe, once, for every RSpec cop at the same time.
- **The per-file gate.** RSpec cops only run on spec files (department
  `Include: **/*_spec.rb` / `**/spec/**/*`). The gem registers a gate
  with the packer: files no wrapper cop would run on use a bundle token
  whose rspec segment is dormant, so the shared walk never builds RSpec
  rules there. The gate is the union of the wrapper cops' own
  `relevant_file?`; if they ever disagree, `Dispatch.offenses_for`
  returns nil and the wrapper falls back to its standalone entry point —
  a gate bug can cost speed, never offenses.

## Usage

```ruby
# Gemfile
gem "rubocop", "= 1.88.2"
gem "rubocop-rspec", "= 3.10.2"
gem "shirobai"
gem "shirobai-rspec"
```

```yaml
# .rubocop.yml
plugins:
  - rubocop-rspec
require:
  - shirobai-rspec
```

`plugins:` merges rubocop-rspec's default config (that part stays stock);
the `require:` swaps the implemented cops for the Rust-backed versions.
Requiring `shirobai-rspec` also loads `shirobai` core.

## Testing

Specs run with this directory's own bundle (kept separate from the repo
root bundle so the core suite never auto-integrates the plugin config):

```sh
cd gems/shirobai-rspec
bundle install
bundle exec rspec
```

The parity oracle for this gem is `benches/parity_diff_rspec.sh` at the
repo root (see `benches/README.md`). It differs from the other oracles:
`--force-default-config` never merges plugin defaults, which would leave
`RSpec/Language` empty and every RSpec cop silent on both sides — an
empty parity. The rspec oracle instead writes a uniform config into the
corpus root that `inherit_from`s the pinned rubocop-rspec default.yml,
and it self-tests against a synthetic fixture that must fire the
implemented cops before the corpus diff counts.

Implemented cop status lives in `docs/cop-status.md` at the repo root.
