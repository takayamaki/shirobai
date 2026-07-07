# Cop implementation status

This document tracks which RuboCop cops shirobai has reimplemented in Rust,
and which cops were attempted but reverted because they did not meet the
project's drop-in compatibility and speed requirements together.

## Implemented (93 cops)

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

### Style (22)

- `Style/ArgumentsForwarding`
- `Style/BlockDelimiters`
- `Style/ColonMethodCall`
- `Style/FileNull`
- `Style/FrozenStringLiteralComment`
- `Style/HashEachMethods`
- `Style/HashSyntax`
- `Style/HashTransformKeys`
- `Style/IfUnlessModifier`
- `Style/LineEndConcatenation`
- `Style/NestedParenthesizedCalls`
- `Style/PercentLiteralDelimiters`
- `Style/RedundantFreeze`
- `Style/RedundantSelf`
- `Style/RedundantSelfAssignment`
- `Style/Semicolon`
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

### Implemented (7 cops)

- `RSpec/LetSetup`
- `RSpec/MultipleMemoizedHelpers`
- `RSpec/NamedSubject`
- `RSpec/RepeatedDescription`
- `RSpec/RepeatedExample`
- `RSpec/VariableDefinition`
- `RSpec/VariableName`

Verified with the same bar as core cops: vendor specs from the
`vendor/rubocop-rspec` submodule (plus its shared contexts and smoke
tests), differential edge-case specs pinning the probed quirks (the
`inside_example_group?` top-level-statement gate, block/numblock/itblock
matcher asymmetries, `LetSetup`'s zero-argument-use search, sym/str
non-shadowing, Unicode style properties, AllowedPatterns +
detected-style bookkeeping, `VariableDefinition`'s style-filtered
autocorrect, `MultipleMemoizedHelpers`' cross-frame helper union with
structural dedup of interpolated names, `RepeatedDescription` /
`RepeatedExample`'s EG-only example collection with structural
signature grouping on real parser nodes), lint-mode correctable parity,
non-ASCII offset parity, and the rspec parity oracle
(`benches/parity_diff_rspec.sh` with its synthetic-fixture self-test) at
zero diff.

`RSpec/VariableDefinition` and `RSpec/MultipleMemoizedHelpers` reuse the
`RSpecDispatcherRule`: the former shares `VariableName`'s send-shaped
candidate and top-level-group gate (style filtering happens in Rust); the
latter unions each spec group's own helper frame with its parser-block
ancestor frames (adding a per-role `subjects` collection alongside
`lets`), counts the bytewise-decidable identities in Rust and hands the
rare `dsym`/`dstr` names to the wrapper, which relocates them through the
shared `Shirobai::RSpec::NodeLocator` and dedups them with parser-gem
structural equality.

`RSpec/RepeatedDescription` and `RSpec/RepeatedExample` share ONE more
`RSpecDispatcherRule` collection: for every `example_group?` frame (EG
only — not shared groups, not `include_*` blocks) the walk collects its
`examples` with the exact stock scope semantics (a plain-block example,
attributed innermost-outward and halting at the first `scope_change` or
`example` frame — a `let`/`subject` body is transparent, a numblock group
opens no frame), and puts the example BLOCK node ranges of every group
with >= 2 examples on the wire (both cops read identical data). Byte-level
signature comparison is impossible: stock groups examples by parser-node
STRUCTURAL equality (`it 'a'` == `it "a"`; a heredoc body != a same-text
string body), so the wrappers relocate the nodes via the shared
`NodeLocator`, wrap them in the stock `RuboCop::RSpec::Example`, and run
stock's grouping (`[metadata, doc_string]` / `[doc_string, example]` for
descriptions, `[metadata, implementation]` + `its` args for examples)
VERBATIM — parity by construction for the equality-sensitive part.

