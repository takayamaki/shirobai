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

use std::collections::HashMap;

use ruby_prism::{CallNode, Node};

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

/// Everything the RSpec cops produced for one file.
#[derive(Debug, Default)]
pub struct RSpecResult {
    /// (style-failing candidates, style-passing (value, kind) pairs).
    /// The passing list feeds the wrapper's `correct_style_detected`
    /// bookkeeping, filtered by AllowedPatterns on the Ruby side.
    pub variable_name: (Vec<VarNameOffense>, Vec<(String, u8)>),
    /// `RSpec/LetSetup` offenses: the `let!` send node ranges.
    pub let_setup: Vec<(usize, usize)>,
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
}

/// One parser-block frame in the (post-walk) arena.
struct Scope {
    parent: Option<usize>,
    /// Full node range (prism call range == parser block node range).
    range: (usize, usize),
    /// `(block {(send #rspec? {SG|EG} ...) (send nil? #Includes.all ...)})`
    /// — halts every role collection; also the `LetSetup` query-root set.
    scope_change: bool,
    /// `example?` — halts every role collection.
    example: bool,
    /// The frame is itself a `let?` node — halts the lets collection only.
    let_barrier: bool,
    /// Collected `let?` items (indexes into `let_items`), document order.
    lets: Vec<u32>,
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
    /// Positive while inside a top-level statement that is `spec_group?`
    /// (any_block; see module doc on `inside_example_group?`).
    top_spec_depth: u32,
    /// Receiverless zero-argument non-block-pass call sites:
    /// name -> start offsets (walk order = ascending).
    zero_arg_calls: HashMap<Vec<u8>, Vec<usize>>,
    vn_offenses: Vec<VarNameOffense>,
    vn_passing: Vec<(String, u8)>,
}

pub fn build_rule<'c>(cfg: &'c RSpecConfig) -> RSpecDispatcherRule<'c> {
    RSpecDispatcherRule {
        cfg,
        stack: Vec::with_capacity(64),
        scope_stack: Vec::with_capacity(16),
        scopes: Vec::new(),
        let_items: Vec::new(),
        top_spec_depth: 0,
        zero_arg_calls: HashMap::new(),
        vn_offenses: Vec::new(),
        vn_passing: Vec::new(),
    }
}

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
        }

        if role_mask == 0 {
            return entry;
        }
        let rspec_recv = recv_none || recv.as_ref().is_some_and(|r| rspec_const(r));

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

        // parser-block frames: barriers + collection roots.
        if kind == BlockKind::Plain {
            let spec_group_send =
                rspec_recv && role_mask & (roles::EG_ALL | roles::SG_ALL) != 0;
            let include_block = recv_none && role_mask & roles::INC_ALL != 0;
            let example = recv_none && role_mask & roles::EX_ALL != 0;
            let loc = call.location();
            self.scopes.push(Scope {
                parent: self.scope_stack.last().copied(),
                range: (loc.start_offset(), loc.end_offset()),
                scope_change: spec_group_send || include_block,
                example,
                let_barrier: is_let,
                lets: Vec::new(),
            });
            self.scope_stack.push(self.scopes.len() - 1);
            entry.opened_scope = true;
        }
        entry
    }

    /// `(send nil? {#Subjects.all #Helpers.all} $({any_sym str dstr} ...)
    /// ...)` under a top-level spec group. Only sym/str feed
    /// `RSpec/VariableName` (stock skips dstr/dsym).
    fn variable_candidate(&mut self, call: &CallNode<'_>) {
        let Some(args) = call.arguments() else { return };
        let Some(first) = args.arguments().iter().next() else {
            return;
        };
        let (vkind, value) = if let Some(s) = first.as_symbol_node() {
            (0u8, String::from_utf8_lossy(s.unescaped()).into_owned())
        } else if let Some(s) = first.as_string_node() {
            (1u8, String::from_utf8_lossy(s.unescaped()).into_owned())
        } else {
            // dstr / dsym / non-literal: no VariableName candidate.
            return;
        };
        let style = self.cfg.variable_name_style;
        if matches_style(&value, style) {
            self.vn_passing.push((value, vkind));
        } else {
            let loc = first.location();
            let valid_alt = matches_style(&value, 1 - style);
            self.vn_offenses.push(VarNameOffense {
                start: loc.start_offset(),
                end: loc.end_offset(),
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
        RSpecResult {
            variable_name: (self.vn_offenses, self.vn_passing),
            let_setup,
        }
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
}
