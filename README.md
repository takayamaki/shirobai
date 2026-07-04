# shirobai

shirobai is an experimental gem that speeds up [RuboCop](https://github.com/rubocop/rubocop)
by replacing some of its cops with fully compatible Rust implementations.

[日本語版は README.ja.md](README.ja.md)

> [!WARNING]
> This gem is experimental.
> I try hard to stay compatible with RuboCop, but I make no guarantee about production use.

## Why shirobai exists

### A drop-in for RuboCop, not a replacement

When people try to speed up a linter, they often rewrite everything from scratch with a new interface.
shirobai does the opposite: RuboCop stays in charge,
and shirobai only replaces the slow parts of each cop (like AST walks) with Rust code.

I respect RuboCop's large ecosystem and its design that lets developers write their own cops.
I have no intention to compete with it.

### Full compatibility with RuboCop

shirobai treats the behavior tested by each stock cop's spec as the absolute truth.

I also run the real `rubocop` CLI on these repositories using each project's own config,
and check that shirobai gives the same results as stock RuboCop:

- RuboCop
- [Mastodon](https://github.com/mastodon/mastodon)
- [Discourse](https://github.com/discourse/discourse)
- [Redmine](https://github.com/redmine/redmine)
- [fluentd](https://github.com/fluent/fluentd)

I also hope to contribute back to RuboCop when I find behavior that should be tested by spec but isn't.

### About the name

In Japan, police officers who patrol on motorcycles are called
"[shiro-bai](https://en.wiktionary.org/wiki/%E3%81%97%E3%82%8D%E3%83%90%E3%82%A4)" (white bikes).
The image is simple: RuboCop hops on a shiro-bai and gets faster.

## Current status

- **76 cops** reimplemented in Rust (Lint / Layout / Metrics / Naming / Style).
- **Full drop-in compatibility** verified on real codebases.
  For every implemented cop, every offense position, message,
  and autocorrected byte matches stock RuboCop.
  I do not ship a cop with pending autocorrect.
  If a cop cannot reach full compatibility, I remove it.
- **Real-world speedup** — real CLI, each project's own `.rubocop.yml`,
  all plugin gems installed, 5-round median:

  | Corpus | files | offenses | stock | shirobai | saving |
  |---|---|---|---|---|---|
  | Mastodon | 3,206 | 0 | 116.25s | 90.57s | **-25.69s (-22.1%)** |
  | Discourse | 10,229 | 16 | 259.56s | 237.86s | **-21.70s (-8.4%)** |
  | Redmine | 1,058 | 2 | 56.73s | 43.24s | **-13.49s (-23.8%)** |
  | fluentd | 456 | 0 | 9.73s | 9.97s | +0.24s (+2.5%) |

  Measured on GitHub Actions `ubuntu-latest` (4-vCPU shared runner)
  against shirobai at commit [`84b6906`](https://github.com/takayamaki/shirobai/commit/84b6906).
  Each run first verifies that stock and shirobai report the **same offense set**
  on the corpus's own config; the table shows the median time to lint the same code.
  Rerun on any commit via `gh workflow run bench.yml`
  (`.github/workflows/bench.yml`).

  shirobai only replaces cops from the rubocop gem itself.
  Plugin cops (rubocop-rails, rubocop-rspec, etc.) run unchanged,
  so projects that spend more time on plugin cops see a lower percentage improvement
  (Discourse is a heavy plugin user).
  fluentd's config disables most default cops, leaving very few rubocop-gem cops
  for shirobai to replace — the small fixed cost of loading the native extension
  slightly outweighs the savings there.

  RuboCop itself is also used for compatibility verification but not benchmarked,
  because its own config needs `rubocop-internal_affairs` / `rubocop-rake`
  and leaves few rubocop-gem cops enabled.

## Requirements

> [!IMPORTANT]
> shirobai's native extension is written in Rust.
> `bundle install` runs `cargo build --release`,
> so you need **Rust toolchain (stable, 1.75 or newer)** on the machine where you install.
> Install it with [rustup](https://rustup.rs/) first.

| | |
|---|---|
| RuboCop | **pinned to `= 1.88.0`** |
| Ruby | `>= 3.1` |
| Rust | `>= 1.75` (stable) |
| Platforms | Linux / macOS (anywhere `cargo build --release` works) |
| Ruby parser | `ruby-prism` (Latest grammar ≈ Ruby 4.1) |

The hard pin on RuboCop is on purpose.
shirobai copies cop behavior at the byte level, so even a minor RuboCop update can break compatibility.
I prefer a failed install over a silent difference.

### Known limitation: `AllCops/TargetRubyVersion`

shirobai always parses with prism's Latest grammar.
In practice, only three cops are affected:

- **Layout/SpaceAroundKeyword** when detecting the Ruby 2.7
  `expr in pat` one-line pattern match.
- **Lint/DuplicateMagicComment** when a file has an INDENTED `__END__`
  line (parsers before 3.4 stop reading there; prism reads on).
- **Lint/DuplicateMethods** when a method is defined inside a block
  that uses the bare `it` parameter (an `it` block only exists in 3.4+).

All other implemented cops work the same regardless of TargetRubyVersion.
If you need strict target-version behavior for these cops,
you can disable shirobai's replacement in your config; the stock cop will run instead.

## Installation

Add to your Gemfile next to `rubocop`:

```ruby
gem "rubocop", "= 1.88.0"
gem "shirobai"
```

Then run `bundle install`.

## Usage

Add one line to your `.rubocop.yml`:

```yaml
require:
  - shirobai
```

That's it.
shirobai registers each Rust-backed cop under the same badge as the stock cop,
so everything in RuboCop keeps working as before:
config, disable comments, `--only`, `--except`, `--auto-correct`, ResultCache, and so on.
No other `.rubocop.yml` change is needed.

## How it works

```
┌───────────────────────────────────────────────────────────────────┐
│ RuboCop (Ruby front end)                                          │
│   Runner -> Team -> Commissioner -> cop instances (per file)      │
└───────────────────────────────────────────────────────────────────┘
                          │
                          │ Rust-backed cops register
                          │ under the same badge as stock
                          ▼
┌───────────────────────────────────────────────────────────────────┐
│ lib/shirobai/cop/<dept>/<name>.rb (Ruby wrapper)                  │
│   - Turns Rust result tuples into Parser::Source::Range,          │
│     offenses, and corrector calls                                 │
│   - Converts byte offsets to char offsets for non-ASCII sources   │
│     (prism uses bytes, parser-gem uses chars)                     │
└───────────────────────────────────────────────────────────────────┘
                          │
                          │ One pass per file via Dispatch
                          ▼
┌───────────────────────────────────────────────────────────────────┐
│ crates/shirobai-core (Rust)                                       │
│   - Shared walk: one prism AST traversal produces results for     │
│     all cops at once (rules/bundle.rs)                            │
│   - Each cop publishes a Rule via build_rule(); standalone and    │
│     shared-walk paths run the same logic (no copy)                │
│ ext/shirobai (magnus bridge): exposes check_all_bundle to Ruby    │
└───────────────────────────────────────────────────────────────────┘
```

Key ideas:

- **Shared walk.**
  `Shirobai.check_all(src, token)` walks the prism AST once per file
  and produces results for all active Rust cops at once.
  Adding one more cop does not add another full-file walk.
- **Same logic, two drivers.**
  Each Rust rule is published via `build_rule()`.
  The standalone path (per-cop fallback) and the bundle path (shared walk) run the same code.
  `cargo test` checks that they stay equal.
- **Drop-in via badge replacement.**
  `inject.rb` calls `registry.enlist(klass)`
  so each Rust cop takes the same registry slot as the stock cop.
  RuboCop sees no difference.

## Repository layout

Each directory has its own `README.md` with details.

| Directory | What it is |
|---|---|
| `lib/shirobai/` | Ruby wrappers, Dispatch, SourceOffsets, inject |
| `crates/shirobai-core/` | Rust analysis core (per-cop rules + shared walk) |
| `ext/shirobai/` | magnus bridge (cdylib) |
| `benches/` | Benchmarks and the parity oracle |
| `spec/` | RSpec, vendor spec inclusion, edge-case parity |
| `vendor/rubocop/` | Git submodule pinned to 1.88.0 for vendor specs |

## Building and testing

```sh
bundle install
bundle exec rake compile          # cargo build --release + copy .so into lib/
bundle exec rspec                 # Ruby: vendor spec + parity spec
cargo test                        # Rust: rule equivalence and unit tests
cargo clippy --all-targets        # No new warnings is the merge bar
```

### Parity check (drop-in compatibility)

First, clone the test corpora:

```sh
bin/setup-corpora
```

This clones Mastodon, Discourse, Redmine, and fluentd into `.tmp/` at pinned commits.
`rubocop_source` is a symlink to `vendor/rubocop` (already tracked in git).

Then run the parity oracle on each corpus:

```sh
benches/parity_diff.sh .tmp/mastodon
benches/parity_diff.sh .tmp/discourse
benches/parity_diff.sh .tmp/redmine
benches/parity_diff.sh .tmp/fluentd
benches/parity_diff.sh .tmp/rubocop_source
```

Each run launches the real `rubocop` CLI twice
— once with `Gemfile.stock` (no shirobai), once with `Gemfile.with_shirobai` —
and diffs per-cop / per-offense (`path:line:column:message`).
**Zero diff on all 5 corpora is required before merging.**

### Speed benchmark

```sh
benches/run_e2e.sh .tmp/mastodon 3
```

This measures in-process speed on Mastodon using its `.rubocop.yml`
(cop enable/disable and parameters are loaded; plugin gems are not required).
It runs three modes per round:

- **stock** — all default cops, unchanged
- **removed** — the implemented cops dropped entirely (speed floor)
- **shirobai** — the implemented cops replaced by Rust (actual speed)

The script prints a summary with compute/cpu/gc medians and the net win.

## For Claude Code agents

This repository is developed with Claude Code.
See [`.claude/CLAUDE.md`](.claude/CLAUDE.md) for project rules.
This README is symlinked into `.claude/rules/repository-overview.md`.

## License

MIT. See [LICENSE.txt](LICENSE.txt).