`RSpec/NamedSubject` reuses the same frames. A reference is stock's
hard-coded `subject_usage` search (`$(send nil? :subject)` — the literal
name `subject`, zero args, no block-pass; an alias or `subject!` never
counts). Rust reports it when a plain-block example/hook frame encloses it
(numblock/itblock examples never qualify, matching stock's `on_block`),
gated by the OUTERMOST example/hook-or-shared frame under
`IgnoreSharedExamples` (`shared_examples`/`shared_examples_for` only —
`shared_context` never suppresses), and, under `named_only`, by the
named-ness of the nearest enclosing `subject` definition. No autocorrect
(stock has none). Range-level offense dedup falls out of one offense per
reference node. It fires 3442 times on Mastodon spec/ — the densest RSpec
signal in the oracle — at zero diff.

One probed parser divergence is handled centrally:
an empty percent-string (`%()`) is a `str` node in prism 1.9 but a `dstr`
in the parser gem, so `string_is_parser_dstr` treats it as `dstr`
(matching every stock `{str dstr}` matcher).

### Known limitation (same family as the README `TargetRubyVersion` note)

RSpec matchers split on the parser block kind (`block` / `numblock` /
`itblock`), and shirobai recovers the split from prism Latest. A block
whose body uses a bare `it` (e.g. `let(:x) { it }`) is an it-block under
prism Latest and under stock with `TargetRubyVersion >= 3.4`, but a plain
`block` (with `it` as a method call) under stock with older targets, so
block-kind-sensitive matchers can disagree there. Real spec code calling
a bare `it` inside a block is essentially nonexistent (RSpec's own `it`
requires arguments or a block).

## Plugin cops: shirobai-rspec (R2 metadata family)

Six more rubocop-rspec cops through the same `RSpecDispatcherRule`.

### Implemented (6 cops)

- `RSpec/Focus`
- `RSpec/PendingWithoutReason`
- `RSpec/MetadataStyle`
- `RSpec/DuplicatedMetadata`
- `RSpec/EmptyMetadata`
- `RSpec/SortMetadata`

These cops' offense detection and (for five of them) autocorrect depend
heavily on parser-AST geometry — sibling/brace-aware pair removal,
metadata insertion relative to hashes, sorted-source replacement, and
selector renaming — that cannot be reproduced byte for byte off prism
offsets alone. So the split is deliberately thin on the Rust side: the
`RSpecDispatcherRule` classifies CANDIDATE node ranges on the shared walk
(a superset), and each wrapper relocates the parser node through the
shared `Shirobai::RSpec::NodeLocator` and runs STOCK's own `on_send` /
`Metadata#on_block` plus autocorrect VERBATIM on the real parser AST. The
wrapper sets `RuboCop::RSpec::Language.config` so the stock matchers
resolve, and renames the stock entry method so the Commissioner never
dispatches a per-node callback (only `on_new_investigation` runs). This is
parity by construction for the equality- and geometry-sensitive parts.

The four `Metadata`-mixin cops share ONE Rust metadata-anchor list (the
direct example/group/hook blocks plus `RSpec.configure` blocks, emitted
for every block kind so the wrapper's `(block ...)` matcher self-filters
parser block kind — this also handles the prism-itblock vs
parser-plain-block target divergence for free). `Focus` and
`PendingWithoutReason` each carry their own candidate send list. No new
wire nums/lists were needed: candidate selection is role-table-only, and
`MetadataStyle`'s `EnforcedStyle` is read by the wrapper.

Probed quirks pinned as differential edge specs: metadata fires on plain
blocks only (numblock never, itblock == plain at target < 3.4); `Focus`'s
non-correctable bare-alias case, the chained / inside-def guards, and
skipped-inside-focusable metadata; `PendingWithoutReason`'s block vs
in-example vs metadata forms and the "regular example without body does
not fire" case; the `RSpec.configure` hook metadata path. Verified with
vendor specs, lint-mode correctable parity, non-ASCII offset parity, a
manual `--only`-scoped `-A` byte comparison for `MetadataStyle` on 58 real
forem files (MATCH), and the rspec parity oracle at zero diff on
factory_bot / forem / discourse / mastodon (corpus positives:
`MetadataStyle` and `PendingWithoutReason` fire; `SortMetadata` once; the
other three have no corpus positives and rest on the synthetic specs).

## Plugin cops: shirobai-rspec (R2: empty-line family)

The first rspec cops with autocorrect. All five wrap rubocop-rspec's
shared `EmptyLineSeparation` mixin: they resolve a "concept" node (an
example / example group / final let / hook / subject) and flag it when it
has a following sibling in a `:begin` sequence and the line after its
`final_end_location` (heredoc-aware) is not blank; autocorrect inserts one
`"\n"`.

### Implemented (5 cops)

- `RSpec/EmptyLineAfterExample`
- `RSpec/EmptyLineAfterExampleGroup`
- `RSpec/EmptyLineAfterFinalLet`
- `RSpec/EmptyLineAfterHook`
- `RSpec/EmptyLineAfterSubject`

A SECOND shared rspec rule (`rspec_empty_line.rs`) runs alongside the R1
`RSpecDispatcherRule` on the same walk and produces all five cops' results
at once. It owns the parts that need the AST — concept classification
(reusing the R1 `RSpecConfig` role table), `last_child?` (the parser
`:begin` recovery), the one-liner allowances (`AllowConsecutiveOneLiners`
for Example / Hook), and the heredoc-aware `final_end_location.line` — and
emits, per cop, `[final_end_line, method_name]` for each surviving
candidate. The Ruby side (a shared `EmptyLineSeparationSupport` module)
replays the REST of the stock mixin — the trailing-comment walk, the
enabled-`# rubocop:enable` directive tracking, the blank-line
suppression, the offense location and the `"\n"` autocorrect — over the
same `ProcessedSource`, so those parts are byte-for-byte stock by
construction.

Probed quirks pinned as differential edge specs (offenses AND `-A`
byte-equal): a plain `begin a; b end` is `:kwbegin` (not `begin_type?`, no
offense) while `begin ... rescue`/`ensure` bodies and rescue/ensure clause
bodies ARE `:begin`; a non-directive trailing comment keeps the offense on
the node end line (blank before the comment) while an enabled directive
moves it onto the directive line (blank after); a comment (or a
trailing-comment code line) followed by a blank line suppresses the
offense; a heredoc spilling below a one-line brace concept fixes the
offense on the heredoc terminator line; `on_block`-only cops (all but Hook)
ignore numbered/`it`-param blocks while Hook fires on every block form; the
Subject `inside_example_group?` gate needs the outermost top-level
statement to be a spec group; and the final-let is the LAST `let?` (block
form or `let(:x, &blk)` send form) among the group body's direct children.

The `:begin` recovery is the subtle part: prism folds `begin`/`rescue`/
`ensure` and single-vs-multi bodies differently from parser, and the
dispatcher visits several statement sequences (the program root, a
BeginNode's main/ensure/else bodies, a RescueNode's body) FRAMELESS via
`visit_statements_node` directly, so the walk's current frame for a concept
in them is the enclosing Program / Begin / Rescue node, resolved shape by
shape.

Whether a second-layer trait would remove duplication between the five
cops' logic: the arena/post-walk pattern stayed sufficient WITHIN this
cluster (one rule, five result vectors, no per-cop copy). The only real
duplication is CROSS-cluster: this rule re-derives the `top_spec_depth`
top-level-spec-group gate and the block-kind / `rspec?` receiver helpers
that `RSpecDispatcherRule` already computes. A shared "rspec walk context"
trait exposing role classification + `top_spec_depth` + block-kind to
multiple rspec rules would remove that ~30-line overlap; the arena pattern
itself did not need abstracting.

## Plugin cops: shirobai-rspec (R3: example-group family)

Three more rubocop-rspec cops, all **Architecture B** (the metadata-family
pattern): the `RSpecDispatcherRule` emits candidate block ranges on the
shared walk, and each wrapper relocates the parser node through
`Shirobai::RSpec::NodeLocator` and runs stock's detection + autocorrect
VERBATIM. All three depend on parser-AST machinery that cannot be
reproduced bytewise.

### Implemented (3 cops)

- `RSpec/EmptyExampleGroup`
- `RSpec/DescribedClass`
- `RSpec/ScatteredSetup`

`RSpec/EmptyExampleGroup` (slot `[2, 18]`) and `RSpec/ScatteredSetup`
(slot `[2, 20]`) share ONE candidate set — every `example_group?` frame
range, computed once on the walk and cloned into both slots (the same
precedent as repeated_description / repeated_example). EmptyExampleGroup's
wrapper runs stock's mutually recursive `examples?` node-pattern grammar
(examples in branches, inside non-hook blocks, across conditionals) plus
the `each_ancestor(:any_def)` and `inside_example?` guards; autocorrect
removes the group by whole lines. ScatteredSetup's wrapper enumerates
hooks through stock's `RuboCop::RSpec::ExampleGroup` wrapper, filters by
`inside_class_method?` / `knowable_scope?` / non-`around`, groups by
`[name, scope, metadata]` via `RepeatedItems`, and replays the body-merge
autocorrect with the heredoc-aware `final_end_location`. The
first-occurrence-not-correctable quirk (stock's autocorrect early-return)
is pinned in the correctable parity spec.

`RSpec/DescribedClass` (slot `[2, 19]`) gets its own narrower candidate
set: blocks whose send is an example-group call with a const first
argument (`describe(Const)`). The wrapper carries stock's full config
surface — `EnforcedStyle` (`described_class` / `explicit`), `SkipBlocks`,
`OnlyStaticConstants` — and runs stock's `Namespace` mixin,
`collapse_namespace` const resolution, and the recursive `find_usage`
descent with scope-change stops (`def` / `class` / `module`, the
`Class.new` / `class_eval` closure family, and non-RSpec blocks under
`SkipBlocks`) unchanged on the parser AST: parity by construction for the
namespace-sensitive part. This is the densest-firing rubocop-rspec cop on
real corpora.

A speed lesson surfaced right after this cluster merged: dense Arch-B
candidate sets make the per-file `NodeLocator.locate` cost visible on
spec-heavy corpora (Discourse), which is what motivated the two-phase
locate (pruned descent first, heredoc-sound full-descent fallback only
for targets the prune misses — see `lib/shirobai/node_locator.rb`).
When scoping future Arch-B cops, include relocation frequency (candidate
density x corpus spec size) in the speed estimate, not just the stock
cop's standalone cost.

## Plugin cops: shirobai-rails (Phase 0 + Application* cluster)

`gems/shirobai-rails` replaces rubocop-rails (pinned `= 2.35.5`) cops with
Rust rules in the shared shirobai-core extension (no second native build).
This batch stands up the plumbing for a NEW plugin origin (origin 3) plus a
proof cluster of four trivial cops.

The rails origin differs from the rspec origin in one structural way: it has
**no per-file gate**. rubocop-rails cops run on every Ruby file (no
department Include like RSpec's `**/*_spec.rb`), so the origin is always
awake once the gem is loaded. The design constraint that follows: candidate
classification on the Rust side must stay cheap (table-driven const-name
checks riding the existing shared walk, no extra AST pass), because there is
no gate to hide the cost behind. `RailsAppVisitor` subscribes only
`ENTER_CLASS_MOD | ENTER_CALL | ENTER_WRITE` and needs no ancestor stack:
the `Class.new(Base)` exemption is pre-marked from the `SUPERCLASS =`
constant-write side, so no `LEAVE` bookkeeping is paid.

### Implemented (4 cops)

- `Rails/ApplicationRecord`
- `Rails/ApplicationController`
- `Rails/ApplicationMailer`
- `Rails/ApplicationJob`

All four share stock's `EnforceSuperclass` machinery: `class X <
RECV::Base` (unless `X`'s terminal name is the enforced superclass) and
`Class.new(RECV::Base)` (unless it is the direct value of a `SUPERCLASS =`
constant write, covering the `do..end` / `{}` block forms). Offense and
autocorrect range = the base const node (leading `::` included), replaced
with the bare superclass name.

`TargetRailsVersion`: Record / Mailer / Job carry stock's
`requires_gem('railties', '>= 5.0')` gate, replicated on the wrappers via
`extend RuboCop::Cop::TargetRailsVersion` + `minimum_target_rails_version
5.0` (the wrappers do not subclass the stock cop, so the gem-requirement
metadata does not come for free). Controller has no gate. The
`Rails/ApplicationRecord` `Exclude: db/**/*.rb` is resolved by RuboCop
through each wrapper's badge, exactly as for the stock cop.

Verified with the same bar as core cops: vendor specs from the
`vendor/rubocop-rails` submodule (`= 2.35.5`, plus its `:rails*` version
shared contexts), differential edge-case specs for every probed quirk
(cbase base, scope-insensitive name exemption, the Class.new exemption /
block / cbase-casgn / namespaced-casgn / cross-cop cases, arity gating,
CRLF fallback), lint-mode correctable parity, non-ASCII offset parity, and
the plugin parity oracle (`benches/parity_diff_rails.sh`) at zero diff on
Mastodon / Redmine / Discourse (rails-dense) and fluentd (no-rails
non-interference).

The rails oracle deviates from the rspec oracle in one place:
`DisabledByDefault: true` (rspec uses `false`). Rails cops share files with
core cops, so enabling every department would fold core-cop parity — owned
by `benches/parity_diff.sh` — into this oracle as noise. Scoping to the
Rails department keeps the signal clean; the ~134 non-replaced rubocop-rails
cops still run as stock on both sides, so only the four replaced cops are
under test.

### Implemented (2 cops, Architecture B)

- `Rails/HttpPositionalArguments`
- `Rails/DeprecatedActiveModelErrorsMethods`

Both are **Architecture B** (the rspec metadata-family pattern): the Rust
prefilter emits candidate SEND ranges on the same `RailsAppVisitor` walk (no
new rule, no new interest flags — it rides `ENTER_CALL`), and the wrapper
relocates the parser send node with `Shirobai::NodeLocator` and runs stock's
`on_send` + autocorrect VERBATIM. This is the right split because both cops
are source-reconstruction heavy (full-node rebuild joining hash pairs with
`, `; receiver-walk offense ranges; `.source` reads) and carry
file-path / target-version heuristics — reproducing that byte-for-byte in
Rust is far riskier than running stock's own Ruby on the located node. The
candidate sets are narrow: bare HTTP-verb sends with >= 2 args (a block-pass
counts, as in the parser AST), and the five errors-chain shapes
(`errors[...]` / `errors.{messages,details}[...]` manipulation & assignment,
`errors.{keys,values,to_h,to_xml}`).

`Rails/HttpPositionalArguments` carries stock's `requires_gem('railties', '>=
5.0')` gate (Rails >= 5), replicated on the wrapper like the Application*
cops; `Include: **/spec/**, **/test/**` resolves through the wrapper badge.
`Rails/DeprecatedActiveModelErrorsMethods` has no gem gate but reads
`target_rails_version` in `on_send` (Rails <= 6.0 exempts
`keys`/`values`/`to_h`/`to_xml`) and `model_file?` (`/models/` allows a nil
receiver) — both run in the wrapper on the parser AST, unchanged from stock.

Verified with the same bar: vendor specs from `vendor/rubocop-rails` (156
examples across both cops), differential edge-case specs for every probed
quirk (session-arg conversion, parentheses, multiline hash join, routing
block / rack-test / kwsplat guards, `keys`-node-not-`include?` offense,
version gate, uncorrectable `details <<` and `[]=`, a heredoc-interpolated
`errors.to_h`, non-ASCII, CRLF fallback), lint-mode correctable parity,
non-ASCII offset parity, and the parity oracle at zero diff on Mastodon /
Redmine / Discourse (`app` and `spec` targets), plus a direct `--only ... -A`
byte comparison. The self-test fixture fires both cops on the stock side.

One `NodeLocator` subtlety surfaced on Discourse: the range-containment prune
the rspec `NodeLocator` uses is UNSOUND for heredocs — a send in a heredoc
body sits outside the expression range of every ancestor up to the root, so
the prune drops it. The core `Shirobai::NodeLocator` (added here, the
plugin-neutral twin) was born as a full descent with an all-found early
exit; after the rspec R3 cluster made the per-file locate cost visible on
Discourse, both twins became **two-phase** — the fast pruned descent runs
first, and the full descent runs only for targets the prune missed (the
heredoc-interior case) — keeping the soundness at pruned-descent cost.

### Wire layout (for the sibling tracks building on this branch)

Rails is origin 3 (`ORIGIN_RAILS` / `N_ORIGINS = 4`). Slots after the
integration of the two 2026-07 clusters: `rails_application_record [3,0]`,
`..._controller [3,1]`, `..._mailer [3,2]`, `..._job [3,3]` (final offense
ranges); `rails_unknown_env [3,4]`, `rails_dynamic_find_by [3,5]`
(send/block-table cluster, see its section below);
`rails_http_positional_arguments [3,6]`,
`rails_deprecated_active_model_errors_methods [3,7]` (Architecture-B
candidate ranges); `rails_pluck [3,8]` (full-Rust offense tuples, see its
section below). The two Architecture-B cops add NOTHING to the segment —
their gating lives in the wrappers — so the segment shape is exactly the
send/block-table cluster's (`nums = [rails_enabled,
unknown_env_supports_local]`, four lists; see "Wire layout delta" below).
Future clusters extend the segment by appending nums / lists in
`rails_config.rs`, never reordering.

## Plugin cops: shirobai-rails (send/block-table cluster)

Two more rubocop-rails cops, both **full-Rust** (byte-computable detection and
autocorrect, no parser-AST relocation needed):

- `Rails/DynamicFindBy`
- `Rails/UnknownEnv`

`Rails/DynamicFindBy` is its own rule with a class-inheritance ancestor stack
(ENTER_ALL + LEAVE, one frame per branch node so the stack stays 1:1 with
`leave`; a counter of ActiveRecord-inheriting class ancestors drives the
receiverless-finder case). It replicates the `/^find_by_(.+?)(!)?$/` name match
(lazy group 1, so a lone `find_by_!` captures `!` as the column and stays
`find_by`), the argument-count / no-splat / no-hash gate (a block-pass `&blk`
counts as a virtual trailing argument, per the trap table), and the
`AllowedMethods` / `AllowedReceivers` (the receiver's raw source) / deprecated
`Whitelist` suppressions. Autocorrect replaces the selector and inserts each
`col: ` keyword before its argument — all in Rust; the wrapper owns only the
`MSG` and derives the method name from the selector range.

`Rails/UnknownEnv` is its own rule (ENTER_CALL + ENTER_OTHER for the `case`
form, no stack). Rust detects the predicate / comparison (both operand orders)
/ `case` shapes off `rails_env?` and emits `[start, end, name]`. The message —
including the `DidYouMean` spell suggestion, which cannot and must not be
reproduced in Rust — is built Ruby-side from the wrapper's own `Environments`
config, so the suggestion text is stock by construction. The `supports_local`
asymmetry (`local` is known for the predicate form only, and only on Rails
>= 7.1) is packed into the segment; the comparison and `case` forms never
allow `local`, matching stock. No autocorrect (stock has none).

Architecture choice: full-Rust for both. `UnknownEnv` needs no node relocation
(offense ranges are byte-computable and the only Ruby-side machinery,
DidYouMean, is fed a plain name string). `DynamicFindBy`'s autocorrect is a
selector replace plus keyword inserts at known byte offsets, so Architecture B
would add cost without buying parity.

Verified with the same bar as core cops: vendor specs from the
`vendor/rubocop-rails` submodule (including the `:rails71` / `:ruby27` version
contexts and the `DidYouMean`-available branch), differential edge-case specs
for every probed quirk (csend; the nested-class `each_ancestor(:class).any?`
case; the block-pass virtual argument; the lone-bang column; multiline keyword
inserts; CRLF fallback; cbase `::Rails.env`; both comparison operand orders;
multi-condition `case`; non-string `when`), lint-mode correctable parity,
non-ASCII offset parity (DynamicFindBy through the full autocorrect, UnknownEnv
on offense offsets), and the rails parity oracle at zero diff on Mastodon /
Redmine / Discourse / fluentd. The oracle self-test fixture was extended so
both cops fire on the stock side first (the hollow-zero-diff guard). Corpus
positives: `DynamicFindBy` fires 81 times on Redmine and 94 on Discourse (a
whole-tree `--only Rails/DynamicFindBy -A` autocorrect on Redmine is
byte-identical to stock); `UnknownEnv` has no corpus positives (env-name typos
are rare in mature code) and rests on the vendor + edge + self-test coverage.

### Wire layout delta

The rails segment grew (append only): `nums = [rails_enabled,
unknown_env_supports_local]`, `lists = [unknown_env_environments,
dynamic_find_by_allowed_methods, dynamic_find_by_allowed_receivers,
dynamic_find_by_whitelist]`. Dormant segment `[[0, 0], [[], [], [], []]]`. New
slots `rails_unknown_env [3,4]`, `rails_dynamic_find_by [3,5]`. Each cop's
`bundle_args` is the single source of its own config; `Shirobai::Rails.segment`
assembles the pieces.

## Plugin cops: shirobai-rails (Pluck)

- `Rails/Pluck` (slot `[3, 8]`) — **full-Rust** detection and autocorrect.
  Deferred out of the send/block-table cluster, then shipped in its own
  probe-and-verify cycle as predicted. The rule handles all three prism
  block kinds (`block` / numbered `_1` / `it`), the
  ancestor-block-with-receiver guard (stock's N+1-iteration suppression:
  `each_ancestor(:any_block).first&.receiver`), the regexp-key exclusion,
  and the block-arg-in-key false-positive suppression
  (`x.map { |x| x[x] }` never fires). Autocorrect replaces
  selector-through-block-end with `pluck(<key source>)`; the wrapper owns
  only the `TargetRailsVersion >= 5.0` gate and the offense message.
  Verified with the standard bar: vendor specs, differential edge specs
  (numblock, itblock, ancestor guard, arg shadowing, regexp key, csend,
  `collect`, string / method-call keys, CRLF fallback), lint-mode
  correctable parity, non-ASCII offset parity through the autocorrect,
  and the rails parity oracle.

### Deferred (same branch family, not in this cluster)

`Rails/IndexBy` and `Rails/IndexWith` were scoped into the send/block-table
cluster but deferred rather than rushed, to keep the shipped cluster
byte-clean:

- `Rails/IndexBy` / `Rails/IndexWith` share the `IndexMethod` mixin whose
  autocorrect is heavy parser-AST geometry (strip prefix/suffix, rename method,
  rewrite block args, replace body) plus cross-offense `ignore_node` state —
  the Architecture B relocate-and-dispatch pattern (as in the rspec metadata
  family), a larger harness than this cluster.

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
