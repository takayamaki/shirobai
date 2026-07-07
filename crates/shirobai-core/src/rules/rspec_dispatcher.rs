//! The RSpec dispatcher rule: ONE `Rule` on the shared walk that classifies
//! every node against `RSpec/Language` once and feeds all RSpec cops.
//!
//! Stock rubocop-rspec has no shared context: every cop re-answers "is this
//! an example group / example / let?" per node (role-list membership) and
//! re-walks ancestors or subtrees per structural question (`TopLevelGroup`,
//! `InsideExampleGroup`, `Concept#find_all_in_scope`). Measured on Mastodon
//! spec/, that re-classification is the department's dominant cost. This
//! rule keeps the classification and the example-group scope tree as walk
//! state, computed once per file, and every RSpec cop reads from it.
//!
//! # The scope tree (stock `find_all_in_scope` semantics)
//!
//! Stock collects role nodes (`lets` / `subjects` / `examples`) from a query
//! root by descending children and halting at "barriers": a scope change
//! (an example-group/shared-group block with an rspec receiver, or an
//! `include_*` block), an example block, or a node matching the queried
//! role itself. The query root is exempt from its own barrier.
//!
//! All query roots and all barriers are parser `block`-type nodes (every
//! stock pattern involved is `(block ...)`, not `any_block`), so `numblock`
//! and `itblock` nodes are TRANSPARENT: a `let` inside `context('x') { _1 }`
//! belongs to the enclosing group. prism folds all three into `BlockNode`;
//! the parser three-way split is recovered from the block parameters
//! (regular/none = `block`, `NumberedParametersNode` = `numblock`,
//! `ItParametersNode` = `itblock`) — the same recovery `duplicate_methods`
//! uses.
//!
//! This walk reproduces the collection in one pass: every parser-block node
//! opens a scope frame; a collected item is attributed to every open frame
//! from the innermost outward until (and including) the first frame that is
//! a barrier for that role. "Including" mirrors the root-exemption: a query
//! rooted AT a barrier node still sees its own subtree.
//!
//! Because stock cops fire `on_block` at node ENTER but read collections of
//! the COMPLETE subtree (the AST is fully built), per-group answers must not
//! be finalized mid-walk — a `let` after a nested `context` still counts for
//! that context's `MultipleMemoizedHelpers` total. The frames therefore live
//! in an arena that survives the walk and cop verdicts run in `finish()`.
//!
//! # Per-cop notes (probed against stock rubocop-rspec 3.10.2)
//!
//! `RSpec/VariableName` / (`RSpec/VariableDefinition`): the candidate
//! matcher is SEND-shaped (`(send nil? {Subjects|Helpers} $lit ...)`), so it
//! fires for any block kind and for bare sends. `inside_example_group?` is
//! NOT "some ancestor is a group": it finds the node's outermost enclosing
//! statement (the root node, or a child of a root-level `begin`) and asks
//! whether THAT is a `spec_group?` (an `any_block` matcher — a numblock
//! group counts). A group wrapped in a top-level `class`/`module` therefore
//! does NOT count (probed: A2/A8), while a `class` INSIDE a top-level group
//! does (A3). Tracked here as `top_spec_depth`.
//!
//! `RSpec/LetSetup`: roots are scope-change frames (groups with rspec
//! receiver + `include_*` blocks). A candidate is a collected `let` whose
//! literal name is exactly `let!` with one plain sym/str argument (block
//! form) or `(name, &block_pass)` (send form). "Used" means a receiverless
//! ZERO-argument send with the same name anywhere in the root's subtree —
//! stock's `(send nil? %)` pattern has no argument wildcard, so `w(1)` and
//! `w(&b)` do NOT count as uses (probed: F3/F12) while `w { }` does (F9).
//! An inner `let!` overridden check compares (kind, value): `let!('w')`
//! and `let!(:w)` do not shadow each other (F6).
//!
//! `RSpec/VariableDefinition`: shares the SEND-shaped candidate matcher and
//! the top-level-spec-group gate with `RSpec/VariableName`. The `EnforcedStyle`
//! decides which literal names are offenses: `symbols` (default) flags plain
//! `str` names only; `strings` flags `sym` AND `dsym` names. `dstr`
//! (plain-string-interpolation) names are NEVER flagged under either style
//! (probed). The offense range is the first-argument literal node. The wrapper
//! autocorrects from the wire tuple `(start, end, kind, value)` (kind 0 sym /
//! 1 str / 2 dsym): a sym becomes `value.inspect`, a str becomes
//! `value.to_sym.inspect`, and a dsym becomes the offense source minus the
//! leading colon (`variable.source[1..]`). Rust filters by style so the wire
//! carries only real offenders.
//!
//! `RSpec/MultipleMemoizedHelpers`: gates on PLAIN-block spec groups (EG/SG
//! with an rspec receiver; a numblock group never gets an offense because
//! stock's `on_block` has no numblock handler — probed). The count for a
//! group G is `all_helpers(G).uniq.count`, where `all_helpers` unions the
//! helpers collected in G's own frame with those of every parser-block
//! ancestor frame (the arena parent chain), then maps each collected `let`
//! (plus `subject` when `AllowSubject: false`) to its
//! `variable_definition?` first-argument literal node — or `nil` when the
//! call has no literal sym/str/dsym/dstr first argument — and `uniq`s by
//! Ruby structural node equality. Two `dstr`/`dsym` names differing only in
//! interpolation whitespace are structurally EQUAL (E11), so they cannot be
//! deduplicated bytewise. Rust therefore counts only the identities that are
//! decidable bytewise (sym value / str value / a single nil bucket) as
//! `rust_distinct` and passes the source ranges of the `dsym`/`dstr` items to
//! the wrapper, which locates them in the parser AST, dedups them with node
//! equality and computes `count = rust_distinct + located_uniq`. Rust emits a
//! group only when `rust_distinct + dsym_dstr_item_count > Max` (a safe upper
//! bound: below it no offense is possible); the wrapper does the exact check
//! and calls `self.max = count` (`exclude_limit 'Max'`) on a real offense.
//! The `subjects` collection mirrors `lets` but with the subject-role barrier
//! (a `let` frame does not stop the subjects query and vice versa).
//!
//! `RSpec/RepeatedDescription` / `RSpec/RepeatedExample`: both gate on an
//! `example_group?` — a PLAIN-block frame with an rspec receiver and a name in
//! `ExampleGroups.all` (EG ONLY, not shared groups, not `include_*` blocks;
//! probed). For every such group the walk collects its `examples`
//! (`find_all_in_scope(node, :example?)`): a PLAIN-block call with nil receiver
//! and a name in `Examples.all`, attributed innermost-outward through the open
//! frames and stopping at (and including) the first `scope_change` OR `example`
//! frame — a `let?`/`subject?` frame is NOT a barrier, so an example inside a
//! `let` body belongs to the group (probed). Numblock/itblock groups never open
//! a frame, so they are transparent: an example inside `context('y') { _1; ...}`
//! belongs to the outer describe (probed: offenses at both nesting levels). Rust
//! puts the example BLOCK node ranges of every group with >= 2 examples on the
//! wire (document order); the wrappers relocate the parser nodes, wrap them in
//! the stock `RuboCop::RSpec::Example`, and run stock's structural grouping
//! (`[metadata, doc_string]` / `[doc_string, example]` for descriptions,
//! `[metadata, implementation]` + `its` arguments for examples) VERBATIM — the
//! equality-sensitive comparison stays on real parser nodes, so it cannot be
//! decided bytewise here. Both cops read identical group data.

use std::collections::{HashMap, HashSet};

use ruby_prism::{CallNode, Node, StringNode};

use super::dispatch::{Interest, Rule};
use super::rspec_language::{roles, RSpecConfig};

/// One style-failing `RSpec/VariableName` candidate. `kind`: 0 = symbol,
/// 1 = string (dstr/dsym candidates are skipped by stock). `value` is the
/// unescaped literal value (what stock matches the style regexp against).
/// `valid_alt` is true when the value satisfies the OTHER supported style
/// (drives `unexpected_style_detected` vs `unrecognized_style_detected`).
#[derive(Debug, Clone, PartialEq)]
pub struct VarNameOffense {
    pub start: usize,
    pub end: usize,
    pub kind: u8,
    pub value: String,
    pub valid_alt: bool,
}

/// One `RSpec/VariableDefinition` offense. `kind`: 0 = sym, 1 = str,
/// 2 = dsym (the styles that stock flags — `dstr` never fails). `value` is
/// the unescaped literal value for sym/str (the wrapper runs
/// `value.inspect` / `value.to_sym.inspect` on it) and empty for dsym (the
/// wrapper slices the offense source via the range instead).
#[derive(Debug, Clone, PartialEq)]
pub struct VarDefOffense {
    pub start: usize,
    pub end: usize,
    pub kind: u8,
    pub value: String,
}

/// One `RSpec/MultipleMemoizedHelpers` candidate group over threshold. Rust
/// gates on the upper bound `rust_distinct + dsym_dstr_ranges.len() > Max`;
/// the wrapper locates `dsym_dstr_ranges` in the parser AST, dedups them and
/// applies the exact `count > Max` test.
#[derive(Debug, Clone, PartialEq)]
pub struct MmhGroup {
    pub start: usize,
    pub end: usize,
    /// Distinct sym-value / str-value / nil identities among the visible
    /// helpers (the identities decidable bytewise).
    pub rust_distinct: usize,
    /// Source ranges of the visible helpers whose name is a `dsym`/`dstr`
    /// literal (one per unique item; the wrapper dedups structurally).
    pub dsym_dstr_ranges: Vec<(usize, usize)>,
}

/// A memoized-helper variable identity: the node `variable_definition?`
/// captures as the first argument, or `Nil` when the call has no literal
/// sym/str/dsym/dstr first argument.
#[derive(Debug, Clone, PartialEq, Eq)]
enum VarDef {
    /// `sym` name — the unescaped value (identity is value equality).
    Sym(Vec<u8>),
    /// `str` name — the unescaped value.
    Str(Vec<u8>),
    /// `dsym`/`dstr` name — the source range (structural equality is decided
    /// in the wrapper, not here).
    Dyn(usize, usize),
    /// No literal name (`let(foo)`, `subject { }`, ...). All nils are one
    /// identity.
    Nil,
}

