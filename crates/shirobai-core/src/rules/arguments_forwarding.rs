//! `Style/ArgumentsForwarding`.
//!
//! Stock's cop is `on_def` / `on_defs`: for every method definition it walks
//! `each_descendant(:call, :super, :yield)`, classifies each call against the
//! def's rest / kwrest / block parameters, and (when the whole def forwards
//! everything) collapses the arguments to `...`, or (Ruby >= 3.2) anonymizes
//! individual `*` / `**` / `&`.
//!
//! shirobai reproduces this on the shared walk with a stack of per-def
//! collectors. A def's classification needs the COMPLETE set of local-variable
//! references in its body first (a bare `args` reference blocks forwarding), so
//! we cannot classify a call the moment we see it: we snapshot every call /
//! super / yield (owned byte data), collect every referenced lvar name, and run
//! the whole `on_def` logic when the def node closes.
//!
//! A call is a descendant of every enclosing def (parser `each_descendant` is
//! recursive), so each snapshot is pushed into every active collector; each def
//! classifies it against its own parameters. Nested defs are therefore
//! classified independently, exactly like stock.
//!
//! The result mirrors stock's exact `add_offense` call sequence (with the
//! duplicates stock emits — RuboCop dedups offenses by range through a Set, so
//! a repeated range's corrector is dropped). Each offense carries the message
//! and the precise corrector op stream (`add_parentheses` reproduced byte for
//! byte, including the double `)` / `((` that stock produces when two
//! different-range offenses each add parentheses to a paren-less def / call).
//!
//! prism vs parser node shapes (probed):
//! - def `(...)`  -> `ForwardingParameterNode` (no rest/kwrest/block: inert)
//! - def anon `*` / `**` / `&` -> `RestParameterNode` / `KeywordRestParameterNode`
//!   / `BlockParameterNode` with `name == None`
//! - call `*args` -> `SplatNode(LocalVariableReadNode)`; anon `*` ->
//!   `SplatNode(expression: None)` (parser `forwarded_restarg`)
//! - call `**kwargs` -> `KeywordHashNode`/`HashNode` element
//!   `AssocSplatNode(LocalVariableReadNode)`; anon `**` -> `AssocSplatNode(value: None)`
//! - call `&block` -> `BlockArgumentNode(LocalVariableReadNode)`; anon `&` /
//!   `&` -> `BlockArgumentNode(expression: None)` (parser `block_pass nil`)
//! - `super(...)` -> `SuperNode`; bare `super` / zsuper -> `ForwardingSuperNode`
//!   (NOT in the call list); `yield(...)` -> `YieldNode`.

use std::collections::HashSet;

use ruby_prism::{CallNode, Node, Visit};

use super::dispatch::{self, Interest, Rule};

/// Config, all resolved on the Ruby side from `AllCops/TargetRubyVersion` and
/// the cop's own config (plus `Naming/BlockForwarding`).
#[derive(Clone)]
pub struct Config {
    /// `target_ruby_version * 10` rounded (2.7 -> 27, 3.2 -> 32, 3.4 -> 34).
    pub target_ruby: i64,
    pub allow_only_rest_arguments: bool,
    pub use_anonymous_forwarding: bool,
    pub explicit_block_name: bool,
    /// Redundant argument SOURCE strings (`*args`, `*`, ...): a named param is
    /// only anonymizable when its source is in this set.
    pub redundant_rest: Vec<String>,
    pub redundant_kwrest: Vec<String>,
    pub redundant_block: Vec<String>,
}

/// One `add_offense` call in stock's order (range + message + corrector ops).
pub struct AfOffense {
    pub start: usize,
    pub end: usize,
    /// 0 FORWARDING, 1 ARGS, 2 KWARGS, 3 BLOCK.
    pub message: u8,
    pub ops: Vec<AfOp>,
}

