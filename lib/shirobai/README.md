---
description: Ruby wrapper layer — cop wrappers, Dispatch, SourceOffsets, inject
paths:
  - lib/shirobai/**
---

# lib/shirobai/

Ruby layer of shirobai.

## Key files

- `inject.rb` — Registers Rust-backed cops into RuboCop's registry
  via `registry.enlist(klass)`. This is the central require list;
  add new cops here.
- `dispatch.rb` — Per-file coordinator.
  Memoizes `Shirobai.check_all(src, token)` by (raw_source identity, config identity)
  so the first cop triggers one Rust call and the rest pull from the cache.
  `Dispatch::SLOTS` is the single source of cop-to-slot mapping:
  `[origin, rule]` pairs into the per-origin result
  (`result[origin][rule]`; `Dispatch::ORIGINS` fixes the origin order).
  Plugin gems (gems/shirobai-performance) register their packed-config
  segment with `Dispatch.register_plugin_packer`; the dormant default
  keeps the Rust-side plugin rules asleep when the plugin gem is not
  loaded.
- `source_offsets.rb` — Converts byte offsets (prism) to char offsets (parser-gem).
  ASCII fast path: one `ascii_only?` check per source, identity conversion if true.
- `cop/base.rb` — Shared base for all wrapper cops.
- `cop/<dept>/<name>.rb` — One wrapper per implemented cop.
  Each wrapper turns Rust result tuples into `Parser::Source::Range`, offenses,
  and corrector calls.

## Rules for wrapper cops

- Every offset field from Rust **must** go through `SourceOffsets`.
- `bundle_args(config)` is the single source of config for each cop.
  It reads `config.for_badge(badge)` (same resolution as `Base#cop_config`).
- `bundle_eligible?` returns false for configs the Rust side can't handle
  (e.g. custom patterns). The cop then falls back to its per-cop entry point.