/// Everything the RSpec cops produced for one file.
#[derive(Debug, Default)]
pub struct RSpecResult {
    /// (style-failing candidates, style-passing (value, kind) pairs).
    /// The passing list feeds the wrapper's `correct_style_detected`
    /// bookkeeping, filtered by AllowedPatterns on the Ruby side.
    pub variable_name: (Vec<VarNameOffense>, Vec<(String, u8)>),
    /// `RSpec/LetSetup` offenses: the `let!` send node ranges.
    pub let_setup: Vec<(usize, usize)>,
    /// `RSpec/VariableDefinition` offenses (already style-filtered).
    pub variable_definition: Vec<VarDefOffense>,
    /// `RSpec/MultipleMemoizedHelpers` over-threshold candidate groups.
    pub multiple_memoized_helpers: Vec<MmhGroup>,
    /// `RSpec/RepeatedDescription`: per example group with >= 2 collected
    /// examples, the example BLOCK node ranges in document order. The wrapper
    /// runs stock's `[metadata, doc_string]` / `[doc_string, example]`
    /// grouping over the located parser nodes.
    pub repeated_description: Vec<Vec<(usize, usize)>>,
    /// `RSpec/RepeatedExample`: the SAME group data as `repeated_description`
    /// (each cop owns its slot; the collection is computed once). The wrapper
    /// runs stock's `[metadata, implementation]` (+ `its` args) grouping.
    pub repeated_example: Vec<Vec<(usize, usize)>>,
    /// `RSpec/NamedSubject`: the selector ranges of bare `subject` references
    /// that need an explicit name (already style/shared-filtered).
    pub named_subject: Vec<(usize, usize)>,
    /// Shared metadata-anchor block ranges (prism call range == parser block
    /// node range), document order. One list feeds all four `Metadata`-mixin
    /// cops (`MetadataStyle` / `DuplicatedMetadata` / `EmptyMetadata` /
    /// `SortMetadata`): each relocates the parser block node and runs stock's
    /// `Metadata#on_block` verbatim (block-kind / receiver / arity self-filter
    /// there). Superset of the direct `rspec_metadata` blocks plus the
    /// `RSpec.configure` blocks (whose inner hook sends the wrapper's
    /// `metadata_in_block` search finds).
    pub metadata_anchors: Vec<(usize, usize)>,
    /// `RSpec/Focus` candidate SEND ranges (parser send range), document order.
    /// The wrapper relocates each parser send node and runs stock's `on_send`
    /// verbatim (the chained / inside-def guards and the exact `focused_block?`
    /// / `metadata` matchers self-filter there).
    pub focus: Vec<(usize, usize)>,
    /// `RSpec/PendingWithoutReason` candidate SEND ranges (parser send range),
    /// document order. The wrapper relocates each parser send node and runs
    /// stock's `on_send` verbatim (parent-relationship logic self-filters there).
    pub pending_without_reason: Vec<(usize, usize)>,
    /// `RSpec/EmptyExampleGroup` candidate example-group BLOCK ranges (prism
    /// call range == parser block node range), document order. Every
    /// plain-block `example_group?` frame (rspec receiver + ExampleGroups.all
    /// name) is emitted. The wrapper relocates each parser block node and
    /// runs stock's `on_block` detection verbatim (the mutually recursive
    /// `examples?` matcher, `offensive?` check, and
    /// `each_ancestor(:any_def)` / `inside_example?` guards self-filter).
    pub empty_example_group: Vec<(usize, usize)>,
    /// `RSpec/DescribedClass` candidate block ranges: `describe(Const)` blocks
    /// whose send is an `example_group?` call with a const as the first
    /// argument. The wrapper relocates the parser block node and runs stock's
    /// full detection + autocorrect verbatim.
    pub described_class: Vec<(usize, usize)>,
}

/// parser block-kind recovery from a prism call (see module doc).
#[derive(Clone, Copy, PartialEq)]
enum BlockKind {
    None,
    BlockArg,
    Plain,
    Numbered,
    It,
}

fn block_kind(call: &CallNode<'_>) -> BlockKind {
    match call.block() {
        None => BlockKind::None,
        Some(b) => match b.as_block_node() {
            Some(bn) => match bn.parameters() {
                Some(p) if p.as_numbered_parameters_node().is_some() => BlockKind::Numbered,
                Some(p) if p.as_it_parameters_node().is_some() => BlockKind::It,
                _ => BlockKind::Plain,
            },
            None => BlockKind::BlockArg,
        },
    }
}

/// `#rspec?` receiver: nil, `RSpec` or `::RSpec` (`(const {nil? cbase}
/// :RSpec)` — `A::RSpec` does not qualify).
fn rspec_const(recv: &Node<'_>) -> bool {
    if let Some(c) = recv.as_constant_read_node() {
        return c.name().as_slice() == b"RSpec";
    }
    if let Some(p) = recv.as_constant_path_node() {
        return p.parent().is_none()
            && p.name().is_some_and(|n| n.as_slice() == b"RSpec");
    }
    false
}

/// Stock `ConfigurableNaming::FORMATS[:snake_case]`:
/// `/^@{0,2}[\d[[:lower:]]_]+[!?=]?$/`. `[[:lower:]]` is the Unicode
/// Lowercase property (Ruby Onigmo and Rust `char::is_lowercase` both
/// derive it from DerivedCoreProperties); `\d` is ASCII-only in Ruby.
fn snake_case(name: &str) -> bool {
    let body = strip_affixes(name);
    !body.is_empty()
        && body
            .chars()
            .all(|c| c.is_ascii_digit() || c == '_' || c.is_lowercase())
}

/// Stock `ConfigurableNaming::FORMATS[:camelCase]`:
/// `/^@{0,2}(?:_|_?[[[:lower:]]][\d[[:lower:]][[:upper:]]]*)[!?=]?$/`.
fn camel_case(name: &str) -> bool {
    let body = strip_affixes(name);
    if body == "_" {
        return true;
    }
    let body = body.strip_prefix('_').unwrap_or(body);
    let mut chars = body.chars();
    match chars.next() {
        Some(first) if first.is_lowercase() => chars
            .all(|c| c.is_ascii_digit() || c.is_lowercase() || c.is_uppercase()),
        _ => false,
    }
}

/// Strip `@{0,2}` and one optional trailing `[!?=]` (the regexp suffix; the
/// character class cannot contain them so a trailing one is always the
/// suffix).
fn strip_affixes(name: &str) -> &str {
    let mut body = name;
    for _ in 0..2 {
        if let Some(rest) = body.strip_prefix('@') {
            body = rest;
        }
    }
    if body.ends_with(['!', '?', '=']) {
        body = &body[..body.len() - 1];
    }
    body
}

fn matches_style(value: &str, style: u8) -> bool {
    if style == 1 {
        camel_case(value)
    } else {
        snake_case(value)
    }
}

/// A collected `let?` node (block form or `&block_pass` send form).
struct LetItem {
    /// parser SEND node range (the `LetSetup` offense range: excludes a
    /// literal block, includes a `&block_pass` argument).
    send_range: (usize, usize),
    /// `let_bang` candidate name: (kind 0 sym / 1 str, unescaped value).
    /// None when the item is not literally `let!` with one plain sym/str
    /// name (extra args, dsym/dstr names and aliases never match stock's
    /// hard-coded `let_bang` pattern).
    bang_name: Option<(u8, Vec<u8>)>,
    /// `variable_definition?` identity for `MultipleMemoizedHelpers`.
    var_def: VarDef,
}

/// One parser-block frame in the (post-walk) arena.
struct Scope {
    parent: Option<usize>,
    /// Full node range (prism call range == parser block node range).
    range: (usize, usize),
    /// `(block {(send #rspec? {SG|EG} ...) (send nil? #Includes.all ...)})`
    /// — halts every role collection; also the `LetSetup` query-root set.
    scope_change: bool,
    /// `spec_group?` — a plain-block EG/SG with an rspec receiver: the
    /// `MultipleMemoizedHelpers` candidate set (a subset of `scope_change`,
    /// excluding `include_*` blocks).
    spec_group: bool,
    /// `example_group?` — a plain-block EG (rspec receiver + `ExampleGroups.all`
    /// name) ONLY, excluding shared groups: the `RepeatedDescription` /
    /// `RepeatedExample` candidate set (a subset of `spec_group`).
    example_group: bool,
    /// `example?` — halts every role collection.
    example: bool,
    /// `example_or_hook_block?` — a plain-block call with nil receiver and a
    /// name in `Examples.all ∪ Hooks.all`: the `RSpec/NamedSubject` block gate
    /// (stock's `on_block` fires only for plain blocks, so numblock/itblock
    /// examples never qualify).
    example_or_hook: bool,
    /// `shared_example?` — a plain-block call with an rspec receiver and a name
    /// in `SharedGroups.examples` ONLY (`shared_context` is `SharedGroups`
    /// context and does NOT count): the `IgnoreSharedExamples` suppressor.
    shared_examples: bool,
    /// `find_subject(block)` for `RSpec/NamedSubject`'s `named_only` style: the
    /// named-ness of the FIRST direct-child statement of this block's body that
    /// is a `subject?` definition (`Some(true)` named `subject(:x)`, `Some(false)`
    /// unnamed `subject`), or `None` when the body has no such direct child.
    subject_def_named: Option<bool>,
    /// The frame is itself a `let?` node — halts the lets collection only.
    let_barrier: bool,
    /// The frame is itself a `subject?` node — halts the subjects collection
    /// only.
    subject_barrier: bool,
    /// `RSpec/DescribedClass` candidate: an `example_group?` block whose first
    /// argument is a constant (`ConstantReadNode` or `ConstantPathNode`).
    described_class_candidate: bool,
    /// Collected `let?` items (indexes into `let_items`), document order.
    lets: Vec<u32>,
    /// Collected `subject?` items (indexes into `subject_items`), document
    /// order.
    subjects: Vec<u32>,
    /// Collected `example?` items (indexes into `example_items`), document
    /// order.
    examples: Vec<u32>,
}

struct StackEntry {
    opened_scope: bool,
    top_spec: bool,
}

pub struct RSpecDispatcherRule<'c> {
    cfg: &'c RSpecConfig,
    /// One entry per branch node currently open (aligned enter/leave).
    stack: Vec<StackEntry>,
    /// Arena indexes of the currently open parser-block frames.
    scope_stack: Vec<usize>,
    /// All parser-block frames, document order; survives the walk.
    scopes: Vec<Scope>,
    let_items: Vec<LetItem>,
    /// Collected `subject?` items' `variable_definition?` identities
    /// (indexed by `Scope::subjects`).
    subject_items: Vec<VarDef>,
    /// Collected `example?` items' BLOCK node ranges (prism call range ==
    /// parser block node range), indexed by `Scope::examples`.
    example_items: Vec<(usize, usize)>,
    /// Positive while inside a top-level statement that is `spec_group?`
    /// (any_block; see module doc on `inside_example_group?`).
    top_spec_depth: u32,
    /// Receiverless zero-argument non-block-pass call sites:
    /// name -> start offsets (walk order = ascending).
    zero_arg_calls: HashMap<Vec<u8>, Vec<usize>>,
    vn_offenses: Vec<VarNameOffense>,
    vn_passing: Vec<(String, u8)>,
    vd_offenses: Vec<VarDefOffense>,
    /// `RSpec/NamedSubject` offenses: the `subject` selector ranges, decided at
    /// reference time against the open frame stack.
    ns_offenses: Vec<(usize, usize)>,
    /// Shared metadata-anchor block ranges (see [`RSpecResult::metadata_anchors`]).
    metadata_anchors: Vec<(usize, usize)>,
    /// `RSpec/Focus` candidate send ranges.
    focus_candidates: Vec<(usize, usize)>,
    /// `RSpec/PendingWithoutReason` candidate send ranges.
    pending_candidates: Vec<(usize, usize)>,
}

