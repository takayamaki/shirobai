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

## Corpora

Test corpora live in `.tmp/` (gitignored).
Run `bin/setup-corpora` to clone them.
`rubocop_source` is a symlink to `vendor/rubocop` (tracked in git).