/// A corrector op. `kind`: 0 replace, 1 remove, 2 insert_before, 3 insert_after.
pub struct AfOp {
    pub kind: u8,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

impl AfOp {
    fn replace(start: usize, end: usize, text: &str) -> AfOp {
        AfOp { kind: 0, start, end, text: text.to_string() }
    }
    fn remove(start: usize, end: usize) -> AfOp {
        AfOp { kind: 1, start, end, text: String::new() }
    }
    fn insert_before(pos: usize, text: &str) -> AfOp {
        AfOp { kind: 2, start: pos, end: pos, text: text.to_string() }
    }
    fn insert_after(pos: usize, text: &str) -> AfOp {
        AfOp { kind: 3, start: pos, end: pos, text: text.to_string() }
    }
}

// ---------------------------------------------------------------------------
// Snapshots
// ---------------------------------------------------------------------------

struct KwsplatInfo {
    begin: usize,
    end: usize,
    lvar: Option<String>,
    anon: bool,
}

struct ArgInfo {
    begin: usize,
    end: usize,
    is_splat: bool,
    splat_lvar: Option<String>,
    splat_anon: bool,
    is_hash: bool,
    hash_n_elements: usize,
    hash_kwsplat: Option<KwsplatInfo>,
    is_block_pass: bool,
    bp_lvar: Option<String>,
    bp_anon: bool,
}

#[derive(PartialEq, Clone, Copy)]
enum SendKind {
    Call,
    Super,
    Yield,
}

struct SendSnapshot {
    #[allow(dead_code)]
    kind: SendKind,
    is_index: bool,
    send_begin: usize,
    send_end: usize,
    has_parens: bool,
    selector_end: usize,
    args: Vec<ArgInfo>,
    in_block: bool,
}

struct ParamInfo {
    begin: usize,
    end: usize,
    name: Option<String>,
}

struct DefInfo {
    params_begin: usize,
    params_end: usize,
    has_parens: bool,
    last_param_end: usize,
    /// Redundant-named (forwardable) rest / kwrest / block, else None.
    rest: Option<ParamInfo>,
    kwrest: Option<ParamInfo>,
    block: Option<ParamInfo>,
    has_optarg: bool,
    has_kwarg: bool,
    all_anonymous: bool,
    n_params: usize,
}

struct Collector {
    def: DefInfo,
    send_indices: Vec<usize>,
    referenced: HashSet<String>,
    /// Byte range of the def's BODY. Stock collects `referenced_lvars` from
    /// `node.body` ONLY (an optarg/kwoptarg default expression that reads a
    /// rest/kwrest/block name does not block forwarding), while `send_nodes`
    /// come from the whole def. So only references are body-filtered.
    body_begin: usize,
    body_end: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Classification {
    All,
    AllAnon,
    RestOrKwrest,
}

/// A resolved forwarded argument on the send side (byte range + arg index).
#[derive(Clone, Copy)]
struct Fwd {
    arg_index: usize,
    begin: usize,
    end: usize,
    /// Number of hash elements (kwrest only; for `forward_additional_kwargs?`).
    hash_n: usize,
}

struct SendClass {
    idx: usize,
    classification: Classification,
    fwd_rest: Option<Fwd>,
    fwd_kwrest: Option<Fwd>,
    fwd_block: Option<Fwd>,
}

// ---------------------------------------------------------------------------
// Rule
// ---------------------------------------------------------------------------

pub struct ArgumentsForwardingRule<'s> {
    source: &'s [u8],
    cfg: Config,
    rest_redundant: HashSet<String>,
    kwrest_redundant: HashSet<String>,
    block_redundant: HashSet<String>,
    /// Node-kind stack (parent tracking for leaf lvar reads + block depth).
    frames: Vec<Frame>,
    block_depth: usize,
    /// Active def collectors (index into `collectors` by stack position).
    def_stack: Vec<usize>,
    collectors: Vec<Collector>,
    sends: Vec<SendSnapshot>,
    /// Start offsets of every `LocalVariableTargetNode` that lives inside a
    /// pattern-match pattern (`case/in`, `expr => pat`, `expr in pat`). prism
    /// reuses `LocalVariableTargetNode` for both `masgn`/`for`/`rescue` targets
    /// (parser `:lvasgn`, a body reference) and pattern bindings (parser
    /// `:match_var`, NOT a body reference). Stock's `each_descendant(:lvar,
    /// :lvasgn)` skips the latter, so we skip a target whose offset is here.
    pattern_target_skips: HashSet<usize>,
    pub offenses: Vec<AfOffense>,
}

enum Frame {
    /// A def; carries its collector index.
    Def(usize),
    /// Block / lambda (bumps `block_depth`).
    Block,
    /// Splat / kwsplat(assoc_splat) / block_pass: an lvar child here is a
    /// forwarding reference and must NOT count as a plain reference.
    ForwardingCarrier,
    Other,
}

pub fn build_rule<'s>(source: &'s [u8], cfg: &Config) -> ArgumentsForwardingRule<'s> {
    let redundant_set = |names: &[String], kw: &str| -> HashSet<String> {
        let mut s: HashSet<String> = names.iter().map(|n| format!("{kw}{n}")).collect();
        s.insert(kw.to_string());
        s
    };
    ArgumentsForwardingRule {
        source,
        rest_redundant: redundant_set(&cfg.redundant_rest, "*"),
        kwrest_redundant: redundant_set(&cfg.redundant_kwrest, "**"),
        block_redundant: redundant_set(&cfg.redundant_block, "&"),
        cfg: cfg.clone(),
        frames: Vec::new(),
        block_depth: 0,
        def_stack: Vec::new(),
        collectors: Vec::new(),
        sends: Vec::new(),
        pattern_target_skips: HashSet::new(),
        offenses: Vec::new(),
    }
}

/// Collect the start offset of every `LocalVariableTargetNode` inside a
/// pattern subtree. Pins (`^foo` / `^(expr)`) reference existing variables and
/// bind nothing, so we do not descend into them.
fn collect_pattern_target_offsets(pattern: &Node<'_>, out: &mut HashSet<usize>) {
    let mut c = PatternTargetOffsetCollector { out };
    c.visit(pattern);
}

struct PatternTargetOffsetCollector<'o> {
    out: &'o mut HashSet<usize>,
}