pub fn build_rule<'c>(cfg: &'c RSpecConfig) -> RSpecDispatcherRule<'c> {
    RSpecDispatcherRule {
        cfg,
        stack: Vec::with_capacity(64),
        scope_stack: Vec::with_capacity(16),
        scopes: Vec::new(),
        let_items: Vec::new(),
        subject_items: Vec::new(),
        example_items: Vec::new(),
        top_spec_depth: 0,
        zero_arg_calls: HashMap::new(),
        vn_offenses: Vec::new(),
        vn_passing: Vec::new(),
        vd_offenses: Vec::new(),
        ns_offenses: Vec::new(),
        metadata_anchors: Vec::new(),
        focus_candidates: Vec::new(),
        pending_candidates: Vec::new(),
    }
}

/// `RSpec/Focus` `focusable_selector?` role set (regular/skipped example
/// groups, regular/skipped/pending examples, all shared groups). The two
/// focused role sets are handled separately.
const FOCUSABLE: u32 = roles::EG_REGULAR
    | roles::EG_SKIPPED
    | roles::EX_REGULAR
    | roles::EX_SKIPPED
    | roles::EX_PENDING
    | roles::SG_ALL;

/// `RSpec/Focus` focused role set (`ExampleGroups.focused` /
/// `Examples.focused`).
const FOCUSED: u32 = roles::EG_FOCUSED | roles::EX_FOCUSED;

/// `RSpec/PendingWithoutReason` role set whose name alone makes a send a
/// candidate (`Examples.skipped` / `Examples.pending` / `ExampleGroups.skipped`).
const SKIP_PENDING: u32 = roles::EX_SKIPPED | roles::EX_PENDING | roles::EG_SKIPPED;

/// Metadata-anchor role set (`Examples.all` / `ExampleGroups.all` /
/// `SharedGroups.all` / `Hooks`).
const METADATA_ROLES: u32 =
    roles::EX_ALL | roles::EG_ALL | roles::SG_ALL | roles::HOOKS;

