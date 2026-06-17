---
description: magnus bridge (cdylib) — Ruby entry points and tuple mappers
paths:
  - ext/shirobai/**
---

# ext/shirobai/

magnus bridge (cdylib) that exposes `shirobai-core` to Ruby.

## Layout

- `src/lib.rs` — Ruby entry points and tuple mappers.
  Defines the wire shape for each cop's result tuple in one place.
  Source strings are borrowed from Ruby via `RString` (no copy).

## Rules

- `Cargo.toml` here is the extension crate that `rake compile` builds.
  The workspace root `Cargo.toml` sets fat LTO + codegen-units=1.
- Keep tuple mappers thin — logic belongs in `shirobai-core`, not here.
- magnus has an argument count limit.
  Config is packed as `Vec<i64>` (nums/lists).
  Regex matching stays on the Ruby side.
