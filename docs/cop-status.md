# Cop implementation status

This document tracks which RuboCop cops shirobai has reimplemented in Rust,
and which cops were attempted but reverted because they did not meet the
project's drop-in compatibility and speed requirements together.

## Implemented (88 cops)

shirobai replaces these cops with Rust implementations.
Every offense position, message, and autocorrected byte matches stock RuboCop
on all five verification corpora (Mastodon, Discourse, Redmine, fluentd,
and RuboCop itself).

### Layout (49)

- `Layout/AccessModifierIndentation`
- `Layout/ArgumentAlignment`
- `Layout/ArrayAlignment`
- `Layout/AssignmentIndentation`
- `Layout/BlockAlignment`
- `Layout/ClosingParenthesisIndentation`
- `Layout/DefEndAlignment`
- `Layout/DotPosition`
- `Layout/ElseAlignment`
- `Layout/EmptyComment`
- `Layout/EmptyLineAfterGuardClause`
- `Layout/EmptyLineAfterMagicComment`
- `Layout/EmptyLineBetweenDefs`
- `Layout/EmptyLines`
- `Layout/EmptyLinesAroundArguments`
- `Layout/EmptyLinesAroundBeginBody`
- `Layout/EmptyLinesAroundBlockBody`
- `Layout/EmptyLinesAroundClassBody`
- `Layout/EmptyLinesAroundExceptionHandlingKeywords`
- `Layout/EmptyLinesAroundMethodBody`
- `Layout/EmptyLinesAroundModuleBody`
- `Layout/EndAlignment`
- `Layout/FirstArgumentIndentation`
- `Layout/FirstArrayElementIndentation`
- `Layout/FirstHashElementIndentation`
- `Layout/HashAlignment`
- `Layout/IndentationConsistency`
- `Layout/IndentationWidth`
- `Layout/LeadingEmptyLines`
- `Layout/LineLength`
- `Layout/MultilineMethodCallBraceLayout`
- `Layout/MultilineMethodCallIndentation`
- `Layout/MultilineOperationIndentation`
- `Layout/SpaceAfterColon`
- `Layout/SpaceAfterComma`
- `Layout/SpaceAfterSemicolon`
- `Layout/SpaceAroundKeyword`
- `Layout/SpaceAroundMethodCallOperator`
- `Layout/SpaceBeforeBlockBraces`
- `Layout/SpaceBeforeComma`
- `Layout/SpaceBeforeComment`
- `Layout/SpaceBeforeFirstArg`
- `Layout/SpaceBeforeSemicolon`
- `Layout/SpaceInsideArrayLiteralBrackets`
- `Layout/SpaceInsideBlockBraces`
- `Layout/SpaceInsideHashLiteralBraces`
- `Layout/SpaceInsideParens`
- `Layout/SpaceInsideReferenceBrackets`
- `Layout/TrailingEmptyLines`

### Lint (11)

- `Lint/AmbiguousBlockAssociation`
- `Lint/Debugger`
- `Lint/DuplicateMagicComment`
- `Lint/DuplicateMethods`
- `Lint/ParenthesesAsGroupedExpression`
- `Lint/RequireParentheses`
- `Lint/SafeNavigationChain`
- `Lint/SelfAssignment`
- `Lint/UnreachableCode`
- `Lint/UselessAccessModifier`
- `Lint/Void`

### Metrics (8)

- `Metrics/AbcSize`
- `Metrics/BlockLength`
- `Metrics/BlockNesting`
- `Metrics/ClassLength`
- `Metrics/CyclomaticComplexity`
- `Metrics/MethodLength`
- `Metrics/ModuleLength`
- `Metrics/PerceivedComplexity`

### Naming (3)

- `Naming/MethodName`
- `Naming/PredicatePrefix`
- `Naming/VariableNumber`

### Style (17)