impl RSpecDispatcherRule<'_> {
    fn handle_call(&mut self, call: &CallNode<'_>) -> StackEntry {
        let mut entry = StackEntry {
            opened_scope: false,
            top_spec: false,
        };
        let name = call.name().as_slice();
        let role_mask = self.cfg.roles_of(name);
        let kind = block_kind(call);
        let recv = call.receiver();
        let recv_none = recv.is_none();
        let n_args = call
            .arguments()
            .map(|a| a.arguments().iter().count())
            .unwrap_or(0);

        // The LetSetup usage index (see module doc: zero args, no receiver,
        // no block-pass — a literal block is fine).
        if recv_none && n_args == 0 && kind != BlockKind::BlockArg {
            self.zero_arg_calls
                .entry(name.to_vec())
                .or_default()
                .push(call.location().start_offset());

            // `RSpec/NamedSubject` reference (`subject_usage`, hard-coded
            // `$(send nil? :subject)` — the literal name `subject`, never an
            // alias nor `subject!`). Evaluated against the CURRENT open frames
            // (R's plain-block ancestors), before R's own frame is pushed.
            if name == b"subject" && self.named_subject_offense() {
                let loc = call.message_loc().unwrap_or_else(|| call.location());
                self.ns_offenses.push((loc.start_offset(), loc.end_offset()));
            }
        }

        let rspec_recv = recv_none || recv.as_ref().is_some_and(|r| rspec_const(r));

        // `RSpec.configure do |c| ... end` metadata-anchor block: the send name
        // is `configure` (not a role name, so role_mask is 0), so probe it
        // before the role gate. The wrapper's `rspec_configure` matcher checks
        // the single block param; here a block of any kind qualifies (superset).
        if rspec_recv
            && name == b"configure"
            && matches!(kind, BlockKind::Plain | BlockKind::Numbered | BlockKind::It)
        {
            let loc = call.location();
            self.metadata_anchors.push((loc.start_offset(), loc.end_offset()));
        }

        if role_mask == 0 {
            return entry;
        }

        // Direct metadata anchor: an example/group/shared-group/hook block with
        // an rspec receiver and >= 1 argument (the `_` description/scope arg).
        // Any block kind is emitted; the wrapper's `(block ...)` matcher filters
        // parser block kind (numblock never fires; itblock == plain at < 3.4).
        if rspec_recv
            && matches!(kind, BlockKind::Plain | BlockKind::Numbered | BlockKind::It)
            && role_mask & METADATA_ROLES != 0
            && n_args >= 1
        {
            let loc = call.location();
            self.metadata_anchors.push((loc.start_offset(), loc.end_offset()));
        }

        // `RSpec/Focus` candidate: a focused alias, or a focusable selector
        // carrying `:focus` / `focus: true` metadata (the wrapper re-checks the
        // exact matcher plus the chained / inside-def guards).
        if rspec_recv
            && (role_mask & FOCUSED != 0
                || (role_mask & FOCUSABLE != 0 && has_focus_metadata(call)))
        {
            self.focus_candidates.push(parser_send_range(call, kind));
        }

        // `RSpec/PendingWithoutReason` candidate: a skipped/pending example or
        // skipped example-group name, or any send carrying `:skip` / `:pending`
        // metadata (the wrapper re-checks receiver and parent relationships).
        if role_mask & SKIP_PENDING != 0 || has_skip_pending_metadata(call) {
            self.pending_candidates
                .push(parser_send_range(call, kind));
        }

        // Top-level `spec_group?` (any_block): the gate for the Variable
        // cops. `self.stack` holds only the ProgramNode entry when a
        // top-level statement enters.
        if self.stack.len() == 1
            && matches!(kind, BlockKind::Plain | BlockKind::Numbered | BlockKind::It)
            && rspec_recv
            && role_mask & (roles::EG_ALL | roles::SG_ALL) != 0
        {
            entry.top_spec = true;
        }

        // Variable-definition candidates (send-shaped: any block kind).
        if self.top_spec_depth > 0
            && recv_none
            && role_mask & (roles::HELPERS | roles::SUBJECTS) != 0
            && n_args > 0
        {
            self.variable_candidate(call);
        }

        // `let?` collection (block form, or exactly `(name, &block_pass)`).
        let is_let = recv_none
            && role_mask & roles::HELPERS != 0
            && (kind == BlockKind::Plain || (kind == BlockKind::BlockArg && n_args == 1));
        if is_let {
            self.collect_let(call, name, n_args, kind);
        }

        // `subject?` collection (block form only — stock's `subject?` has no
        // send-form pattern). Feeds `MultipleMemoizedHelpers` when
        // `AllowSubject: false`.
        let is_subject =
            recv_none && role_mask & roles::SUBJECTS != 0 && kind == BlockKind::Plain;
        if is_subject {
            self.collect_subject(call);
        }

        // `example?` collection (plain block only). Feeds
        // `RepeatedDescription` / `RepeatedExample`.
        let is_example =
            recv_none && role_mask & roles::EX_ALL != 0 && kind == BlockKind::Plain;
        if is_example {
            self.collect_example(call);
        }

        // parser-block frames: barriers + collection roots.
        if kind == BlockKind::Plain {
            let spec_group_send =
                rspec_recv && role_mask & (roles::EG_ALL | roles::SG_ALL) != 0;
            let example_group = rspec_recv && role_mask & roles::EG_ALL != 0;
            let include_block = recv_none && role_mask & roles::INC_ALL != 0;
            let example_or_hook =
                recv_none && role_mask & (roles::EX_ALL | roles::HOOKS) != 0;
            let shared_examples = rspec_recv && role_mask & roles::SG_EXAMPLES != 0;
            let described_class_candidate = example_group && call
                .arguments()
                .and_then(|a| a.arguments().iter().next())
                .is_some_and(|arg| {
                    arg.as_constant_read_node().is_some()
                        || arg.as_constant_path_node().is_some()
                });
            let subject_def_named = self.find_direct_subject_def(call);
            let loc = call.location();
            self.scopes.push(Scope {
                parent: self.scope_stack.last().copied(),
                range: (loc.start_offset(), loc.end_offset()),
                scope_change: spec_group_send || include_block,
                spec_group: spec_group_send,
                example_group,
                example: is_example,
                example_or_hook,
                shared_examples,
                subject_def_named,
                let_barrier: is_let,
                subject_barrier: is_subject,
                described_class_candidate,
                lets: Vec::new(),
                subjects: Vec::new(),
                examples: Vec::new(),
            });
            self.scope_stack.push(self.scopes.len() - 1);
            entry.opened_scope = true;
        }
        entry
    }

    /// `RSpec/NamedSubject`: verdict for a bare `subject` reference against the
    /// currently open plain-block frames (R's `each_ancestor(:block)`).
    ///
    /// Ancestor gate (stock `on_block` + `ignored_shared_example?`): R needs a
    /// plain-block example/hook ancestor that is not enclosed by a shared
    /// example group when `IgnoreSharedExamples` is on. Because a shared group
    /// only suppresses example/hook frames INNER to it, the OUTERMOST frame
    /// that is either an example/hook or a shared group decides: example/hook
    /// -> report, shared -> suppress. With `IgnoreSharedExamples` off, any
    /// example/hook ancestor reports.
    ///
    /// Style gate (`ConfigurableEnforcedStyle`): `always` always reports;
    /// `named_only` reports only when `nearest_subject` (the innermost
    /// plain-block ancestor whose body directly defines a `subject?`) is a
    /// NAMED definition.
    fn named_subject_offense(&self) -> bool {
        let ancestor_ok = if self.cfg.named_subject_ignore_shared {
            let mut ok = false;
            for &s in &self.scope_stack {
                let sc = &self.scopes[s];
                if sc.example_or_hook {
                    ok = true;
                    break;
                }
                if sc.shared_examples {
                    break;
                }
            }
            ok
        } else {
            self.scope_stack
                .iter()
                .any(|&s| self.scopes[s].example_or_hook)
        };
        if !ancestor_ok {
            return false;
        }
        if self.cfg.named_subject_style != 1 {
            return true; // always
        }
        // named_only: the nearest enclosing subject definition must be named.
        for &s in self.scope_stack.iter().rev() {
            if let Some(named) = self.scopes[s].subject_def_named {
                return named;
            }
        }
        false
    }

    /// `find_subject(block)`: the named-ness of the first direct-child statement
    /// of the block body that is a `subject?` definition (`(block (send nil?
    /// #Subjects.all ...) ...)`), or `None`. Stock reads `body.child_nodes`;
    /// for a single-statement body that statement is R's own container (never a
    /// subject definition when R is nested inside), so scanning prism's
    /// statement list matches for every reachable reference.
    fn find_direct_subject_def(&self, call: &CallNode<'_>) -> Option<bool> {
        let stmts = call.block()?.as_block_node()?.body()?;
        let stmts = stmts.as_statements_node()?;
        for stmt in stmts.body().iter() {
            let Some(c) = stmt.as_call_node() else { continue };
            if c.receiver().is_none()
                && self.cfg.roles_of(c.name().as_slice()) & roles::SUBJECTS != 0
                && block_kind(&c) == BlockKind::Plain
            {
                let named = c
                    .arguments()
                    .is_some_and(|a| a.arguments().iter().next().is_some());
                return Some(named);
            }
        }
        None
    }

    /// `(send nil? {#Subjects.all #Helpers.all} $({any_sym str dstr} ...)
    /// ...)` under a top-level spec group. Feeds both `RSpec/VariableName`
    /// (sym/str only — stock skips dstr/dsym) and `RSpec/VariableDefinition`
    /// (sym/str/dsym, style-filtered; dstr never fails).
    fn variable_candidate(&mut self, call: &CallNode<'_>) {
        let Some(args) = call.arguments() else { return };
        let Some(first) = args.arguments().iter().next() else {
            return;
        };
        let loc = first.location();
        let (start, end) = (loc.start_offset(), loc.end_offset());
        if let Some(s) = first.as_symbol_node() {
            let value = String::from_utf8_lossy(s.unescaped()).into_owned();
            self.variable_name_candidate(0, value.clone(), start, end);
            // VariableDefinition: a `sym` name fails only under `strings`.
            if self.cfg.variable_definition_style == 1 {
                self.vd_offenses.push(VarDefOffense { start, end, kind: 0, value });
            }
        } else if let Some(s) = first.as_string_node() {
            // An empty percent-string is a parser-gem `dstr` (see
            // `string_is_parser_dstr`): neither cop treats it as a `str`.
            if string_is_parser_dstr(&s) {
                return;
            }
            let value = String::from_utf8_lossy(s.unescaped()).into_owned();
            self.variable_name_candidate(1, value.clone(), start, end);
            // VariableDefinition: a `str` name fails only under `symbols`.
            if self.cfg.variable_definition_style == 0 {
                self.vd_offenses.push(VarDefOffense { start, end, kind: 1, value });
            }
        } else if first.as_interpolated_symbol_node().is_some() {
            // dsym: a `RSpec/VariableName` non-candidate, but a
            // VariableDefinition offense under `strings` (the wrapper slices
            // the source, so no value is carried).
            if self.cfg.variable_definition_style == 1 {
                self.vd_offenses.push(VarDefOffense {
                    start,
                    end,
                    kind: 2,
                    value: String::new(),
                });
            }
        }
        // dstr / non-literal first argument: neither cop fires.
    }

    /// `RSpec/VariableName` style classification for one sym/str candidate.
    fn variable_name_candidate(&mut self, vkind: u8, value: String, start: usize, end: usize) {
        let style = self.cfg.variable_name_style;
        if matches_style(&value, style) {
            self.vn_passing.push((value, vkind));
        } else {
            let valid_alt = matches_style(&value, 1 - style);
            self.vn_offenses.push(VarNameOffense {
                start,
                end,
                kind: vkind,
                value,
                valid_alt,
            });
        }
    }

    fn collect_let(&mut self, call: &CallNode<'_>, name: &[u8], n_args: usize, kind: BlockKind) {
        let bang_name = if name == b"let!" && n_args == 1 {
            call.arguments()
                .and_then(|a| a.arguments().iter().next())
                .and_then(|n| {
                    if let Some(s) = n.as_symbol_node() {
                        Some((0u8, s.unescaped().to_vec()))
                    } else {
                        n.as_string_node().map(|s| (1u8, s.unescaped().to_vec()))
                    }
                })
        } else {
            None
        };
        let idx = u32::try_from(self.let_items.len()).expect("more lets than u32");
        self.let_items.push(LetItem {
            send_range: parser_send_range(call, kind),
            bang_name,
            var_def: classify_var_def(call),
        });
        // Attribute to open frames, innermost outward, stopping at (and
        // including) the first lets-barrier.
        for &s in self.scope_stack.iter().rev() {
            self.scopes[s].lets.push(idx);
            let sc = &self.scopes[s];
            if sc.scope_change || sc.example || sc.let_barrier {
                break;
            }
        }
    }

    /// Collect a `subject?` node for `MultipleMemoizedHelpers`, attributing
    /// it to every open frame from the innermost outward until (and
    /// including) the first subjects-barrier (`scope_change` / `example` /
    /// a `subject?` frame). A `let?` frame does NOT stop the query.
    fn collect_subject(&mut self, call: &CallNode<'_>) {
        let idx = u32::try_from(self.subject_items.len()).expect("more subjects than u32");
        self.subject_items.push(classify_var_def(call));
        for &s in self.scope_stack.iter().rev() {
            self.scopes[s].subjects.push(idx);
            let sc = &self.scopes[s];
            if sc.scope_change || sc.example || sc.subject_barrier {
                break;
            }
        }
    }

    /// Collect an `example?` node for `RepeatedDescription` / `RepeatedExample`,
    /// attributing it to every open frame from the innermost outward until (and
    /// including) the first examples-barrier (`scope_change` / `example`). A
    /// `let?`/`subject?` frame does NOT stop the query, so an example inside a
    /// `let` body still belongs to the enclosing group. Called before this
    /// example's own frame is pushed, so it attributes to ancestors only.
    fn collect_example(&mut self, call: &CallNode<'_>) {
        let loc = call.location();
        let idx = u32::try_from(self.example_items.len()).expect("more examples than u32");
        self.example_items.push((loc.start_offset(), loc.end_offset()));
        for &s in self.scope_stack.iter().rev() {
            self.scopes[s].examples.push(idx);
            let sc = &self.scopes[s];
            if sc.scope_change || sc.example {
                break;
            }
        }
    }

    /// Post-walk verdicts.
    pub fn finish(self) -> RSpecResult {
        let mut let_setup = Vec::new();
        for scope in &self.scopes {
            if !scope.scope_change {
                continue;
            }
            for &li in &scope.lets {
                let item = &self.let_items[li as usize];
                let Some(bang) = &item.bang_name else { continue };
                if self.overridden(scope.parent, bang) {
                    continue;
                }
                if self.called_within(scope.range, &bang.1) {
                    continue;
                }
                let_setup.push(item.send_range);
            }
        }
        let multiple_memoized_helpers = self.multiple_memoized_helpers();
        // Both cops read identical group data (each owns its slot); the
        // collection is computed once and cloned.
        let groups = self.repeated_example_groups();
        let empty_example_group = self.empty_example_group_candidates();
        let described_class: Vec<(usize, usize)> = self
            .scopes
            .iter()
            .filter(|s| s.described_class_candidate)
            .map(|s| s.range)
            .collect();
        RSpecResult {
            variable_name: (self.vn_offenses, self.vn_passing),
            let_setup,
            variable_definition: self.vd_offenses,
            multiple_memoized_helpers,
            repeated_description: groups.clone(),
            repeated_example: groups,
            named_subject: self.ns_offenses,
            metadata_anchors: self.metadata_anchors,
            focus: self.focus_candidates,
            pending_without_reason: self.pending_candidates,
            empty_example_group,
            described_class,
        }
    }

    /// For every `example_group?` frame with >= 2 collected examples, the
    /// example BLOCK node ranges in document order (groups emitted in document
    /// order). No further Rust filtering: the wrapper runs stock's exact
    /// structural grouping, so a group of 2 non-repeating examples is emitted
    /// here and filtered there.
    fn repeated_example_groups(&self) -> Vec<Vec<(usize, usize)>> {
        let mut out = Vec::new();
        for scope in &self.scopes {
            if !scope.example_group || scope.examples.len() < 2 {
                continue;
            }
            out.push(
                scope
                    .examples
                    .iter()
                    .map(|&e| self.example_items[e as usize])
                    .collect(),
            );
        }
        out
    }

    /// `EmptyExampleGroup`: every `example_group?` frame's block range in
    /// document order. No filtering here — the wrapper runs stock's
    /// `each_ancestor(:any_def)`, `inside_example?`, `example_group_body`,
    /// and the mutually recursive `examples?` / `offensive?` matchers on the
    /// located parser block node.
    fn empty_example_group_candidates(&self) -> Vec<(usize, usize)> {
        self.scopes
            .iter()
            .filter(|s| s.example_group)
            .map(|s| s.range)
            .collect()
    }

    /// `MultipleMemoizedHelpers`: for every plain-block spec group, union the
    /// helpers visible from the group's own frame and every parser-block
    /// ancestor frame (`all_helpers`), classify them into bytewise-decidable
    /// identities (`rust_distinct`) plus dsym/dstr source ranges, and emit
    /// the group when the safe upper bound exceeds `Max`.
    fn multiple_memoized_helpers(&self) -> Vec<MmhGroup> {
        let mut groups = Vec::new();
        let allow_subject = self.cfg.mmh_allow_subject;
        for (gi, scope) in self.scopes.iter().enumerate() {
            if !scope.spec_group {
                continue;
            }
            // Gather the unique visible helper items (the same physical item
            // reached through several ancestor paths is one item).
            let mut let_idxs: Vec<u32> = Vec::new();
            let mut subj_idxs: Vec<u32> = Vec::new();
            let mut cur = Some(gi);
            while let Some(f) = cur {
                let frame = &self.scopes[f];
                let_idxs.extend_from_slice(&frame.lets);
                if !allow_subject {
                    subj_idxs.extend_from_slice(&frame.subjects);
                }
                cur = frame.parent;
            }
            let_idxs.sort_unstable();
            let_idxs.dedup();
            subj_idxs.sort_unstable();
            subj_idxs.dedup();

            let mut distinct: HashSet<(u8, &[u8])> = HashSet::new();
            let mut dsym_dstr_ranges: Vec<(usize, usize)> = Vec::new();
            let var_defs = let_idxs
                .iter()
                .map(|&li| &self.let_items[li as usize].var_def)
                .chain(subj_idxs.iter().map(|&si| &self.subject_items[si as usize]));
            for var_def in var_defs {
                match var_def {
                    VarDef::Sym(v) => {
                        distinct.insert((0, v.as_slice()));
                    }
                    VarDef::Str(v) => {
                        distinct.insert((1, v.as_slice()));
                    }
                    VarDef::Nil => {
                        distinct.insert((2, b""));
                    }
                    VarDef::Dyn(s, e) => dsym_dstr_ranges.push((*s, *e)),
                }
            }
            let rust_distinct = distinct.len();
            // Safe upper bound: below it no offense is possible even before
            // the wrapper dedups the dsym/dstr items structurally.
            if (rust_distinct + dsym_dstr_ranges.len()) as i64 > self.cfg.mmh_max {
                groups.push(MmhGroup {
                    start: scope.range.0,
                    end: scope.range.1,
                    rust_distinct,
                    dsym_dstr_ranges,
                });
            }
        }
        groups
    }

    /// `overrides_outer_let_bang?`: some ancestor scope-change frame
    /// collects a `let!` with the same (kind, value) name.
    fn overridden(&self, mut parent: Option<usize>, bang: &(u8, Vec<u8>)) -> bool {
        while let Some(p) = parent {
            let scope = &self.scopes[p];
            if scope.scope_change
                && scope.lets.iter().any(|&li| {
                    self.let_items[li as usize]
                        .bang_name
                        .as_ref()
                        .is_some_and(|b| b == bang)
                })
            {
                return true;
            }
            parent = scope.parent;
        }
        false
    }

    /// `method_called?`: a receiverless zero-argument send with this name
    /// inside the scope's subtree (start offsets are walk-ordered).
    fn called_within(&self, range: (usize, usize), name: &[u8]) -> bool {
        let Some(positions) = self.zero_arg_calls.get(name) else {
            return false;
        };
        let from = positions.partition_point(|&p| p < range.0);
        positions.get(from).is_some_and(|&p| p < range.1)
    }
}

