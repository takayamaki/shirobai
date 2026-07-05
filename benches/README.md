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

When `modes` is empty the workflow picks them by ref: `stock removed shirobai`
on main, plus `main` on a branch (so a branch is compared against main on the
same runner). A branch run adds a second checkout of main, builds it, and
installs `Gemfile.realconfig.main`.

Runner times are slower and noisier than the README numbers
(shared 4-vCPU runners); do not compare seconds across runs. Trust the
same-runner derived rows instead — `branch - main` is the decision line,
and each job verifies the offense sets match before timing.

## Corpora

Test corpora live in `.tmp/` (gitignored).
Run `bin/setup-corpora` to clone them.
`rubocop_source` is a symlink to `vendor/rubocop` (tracked in git).
