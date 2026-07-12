---
description: Benchmarks and parity oracle — how to measure speed and verify compatibility
paths:
  - benches/**
---

# benches/

Benchmarks and the parity oracle.

## Key files

- `parity_diff.sh` — **The truth oracle.**
  Runs the real `rubocop` CLI twice (stock vs shirobai) on a corpus
  and diffs per-cop / per-offense output plus autocorrected bytes.
  Zero diff is the only acceptable result before merging.
- `autocorrect_audit.sh` — **The `-a` byte oracle.**
  Copies a corpus twice (`cp -rL`), runs stock and shirobai arms with
  `-a` sequentially (`RUBOCOP_TARGET_RUBY_VERSION` pinned to 3.1 — the
  corpora have no `.ruby-version`, and an unpinned run falls back to the
  2.7 parser), then byte-compares the trees with
  `diff -rq --no-dereference`. Lint-mode parity cannot see divergences
  that only materialize as corrections cascade across `-a` iterations;
  this audit is the only gate that can. Zero differing files is the only
  acceptable result. Run it before releases, on autocorrect-surface
  branches (redmine + fluentd at minimum), and on rubocop core / plugin
  pin bumps (all five corpora). The offense counts printed by the two
  arms may differ by a few — intermediate cascade states are not
  required to match; the gate is the final tree diff only.
  Usage: `autocorrect_audit.sh .tmp/fluentd` (exit 1 + kept trees on
  divergence, `KEEP=1` to keep them either way).
- `parity_diff_performance.sh` — The same oracle for the
  shirobai-performance plugin gem: both sides run with
  `--plugin rubocop-performance --enable-pending-cops`
  (Gemfile.stock.performance vs Gemfile.with_shirobai.performance).
  Zero diff required on the same corpora.
- `parity_diff_rspec.sh` — The oracle for the shirobai-rspec plugin gem
  (Gemfile.stock.rspec vs Gemfile.with_shirobai.rspec). Unlike the other
  two it does NOT use `--force-default-config`: that flag skips the
  plugin config merge, leaves `RSpec/Language` empty and silences every
  RSpec cop on both sides (an empty parity). It writes a uniform config
  into the corpus root (`inherit_from` of the pinned rubocop-rspec
  default.yml + `DisabledByDefault: false`), runs both CLIs from inside
  the corpus, and self-tests on a synthetic fixture that must fire the
  implemented cops on the stock side first.
  Main corpora: discourse / forem / factory_bot (densest RSpec offense
  surface), mastodon as the clean non-interference check.
- `parity_diff_rails.sh` — The oracle for the shirobai-rails plugin gem
  (Gemfile.stock.rails vs Gemfile.with_shirobai.rails). Like the rspec
  oracle it writes a uniform config into the corpus root (`inherit_from`
  of the pinned rubocop-rails default.yml) and runs both CLIs from inside
  the corpus, but with `DisabledByDefault: true`: rails cops share files
  with core cops, so scoping to the Rails department keeps this oracle
  from re-litigating core parity (owned by `parity_diff.sh`). The
  Application* cops are gated on railties `>= 5.0` resolved from the
  target lockfile, so the synthetic self-test dir gets a minimal
  Gemfile.lock written. Main corpora: mastodon / redmine / discourse
  (rails-dense), fluentd as the no-rails non-interference check.
- `e2e_bench.rb` — In-process speed measurement harness.
  Accepts a corpus path and loads its `.rubocop.yml`
  (skips require/inherit_gem so plugin gems are not needed).
  **Not** for final parity checks (use `parity_diff.sh` for that).
- `run_e2e.sh` — Runs removed/shirobai modes N rounds back to back.
  Usage: `run_e2e.sh .tmp/mastodon 3`
- `aggregate_e2e.rb` — Aggregates `e2e_bench.rb` output
  and prints compute/cpu/gc medians and the net win.
- `real_cli_bench.sh` — Real-CLI benchmark with the corpus's own config
  and all plugin gems (the README speed table comes from this).
  Usage: `real_cli_bench.sh .tmp/mastodon 3`
  Runs rubocop from **inside** the corpus directory: relative Exclude
  patterns in default configs (e.g. rubocop-rails's `bin/*`) anchor to
  the working directory, so running from outside would lint files that
  users of that project never lint.
  - `MODES="stock shirobai"` picks which modes run each round, in order
    (default is the historical `stock shirobai`). Supported modes:
    - `stock` — unchanged rubocop.
    - `shirobai` — rubocop + shirobai from this tree.
    - `removed` — stock rubocop with the implemented cops dropped via
      `--except`. This is the theoretical upper bound of the replacement
      (`stock - removed`); the summary reports how much of it the branch
      actually captured (`achieved`).
    - `main` — rubocop + shirobai from a second checkout of the main
      branch (`Gemfile.realconfig.main`, needs `SHIROBAI_MAIN_TREE`). Run
      alongside `shirobai` to compare branch vs main on the same runner
      (`branch - main`, the decision line) without cross-run noise.
    - `plugins` — rubocop + shirobai core + the plugin shells
      (shirobai-performance / shirobai-rspec) from this tree
      (`Gemfile.realconfig.plugins`). This is the release instrument for
      the plugin replacement: it measures the plugin cops end to end. Each
      shell is required only when the corpus's resolved config really loads
      the matching stock plugin — the same require line a real user would
      write. Force-requiring shirobai-rspec on a corpus that never loads
      rubocop-rspec pulls in the stock rubocop-rspec cops without their
      default.yml Includes, and under `NewCops: enable` the non-replaced
      ones fire on non-spec files (seen on Redmine). The summary adds
      `plugins saving` (`stock - plugins`) and `plugin effect`
      (`shirobai - plugins`, the extra speed the plugin cops add on top of
      core-only shirobai).
    - `plugins-main` — the same trio from a second checkout of the main
      branch (`Gemfile.realconfig.plugins.main`, needs `SHIROBAI_MAIN_TREE`),
      with the same conditional requires as `plugins`. Run alongside
      `plugins` to compare a plugin-cop branch vs main on the same runner
      (`plugins: branch - main`, the decision line for plugin branches).
  - `VERIFY=1` runs each mode once with JSON output first and fails when a
    shirobai-family mode's offense set differs from stock. These runs also
    warm the file cache.
  - `SUMMARY_FILE=<path>` appends a markdown summary
    (CI passes `$GITHUB_STEP_SUMMARY`).
  - Every run records `provenance:` lines — the resolved gem versions
    (rubocop / rubocop-ast / parser / prism / rubocop-performance) per mode,
    so a result is reproducible and version-driven changes are visible.
- `implemented_cops.rb` — Prints the badge names shirobai replaces, one
  comma-separated line, read from the resolved registry (the source of
  truth). `real_cli_bench.sh` feeds it to `--except` for the removed mode.
- `offense_diff.rb` — Compares two rubocop JSON outputs per offense.
  Used by `VERIFY=1`. Unlike `parity_diff.sh` it allows offenses to exist;
  the check is that both sides report the same set.

## Bench on GitHub Actions

`.github/workflows/bench.yml` runs `real_cli_bench.sh` with `VERIFY=1`
on one runner per corpus and writes the result to the job summary.

```sh
gh workflow run bench.yml                                      # all four corpora
gh workflow run bench.yml -f corpora="redmine" -f rounds=5     # subset
gh workflow run bench.yml -f modes="stock shirobai"            # pick modes
gh workflow run bench.yml --ref my-branch                      # bench a branch
```

When `modes` is empty the workflow picks them by ref. On main it runs
`stock removed shirobai plugins`: main carries the plugins release instrument,
so it also measures shirobai-performance + shirobai-rspec end to end. On a
branch it runs `stock removed shirobai main`, so the branch is compared against
main on the same runner. A branch run adds a second checkout of main, builds
it, and installs `Gemfile.realconfig.main`.

A plugin-cop branch opts into the plugin comparison explicitly with
`-f modes="stock shirobai plugins plugins-main"`. `plugins-main` reuses the
same second checkout of main (its name contains `main`, so the checkout/build
steps fire for it too) and installs `Gemfile.realconfig.plugins.main`. The
`plugins: branch - main` row is then the decision line for the plugin branch.

Runner times are slower and noisier than the README numbers
(shared 4-vCPU runners); do not compare seconds across runs. Trust the
same-runner derived rows instead — `branch - main` is the decision line,
and each job verifies the offense sets match before timing.

## Corpora

Test corpora live in `.tmp/` (gitignored).
Run `bin/setup-corpora` to clone them.
`rubocop_source` is a symlink to `vendor/rubocop` (tracked in git).
