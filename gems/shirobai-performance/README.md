# shirobai-performance

Drop-in Rust replacement for [rubocop-performance](https://github.com/rubocop/rubocop-performance)
cops, built on the shirobai core.

> [!WARNING]
> Proof of concept. The plumbing (gem boundary, load order, shared native
> extension, parity oracle) is what this gem currently proves; the cop set
> is a small template batch.

## How it fits together

This gem is a **pure-Ruby thin shell**: wrapper cop classes plus load-order
glue. The Rust rules live in the shirobai core gem's native extension —
one shared cdylib, no extra cargo build for users.

- `rubocop-performance` is pinned exactly (`= 1.26.1`), same policy as
  core's rubocop pin: byte-level behavior copies break on minor updates.
- `shirobai` is pinned to the exact same version as this gem: the bundle
  slot layout and the packed-config segment are an internal wire contract.
- `lib/shirobai-performance.rb` owns the load order (shirobai →
  rubocop-performance → wrappers) and registers the packed-config segment
  with `Shirobai::Dispatch.register_plugin_packer(:performance)`. Without
  this gem the core packs a dormant segment and the Rust side skips every
  Performance rule.

## Usage

```ruby
# Gemfile
gem "rubocop", "= 1.88.0"
gem "rubocop-performance", "= 1.26.1"
gem "shirobai"
gem "shirobai-performance"
```

```yaml
# .rubocop.yml
plugins:
  - rubocop-performance
require:
  - shirobai-performance
```

`plugins:` merges rubocop-performance's default config (that part stays
stock); the `require:` swaps the implemented cops for the Rust-backed
versions. Requiring `shirobai-performance` also loads `shirobai` core.

## Testing

Specs run with this directory's own bundle (kept separate from the repo
root bundle so the core suite never auto-integrates the plugin config):

```sh
cd gems/shirobai-performance
bundle install
bundle exec rspec
```

The parity oracle for this gem is `benches/parity_diff_performance.sh`
at the repo root (see `benches/README.md`).

Implemented cop status lives in `docs/cop-status.md` at the repo root.