- `Style/BlockDelimiters`
- `Style/ColonMethodCall`
- `Style/HashEachMethods`
- `Style/HashSyntax`
- `Style/HashTransformKeys`
- `Style/IfUnlessModifier`
- `Style/LineEndConcatenation`
- `Style/NestedParenthesizedCalls`
- `Style/PercentLiteralDelimiters`
- `Style/RedundantSelf`
- `Style/RedundantSelfAssignment`
- `Style/StabbyLambdaParentheses`
- `Style/StringLiterals`
- `Style/StringLiteralsInInterpolation`
- `Style/TrailingCommaInArguments`
- `Style/TrailingCommaInArrayLiteral`
- `Style/TrailingCommaInHashLiteral`

## Plugin cops: shirobai-performance (proof of concept)

`gems/shirobai-performance` replaces rubocop-performance (pinned `= 1.26.1`)
cops with Rust rules that live in the shared shirobai-core extension
(no second native build). This batch is a proof of concept for the plugin
plumbing — monorepo gem boundary, load order, packed-config segment with
dormant slots, and the plugin parity oracle
(`benches/parity_diff_performance.sh`) — not a speed play: the measured
per-cop cost pool of the whole Performance department is flat and small
(restrict_on_send gating makes stock detection cheap).

### Implemented (5 cops)

- `Performance/Detect`
- `Performance/EndWith`
- `Performance/StartWith`
- `Performance/StringInclude`
- `Performance/TimesMap`

Verified with the same bar as core cops: vendor specs from the
`vendor/rubocop-performance` submodule, differential edge-case specs,
lint-mode correctable parity, non-ASCII offset parity, and the plugin
parity oracle with the real CLI
(`--plugin rubocop-performance --enable-pending-cops`) at zero diff.

### Known limitation (same family as the README `TargetRubyVersion` note)

`Performance/Detect` follows prism Latest for `it` blocks:
`foo.select { it.odd? }.first` parses as an it-block, which the stock
pattern (plain `block` only) does not match, so shirobai does not flag it.
Stock behaves the same for `TargetRubyVersion >= 3.4`; for targets `<= 3.3`
stock parses `it` as a plain method call and DOES flag it.
`Performance/TimesMap` is unaffected (its pattern is `any_block`).

## Plugin cops: shirobai-rspec (R1 cluster)

