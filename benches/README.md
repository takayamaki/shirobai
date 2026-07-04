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
  - `VERIFY=1` runs each mode once with JSON output first and fails
    when the offense sets differ. These runs also warm the file cache.
  - `SUMMARY_FILE=<path>` appends a markdown summary
    (CI passes `$GITHUB_STEP_SUMMARY`).
- `offense_diff.rb` — Compares two rubocop JSON outputs per offense.
  Used by `VERIFY=1`. Unlike `parity_diff.sh` it allows offenses to exist;
  the check is that both sides report the same set.

## Bench on GitHub Actions

`.github/workflows/bench.yml` runs `real_cli_bench.sh` with `VERIFY=1`
on one runner per corpus and writes the result to the job summary.

```sh
gh workflow run bench.yml                                      # all four corpora
gh workflow run bench.yml -f corpora="redmine" -f rounds=5     # subset
gh workflow run bench.yml --ref my-branch                      # bench a branch
```

Runner times are slower and noisier than the README numbers
(shared 4-vCPU runners); compare the percentage, not the seconds.
Each job is internally consistent — stock and shirobai run
alternately on the same runner.

## Corpora

Test corpora live in `.tmp/` (gitignored).
Run `bin/setup-corpora` to clone them.
`rubocop_source` is a symlink to `vendor/rubocop` (tracked in git).