impl<'pr> Visit<'pr> for PatternTargetOffsetCollector<'_> {
    fn visit_local_variable_target_node(
        &mut self,
        node: &ruby_prism::LocalVariableTargetNode<'pr>,
    ) {
        self.out.insert(node.as_node().location().start_offset());
        ruby_prism::visit_local_variable_target_node(self, node);
    }

    fn visit_pinned_variable_node(&mut self, _node: &ruby_prism::PinnedVariableNode<'pr>) {}

    fn visit_pinned_expression_node(&mut self, _node: &ruby_prism::PinnedExpressionNode<'pr>) {}
}

/// Standalone entry point (per-cop fallback path).
pub fn check_arguments_forwarding(source: &[u8], cfg: &Config) -> Vec<AfOffense> {
    let mut rule = build_rule(source, cfg);
    dispatch::run(source, &mut [&mut rule]);
    rule.take_offenses()
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `range_with_surrounding_space(range, side: :left, newlines: true)`:
/// move left over ` `/`\t`, then over `\n`.
fn final_pos_left(src: &[u8], mut pos: usize) -> usize {
    while pos > 0 && matches!(src[pos - 1], b' ' | b'\t') {
        pos -= 1;
    }
    while pos > 0 && src[pos - 1] == b'\n' {
        pos -= 1;
    }
    pos
}

impl<'s> ArgumentsForwardingRule<'s> {
    pub fn take_offenses(&mut self) -> Vec<AfOffense> {
        std::mem::take(&mut self.offenses)
    }

    fn src(&self, begin: usize, end: usize) -> &[u8] {
        &self.source[begin..end]
    }

    // --- snapshotting -----------------------------------------------------

    fn arg_info(&self, node: &Node<'_>) -> ArgInfo {
        let loc = node.location();
        let (begin, end) = (loc.start_offset(), loc.end_offset());
        let mut info = ArgInfo {
            begin,
            end,
            is_splat: false,
            splat_lvar: None,
            splat_anon: false,
            is_hash: false,
            hash_n_elements: 0,
            hash_kwsplat: None,
            is_block_pass: false,
            bp_lvar: None,
            bp_anon: false,
        };
        if let Some(s) = node.as_splat_node() {
            info.is_splat = true;
            match s.expression() {
                None => info.splat_anon = true,
                Some(e) => {
                    if let Some(lv) = e.as_local_variable_read_node() {
                        info.splat_lvar = Some(lossy(lv.name().as_slice()));
                    }
                }
            }
        } else if let Some(b) = node.as_block_argument_node() {
            info.is_block_pass = true;
            match b.expression() {
                None => info.bp_anon = true,
                Some(e) => {
                    if let Some(lv) = e.as_local_variable_read_node() {
                        info.bp_lvar = Some(lossy(lv.name().as_slice()));
                    }
                }
            }
        } else {
            // Hash carrier? braceless kwargs (KeywordHashNode) or braced (HashNode).
            let elements: Option<Vec<Node<'_>>> = if let Some(h) = node.as_keyword_hash_node() {
                Some(h.elements().iter().collect())
            } else {
                node.as_hash_node().map(|h| h.elements().iter().collect())
            };
            if let Some(elements) = elements {
                info.is_hash = true;
                info.hash_n_elements = elements.len();
                for el in &elements {
                    if let Some(a) = el.as_assoc_splat_node() {
                        let al = a.as_node().location();
                        let mut ks = KwsplatInfo {
                            begin: al.start_offset(),
                            end: al.end_offset(),
                            lvar: None,
                            anon: false,
                        };
                        match a.value() {
                            None => ks.anon = true,
                            Some(v) => {
                                if let Some(lv) = v.as_local_variable_read_node() {
                                    ks.lvar = Some(lossy(lv.name().as_slice()));
                                }
                            }
                        }
                        info.hash_kwsplat = Some(ks);
                        break;
                    }
                }
            }
        }
        info
    }

    fn collect_args(&self, arguments: Option<ruby_prism::ArgumentsNode<'_>>, block: Option<Node<'_>>) -> Vec<ArgInfo> {
        let mut args: Vec<ArgInfo> = Vec::new();
        if let Some(a) = arguments {
            for arg in a.arguments().iter() {
                args.push(self.arg_info(&arg));
            }
        }
        // A block_pass (`&x`) is a trailing argument in parser; prism keeps it
        // in the block slot. A literal block (`{ }` / `do end`) is not.
        if let Some(b) = block
            && b.as_block_argument_node().is_some()
        {
            args.push(self.arg_info(&b));
        }
        args
    }

    fn snapshot_call(&mut self, call: &CallNode<'_>, own_block: bool) {
        let loc = call.location();
        let name = call.name().as_slice();
        let selector_end = call
            .message_loc()
            .map(|m| m.end_offset())
            .unwrap_or(loc.start_offset());
        let args = self.collect_args(call.arguments(), call.block());
        self.sends.push(SendSnapshot {
            kind: SendKind::Call,
            // `add_parens_if_missing` exempts only `node.method?(:[])`.
            is_index: name == b"[]",
            send_begin: loc.start_offset(),
            send_end: loc.end_offset(),
            has_parens: call.closing_loc().is_some_and(|c| self.src(c.start_offset(), c.end_offset()) == b")"),
            selector_end,
            args,
            in_block: self.block_depth > 0 || own_block,
        });
    }

    fn snapshot_super(&mut self, sup: &ruby_prism::SuperNode<'_>, own_block: bool) {
        let loc = sup.location();
        let args = self.collect_args(sup.arguments(), sup.block());
        self.sends.push(SendSnapshot {
            kind: SendKind::Super,
            is_index: false,
            send_begin: loc.start_offset(),
            send_end: loc.end_offset(),
            has_parens: sup.rparen_loc().is_some(),
            selector_end: sup.keyword_loc().end_offset(),
            args,
            in_block: self.block_depth > 0 || own_block,
        });
    }

    fn snapshot_yield(&mut self, y: &ruby_prism::YieldNode<'_>) {
        let loc = y.location();
        let args = self.collect_args(y.arguments(), None);
        self.sends.push(SendSnapshot {
            kind: SendKind::Yield,
            is_index: false,
            send_begin: loc.start_offset(),
            send_end: loc.end_offset(),
            has_parens: y.rparen_loc().is_some(),
            selector_end: y.keyword_loc().end_offset(),
            args,
            in_block: self.block_depth > 0,
        });
    }

    fn register_send(&mut self, idx: usize) {
        for &ci in &self.def_stack {
            self.collectors[ci].send_indices.push(idx);
        }
    }

    /// Record a plain lvar reference at byte offset `at`. Stock scans
    /// `node.body` only, so each active def counts the reference only when it
    /// falls inside that def's body range (a reference in a param default
    /// expression is outside it).
    fn record_reference(&mut self, name: String, at: usize) {
        for &ci in &self.def_stack {
            let c = &mut self.collectors[ci];
            if c.body_begin <= at && at < c.body_end {
                c.referenced.insert(name.clone());
            }
        }
    }

    // --- def analysis (run at def leave) ----------------------------------

    fn make_def_info(&self, def: &ruby_prism::DefNode<'_>) -> Option<DefInfo> {
        let params = def.parameters()?;
        let raw_rest = params.rest().and_then(|r| {
            r.as_rest_parameter_node().map(|n| {
                let l = n.as_node().location();
                (l.start_offset(), l.end_offset(), n.name().map(|c| lossy(c.as_slice())))
            })
        });
        let raw_kwrest = params.keyword_rest().and_then(|r| {
            r.as_keyword_rest_parameter_node().map(|n| {
                let l = n.as_node().location();
                (l.start_offset(), l.end_offset(), n.name().map(|c| lossy(c.as_slice())))
            })
        });
        let raw_block = params.block().map(|n| {
            let l = n.as_node().location();
            (l.start_offset(), l.end_offset(), n.name().map(|c| lossy(c.as_slice())))
        });

        let redundant = |raw: &Option<(usize, usize, Option<String>)>, set: &HashSet<String>| -> Option<ParamInfo> {
            let (b, e, name) = raw.as_ref()?;
            let source = lossy(self.src(*b, *e));
            if set.contains(&source) {
                Some(ParamInfo { begin: *b, end: *e, name: name.clone() })
            } else {
                None
            }
        };

        let posts_empty = params.posts().iter().count() == 0;
        let keywords_empty = params.keywords().iter().count() == 0;
        let all_anonymous = posts_empty
            && keywords_empty
            && raw_rest.as_ref().is_some_and(|(_, _, n)| n.is_none())
            && raw_kwrest.as_ref().is_some_and(|(_, _, n)| n.is_none())
            && raw_block.as_ref().is_some_and(|(_, _, n)| n.is_none());

        let n_params = params.requireds().iter().count()
            + params.optionals().iter().count()
            + usize::from(params.rest().is_some())
            + params.posts().iter().count()
            + params.keywords().iter().count()
            + usize::from(params.keyword_rest().is_some())
            + usize::from(params.block().is_some());

        // Last param end (for the def-side forward-all `arg_range` end,
        // stock's `node.last_argument`).
        let mut last_param_end = params.location().end_offset();
        for child in [&raw_rest, &raw_kwrest, &raw_block].into_iter().flatten() {
            last_param_end = last_param_end.max(child.1);
        }

        let ploc = params.location();
        Some(DefInfo {
            params_begin: ploc.start_offset(),
            params_end: ploc.end_offset(),
            has_parens: def.rparen_loc().is_some(),
            last_param_end,
            rest: redundant(&raw_rest, &self.rest_redundant),
            kwrest: redundant(&raw_kwrest, &self.kwrest_redundant),
            block: redundant(&raw_block, &self.block_redundant),
            has_optarg: params.optionals().iter().count() > 0,
            has_kwarg: params.keywords().iter().count() > 0,
            all_anonymous,
            n_params,
        })
    }

    fn finalize_collector(&mut self, ci: usize) {
        // Move data out to avoid borrow conflicts.
        let send_indices = std::mem::take(&mut self.collectors[ci].send_indices);
        let referenced = std::mem::take(&mut self.collectors[ci].referenced);
        let def = &self.collectors[ci].def;

        let mut classes: Vec<SendClass> = Vec::new();
        for &si in &send_indices {
            if let Some(c) = self.classify(def, &self.sends[si], &referenced, si) {
                classes.push(c);
            }
        }
        if classes.is_empty() {
            return;
        }

        let only_all = classes
            .iter()
            .all(|c| matches!(c.classification, Classification::All | Classification::AllAnon));

        let mut new_offenses: Vec<AfOffense> = Vec::new();
        if only_all {
            self.add_forward_all_offenses(def, &classes, &mut new_offenses);
        } else if self.cfg.target_ruby >= 32 {
            self.add_post_ruby_32_offenses(def, &classes, &mut new_offenses);
        }
        self.offenses.append(&mut new_offenses);
    }

    fn classify(
        &self,
        def: &DefInfo,
        send: &SendSnapshot,
        referenced: &HashSet<String>,
        idx: usize,
    ) -> Option<SendClass> {
        let rest_name = def.rest.as_ref().and_then(|p| p.name.clone());
        let kwrest_name = def.kwrest.as_ref().and_then(|p| p.name.clone());
        let block_name = def.block.as_ref().and_then(|p| p.name.clone());

        let referenced_rest = rest_name.as_ref().is_some_and(|n| referenced.contains(n));
        let referenced_kwrest = kwrest_name.as_ref().is_some_and(|n| referenced.contains(n));
        let referenced_block = block_name.as_ref().is_some_and(|n| referenced.contains(n));

        // forwarded_rest_arg: (splat (lvar rest_name))
        let fwd_rest = if referenced_rest {
            None
        } else {
            rest_name.as_ref().and_then(|rn| {
                send.args.iter().enumerate().find_map(|(i, a)| {
                    if a.is_splat && a.splat_lvar.as_ref() == Some(rn) {
                        Some(Fwd { arg_index: i, begin: a.begin, end: a.end, hash_n: 0 })
                    } else {
                        None
                    }
                })
            })
        };
        // forwarded_kwrest_arg: (hash <$(kwsplat (lvar kwrest_name)) ...>)
        let fwd_kwrest = if referenced_kwrest {
            None
        } else {
            kwrest_name.as_ref().and_then(|kn| {
                send.args.iter().enumerate().find_map(|(i, a)| {
                    if a.is_hash
                        && let Some(ks) = &a.hash_kwsplat
                        && ks.lvar.as_ref() == Some(kn)
                    {
                        return Some(Fwd {
                            arg_index: i,
                            begin: ks.begin,
                            end: ks.end,
                            hash_n: a.hash_n_elements,
                        });
                    }
                    None
                })
            })
        };
        // forwarded_block_arg: (block_pass {(lvar block_name) nil?})
        let fwd_block = if referenced_block {
            None
        } else {
            send.args.iter().enumerate().find_map(|(i, a)| {
                if a.is_block_pass
                    && (a.bp_anon || (block_name.is_some() && a.bp_lvar == block_name))
                {
                    Some(Fwd { arg_index: i, begin: a.begin, end: a.end, hash_n: 0 })
                } else {
                    None
                }
            })
        };

        if fwd_rest.is_none() && fwd_kwrest.is_none() && fwd_block.is_none() {
            return None;
        }

        let classification = if self.ruby_32_only_anonymous(def, send) {
            Classification::AllAnon
        } else if self.can_forward_all(
            def,
            send,
            fwd_rest,
            fwd_kwrest,
            fwd_block,
            referenced_rest,
            referenced_kwrest,
            referenced_block,
        ) {
            Classification::All
        } else {
            Classification::RestOrKwrest
        };

        Some(SendClass { idx, classification, fwd_rest, fwd_kwrest, fwd_block })
    }

    fn ruby_32_only_anonymous(&self, def: &DefInfo, send: &SendSnapshot) -> bool {
        if send.in_block {
            return false;
        }
        if !def.all_anonymous {
            return false;
        }
        // send_all_anonymous_args?: last three args exactly
        // (forwarded_restarg) (hash (forwarded_kwrestarg)) (block_pass nil?)
        let n = send.args.len();
        if n < 3 {
            return false;
        }
        let a = &send.args[n - 3];
        let b = &send.args[n - 2];
        let c = &send.args[n - 1];
        a.is_splat
            && a.splat_anon
            && b.is_hash
            && b.hash_n_elements == 1
            && b.hash_kwsplat.as_ref().is_some_and(|k| k.anon)
            && c.is_block_pass
            && c.bp_anon
    }

    #[allow(clippy::too_many_arguments)]
    fn can_forward_all(
        &self,
        def: &DefInfo,
        send: &SendSnapshot,
        fwd_rest: Option<Fwd>,
        fwd_kwrest: Option<Fwd>,
        fwd_block: Option<Fwd>,
        referenced_rest: bool,
        referenced_kwrest: bool,
        referenced_block: bool,
    ) -> bool {
        if referenced_rest || referenced_kwrest || referenced_block {
            return false;
        }
        // ruby_30_or_lower_optarg?
        if self.cfg.target_ruby <= 30 && def.has_optarg {
            return false;
        }
        // ruby_32_or_higher_missing_rest_or_kwest?
        if self.cfg.target_ruby >= 32 && !(fwd_rest.is_some() && fwd_kwrest.is_some()) {
            return false;
        }
        // offensive_block_forwarding?
        let offensive = if def.block.is_some() {
            fwd_block.is_some()
        } else {
            !self.cfg.allow_only_rest_arguments
        };
        if !offensive {
            return false;
        }
        // additional_kwargs_or_forwarded_kwargs?
        let forward_additional = fwd_kwrest.is_some_and(|f| f.hash_n != 1);
        if def.has_kwarg || forward_additional {
            return false;
        }

        self.no_additional_args(def, send, fwd_rest, fwd_kwrest)
            || (self.cfg.target_ruby >= 30 && self.no_post_splat_args(send, fwd_rest))
    }

    fn no_additional_args(
        &self,
        def: &DefInfo,
        send: &SendSnapshot,
        fwd_rest: Option<Fwd>,
        fwd_kwrest: Option<Fwd>,
    ) -> bool {
        let forwardable_count = usize::from(def.rest.is_some())
            + usize::from(def.kwrest.is_some())
            + usize::from(def.block.is_some());
        let rest_name = def.rest.as_ref().and_then(|p| p.name.as_ref());
        let kwrest_name = def.kwrest.as_ref().and_then(|p| p.name.as_ref());
        let missing = (rest_name.is_some() && fwd_rest.is_none())
            || (kwrest_name.is_some() && fwd_kwrest.is_none());
        if missing {
            return false;
        }
        def.n_params == forwardable_count && send.args.len() == forwardable_count
    }

    fn no_post_splat_args(&self, send: &SendSnapshot, fwd_rest: Option<Fwd>) -> bool {
        let Some(f) = fwd_rest else { return true };
        match send.args.get(f.arg_index + 1) {
            None => true,
            Some(a) => a.is_hash || a.is_block_pass,
        }
    }

    // --- offense construction --------------------------------------------

    fn add_parens_ops_def(&self, def: &DefInfo, ops: &mut Vec<AfOp>) {
        if def.has_parens {
            return;
        }
        let leading = final_pos_left(self.source, def.params_begin);
        ops.push(AfOp::replace(leading, def.params_begin, "("));
        ops.push(AfOp::insert_after(def.params_end, ")"));
    }

    fn add_parens_ops_send(&self, send: &SendSnapshot, ops: &mut Vec<AfOp>) {
        if send.has_parens || send.is_index {
            return;
        }
        let sel = send.selector_end;
        ops.push(AfOp::remove(sel, sel + 1));
        ops.push(AfOp::insert_before(sel, "("));
        ops.push(AfOp::insert_after(send.send_end, ")"));
    }

    /// `register_forward_all_offense` on the SEND side.
    fn forward_all_send(&self, send: &SendSnapshot, first_begin: usize, out: &mut Vec<AfOffense>) {
        let start = first_begin;
        let end = send.args.last().map(|a| a.end).unwrap_or(send.send_end);
        let mut ops = Vec::new();
        self.add_parens_ops_send(send, &mut ops);
        ops.push(AfOp::replace(start, end, "..."));
        out.push(AfOffense { start, end, message: 0, ops });
    }

    /// `register_forward_all_offense` on the DEF side.
    fn forward_all_def(&self, def: &DefInfo, first_begin: usize, out: &mut Vec<AfOffense>) {
        let start = first_begin;
        let end = def.last_param_end;
        let mut ops = Vec::new();
        self.add_parens_ops_def(def, &mut ops);
        ops.push(AfOp::replace(start, end, "..."));
        out.push(AfOffense { start, end, message: 0, ops });
    }

    fn add_forward_all_offenses(&self, def: &DefInfo, classes: &[SendClass], out: &mut Vec<AfOffense>) {
        let mut registered_block_arg_offense = false;
        for c in classes {
            let send = &self.sends[c.idx];
            if c.fwd_rest.is_none()
                && c.fwd_kwrest.is_none()
                && c.classification != Classification::AllAnon
            {
                // Block-only forward-all: anonymize `&` on both sides.
                if self.allow_anon_in_block(c.fwd_block.is_some(), send.in_block) {
                    // def side (add_parens = !forward_rest = true)
                    self.forward_block_offense(true, ParenTarget::Def(def), def.block.as_ref().map(|p| (p.begin, p.end)), out);
                    // send side
                    if let Some(fb) = c.fwd_block {
                        self.forward_block_offense(true, ParenTarget::Send(send), Some((fb.begin, fb.end)), out);
                    }
                }
                registered_block_arg_offense = true;
                break;
            } else {
                let first = c
                    .fwd_rest
                    .or(c.fwd_kwrest)
                    .map(|f| f.begin)
                    .unwrap_or_else(|| self.forward_all_first_argument(send));
                self.forward_all_send(send, first, out);
            }
        }
        if registered_block_arg_offense {
            return;
        }
        let first = def
            .rest
            .as_ref()
            .or(def.kwrest.as_ref())
            .map(|p| p.begin)
            .unwrap_or(def.params_begin);
        self.forward_all_def(def, first, out);
    }

    /// `forward_all_first_argument`: last anonymous forwarded_restarg in the send.
    fn forward_all_first_argument(&self, send: &SendSnapshot) -> usize {
        send.args
            .iter()
            .rev()
            .find(|a| a.is_splat && a.splat_anon)
            .map(|a| a.begin)
            .unwrap_or(send.send_begin)
    }

    fn add_post_ruby_32_offenses(&self, def: &DefInfo, classes: &[SendClass], out: &mut Vec<AfOffense>) {
        if !self.cfg.use_anonymous_forwarding {
            return;
        }
        // all_forwarding_offenses_correctable?
        if self.cfg.target_ruby < 34 && classes.iter().any(|c| self.sends[c.idx].in_block) {
            return;
        }
        for c in classes {
            let send = &self.sends[c.idx];
            let forward_rest = c.fwd_rest.is_some();
            if self.allow_anon_in_block(c.fwd_rest.is_some(), send.in_block) {
                // def side (args always adds parens)
                self.forward_args_offense(ParenTarget::Def(def), def.rest.as_ref().map(|p| (p.begin, p.end)), out);
                if let Some(f) = c.fwd_rest {
                    self.forward_args_offense(ParenTarget::Send(send), Some((f.begin, f.end)), out);
                }
            }
            if self.allow_anon_in_block(c.fwd_kwrest.is_some(), send.in_block) {
                self.forward_kwargs_offense(!forward_rest, ParenTarget::Def(def), def.kwrest.as_ref().map(|p| (p.begin, p.end)), out);
                if let Some(f) = c.fwd_kwrest {
                    self.forward_kwargs_offense(!forward_rest, ParenTarget::Send(send), Some((f.begin, f.end)), out);
                }
            }
            if self.allow_anon_in_block(c.fwd_block.is_some(), send.in_block) {
                self.forward_block_offense(!forward_rest, ParenTarget::Def(def), def.block.as_ref().map(|p| (p.begin, p.end)), out);
                if let Some(f) = c.fwd_block {
                    self.forward_block_offense(!forward_rest, ParenTarget::Send(send), Some((f.begin, f.end)), out);
                }
            }
        }
    }

    /// `allow_anonymous_forwarding_in_block?`: node present and (>=3.4 or no
    /// block ancestor).
    fn allow_anon_in_block(&self, node_present: bool, in_block: bool) -> bool {
        if !node_present {
            return false;
        }
        if self.cfg.target_ruby >= 34 {
            return true;
        }
        !in_block
    }

    fn paren_ops(&self, target: ParenTarget<'_>, ops: &mut Vec<AfOp>) {
        match target {
            ParenTarget::Def(d) => self.add_parens_ops_def(d, ops),
            ParenTarget::Send(s) => self.add_parens_ops_send(s, ops),
        }
    }

    fn forward_args_offense(&self, target: ParenTarget<'_>, range: Option<(usize, usize)>, out: &mut Vec<AfOffense>) {
        let Some((start, end)) = range else { return };
        let mut ops = Vec::new();
        self.paren_ops(target, &mut ops);
        ops.push(AfOp::replace(start, end, "*"));
        out.push(AfOffense { start, end, message: 1, ops });
    }

    fn forward_kwargs_offense(&self, add_parens: bool, target: ParenTarget<'_>, range: Option<(usize, usize)>, out: &mut Vec<AfOffense>) {
        let Some((start, end)) = range else { return };
        let mut ops = Vec::new();
        if add_parens {
            self.paren_ops(target, &mut ops);
        }
        ops.push(AfOp::replace(start, end, "**"));
        out.push(AfOffense { start, end, message: 2, ops });
    }

    fn forward_block_offense(&self, add_parens: bool, target: ParenTarget<'_>, range: Option<(usize, usize)>, out: &mut Vec<AfOffense>) {
        // register_forward_block_arg_offense guards.
        if self.cfg.target_ruby <= 30 || self.cfg.explicit_block_name {
            return;
        }
        let Some((start, end)) = range else { return };
        // block_arg.source == '&' (already anonymous) -> no offense.
        if self.src(start, end) == b"&" {
            return;
        }
        let mut ops = Vec::new();
        if add_parens {
            self.paren_ops(target, &mut ops);
        }
        ops.push(AfOp::replace(start, end, "&"));
        out.push(AfOffense { start, end, message: 3, ops });
    }
}

enum ParenTarget<'a> {
    Def(&'a DefInfo),
    Send(&'a SendSnapshot),
}

/// Does this call/super carry a LITERAL block (`do end` / `{}`)? A `&arg`
/// block pass is a `BlockArgumentNode`, not a block, and does not count.
fn call_has_literal_block(block: Option<Node<'_>>) -> bool {
    block.is_some_and(|b| b.as_block_node().is_some())
}

impl<'s> Rule for ArgumentsForwardingRule<'s> {
    fn enter(&mut self, node: &Node<'_>) {
        // Parser wraps `recv.m(args) do end` in a single `(block ...)` node
        // whose range covers the receiver and arguments too, so anything under
        // a call/super that owns a literal block is "inside a block" for
        // `each_ancestor(:any_block)`. Prism instead keeps the block in the
        // call's own slot (visited AFTER the receiver), so we bump
        // `block_depth` when ENTERING such a call — before its receiver/args
        // are walked — and count a call's own block toward its own `in_block`.
        let mut bumped = false;
        let frame = match node {
            Node::DefNode { .. } => {
                let d = node.as_def_node().unwrap();
                if let Some(body) = d.body() {
                    if let Some(info) = self.make_def_info(&d) {
                        let bloc = body.location();
                        let ci = self.collectors.len();
                        self.collectors.push(Collector {
                            def: info,
                            send_indices: Vec::new(),
                            referenced: HashSet::new(),
                            body_begin: bloc.start_offset(),
                            body_end: bloc.end_offset(),
                        });
                        self.def_stack.push(ci);
                        Frame::Def(ci)
                    } else {
                        Frame::Other
                    }
                } else {
                    Frame::Other
                }
            }
            Node::LambdaNode { .. } => {
                self.block_depth += 1;
                Frame::Block
            }
            Node::CallNode { .. } => {
                let c = node.as_call_node().unwrap();
                let own_block = call_has_literal_block(c.block());
                let idx = self.sends.len();
                self.snapshot_call(&c, own_block);
                self.register_send(idx);
                if own_block {
                    self.block_depth += 1;
                    bumped = true;
                }
                if bumped { Frame::Block } else { Frame::Other }
            }
            Node::SuperNode { .. } => {
                let s = node.as_super_node().unwrap();
                let own_block = call_has_literal_block(s.block());
                let idx = self.sends.len();
                self.snapshot_super(&s, own_block);
                self.register_send(idx);
                if own_block {
                    self.block_depth += 1;
                    bumped = true;
                }
                if bumped { Frame::Block } else { Frame::Other }
            }
            Node::ForwardingSuperNode { .. } => {
                // zsuper (`super { }`): not a forwarding send, but its literal
                // block still wraps its body for `each_ancestor(:any_block)`.
                let fs = node.as_forwarding_super_node().unwrap();
                if fs.block().is_some() {
                    self.block_depth += 1;
                    Frame::Block
                } else {
                    Frame::Other
                }
            }
            Node::YieldNode { .. } => {
                let y = node.as_yield_node().unwrap();
                let idx = self.sends.len();
                self.snapshot_yield(&y);
                self.register_send(idx);
                Frame::Other
            }
            Node::SplatNode { .. } | Node::AssocSplatNode { .. } | Node::BlockArgumentNode { .. } => {
                Frame::ForwardingCarrier
            }
            Node::LocalVariableWriteNode { .. } => {
                let w = node.as_local_variable_write_node().unwrap();
                self.record_reference(
                    lossy(w.name().as_slice()),
                    node.location().start_offset(),
                );
                Frame::Other
            }
            // Operator-assignment forms (`x ||= v`, `x &&= v`, `x += v`) are
            // parser `:lvasgn` too. Prism gives each its own node kind, none of
            // which is a `LocalVariable{Read,Write}Node`, so record the name
            // here. Never excluded by `FORWARDING_LVAR_TYPES` (that skip is
            // `:lvar`-only).
            Node::LocalVariableOrWriteNode { .. } => {
                let w = node.as_local_variable_or_write_node().unwrap();
                self.record_reference(lossy(w.name().as_slice()), node.location().start_offset());
                Frame::Other
            }
            Node::LocalVariableAndWriteNode { .. } => {
                let w = node.as_local_variable_and_write_node().unwrap();
                self.record_reference(lossy(w.name().as_slice()), node.location().start_offset());
                Frame::Other
            }
            Node::LocalVariableOperatorWriteNode { .. } => {
                let w = node.as_local_variable_operator_write_node().unwrap();
                self.record_reference(lossy(w.name().as_slice()), node.location().start_offset());
                Frame::Other
            }
            // Pattern-match patterns bind names with `LocalVariableTargetNode`
            // too, but parser calls those `:match_var` (not `:lvasgn`), so stock
            // does NOT count them. Record the offsets of a pattern's targets so
            // the leaf hook can tell them apart from real `masgn`/`for`/`rescue`
            // targets. Only the pattern subtree is scanned — the value side of
            // `expr => pat` / `expr in pat` and the `in` body stay live code.
            Node::InNode { .. } => {
                let n = node.as_in_node().unwrap();
                collect_pattern_target_offsets(&n.pattern(), &mut self.pattern_target_skips);
                Frame::Other
            }
            Node::MatchRequiredNode { .. } => {
                let n = node.as_match_required_node().unwrap();
                collect_pattern_target_offsets(&n.pattern(), &mut self.pattern_target_skips);
                Frame::Other
            }
            Node::MatchPredicateNode { .. } => {
                let n = node.as_match_predicate_node().unwrap();
                collect_pattern_target_offsets(&n.pattern(), &mut self.pattern_target_skips);
                Frame::Other
            }
            _ => Frame::Other,
        };
        self.frames.push(frame);
    }

    fn leave(&mut self) {
        if let Some(f) = self.frames.pop() {
            match f {
                Frame::Def(ci) => {
                    self.def_stack.pop();
                    self.finalize_collector(ci);
                }
                Frame::Block => self.block_depth -= 1,
                _ => {}
            }
        }
    }

    fn enter_leaf(&mut self, node: &Node<'_>) {
        if let Some(lv) = node.as_local_variable_read_node() {
            // Skip when the parent is a splat / kwsplat / block_pass (that is a
            // forwarding use, not a plain reference). This `FORWARDING_LVAR_TYPES`
            // skip is `:lvar`-only in stock.
            if !matches!(self.frames.last(), Some(Frame::ForwardingCarrier)) {
                self.record_reference(
                    lossy(lv.name().as_slice()),
                    node.location().start_offset(),
                );
            }
        } else if let Some(lt) = node.as_local_variable_target_node() {
            // A `masgn` / `for` / `rescue` target is parser `:lvasgn` -> a body
            // reference (never `FORWARDING_LVAR_TYPES`-excluded, even under a
            // `masgn` splat rest). A pattern binding shares this node kind but is
            // `:match_var` in parser, so skip the ones the pattern hooks marked.
            let at = node.location().start_offset();
            if !self.pattern_target_skips.contains(&at) {
                self.record_reference(lossy(lt.name().as_slice()), at);
            }
        }
    }

    fn interest(&self) -> Interest {
        Interest(Interest::LEAVE | Interest::LEAF | Interest::ENTER_ALL)
    }
}