`gems/shirobai-rspec` replaces rubocop-rspec (pinned `= 3.10.2`) cops.
Unlike the performance batch this IS a speed play: measured on Mastodon
spec/, stock RSpec cops re-answer the `RSpec/Language` role question
about 87 times per send node and re-walk ancestors/subtrees per cop for
every structural question. shirobai routes ALL RSpec cops through one
`RSpecDispatcherRule` on the shared walk (a single second-layer
dispatcher): the role classification is one hash probe against a
per-config `name -> role mask` table, and the example-group scope tree
(stock's `find_all_in_scope`) is built once per file and shared.

The rspec origin is gated per file: the RSpec department only includes
spec files, so other files use a bundle token whose rspec segment is
dormant. The gate is the union of the wrapper cops' `relevant_file?`;
if they ever disagree, `Dispatch.offenses_for` returns nil and the
wrapper falls back to its standalone entry point (speed bug, never an
offense bug).

### Implemented (2 cops)

- `RSpec/LetSetup`
- `RSpec/VariableName`

Verified with the same bar as core cops: vendor specs from the
`vendor/rubocop-rspec` submodule (plus its shared contexts and smoke
tests), differential edge-case specs pinning the probed quirks (the
`inside_example_group?` top-level-statement gate, block/numblock/itblock
matcher asymmetries, `LetSetup`'s zero-argument-use search, sym/str
non-shadowing, Unicode style properties, AllowedPatterns +
detected-style bookkeeping), lint-mode correctable parity, non-ASCII
offset parity, and the rspec parity oracle (`benches/parity_diff_rspec.sh`
with its synthetic-fixture self-test) at zero diff.

### Known limitation (same family as the README `TargetRubyVersion` note)

RSpec matchers split on the parser block kind (`block` / `numblock` /
`itblock`), and shirobai recovers the split from prism Latest. A block
whose body uses a bare `it` (e.g. `let(:x) { it }`) is an it-block under
prism Latest and under stock with `TargetRubyVersion >= 3.4`, but a plain
`block` (with `it` as a method call) under stock with older targets, so
block-kind-sensitive matchers can disagree there. Real spec code calling
a bare `it` inside a block is essentially nonexistent (RSpec's own `it`
requires arguments or a block).

## Attempted but reverted

These cops were implemented to full drop-in compatibility but reverted because
the implementation's per-file overhead led to a net regression on at least one
large corpus when measured by paired end-to-end benchmark (main HEAD vs.
post-cluster HEAD on the corpus's own `.rubocop.yml`).

### `Layout/EmptyLinesAroundAccessModifier`

- **Where**: `feat/empty-lines-cluster` (2026-06-21).
- **What we tried**: shared-walk cop with a prior-sibling pre-walk to
  reproduce stock's plain-instance-variable mirror writes from
  `on_class` / `on_module` / `on_block`. Stock's ivars are not reset between
  siblings, so by the time `on_send` fires for a bare modifier the mirrors
  reflect the last class/block visited before it (including a nested class
  earlier in the same body). The naive port missed this because the bundle
  walk's wrapper hook fires before siblings are entered. The fix pre-walks
  every prior sibling subtree on each modifier visit and replays the same
  writes — the writes are pure last-write-wins, so the double-application is
  a no-op semantically, but it costs CPU.
- **Why it was reverted**: the pre-walk overhead scales with file size. On
  Discourse (10,519 files) it added enough per-file cost that the cluster's
  cumulative paired bench against `main` showed a clear regression on
  Discourse and a smaller one on Mastodon. The per-cop saving was not
  large enough to offset the pre-walk cost.

### `Layout/EmptyLinesAroundAttributeAccessor`

- **Where**: `feat/empty-lines-cluster` (2026-06-21).
- **What we tried**: shared-walk cop matching stock's
  `on_send`-then-look-at-next-sibling pattern, with config-aware
  `IgnoreClasses` handling.
- **Why it was reverted**: the per-cop paired bench on Mastodon showed a
  regression. With the cluster's other regressors dropped, this cop's
  cumulative contribution was still net-negative.

### `Layout/InitialIndentation`

- **Where**: `feat/empty-lines-cluster` (2026-06-21).
- **What we tried**: source-scan cop. Stock walks `processed_source.tokens`
  to find the first non-comment column; prism exposes no token stream, so the
  port included a byte-width lexer that replicates parser-gem's tFOO token
  widths for the cases that appear at the start of a file (identifiers,
  ivar/cvar/gvar markers, heredoc openers, percent literals, `::`, `->`,
  `&.`, `**`, `..`, `...`).
- **Why it was reverted**: the byte-width lexer ran on every file regardless
  of whether the file actually had any offense, and the per-file overhead was
  disproportionate to the saving. Per-cop paired bench on Mastodon showed a
  regression.

### Token-spacing cluster (6 cops)

`Layout/ExtraSpacing`, `Layout/SpaceInsideParens`, `Layout/SpaceBeforeComma`,
`Layout/SpaceAfterComma`, `Layout/SpaceAroundOperators` (AllowForAlignment
path), `Layout/SpaceBeforeFirstArg`.

- **Where**: branch isolated from `main` from the start (2026-06-14), never
  merged.
- **What we tried**: a shared token-scan rule built on top of a
  parser-gem-compatible token stream reconstructed from prism's lex output.
  All six cops share the token pass. The three AST-only siblings that don't
  depend on tokens (`SpaceAroundKeyword`, `SpaceInsideBlockBraces`,
  `SpaceAroundMethodCallOperator`) shipped separately and are in the
  Implemented list.
- **Why it was reverted**: stock RuboCop gets the parser-gem token stream
  for free as a by-product of its parse. The prism-based port has no such
  stream and has to spend an extra per-file pass to manufacture one — the
  "lex tax". The cluster's paired bench against `main` was net-negative even
  though detection and autocorrect matched stock byte-for-byte. Recovering
  the saving requires a parse-and-lex single-pass overhaul of the parsing
  layer, which is a much larger investment than the cluster itself.
- **Re-landed later (2026-07)**: `Layout/SpaceBeforeComma` and
  `Layout/SpaceAfterComma` shipped in the punctuation-spacing cluster
  without any token stream. These cops only read the tokens directly next
  to a `,` / `;` byte, so the token facts reduce to byte adjacency plus an
  opaque-region mask (strings / comments / heredoc bodies / gvar names /
  `__END__` data) collected on the shared walk — no lex tax. The four
  remaining cops above genuinely iterate the whole token stream and stay
  reverted.
- **Re-landed later (2026-07, cluster B)**: `Layout/SpaceInsideParens` and
  `Layout/SpaceBeforeFirstArg` shipped with the same reclassification.
  `SpaceInsideParens` reads the neighbors of every unmasked `(` / `)` byte;
  its one real token fact — the `tLPAREN_ARG` positions, which are not
  `left_parens?` — comes from the AST (a space-separated parenthesized
  first argument of a parenless call, plus the `yield` / `super` /
  `defined?` / `not` keyword forms). `SpaceBeforeFirstArg`'s
  `AllowForAlignment` scan is line-text-shaped except for one rare branch
  (a `:sym=`-shaped argument aligned with the first assignment token on a
  nearby line), reconstructed with a longest-match operator scan over
  unmasked bytes. Only `Layout/ExtraSpacing` and
  `Layout/SpaceAroundOperators` still iterate the whole token stream and
  stay reverted.

### `Style/RedundantBegin`

- **Where**: branch isolated from `main`, never merged (2026-06-17).
- **What we tried**: drop-in port of the redundant `begin` / `end` removal
  cop.
- **Why it was reverted**: detection over-fired by 5 offenses across the
  verification corpora — drop-in compat violation. Reverted regardless of
  speed.

### `Style/RedundantPercentQ` / `Lint/RedundantStringCoercion`

- **Where**: branch isolated from `main`, never merged (2026-06-17). Bundled
  with `Style/RedundantBegin` as a three-cop bulk candidate.
- **What we tried**: drop-in ports of redundant-removal cops grouped with
  `RedundantBegin`.
- **Why it was reverted**: per-cop parity was clean for these two, but the
  three-cop bulk paired bench against `main` showed no signal —
  round-to-round sign-consistency was zero. With `RedundantBegin` dropped
  for the parity failure, the remaining two were too small to justify
  shipping on their own. Worth re-evaluating in a future cluster.

## Notes on cop selection

- **Net speedup is the gating criterion.** A cop can match stock byte-for-byte
  in detection and autocorrect and still be a net negative if its
  implementation requires per-file overhead (token re-lexing, multi-pass AST
  walking, prior-sibling pre-walk) that scales worse than the saving.
- **Cops are merged in clusters** with the constraint that at least 4 of the
  5 corpora show a clear speedup in real-config end-to-end paired benchmarks
  (`main` HEAD vs. cluster HEAD). A regression on a large corpus (e.g.
  Discourse) outweighs speedups on smaller ones (e.g. fluentd).
- **Probe stock first, then implement.** Do not ship a cop with pending
  autocorrect. If full drop-in compatibility cannot be reached, revert the
  wiring and document why here.
- **A per-cop saving measured in isolation is an upper bound on the net
  speedup, not a prediction of it.** Stock cops have free access to
  parser-gem tokens and to document-order sibling state. When the
  prism-based port has to manufacture those signals (token re-lexing,
  prior-sibling pre-walk), the per-file overhead can erase the saving even
  though detection and autocorrect still match byte-for-byte.
