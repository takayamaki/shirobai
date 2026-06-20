# shirobai project rules

See [`rules/repository-overview.md`](rules/repository-overview.md) (symlink to `README.md`)
for the repository overview.
This file only has rules that are not in the README.

## Language rule

All documents in this repository — including commit messages — must be in **simple, easy English**.
The only exception is `README.ja.md` (Japanese).

## Core principle: full drop-in compatibility

Two things matter, and both are required:

1. **Detection and autocorrect (`-a`/`-A`) output must match stock byte for byte.**
   Do not ship a cop with pending autocorrect.
   If a cop cannot reach full compatibility, revert the wiring and drop it.
2. **Speed.**
   Do not make changes that slow things down.

When in doubt, compatibility wins over speed.
"Almost matches", "doesn't show up in the corpus",
and "only happens in test code" are not reasons to ship.

## Commit message rules

- **Line 1 (summary):** What changed and how, in one short line.
- **Line 3+ (detail):** Why the change was made.
  Do not repeat what changed — the diff shows that.

## How to add a new cop

1. **Always work on a separate branch.**
   Do not commit directly to main.
2. Steps:
   - Read the stock implementation (in `vendor/rubocop`) and its vendor spec.
   - Probe the stock cop on real code to find quirks. Do not guess.
   - Build the Rust rule + Ruby wrapper + wiring (the 4+1 checklist):
     - Per-cop entry point (for fallback)
     - `bundle_args` (single source of config)
     - Slot in `BundleConfig` / `check_all_bundle`
     - Entry in `Dispatch::SLOTS` (the single source of cop-to-slot mapping)
     - If the cop joins the shared walk: publish `build_rule` + add an equivalence test
   - Pass all vendor specs.
     Add cases to `correctable_parity_spec` and the non-ASCII parity spec.
   - Turn any quirks found during probing into **edge-case regression specs**
     (`spec/shirobai/cop/<dept>/<name>_edge_cases_spec.rb`).
     Prefer differential style: run the same snippet through both stock and shirobai,
     assert that offenses and autocorrect output match.
   - Get `benches/parity_diff.sh` to show zero diff on all 5 corpora.
   - Finish as one commit on the branch.

## Four merge gates

The branch must have one commit where all of these pass:

1. `bundle exec rspec` — all pass (pending is OK, fail is not)
2. `cargo test` — all pass (run at workspace root)
3. `cargo clippy --all-targets` — **zero new warnings**
4. **`benches/parity_diff.sh` with zero diff on all 5 corpora**
   (Mastodon / Discourse / Redmine / RuboCop itself / fluentd)

## Speed criteria for merging

Merge in units where **at least 4 out of 5 corpora** show a clear speedup
in real-config end-to-end benchmarks (each corpus's own `.rubocop.yml`).
This can be a single cop or a group of cops —
what matters is that the merged unit produces a measurable improvement.

## The truth oracle is benches/parity_diff.sh

- `benches/e2e_bench.rb` is for speed measurement only.
  **Do not use it as the final parity check.**
- When writing your own check scripts, **always set config on ProcessedSource.**
  Without config, AlignmentCorrector cops crash silently on both sides
  and report zero offenses, hiding real differences.
- **The truth is always the real CLI (fresh cop per file).**
  Never reuse cop instances or the Commissioner across files — stock cops leak state.

## Environment notes

- RuboCop is pinned with `spec.add_dependency "rubocop", "= 1.88.0"`.
  Only bump it on purpose — even minor updates can break compatibility.
- Build with `bundle exec rake compile`
  (runs `cargo build --release -p shirobai-ext` + copies `.so` to `lib/shirobai/`).
  The workspace `Cargo.toml` uses fat LTO + codegen-units=1.
- `vendor/rubocop` is a git submodule pinned to 1.88.0.
  Vendor specs are pulled into the spec suite from there.