/// A prism `StringNode` that the parser gem parses as an (empty) `dstr`
/// rather than a `str`: an EMPTY percent-string literal (`%()`, `%q()`,
/// `%Q()`, `%{}`, `%[]`). prism 1.9 folds these into `StringNode`, but every
/// stock RSpec matcher's `{str dstr}` split follows the parser gem, which
/// treats an empty percent-string as an empty `dstr` (probed). Non-empty
/// percent-strings and empty quote strings (`''`) stay `str` in both. This is
/// the only str/dstr divergence between the two parsers for these matchers
/// (adjacent string concatenation is already an `InterpolatedStringNode` in
/// prism, so it never reaches the `StringNode` arm).
fn string_is_parser_dstr(s: &StringNode<'_>) -> bool {
    s.unescaped().is_empty()
        && s.opening_loc()
            .is_some_and(|o| o.as_slice().first() == Some(&b'%'))
}

/// `variable_definition?` first-argument classification for a `let?`/
/// `subject?` call: the captured `{any_sym str dstr}` node, or `Nil` when the
/// call has no such literal first argument.
fn classify_var_def(call: &CallNode<'_>) -> VarDef {
    let Some(args) = call.arguments() else {
        return VarDef::Nil;
    };
    let Some(first) = args.arguments().iter().next() else {
        return VarDef::Nil;
    };
    if let Some(s) = first.as_symbol_node() {
        VarDef::Sym(s.unescaped().to_vec())
    } else if let Some(s) = first.as_string_node() {
        if string_is_parser_dstr(&s) {
            let loc = first.location();
            VarDef::Dyn(loc.start_offset(), loc.end_offset())
        } else {
            VarDef::Str(s.unescaped().to_vec())
        }
    } else if first.as_interpolated_symbol_node().is_some()
        || first.as_interpolated_string_node().is_some()
    {
        let loc = first.location();
        VarDef::Dyn(loc.start_offset(), loc.end_offset())
    } else {
        VarDef::Nil
    }
}

/// parser SEND node end for a call: prism's call range includes an attached
/// literal block; the parser `(send)` child does not. A `&block_pass` IS a
/// parser argument, so it stays inside the send range.
fn parser_send_range(call: &CallNode<'_>, kind: BlockKind) -> (usize, usize) {
    let start = call.location().start_offset();
    if let Some(cl) = call.closing_loc() {
        return (start, cl.end_offset());
    }
    if kind == BlockKind::BlockArg
        && let Some(b) = call.block()
    {
        return (start, b.location().end_offset());
    }
    let end = call
        .arguments()
        .and_then(|a| a.arguments().iter().last().map(|n| n.location().end_offset()))
        .or_else(|| call.message_loc().map(|m| m.end_offset()))
        .unwrap_or_else(|| call.location().end_offset());
    (start, end)
}

/// True when any direct argument is a `(sym :name)` for a name in `syms`, or
/// the LAST argument is a hash-like literal (`{...}` or trailing keyword args)
/// containing a pair whose key is such a symbol. The pair VALUE is not checked
/// here — the wrapper re-runs the exact stock matcher, which verifies `true`;
/// this is a safe superset for candidate selection.
fn has_symbol_metadata(call: &CallNode<'_>, syms: &[&[u8]]) -> bool {
    let Some(args) = call.arguments() else {
        return false;
    };
    let list = args.arguments();
    let mut last = None;
    for arg in list.iter() {
        if let Some(s) = arg.as_symbol_node()
            && syms.iter().any(|k| *k == s.unescaped())
        {
            return true;
        }
        last = Some(arg);
    }
    let Some(last) = last else { return false };
    let elements = if let Some(h) = last.as_hash_node() {
        h.elements()
    } else if let Some(h) = last.as_keyword_hash_node() {
        h.elements()
    } else {
        return false;
    };
    for el in elements.iter() {
        if let Some(assoc) = el.as_assoc_node()
            && let Some(s) = assoc.key().as_symbol_node()
            && syms.iter().any(|k| *k == s.unescaped())
        {
            return true;
        }
    }
    false
}

fn has_focus_metadata(call: &CallNode<'_>) -> bool {
    has_symbol_metadata(call, &[b"focus"])
}

fn has_skip_pending_metadata(call: &CallNode<'_>) -> bool {
    has_symbol_metadata(call, &[b"skip", b"pending"])
}

impl Rule for RSpecDispatcherRule<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        let entry = if let Some(call) = node.as_call_node() {
            self.handle_call(&call)
        } else {
            StackEntry {
                opened_scope: false,
                top_spec: false,
            }
        };
        if entry.top_spec {
            self.top_spec_depth += 1;
        }
        self.stack.push(entry);
    }

    fn leave(&mut self) {
        if let Some(entry) = self.stack.pop() {
            if entry.opened_scope {
                self.scope_stack.pop();
            }
            if entry.top_spec {
                self.top_spec_depth -= 1;
            }
        }
    }

    fn interest(&self) -> Interest {
        Interest(Interest::LEAVE | Interest::ENTER_ALL)
    }
}

/// Standalone entry point for `RSpec/VariableName` (the wrapper's fallback
/// path). Runs the whole dispatcher rule and keeps one cop's slice.
pub fn check_rspec_variable_name(
    source: &[u8],
    cfg: &RSpecConfig,
) -> (Vec<VarNameOffense>, Vec<(String, u8)>) {
    run(source, cfg).variable_name
}

/// Standalone entry point for `RSpec/LetSetup`.
pub fn check_rspec_let_setup(source: &[u8], cfg: &RSpecConfig) -> Vec<(usize, usize)> {
    run(source, cfg).let_setup
}

/// Standalone entry point for `RSpec/VariableDefinition` (the wrapper's
/// fallback path).
pub fn check_rspec_variable_definition(source: &[u8], cfg: &RSpecConfig) -> Vec<VarDefOffense> {
    run(source, cfg).variable_definition
}

/// Standalone entry point for `RSpec/MultipleMemoizedHelpers`.
pub fn check_rspec_multiple_memoized_helpers(source: &[u8], cfg: &RSpecConfig) -> Vec<MmhGroup> {
    run(source, cfg).multiple_memoized_helpers
}

/// Standalone entry point for `RSpec/RepeatedDescription` (the wrapper's
/// fallback path). Per qualifying example group, the example block ranges.
pub fn check_rspec_repeated_description(
    source: &[u8],
    cfg: &RSpecConfig,
) -> Vec<Vec<(usize, usize)>> {
    run(source, cfg).repeated_description
}

/// Standalone entry point for `RSpec/RepeatedExample` (same group data as
/// `check_rspec_repeated_description`).
pub fn check_rspec_repeated_example(source: &[u8], cfg: &RSpecConfig) -> Vec<Vec<(usize, usize)>> {
    run(source, cfg).repeated_example
}

/// Standalone entry point for `RSpec/NamedSubject` (the wrapper's fallback
/// path). The `subject` selector ranges to report.
pub fn check_rspec_named_subject(source: &[u8], cfg: &RSpecConfig) -> Vec<(usize, usize)> {
    run(source, cfg).named_subject
}

/// Standalone entry point for the four `Metadata`-mixin cops
/// (`MetadataStyle` / `DuplicatedMetadata` / `EmptyMetadata` / `SortMetadata`):
/// the shared metadata-anchor block ranges. Each cop's wrapper relocates these
/// parser block nodes and runs stock's `Metadata#on_block` verbatim.
pub fn check_rspec_metadata_anchors(source: &[u8], cfg: &RSpecConfig) -> Vec<(usize, usize)> {
    run(source, cfg).metadata_anchors
}

/// Standalone entry point for `RSpec/Focus` (candidate send ranges).
pub fn check_rspec_focus(source: &[u8], cfg: &RSpecConfig) -> Vec<(usize, usize)> {
    run(source, cfg).focus
}

/// Standalone entry point for `RSpec/PendingWithoutReason` (candidate send
/// ranges).
pub fn check_rspec_pending_without_reason(source: &[u8], cfg: &RSpecConfig) -> Vec<(usize, usize)> {
    run(source, cfg).pending_without_reason
}

/// Standalone entry point for `RSpec/EmptyExampleGroup` (candidate
/// example-group block ranges). The wrapper locates each parser block node
/// and runs stock's `on_block` detection verbatim.
pub fn check_rspec_empty_example_group(source: &[u8], cfg: &RSpecConfig) -> Vec<(usize, usize)> {
    run(source, cfg).empty_example_group
}

/// Standalone entry point for `RSpec/DescribedClass` (candidate block
/// ranges). The wrapper relocates the parser block node and runs stock's
/// full detection + autocorrect verbatim.
pub fn check_rspec_described_class(source: &[u8], cfg: &RSpecConfig) -> Vec<(usize, usize)> {
    run(source, cfg).described_class
}

