---
description: RSpec test suite — vendor specs, edge-case parity, correctable/non-ASCII parity
paths:
  - spec/**
---

# spec/

RSpec test suite.

## Structure

- `shirobai/cop/<dept>/<name>_spec.rb` — Vendor spec inclusion.
  These pull in the stock cop's spec from `vendor/rubocop/spec/`
  via `VendorSpecHelper` in `spec_helper.rb`,
  so the Rust implementation passes the same tests as stock.
- `shirobai/cop/<dept>/<name>_edge_cases_spec.rb` — Edge-case regression specs.
  Differential style preferred: run the same snippet through both stock and shirobai,
  assert that offenses and autocorrect output match.
  Uses shared helpers from `support/edge_case_parity.rb`.
- `shirobai/correctable_parity_spec.rb` — Tests that `correctable?` status matches stock
  when autocorrect is **not** enabled (vendor specs always enable it, so they miss this).
- `shirobai/non_ascii_offset_parity_spec.rb` — Per-offense offset parity on non-ASCII sources.
- `spec_helper.rb` — Loads shirobai, RuboCop test support,
  and vendor spec helpers (misc_helper, shared examples, etc.).

## Rules

- Every new cop must add cases to `correctable_parity_spec`
  and `non_ascii_offset_parity_spec`.
- Edge-case specs go in `shirobai/cop/<dept>/<name>_edge_cases_spec.rb`.
  They capture quirks found during stock probing
  that vendor specs don't cover.
