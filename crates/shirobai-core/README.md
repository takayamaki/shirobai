---
description: Rust analysis core — per-cop rules, shared walk, and known pitfalls (trap table)
paths:
  - crates/shirobai-core/**
  - lib/shirobai/cop/**
  - ext/shirobai/**
---

# crates/shirobai-core/

Rust analysis core. All cop logic lives here.

## Layout

- `src/rules/<name>.rs` — One file per cop.
  Each publishes a `Rule` via `build_rule()`.
  The standalone path and the shared-walk (bundle) path
  run the same logic — no copy allowed.
  `cargo test` guards equivalence.
- `src/rules/bundle.rs` — Shared-walk dispatcher.
  `check_all_bundle` does one prism AST traversal per file
  and produces results for every active cop at once.
  `BundleConfig` wire format is documented in its doc comment.
- `src/lib.rs` — Crate root, re-exports.

## Rules

- Never duplicate cop logic between standalone and bundle paths.
  Both must call the same `Rule` implementation.
- prism uses byte offsets.
  The Ruby wrapper handles byte-to-char conversion;
  Rust code should work in bytes throughout.

## Trap table

Known pitfalls found during past cop implementations.
Read this before starting a new cop.

| Trap | Fix |
|---|---|
| **multi-pass autocorrect `other_offense_in_same_range?`** builds up state across iterations | `expect_correction` must loop until stable |
| ruby-prism `Visit` generic enter hook **does not fire for leaf nodes** (String, etc.) | Override typed visitor (`visit_string_node`, etc.) and recurse |
| Prism **implicit begin** (rescue/ensure without begin) has location at def/block start | Normalize with `make_body` equivalent |
| Closing paren/`end` location is **token begin**, not node end_offset | Use `closing_loc().start_offset()` in frame |
| parse_cache is a single RefCell; calling `with_parsed` during walk **panics** | Collect comments/literals/semicolons before starting the walk |
| Prism braceless hash: last arg is `KeywordHashNode` with Arguments wrapping it | Use effective parent to see through; `.pairs` excludes kwsplat |
| Full-width / multibyte display width | Use `unicode-width` crate; split strings by char, convert to byte |
| **Offset unit mismatch** (prism = bytes, parser-gem = chars) | Apply SourceOffsets to every offset field in the wrapper |
| `RedundantSelf` `add_scope` uses **shared array semantics** | Reproduce with owner-array stack |
| magnus argument count limit | Pack config as `Vec<i64>` (nums/lists); run regex on Ruby side |
| **Stock cops change behavior when instances are reused** (state leaks) | Always use **real CLI (fresh per file)** as the oracle |
| **`raw_source` vs `buffer.source` byte mismatch** (CRLF/BOM normalization) | Only use bundle path when `buffer.source == raw_source` |
| **Parser `block` node includes lambda literals** but prism splits them into `BlockNode` / `LambdaNode` | Handle LambdaNode the same way |
| **Prism write-targets bypass read-node typed fields** | Override write-node visitors to reprocess or exclude targets |
| **Prism CallNode setter `=` is not in `message_loc`** | Use `equal_loc` instead |
| **Stock `string_source` returns nil for `:dstr`/`:dsym`** | Do not descend into dstr/dsym parts |
| **Chained `foo(...).bar` shares send_range begin_pos** | Return (begin, end) pair from Rust; pick the exact-match send in wrapper |
| **block_pass arg mapping differs between parser-gem and prism** | Include block_pass as a virtual trailing argument in `arg_count`/`arg_range_bounds` |
| **Prism `RegularExpressionNode.closing_loc` includes options** | Snap autocorrect range to 1 byte |
| **Prism uses `InterpolatedStringNode` for empty `%()`** | Override both typed visitors |
| **ruby-prism `Visit` generated code calls typed visitors directly** (StatementsNode skips branch hooks) | In `Rule::enter`, catch `program_node` and explicitly process `statements()` |
| **Prism `IndexOrWriteNode` family** | Also override `IndexOperatorWriteNode` / `IndexOrWriteNode` / `IndexAndWriteNode` |
| **`ProcessedSource#lines.size` counts a phantom trailing empty entry** when source ends with `\n` (e.g. `"private\n".lines == ["private", ""]`); stock's `next_line_empty_and_exists?` compares against this size | Use `line_starts().len()` as-is — it matches stock's `lines.size`. Do NOT subtract 1 |