fn run(source: &[u8], cfg: &RSpecConfig) -> RSpecResult {
    let mut rule = build_rule(cfg);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::rspec_language;

    fn cfg() -> RSpecConfig {
        let mut c = RSpecConfig::from_role_lists(
            &rspec_language::tests::default_role_lists(),
        )
        .unwrap();
        c.variable_name_style = 0;
        c
    }

    fn vn(src: &str) -> Vec<(usize, usize, u8, String, bool)> {
        check_rspec_variable_name(src.as_bytes(), &cfg())
            .0
            .into_iter()
            .map(|o| (o.start, o.end, o.kind, o.value, o.valid_alt))
            .collect()
    }

    fn vn_sources(src: &str) -> Vec<String> {
        vn(src)
            .into_iter()
            .map(|(s, e, ..)| String::from_utf8_lossy(&src.as_bytes()[s..e]).into_owned())
            .collect()
    }

    fn ls_sources(src: &str) -> Vec<String> {
        check_rspec_let_setup(src.as_bytes(), &cfg())
            .into_iter()
            .map(|(s, e)| String::from_utf8_lossy(&src.as_bytes()[s..e]).into_owned())
            .collect()
    }

    fn cfg_vd(style: u8) -> RSpecConfig {
        let mut c = cfg();
        c.variable_definition_style = style;
        c
    }

    /// `(offense source slice, kind, value)` per VariableDefinition offense.
    fn vd(src: &str, style: u8) -> Vec<(String, u8, String)> {
        check_rspec_variable_definition(src.as_bytes(), &cfg_vd(style))
            .into_iter()
            .map(|o| {
                (
                    String::from_utf8_lossy(&src.as_bytes()[o.start..o.end]).into_owned(),
                    o.kind,
                    o.value,
                )
            })
            .collect()
    }

    fn cfg_mmh(max: i64, allow_subject: bool) -> RSpecConfig {
        let mut c = cfg();
        c.mmh_max = max;
        c.mmh_allow_subject = allow_subject;
        c
    }

    /// `(group source slice, rust_distinct, dsym/dstr source slices)` per
    /// emitted MMH group.
    fn mmh(src: &str, max: i64, allow_subject: bool) -> Vec<(String, usize, Vec<String>)> {
        check_rspec_multiple_memoized_helpers(src.as_bytes(), &cfg_mmh(max, allow_subject))
            .into_iter()
            .map(|g| {
                (
                    String::from_utf8_lossy(&src.as_bytes()[g.start..g.end]).into_owned(),
                    g.rust_distinct,
                    g.dsym_dstr_ranges
                        .into_iter()
                        .map(|(s, e)| String::from_utf8_lossy(&src.as_bytes()[s..e]).into_owned())
                        .collect(),
                )
            })
            .collect()
    }

    // --- style regexps (stock FORMATS, probed) ---

    #[test]
    fn snake_case_matches_stock_formats() {
        for ok in ["foo", "foo_bar", "_", "__", "f1", "1", "@foo", "@@foo", "foo!", "foo?", "foo=", "café_name"] {
            assert!(snake_case(ok), "{ok:?} should be snake_case");
        }
        for bad in ["Foo", "fooBar", "", "!", "foo!!", "foo bar", "ユーザ", "@@@foo", "foo-bar"] {
            assert!(!snake_case(bad), "{bad:?} should NOT be snake_case");
        }
    }

    #[test]
    fn camel_case_matches_stock_formats() {
        for ok in ["foo", "fooBar", "_", "_fooBar", "foo1Bar", "@fooBar", "fooBar!", "caféName"] {
            assert!(camel_case(ok), "{ok:?} should be camelCase");
        }
        for bad in ["Foo", "foo_bar", "__", "1foo", "", "ユーザ", "foo bar"] {
            assert!(!camel_case(bad), "{bad:?} should NOT be camelCase");
        }
    }

    // --- inside_example_group? gate (probes A1-A13) ---

    #[test]
    fn flags_bad_names_inside_a_top_level_group() {
        let src = "describe 'x' do\n  let(:userName) { 1 }\nend\n";
        assert_eq!(vn_sources(src), vec![":userName"]);
    }

    #[test]
    fn ignores_groups_wrapped_in_a_top_level_class_or_module() {
        // A2/A8: the OUTERMOST statement decides; class/module wrappers of
        // the group defeat the gate.
        for wrapper in ["class Foo", "module Foo"] {
            let src = format!(
                "{wrapper}\n  describe 'x' do\n    let(:userName) {{ 1 }}\n  end\nend\n"
            );
            assert_eq!(vn(&src), vec![], "{wrapper} should not count");
        }
    }

    #[test]
    fn flags_through_a_class_inside_the_group() {
        // A3: only the top level matters; a class INSIDE the group is fine.
        let src = "describe 'x' do\n  class Foo\n    let(:userName) { 1 }\n  end\nend\n";
        assert_eq!(vn_sources(src), vec![":userName"]);
    }

    #[test]
    fn ignores_top_level_lets() {
        assert_eq!(vn("let(:userName) { 1 }\n"), vec![]);
    }

    #[test]
    fn numblock_and_itblock_groups_count_as_spec_groups() {
        // A5: spec_group? is any_block.
        let src = "RSpec.describe('x') { let(:userName) { _1 } }\n";
        assert_eq!(vn_sources(src), vec![":userName"]);
        let src = "RSpec.describe('x') {\n  it\n  let(:userName) { 1 }\n}\n";
        assert_eq!(vn_sources(src), vec![":userName"]);
    }

    #[test]
    fn shared_groups_count_and_include_blocks_do_not() {
        // A6 / A13.
        let src = "shared_examples 'x' do\n  let(:userName) { 1 }\nend\n";
        assert_eq!(vn_sources(src), vec![":userName"]);
        let src = "include_context 'x' do\n  let(:userName) { 1 }\nend\n";
        assert_eq!(vn(src), vec![]);
    }

    #[test]
    fn candidates_are_send_shaped() {
        // A7 (bare let), A9 (numblock let), A10 (subject), A11 (string).
        let src = "describe 'x' do\n  let(:userName)\n  subject(:badName) { 1 }\n  let('strName') { 1 }\n  let(:okay_name) { _1 }\nend\n";
        assert_eq!(
            vn_sources(src),
            vec![":userName", ":badName", "'strName'"]
        );
    }

    #[test]
    fn non_rspec_receivers_do_not_make_groups() {
        let src = "Foo.describe 'x' do\n  let(:userName) { 1 }\nend\n";
        assert_eq!(vn(src), vec![]);
        let src = "RSpec::Core.describe 'x' do\n  let(:userName) { 1 }\nend\n";
        assert_eq!(vn(src), vec![]);
        let src = "::RSpec.describe 'x' do\n  let(:userName) { 1 }\nend\n";
        assert_eq!(vn_sources(src), vec![":userName"]);
    }

    #[test]
    fn valid_alt_distinguishes_the_opposite_style() {
        let src = "describe 'x' do\n  let(:userName) { 1 }\n  let(:'user name') { 1 }\nend\n";
        let offs = vn(src);
        assert_eq!(offs.len(), 2);
        assert!(offs[0].4, "camelCase name is valid in the alternative");
        assert!(!offs[1].4, "spaced name is valid in neither");
    }

    #[test]
    fn passing_candidates_are_reported_for_style_detection() {
        let src = "describe 'x' do\n  let(:good_name) { 1 }\n  let(:badName) { 1 }\nend\n";
        let (offs, passing) = check_rspec_variable_name(src.as_bytes(), &cfg());
        assert_eq!(offs.len(), 1);
        assert_eq!(passing, vec![("good_name".to_string(), 0)]);
    }

    // --- LetSetup (probes F1-F15) ---

    #[test]
    fn flags_unused_let_bang() {
        let src = "describe 'x' do\n  let!(:w) { create(:widget) }\n  it 'a' do\n    expect(1).to eq 1\n  end\nend\n";
        assert_eq!(ls_sources(src), vec!["let!(:w)"]);
    }

    #[test]
    fn zero_arg_use_counts_but_argument_or_block_pass_use_does_not() {
        // F2 / F3 / F12 / F9.
        let used = "describe 'x' do\n  let!(:w) { create(:widget) }\n  it('a') { expect(w).to be }\nend\n";
        assert_eq!(ls_sources(used), Vec::<String>::new());
        let arg_use = "describe 'x' do\n  let!(:w) { create(:widget) }\n  it('a') { expect(w(1)).to be }\nend\n";
        assert_eq!(ls_sources(arg_use), vec!["let!(:w)"]);
        let block_pass_use = "describe 'x' do\n  let!(:w) { create(:widget) }\n  it('a') { w(&b) }\nend\n";
        assert_eq!(ls_sources(block_pass_use), vec!["let!(:w)"]);
        let block_use = "describe 'x' do\n  let!(:w) { create(:widget) }\n  it('a') { w { 1 } }\nend\n";
        assert_eq!(ls_sources(block_use), Vec::<String>::new());
    }

    #[test]
    fn string_names_resolve_uses_and_do_not_shadow_symbol_names() {
        // F4 / F6.
        let used = "describe 'x' do\n  let!('w') { create(:widget) }\n  it('a') { expect(w).to be }\nend\n";
        assert_eq!(ls_sources(used), Vec::<String>::new());
        let mixed = "describe 'x' do\n  let!('w') { create(:widget) }\n  context 'y' do\n    let!(:w) { create(:other) }\n    it('a') { expect(1).to eq 1 }\n  end\nend\n";
        assert_eq!(ls_sources(mixed), vec!["let!('w')", "let!(:w)"]);
    }

    #[test]
    fn inner_overriding_let_bang_is_skipped() {
        // F5: the outer one is reported, the inner override is not.
        let src = "describe 'x' do\n  let!(:w) { create(:widget) }\n  context 'y' do\n    let!(:w) { create(:other) }\n    it('a') { expect(1).to eq 1 }\n  end\nend\n";
        assert_eq!(ls_sources(src), vec!["let!(:w)"]);
    }

    #[test]
    fn use_anywhere_in_the_subtree_counts_including_nested_groups() {
        // F7 / F13.
        let nested = "describe 'x' do\n  let!(:w) { create(:widget) }\n  context 'y' do\n    it('a') { expect(w).to be }\n  end\nend\n";
        assert_eq!(ls_sources(nested), Vec::<String>::new());
        let self_use = "describe 'x' do\n  let!(:w) { w }\n  it('a') { expect(1).to eq 1 }\nend\n";
        assert_eq!(ls_sources(self_use), Vec::<String>::new());
    }

    #[test]
    fn block_pass_form_and_include_roots_are_candidates() {
        // F8 / F11.
        let src = "describe 'x' do\n  let!(:w, &blk)\n  it('a') { expect(1).to eq 1 }\nend\n";
        assert_eq!(ls_sources(src), vec!["let!(:w, &blk)"]);
        let src = "include_context 'shared' do\n  let!(:w) { create(:widget) }\nend\n";
        assert_eq!(ls_sources(src), vec!["let!(:w)"]);
    }

    #[test]
    fn non_candidates_extra_args_dstr_numblock() {
        // F10 / F14 / F15.
        for src in [
            "describe 'x' do\n  let!(:w, :extra) { create(:widget) }\n  it('a') { expect(1).to eq 1 }\nend\n",
            "describe 'x' do\n  let!(\"w#{x}\") { create(:widget) }\n  it('a') { expect(1).to eq 1 }\nend\n",
            "describe 'x' do\n  let!(:w) { _1 }\n  it('a') { expect(1).to eq 1 }\nend\n",
        ] {
            assert_eq!(ls_sources(src), Vec::<String>::new(), "src: {src}");
        }
    }

    #[test]
    fn numblock_groups_are_transparent_scopes() {
        // The C8/E8 family: a numblock context is not a scope change, so
        // its let! belongs to the outer describe; a use anywhere in the
        // outer subtree resolves it.
        let src = "describe 'x' do\n  context('y') {\n    _1\n    let!(:w) { create(:widget) }\n  }\n  it('a') { expect(1).to eq 1 }\nend\n";
        assert_eq!(ls_sources(src), vec!["let!(:w)"]);
        let used = "describe 'x' do\n  context('y') {\n    _1\n    let!(:w) { create(:widget) }\n  }\n  it('a') { expect(w).to be }\nend\n";
        assert_eq!(ls_sources(used), Vec::<String>::new());
    }

    #[test]
    fn no_parens_send_range() {
        let src = "describe 'x' do\n  let! :w do\n    create(:widget)\n  end\n  it('a') { expect(1).to eq 1 }\nend\n";
        assert_eq!(ls_sources(src), vec!["let! :w"]);
    }

    // --- VariableDefinition (probes B1/B2) ---

    #[test]
    fn variable_definition_symbols_flags_only_str() {
        // symbols style: str names fail, sym / dsym / dstr do not.
        let src = "describe 'x' do\n  let('user') { 1 }\n  subject(\"other\") { 2 }\n  let(:okay) { 3 }\n  let(:\"a#{b}\") { 4 }\n  let(\"dstr#{x}\") { 5 }\nend\n";
        assert_eq!(
            vd(src, 0),
            vec![
                ("'user'".to_string(), 1, "user".to_string()),
                ("\"other\"".to_string(), 1, "other".to_string()),
            ]
        );
    }

    #[test]
    fn variable_definition_strings_flags_sym_and_dsym() {
        // strings style: sym AND dsym fail (dsym carries no value — the
        // wrapper slices the source), str / dstr do not.
        let src = "describe 'x' do\n  let(:user) { 1 }\n  let(:\"a b\") { 2 }\n  let(:\"a#{x}\") { 3 }\n  let('str') { 4 }\n  let(\"dstr#{x}\") { 5 }\nend\n";
        assert_eq!(
            vd(src, 1),
            vec![
                (":user".to_string(), 0, "user".to_string()),
                (":\"a b\"".to_string(), 0, "a b".to_string()),
                (":\"a#{x}\"".to_string(), 2, String::new()),
            ]
        );
    }

    #[test]
    fn variable_definition_needs_the_top_level_group_gate() {
        // Same gate as VariableName: a top-level let is not a candidate.
        assert_eq!(vd("let('user') { 1 }\n", 0), Vec::new());
        // A group wrapped in a top-level class does not count.
        let src = "class Foo\n  describe 'x' do\n    let('user') { 1 }\n  end\nend\n";
        assert_eq!(vd(src, 0), Vec::new());
    }

    #[test]
    fn variable_definition_empty_percent_string_is_dstr() {
        // `%()` is a dstr (never flagged); `%(abc)` is a str (flagged under
        // symbols).
        let src = "describe 'x' do\n  let(%()) { 1 }\n  let(%(abc)) { 2 }\nend\n";
        assert_eq!(vd(src, 0), vec![("%(abc)".to_string(), 1, "abc".to_string())]);
    }

    // --- MultipleMemoizedHelpers (probes E1-E12) ---

    #[test]
    fn mmh_flags_the_inner_context_when_helpers_accumulate() {
        // E1: the inner context sees the outer `let` too -> 2 > 1.
        let src = "describe 'x' do\n  let(:a) { 1 }\n  context 'y' do\n    let(:b) { 1 }\n  end\nend\n";
        let groups = mmh(src, 1, true);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].0.starts_with("context 'y' do"));
        assert_eq!(groups[0].1, 2);
        assert!(groups[0].2.is_empty());
    }

    #[test]
    fn mmh_later_let_counts_for_an_earlier_nested_context() {
        // E2: a `let` after the nested context still counts (post-walk).
        let src = "describe 'x' do\n  context 'y' do\n    let(:b) { 1 }\n  end\n  let(:a) { 1 }\nend\n";
        let groups = mmh(src, 1, true);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].0.starts_with("context 'y' do"));
        assert_eq!(groups[0].1, 2);
    }

    #[test]
    fn mmh_overridden_and_distinct_names() {
        // E3: overridden `let(:a)` in parent and child is one identity.
        let overridden = "describe 'x' do\n  let(:a) { 1 }\n  context 'y' do\n    let(:a) { 2 }\n  end\nend\n";
        assert_eq!(mmh(overridden, 1, true), Vec::new());
        // E5: sym vs str are distinct.
        let distinct = "describe 'x' do\n  let(:a) { 1 }\n  let('a') { 2 }\nend\n";
        let groups = mmh(distinct, 1, true);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1, 2);
    }

    #[test]
    fn mmh_non_literal_and_nil_merging() {
        // E4: a non-literal name is one nil item -> :a + nil = 2.
        let nil_item = "describe 'x' do\n  let(:a) { 1 }\n  let(foo) { 2 }\nend\n";
        assert_eq!(mmh(nil_item, 1, true)[0].1, 2);
        // E6: unnamed subject + non-literal let merge into ONE nil (AllowSubject
        // false) -> 1, no offense.
        let merge = "describe 'x' do\n  subject { 1 }\n  let(foo) { 2 }\nend\n";
        assert_eq!(mmh(merge, 1, false), Vec::new());
    }

    #[test]
    fn mmh_arbitrary_dsl_block_is_transparent_and_an_ancestor() {
        // E7: describe sees [a, b] (context'y' is a barrier for its lets);
        // context'y' sees [a, b, c] via the ancestor union.
        let src = "describe 'x' do\n  let(:a) { 1 }\n  weird_dsl do\n    let(:b) { 1 }\n    context 'y' do\n      let(:c) { 1 }\n    end\n  end\nend\n";
        let groups = mmh(src, 1, true);
        assert_eq!(groups.len(), 2);
        assert!(groups[0].0.starts_with("describe 'x' do"));
        assert_eq!(groups[0].1, 2);
        assert!(groups[1].0.starts_with("context 'y' do"));
        assert_eq!(groups[1].1, 3);
    }

    #[test]
    fn mmh_numblock_context_is_transparent() {
        // E8: the numblock context's lets belong to the outer describe, and
        // the numblock context itself never gets an offense.
        let src = "describe 'x' do\n  let(:a) { 1 }\n  context('y') {\n    _1\n    let(:b) { 1 }\n    let(:c) { 1 }\n  }\nend\n";
        let groups = mmh(src, 1, true);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].0.starts_with("describe 'x' do"));
        assert_eq!(groups[0].1, 3);
    }

    #[test]
    fn mmh_lets_in_hook_and_subject_bodies_and_shared_groups() {
        // E9: a let inside a `before` hook body counts.
        let hook = "describe 'x' do\n  let(:a) { 1 }\n  before do\n    let(:b) { 1 }\n  end\nend\n";
        assert_eq!(mmh(hook, 1, true)[0].1, 2);
        // E12: a let inside a `subject(:s)` body counts (subject is not a
        // lets barrier).
        let subj = "describe 'x' do\n  subject(:s) do\n    let(:inner) { 1 }\n  end\n  let(:a) { 1 }\nend\n";
        assert_eq!(mmh(subj, 1, true)[0].1, 2);
        // E10: shared groups are checked.
        let shared = "shared_examples 'x' do\n  let(:a) { 1 }\n  let(:b) { 1 }\nend\n";
        let groups = mmh(shared, 1, true);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1, 2);
    }

    #[test]
    fn mmh_subject_counting_respects_allow_subject() {
        let src = "describe 'x' do\n  subject(:bar) { 1 }\n  let(:foo) { 2 }\nend\n";
        // AllowSubject true (default): subjects ignored -> 1, no offense.
        assert_eq!(mmh(src, 1, true), Vec::new());
        // AllowSubject false: subject counts -> 2.
        let groups = mmh(src, 1, false);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1, 2);
    }

    #[test]
    fn mmh_dsym_dstr_items_are_left_for_the_wrapper() {
        // E11: two dstr names differ only in interpolation whitespace ->
        // Rust cannot dedup them, so it passes both ranges (rust_distinct 0)
        // and gates on the upper bound 2 > 1.
        let src = "describe 'x' do\n  let(\"a#{ b }\") { 1 }\n  let(\"a#{b}\") { 2 }\nend\n";
        let groups = mmh(src, 1, true);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1, 0);
        assert_eq!(groups[0].2, vec!["\"a#{ b }\"".to_string(), "\"a#{b}\"".to_string()]);
        // A dsym + dstr + sym under Max 2: rust_distinct 1 (:c) + 2 dyn = 3.
        let mixed = "describe 'x' do\n  let(:\"a#{x}\") { 1 }\n  let(\"a#{x}\") { 2 }\n  let(:c) { 3 }\nend\n";
        let groups = mmh(mixed, 2, true);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].1, 1);
        assert_eq!(groups[0].2.len(), 2);
    }

    #[test]
    fn mmh_ignores_a_reasonable_number() {
        let src = "describe 'x' do\n  let(:a) { 1 }\nend\n";
        assert_eq!(mmh(src, 1, true), Vec::new());
        // Distributed lets in sibling contexts do not accumulate.
        let distributed = "describe 'x' do\n  context 'a' do\n    let(:a) { 1 }\n  end\n  context 'b' do\n    let(:b) { 1 }\n  end\nend\n";
        assert_eq!(mmh(distributed, 1, true), Vec::new());
    }

    // --- RepeatedDescription / RepeatedExample example collection ---

    /// Per emitted group, the source slices of the collected example blocks.
    /// Both cops read the same data, so one helper covers both.
    fn re(src: &str) -> Vec<Vec<String>> {
        let groups = check_rspec_repeated_description(src.as_bytes(), &cfg());
        // The other slot must carry identical content.
        assert_eq!(
            groups,
            check_rspec_repeated_example(src.as_bytes(), &cfg())
        );
        groups
            .into_iter()
            .map(|g| {
                g.into_iter()
                    .map(|(s, e)| String::from_utf8_lossy(&src.as_bytes()[s..e]).into_owned())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn re_collects_the_examples_of_an_example_group() {
        let src = "describe 'x' do\n  it('a') { foo }\n  it('b') { bar }\nend\n";
        assert_eq!(re(src), vec![vec!["it('a') { foo }", "it('b') { bar }"]]);
    }

    #[test]
    fn re_gates_on_example_groups_only() {
        // A shared group is NOT an example group -> no emission even with two
        // examples (probed B1). include_* blocks likewise (probed B8).
        assert!(re("shared_examples 'x' do\n  it('a') { foo }\n  it('b') { bar }\nend\n").is_empty());
        assert!(
            re("include_examples 'x' do\n  it('a') { foo }\n  it('b') { bar }\nend\n").is_empty()
        );
    }

    #[test]
    fn re_needs_at_least_two_examples() {
        assert!(re("describe 'x' do\n  it('a') { foo }\nend\n").is_empty());
    }

    #[test]
    fn re_numblock_group_is_transparent_and_never_a_group() {
        // The numblock context opens no frame: its examples belong to the
        // outer describe, and the numblock itself is never emitted (probed).
        let src = "describe 'x' do\n  it('a') { foo }\n  context('y') {\n    _1\n    it('b') { bar }\n  }\nend\n";
        assert_eq!(re(src), vec![vec!["it('a') { foo }", "it('b') { bar }"]]);
    }

    #[test]
    fn re_a_numblock_example_is_not_an_example() {
        // `it('a') { _1 }` is a numblock, not a plain-block example (probed).
        assert!(re("describe 'x' do\n  it('a') { _1 }\n  it('b') { _1 }\nend\n").is_empty());
    }

    #[test]
    fn re_nested_example_halts_collection() {
        // An example inside an example belongs to the inner example's frame,
        // never the group (probed B4).
        let src = "describe 'x' do\n  it('a') do\n    it('b') { foo }\n  end\nend\n";
        assert!(re(src).is_empty());
    }

    #[test]
    fn re_example_inside_a_let_body_belongs_to_the_group() {
        // A `let` frame is not an examples barrier, so the example in its body
        // pairs with the sibling (probed B2).
        let src = "describe 'x' do\n  it('a') { foo }\n  let(:y) do\n    it('b') { bar }\n  end\nend\n";
        assert_eq!(re(src), vec![vec!["it('a') { foo }", "it('b') { bar }"]]);
    }

    #[test]
    fn re_nested_group_examples_belong_to_the_inner_group() {
        // A context is a scope change: its example belongs to the context, not
        // the describe. Each group then has one example -> no emission.
        let src = "describe 'x' do\n  it('a') { foo }\n  context 'y' do\n    it('b') { bar }\n  end\nend\n";
        assert!(re(src).is_empty());
        // Two examples in the inner context -> the context is emitted.
        let src2 = "describe 'x' do\n  it('a') { foo }\n  context 'y' do\n    it('b') { bar }\n    it('c') { baz }\n  end\nend\n";
        assert_eq!(re(src2), vec![vec!["it('b') { bar }", "it('c') { baz }"]]);
    }

    // --- NamedSubject (probed against stock rubocop-rspec 3.10.2) ---

    fn cfg_ns(style: u8, ignore_shared: bool) -> RSpecConfig {
        let mut c = cfg();
        c.named_subject_style = style;
        c.named_subject_ignore_shared = ignore_shared;
        c
    }

    /// The `subject` selector source slices reported by NamedSubject.
    fn ns(src: &str, style: u8, ignore_shared: bool) -> Vec<String> {
        check_rspec_named_subject(src.as_bytes(), &cfg_ns(style, ignore_shared))
            .into_iter()
            .map(|(s, e)| String::from_utf8_lossy(&src.as_bytes()[s..e]).into_owned())
            .collect()
    }

    #[test]
    fn ns_flags_bare_subject_in_examples_and_hooks() {
        let src = "describe 'd' do\n  it('a') { subject.foo }\n  before { subject }\n  around(:each) do |t|\n    do_x(subject)\n  end\nend\n";
        assert_eq!(ns(src, 0, true), vec!["subject", "subject", "subject"]);
    }

    #[test]
    fn ns_needs_a_plain_block_example_or_hook_ancestor() {
        // No group at all, and a bare arg to a blockless `it` do not fire.
        assert_eq!(ns("subject.foo\n", 0, true), Vec::<String>::new());
        assert_eq!(ns("def foo\n  it(subject)\nend\n", 0, true), Vec::<String>::new());
        // A numblock example is not a plain block, so `on_block` never fires.
        assert_eq!(
            ns("describe 'd' do\n  it('a') { subject.foo; _1 }\nend\n", 0, true),
            Vec::<String>::new()
        );
    }

    #[test]
    fn ns_matches_the_literal_subject_send_only() {
        // A subject definition send inside an example matches (`subject { }`),
        // but `is_expected`, `subject(:x)` and `subject!` do not.
        let src = "describe 'd' do\n  it('a') do\n    subject { 1 }\n    is_expected.to be\n    subject(:x)\n  end\nend\n";
        assert_eq!(ns(src, 0, true), vec!["subject"]);
    }

    #[test]
    fn ns_ignore_shared_examples() {
        let shared = "describe 'd' do\n  shared_examples 'x' do\n    it('a') { subject.foo }\n  end\nend\n";
        assert_eq!(ns(shared, 0, true), Vec::<String>::new());
        assert_eq!(ns(shared, 0, false), vec!["subject"]);
        // `shared_context` is NOT a shared example -> never suppressed.
        let ctx = "describe 'd' do\n  shared_context 'x' do\n    it('a') { subject.foo }\n  end\nend\n";
        assert_eq!(ns(ctx, 0, true), vec!["subject"]);
        // An example ABOVE a shared group is not enclosed by it -> reported.
        let above = "it 'outer' do\n  shared_examples 'x' do\n    it 'inner' do\n      subject.foo\n    end\n  end\nend\n";
        assert_eq!(ns(above, 0, true), vec!["subject"]);
    }

    #[test]
    fn ns_named_only_uses_the_nearest_subject_definition() {
        let unnamed = "RSpec.describe User do\n  subject { x }\n  it('a') { subject.foo }\nend\n";
        assert_eq!(ns(unnamed, 1, true), Vec::<String>::new());
        assert_eq!(ns(unnamed, 0, true), vec!["subject"]);
        let named = "RSpec.describe User do\n  subject(:u) { x }\n  it('a') { subject.foo }\nend\n";
        assert_eq!(ns(named, 1, true), vec!["subject"]);
        // The innermost declaration wins even when an outer one is named.
        let nearest = "RSpec.describe User do\n  subject(:u) { x }\n  describe 'age' do\n    subject { u.age }\n    it('a') { subject.foo }\n  end\nend\n";
        assert_eq!(ns(nearest, 1, true), Vec::<String>::new());
        // subject! definitions count for the nearest resolution too.
        let bang = "RSpec.describe User do\n  subject! { x }\n  it('a') { subject.foo }\nend\n";
        assert_eq!(ns(bang, 1, true), Vec::<String>::new());
    }

    #[test]
    fn re_transparent_dsl_block_passes_examples_to_the_group() {
        // `%i[...].each do ... end` is a plain block but not a scope change, so
        // its examples belong to the describe (vendor: repeated in iterator).
        let src = "describe 'x' do\n  %i[a b].each do |t|\n    it(\"d#{t}\") { foo }\n    it(\"d#{t}\") { bar }\n  end\nend\n";
        let groups = re(src);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }

    // --- candidate emission for the R2 metadata-family cops ---

    fn slices(src: &str, ranges: Vec<(usize, usize)>) -> Vec<String> {
        ranges
            .into_iter()
            .map(|(s, e)| String::from_utf8_lossy(&src.as_bytes()[s..e]).into_owned())
            .collect()
    }

    fn anchors(src: &str) -> Vec<String> {
        slices(src, check_rspec_metadata_anchors(src.as_bytes(), &cfg()))
    }

    fn focus(src: &str) -> Vec<String> {
        slices(src, check_rspec_focus(src.as_bytes(), &cfg()))
    }

    fn pending(src: &str) -> Vec<String> {
        slices(src, check_rspec_pending_without_reason(src.as_bytes(), &cfg()))
    }

    #[test]
    fn metadata_anchor_covers_every_block_kind() {
        // Every block kind is emitted; the wrapper's parser matcher filters
        // block kind. Prism call range == parser block node range.
        let src = "RSpec.describe 'x' do\n  it('plain', a: true) { foo }\n  it('num', b: true) { _1 }\n  it('it', c: true) { it.foo }\n  context('ctx', d: true) { bar }\nend\n";
        let a = anchors(src);
        // Outer describe (1 arg) + all four inner blocks (all kinds).
        assert!(a.iter().any(|s| s.starts_with("RSpec.describe 'x'")));
        assert!(a.iter().any(|s| s.starts_with("it('plain', a: true)")));
        assert!(a.iter().any(|s| s.starts_with("it('num', b: true)")));
        assert!(a.iter().any(|s| s.starts_with("it('it', c: true)")));
        assert!(a.iter().any(|s| s.starts_with("context('ctx', d: true)")));
    }

    #[test]
    fn metadata_anchor_needs_arg_and_rspec_receiver() {
        // Zero-arg blocks and non-rspec receivers are not anchors.
        let src = "describe 'x' do\n  it { foo }\n  before { bar }\n  Foo.describe('y', a: true) { baz }\nend\n";
        // Only the outer describe (1 arg, rspec recv) qualifies.
        assert_eq!(anchors(src), vec!["describe 'x' do\n  it { foo }\n  before { bar }\n  Foo.describe('y', a: true) { baz }\nend".to_string()]);
    }

    #[test]
    fn metadata_anchor_hooks_and_shared_groups_and_configure() {
        let src = "describe 'x' do\n  before(:each, a: true) { foo }\nend\nshared_examples 'y', b: true do\n  it { foo }\nend\nRSpec.configure do |c|\n  c.before(:each, d: true) { bar }\nend\n";
        let a = anchors(src);
        assert!(a.iter().any(|s| s.starts_with("before(:each, a: true)")));
        assert!(a.iter().any(|s| s.starts_with("shared_examples 'y', b: true")));
        assert!(a.iter().any(|s| s.starts_with("RSpec.configure")));
    }

    #[test]
    fn focus_candidates_cover_aliases_and_metadata() {
        let src = "RSpec.describe MyClass, focus: true do\n  describe 'a', :focus do\n    fit 'b' do; end\n    fdescribe 'c' do; end\n    focus 'd' do; end\n    it 'e', :focus do; end\n    xit 'f', :focus do; end\n    shared_examples 'g', focus: true do; end\n  end\n  foo.fdescribe 'chained' do; end\nend\n";
        let f = focus(src);
        // Every stock offender is a candidate; the non-rspec `foo.fdescribe` is not.
        assert!(f.iter().any(|s| s.starts_with("RSpec.describe MyClass, focus: true")));
        assert!(f.iter().any(|s| s == "describe 'a', :focus"));
        assert!(f.iter().any(|s| s == "fit 'b'"));
        assert!(f.iter().any(|s| s == "fdescribe 'c'"));
        assert!(f.iter().any(|s| s == "focus 'd'"));
        assert!(f.iter().any(|s| s == "it 'e', :focus"));
        assert!(f.iter().any(|s| s == "xit 'f', :focus"));
        assert!(f.iter().any(|s| s == "shared_examples 'g', focus: true"));
        assert!(!f.iter().any(|s| s.contains("chained")));
    }

    #[test]
    fn focus_ignores_plain_examples_without_focus_metadata() {
        let src = "describe 'x' do\n  it 'a' do; end\n  it 'b', slow: true do; end\nend\n";
        assert_eq!(focus(src), Vec::<String>::new());
    }

    #[test]
    fn pending_candidates_cover_names_and_metadata() {
        let src = "RSpec.describe 'x' do\n  pending 'p' do; end\n  it 'a', :pending do; end\n  it 'b' do; pending; end\n  xdescribe 'c' do; end\n  skip 'd' do; end\n  it 'e', :skip do; end\n  it 'f' do; skip; end\n  it 'g'\n  it 'h', pending: 'reason' do; end\n  xit 'j' do; end\nend\n";
        let p = pending(src);
        assert!(p.iter().any(|s| s == "pending 'p'"));
        assert!(p.iter().any(|s| s == "it 'a', :pending"));
        assert!(p.iter().any(|s| s == "pending")); // bare in example body
        assert!(p.iter().any(|s| s == "xdescribe 'c'"));
        assert!(p.iter().any(|s| s == "skip 'd'"));
        assert!(p.iter().any(|s| s == "it 'e', :skip"));
        assert!(p.iter().any(|s| s == "skip")); // bare in example body
        assert!(p.iter().any(|s| s == "xit 'j'"));
        // `it 'h', pending: 'reason'` carries pending metadata -> candidate
        // (the wrapper drops it because the value is a string, not true).
        assert!(p.iter().any(|s| s == "it 'h', pending: 'reason'"));
        // A plain regular example without a body is never a candidate.
        assert!(!p.iter().any(|s| s == "it 'g'"));
    }

    // --- DescribedClass candidate emission ---

    fn dc(src: &str) -> Vec<String> {
        slices(src, check_rspec_described_class(src.as_bytes(), &cfg()))
    }

    #[test]
    fn described_class_identifies_describe_const_blocks() {
        let src = "describe MyClass do\n  subject { MyClass }\nend\n";
        let result = dc(src);
        assert_eq!(result.len(), 1);
        assert!(result[0].starts_with("describe MyClass do"));
    }

    #[test]
    fn described_class_ignores_describe_string() {
        let src = "describe 'MyClass' do\n  subject { MyClass }\nend\n";
        assert!(dc(src).is_empty());
    }

    #[test]
    fn described_class_identifies_nested_const_path() {
        let src = "describe A::B do\n  it { A::B }\nend\n";
        let result = dc(src);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn described_class_identifies_cbase_const() {
        let src = "describe ::MyClass do\n  it { ::MyClass }\nend\n";
        let result = dc(src);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn described_class_ignores_shared_groups_with_const() {
        let src = "shared_examples MyClass do\n  it { MyClass }\nend\n";
        assert!(dc(src).is_empty());
    }
}
