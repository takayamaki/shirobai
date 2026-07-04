//! `Lint/DuplicateMethods`.
//!
//! Stock accumulates `@definitions` / `@scopes` on the cop INSTANCE, which
//! RuboCop reuses across every file sharing a config — duplicate detection
//! is deliberately cross-file (the vendor spec pins `first.rb` /
//! `second.rb`). That state cannot live in Rust: the bundle result is
//! memoized per (source, config) and shared across cop instances, while
//! stock's state is per instance. So the Rust side reproduces the pure
//! per-file part — the exact stream of `found_method(key, name, location,
//! rescue-scope)` calls in stock's callback order — and the Ruby wrapper
//! replays stock's `found_method` bookkeeping (register / rescue-scope
//! replace / offense) against its own cross-investigation state.
//!
//! The result is therefore O(method definitions per file), not O(offenses):
//! every event mutates the wrapper's registry, so each is load-bearing.
//! This is the minimum information the cross-file replay needs.
//!
//! Two pieces cannot be finished in Rust and are marked on the event:
//!
//! - `scope_line >= 0`: the anonymous-block scope id is
//!   `source_location(anon_block)` = `"#{smart_path(buffer.name)}:#{line}"`;
//!   the wrapper appends `"@#{path}:#{line}"` (Rust does not know the file
//!   path).
//! - `sexp_start >= 0`: `lookup_constant` failed. Stock's `each_ancestor`
//!   returns `self` when the block never breaks, so the "qualified name"
//!   silently becomes the DEFS NODE itself and the key embeds the
//!   parser-gem s-expression dump (`"(defs\n  (const nil :B) :foo\n ...).foo"`).
//!   Reproducing parser-gem `Node#to_s` for arbitrary bodies in Rust is a
//!   non-starter; the wrapper finds the parser node at the given range and
//!   interpolates it, using stock's own `to_s`.
//!
//! Everything else (probed against stock, see the edge-case spec):
//!
//! - `parent_module_name` frame semantics: `each_ancestor(:class, :module,
//!   :sclass, :casgn, :block)` — numblock/itblock ancestors are INVISIBLE
//!   (skipped, not aborting); a plain block aborts unless it is
//!   `Const.class_eval do` (contributes the const, `foo.class_eval` aborts,
//!   receiverless `class_eval` skips) or the zero-arg `casgn = Class.new do`
//!   form (skips; the casgn ancestor then contributes iff its receiver is
//!   the GLOBAL `Class`/`Module`); `class << expr` aborts for non-const,
//!   non-self subjects.
//! - parser's `(block ...)` node covers the call's receiver and arguments,
//!   so the "block ancestor" semantics attach to the prism `CallNode` that
//!   carries the block (a def in `Class.new(def a; end) do ... end`'s
//!   argument list is inside the anonymous class).
//! - the anonymous-class path (`Class.new`/`Module.new` with any const
//!   namespace, args allowed) computes `base = qualified_name(pmn, nil,
//!   'Object')` — `"::Object"` when the pmn is nil (probed: describe
//!   blocks), `"Foo::Object"` under a non-Object pmn (casgn with args).
//! - `anon_block_scope_id` distinguishes the parser parent: parser wraps
//!   multi-statement bodies in `(begin)` but not single-statement ones, so
//!   the same code changes scope id shape with the statement count around
//!   it (probed: `test.rb:2` vs no suffix in a def body).
//! - the rescue/ensure redefinition allowance keys on the NEAREST parser
//!   `rescue`/`ensure` ancestor: parser nests `(ensure (rescue body ...)
//!   ens)`, so everything except the ensure body maps to `:rescue` when a
//!   rescue clause exists, and to `:ensure` when only an ensure exists.
//! - `if` ancestors (if/unless/elsif/ternary/modifier) suppress def / defs
//!   / alias / alias_method / delegate / def_delegator(s) — but NOT
//!   attr_* (stock has no condition check on the attr path).

use std::rc::Rc;

use ruby_prism::{CallNode, Node};

use super::dispatch::{self, Interest, Rule};
use super::line_index::{self, LineIndex};

/// `AllCops/ActiveSupportExtensionsEnabled` — gates `delegate` tracking.
#[derive(Clone, Copy)]
pub struct Config {
    pub active_support_extensions_enabled: bool,
}

/// One `found_method` call, in stock's callback order.
pub struct DupMethodEvent {
    /// `method_name` as interpolated into the message. For sexp fallback
    /// events this is only the short method name; the wrapper builds
    /// `"#{parser_node}.#{name}"`.
    pub name: String,
    /// The full key body (`method_key` prefix + name + any static
    /// `"@scope_id"`). For sexp fallback events this is just the
    /// `"outer."` prefix (usually empty).
    pub key: String,
    /// `>= 0`: byte range of the defs node whose parser s-expression the
    /// wrapper must interpolate between `key` and `".#{name}"`.
    pub sexp_start: i64,
    pub sexp_end: i64,
    /// `>= 0`: 1-based line for the `"@#{smart_path}:#{line}"` suffix the
    /// wrapper appends to the key (`source_location`-based scope id).
    pub scope_line: i64,
    /// Offense range: `loc.keyword.join(loc.name)` for def/defs, the full
    /// `source_range` for alias / send events.
    pub off_start: usize,
    pub off_end: usize,
    /// `node.source_range.line` — drives `source_location` in messages.
    pub line: usize,
    /// Nearest parser rescue/ensure ancestor: 0 none, 1 `:rescue`,
    /// 2 `:ensure`.
    pub rescue_scope: u8,
}

const RESTRICT_ON_SEND: &[&[u8]] = &[
    b"alias_method",
    b"attr_reader",
    b"attr_writer",
    b"attr_accessor",
    b"attr",
    b"delegate",
    b"def_delegator",
    b"def_instance_delegator",
    b"def_delegators",
    b"def_instance_delegators",
];

/// How a frame participates in `parent_module_name`.
enum Pmn {
    /// Node type not in the `each_ancestor` filter (or a skipped casgn /
    /// receiverless class_eval / casgn-new block): contributes nothing.
    Invisible,
    /// Contributes a scope name part.
    Part(String),
    /// Yields nil: `parent_module_name` is nil for everything inside.
    Abort,
}

/// Root of a constant path (`A::B` / `::A::B` / `self::B`).
#[derive(Clone, Copy, PartialEq)]
enum Root {
    Nil,
    Cbase,
    Other,
}

/// Decomposed constant path for class/module/casgn frames.
struct ConstChain {
    root: Root,
    /// Outermost first.
    names: Vec<String>,
}

impl ConstChain {
    /// rubocop-ast `Node#const_name`: cbase dropped, non-const namespace
    /// interpolates as nil (`"::B"` for `self::B`).
    fn const_name(&self) -> String {
        let joined = self.names.join("::");
        match self.root {
            Root::Other => format!("::{joined}"),
            _ => joined,
        }
    }
}

/// Info about a call, kept on `Call` frames for block semantics and
/// `named_receiver`-based scope ids.
struct CallInfo {
    name: String,
    recv: Option<(usize, usize)>,
    recv_is_com_new_block: bool,
    /// This call is `Class.new`/`Module.new` (any const namespace, any
    /// args) with a PLAIN literal block — an anonymous-class candidate.
    is_com_new_block: bool,
    has_plain_block: bool,
    start: usize,
}

enum Kind {
    Program {
        multi: bool,
    },
    Statements {
        multi: bool,
        holder: usize,
    },
    Parens,
    Begin {
        has_rescue: bool,
        has_ensure: bool,
        ensure_stmts: Option<(usize, usize)>,
    },
    RescueMod,
    Call(CallInfo),
    /// The literal BlockNode itself (inert; its semantics live on the
    /// owning Call frame). `call` is the owning Call frame's index.
    Block {
        call: usize,
    },
    Lambda,
    Def {
        name: String,
        recv: Option<(usize, usize)>,
    },
    ClassMod {
        chain: ConstChain,
    },
    Sclass {
        subject_send_name: Option<String>,
    },
    Casgn {
        chain: Option<ConstChain>,
    },
    Lvasgn,
    If,
    Other,
}

struct Frame {
    kind: Kind,
    pmn: Pmn,
    /// Bumped counters to undo on pop.
    bump_if: bool,
    bump_sclass_any: bool,
    bump_sclass_nonself: bool,
}

enum ScopeId {
    None,
    Static(String),
    PathLine(usize),
}

pub struct DuplicateMethodsRule<'s> {
    source: &'s [u8],
    cfg: Config,
    li: Rc<LineIndex>,
    frames: Vec<Frame>,
    if_depth: usize,
    sclass_any: usize,
    sclass_nonself: usize,
    pub events: Vec<DupMethodEvent>,
}

pub fn build_rule<'s>(source: &'s [u8], cfg: &Config) -> DuplicateMethodsRule<'s> {
    let li = line_index::with_line_index(source, |li| li.clone());
    DuplicateMethodsRule {
        source,
        cfg: *cfg,
        li,
        frames: Vec::new(),
        if_depth: 0,
        sclass_any: 0,
        sclass_nonself: 0,
        events: Vec::new(),
    }
}

/// Standalone entry point (per-cop fallback path).
pub fn check_duplicate_methods(source: &[u8], cfg: &Config) -> Vec<DupMethodEvent> {
    let mut rule = build_rule(source, cfg);
    dispatch::run(source, &mut [&mut rule]);
    rule.events
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `(const {nil? cbase} {:Class :Module})` — the GLOBAL Class/Module
/// constant (`defined_module0`'s `#global_const?`).
fn is_global_class_or_module(node: &Node<'_>) -> bool {
    match node {
        Node::ConstantReadNode { .. } => {
            let n = node.as_constant_read_node().unwrap();
            matches!(n.name().as_slice(), b"Class" | b"Module")
        }
        Node::ConstantPathNode { .. } => {
            let n = node.as_constant_path_node().unwrap();
            n.parent().is_none() && matches!(n.name().map(|c| c.as_slice().to_vec()), Some(ref v) if v == b"Class" || v == b"Module")
        }
        _ => false,
    }
}

/// `(const _ {:Class :Module})` — ANY namespace (`class_or_module_new_block?`).
fn is_any_class_or_module_const(node: &Node<'_>) -> bool {
    match node {
        Node::ConstantReadNode { .. } => {
            let n = node.as_constant_read_node().unwrap();
            matches!(n.name().as_slice(), b"Class" | b"Module")
        }
        Node::ConstantPathNode { .. } => {
            let n = node.as_constant_path_node().unwrap();
            matches!(n.name().map(|c| c.as_slice().to_vec()), Some(ref v) if v == b"Class" || v == b"Module")
        }
        _ => false,
    }
}

/// Kind of block attached to a call, in parser terms.
#[derive(PartialEq, Clone, Copy)]
enum BlockKind {
    None,
    Plain,
    Numbered,
    It,
    BlockArg,
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

/// `class_or_module_new_block?`: a call that in parser is
/// `(block (send (const _ {:Class :Module}) :new ...) ...)` — a plain
/// literal block on `Class.new`/`Module.new` with any const namespace.
fn is_com_new_block(call: &CallNode<'_>, kind: BlockKind) -> bool {
    kind == BlockKind::Plain
        && call.name().as_slice() == b"new"
        && !call.is_safe_navigation()
        && call
            .receiver()
            .is_some_and(|r| is_any_class_or_module_const(&r))
}

/// Decompose a ConstantRead / ConstantPath node.
fn const_chain(node: &Node<'_>) -> Option<ConstChain> {
    match node {
        Node::ConstantReadNode { .. } => {
            let n = node.as_constant_read_node().unwrap();
            Some(ConstChain {
                root: Root::Nil,
                names: vec![lossy(n.name().as_slice())],
            })
        }
        Node::ConstantPathNode { .. } => {
            let n = node.as_constant_path_node().unwrap();
            let name = lossy(n.name()?.as_slice());
            match n.parent() {
                None => Some(ConstChain {
                    root: Root::Cbase,
                    names: vec![name],
                }),
                Some(p) => match const_chain(&p) {
                    Some(mut chain) => {
                        chain.names.push(name);
                        Some(chain)
                    }
                    None => Some(ConstChain {
                        root: Root::Other,
                        names: vec![name],
                    }),
                },
            }
        }
        _ => None,
    }
}

/// Plain (non-interpolated) sym/str value, as interpolated by stock.
fn sym_or_str_value(node: &Node<'_>) -> Option<String> {
    if let Some(s) = node.as_symbol_node() {
        return Some(lossy(s.unescaped()));
    }
    if let Some(s) = node.as_string_node() {
        return Some(lossy(s.unescaped()));
    }
    None
}

fn sym_value(node: &Node<'_>) -> Option<String> {
    node.as_symbol_node().map(|s| lossy(s.unescaped()))
}

/// `humanize_scope`: one `sub` of
/// `/(?:(?<name>.*)::)#<Class:\k<name>>|#<Class:(?<name>.*)>(?:::)?/` with
/// `'\k<name>.'`, then a trailing `#` unless the result ends with `.`.
fn humanize_scope(scope: &str) -> String {
    let b = scope.as_bytes();
    let n = b.len();
    let mut matched: Option<(usize, usize, &str)> = None;
    'outer: for p in 0..n {
        // Alternative 1: `name::#<Class:name>`, greedy name (longest first).
        if n >= p + 11 {
            let max_l = (n - p - 11) / 2;
            for l in (0..=max_l).rev() {
                let ne = p + l;
                if b[ne..ne + 2] == *b"::"
                    && b[ne + 2..ne + 10] == *b"#<Class:"
                    && b[ne + 10..ne + 10 + l] == b[p..ne]
                    && b[ne + 10 + l] == b'>'
                {
                    matched = Some((p, ne + 10 + l + 1, &scope[p..ne]));
                    break 'outer;
                }
            }
        }
        // Alternative 2: `#<Class:name>(::)?`, name greedy to the LAST `>`.
        if b[p..].starts_with(b"#<Class:")
            && let Some(r) = (p + 8..n).rev().find(|&r| b[r] == b'>')
        {
            let mut end = r + 1;
            if b[end..].starts_with(b"::") {
                end += 2;
            }
            matched = Some((p, end, &scope[p + 8..r]));
            break 'outer;
        }
    }
    let mut out = String::with_capacity(scope.len() + 1);
    match matched {
        Some((s, e, name)) => {
            out.push_str(&scope[..s]);
            out.push_str(name);
            out.push('.');
            out.push_str(&scope[e..]);
        }
        None => out.push_str(scope),
    }
    if !out.ends_with('.') {
        out.push('#');
    }
    out
}

/// Stock `qualified_name(enclosing, namespace, mod_name)`. `ns` is `None`
/// when there is no namespace node, `Some(inner)` otherwise (with `inner`
/// the namespace's `const_name`, which is `None` for non-const nodes and
/// interpolates as the empty string).
fn qualified_name(enclosing: Option<&str>, ns: Option<Option<&str>>, mod_name: &str) -> String {
    if enclosing != Some("Object") {
        let e = enclosing.unwrap_or("");
        match ns {
            Some(inner) => format!("{}::{}::{}", e, inner.unwrap_or(""), mod_name),
            None => format!("{e}::{mod_name}"),
        }
    } else {
        match ns {
            Some(inner) => format!("{}::{}", inner.unwrap_or(""), mod_name),
            None => mod_name.to_string(),
        }
    }
}

/// Effective parser parent of the parser `(block ...)` node owned by the
/// Call frame at `call_idx` (== the parent of the prism CallNode).
enum PParent<'a> {
    Call(&'a CallInfo),
    /// parser any_block parent; the owning Call frame index (`None` for a
    /// lambda literal, whose "call" child is `(lambda)`).
    AnyBlock(Option<usize>),
    /// parser `(begin)` parent; `pp_block` = `parent.parent&.any_block_type?`.
    BeginWrap { pp_block: bool },
    Def(usize),
    Casgn,
    Lvasgn,
    Disallowed,
}

impl<'s> DuplicateMethodsRule<'s> {
    fn src(&self, range: (usize, usize)) -> String {
        lossy(&self.source[range.0..range.1])
    }

    /// `parent_module_name` over `frames[..upto]` (inner→outer walk).
    fn pmn(&self, upto: usize) -> Option<String> {
        let mut parts: Vec<&str> = Vec::new();
        for f in self.frames[..upto].iter().rev() {
            match &f.pmn {
                Pmn::Invisible => {}
                Pmn::Part(s) => parts.push(s),
                Pmn::Abort => return None,
            }
        }
        if parts.is_empty() {
            return Some("Object".to_string());
        }
        parts.reverse();
        Some(parts.join("::"))
    }

    /// Nearest parser `:block` ancestor frame index (a Call frame with a
    /// plain literal block, or a lambda literal). numblock/itblock frames
    /// do not count (`each_ancestor(:block)`).
    fn first_block(&self) -> Option<usize> {
        self.frames.iter().enumerate().rev().find_map(|(i, f)| match &f.kind {
            Kind::Call(info) if info.has_plain_block => Some(i),
            Kind::Lambda => Some(i),
            _ => None,
        })
    }

    fn resolve_parent(&self, call_idx: usize) -> PParent<'_> {
        if call_idx == 0 {
            return PParent::Disallowed;
        }
        match &self.frames[call_idx - 1].kind {
            Kind::Statements { multi, holder } => {
                let holder = *holder;
                if matches!(self.frames[holder].kind, Kind::Parens) {
                    PParent::BeginWrap {
                        pp_block: self.parser_parent_is_anyblock(holder),
                    }
                } else if *multi {
                    PParent::BeginWrap {
                        pp_block: matches!(
                            self.frames[holder].kind,
                            Kind::Block { .. } | Kind::Lambda
                        ),
                    }
                } else {
                    match &self.frames[holder].kind {
                        Kind::Block { call } => PParent::AnyBlock(Some(*call)),
                        Kind::Lambda => PParent::AnyBlock(None),
                        Kind::Def { .. } => PParent::Def(holder),
                        _ => PParent::Disallowed,
                    }
                }
            }
            Kind::Program { multi } => {
                if *multi {
                    PParent::BeginWrap { pp_block: false }
                } else {
                    PParent::Disallowed
                }
            }
            Kind::Call(info) => PParent::Call(info),
            Kind::Casgn { .. } => PParent::Casgn,
            Kind::Lvasgn => PParent::Lvasgn,
            Kind::Def { .. } => PParent::Def(call_idx - 1),
            _ => PParent::Disallowed,
        }
    }

    /// Is the parser parent of the node whose frame is `idx` an any_block?
    /// (Used for the parens-begin `parent.parent` check.)
    fn parser_parent_is_anyblock(&self, idx: usize) -> bool {
        if idx == 0 {
            return false;
        }
        match &self.frames[idx - 1].kind {
            Kind::Statements { multi, holder } => {
                !matches!(self.frames[*holder].kind, Kind::Parens)
                    && !*multi
                    && matches!(
                        self.frames[*holder].kind,
                        Kind::Block { .. } | Kind::Lambda
                    )
            }
            _ => false,
        }
    }

    /// `anon_block_scope_id(anon_block)` for the anonymous block owned by
    /// the Call frame at `call_idx`.
    fn anon_scope_id(&self, call_idx: usize) -> ScopeId {
        let path_line = || {
            let start = match &self.frames[call_idx].kind {
                Kind::Call(info) => info.start,
                _ => unreachable!(),
            };
            ScopeId::PathLine(self.li.line_of(start))
        };
        match self.resolve_parent(call_idx) {
            PParent::Call(info) => match info.recv {
                Some(r) if !info.recv_is_com_new_block => {
                    ScopeId::Static(format!("{}.{}", self.src(r), info.name))
                }
                _ => path_line(),
            },
            PParent::AnyBlock(Some(owner)) => {
                let Kind::Call(info) = &self.frames[owner].kind else {
                    return ScopeId::None;
                };
                match info.recv {
                    Some(r) if !info.recv_is_com_new_block => {
                        ScopeId::Static(format!("{}.{}", self.src(r), info.name))
                    }
                    _ => path_line(),
                }
            }
            PParent::AnyBlock(None) => path_line(),
            PParent::Def(d) => {
                let Kind::Def { name, recv } = &self.frames[d].kind else {
                    return ScopeId::None;
                };
                match recv {
                    Some(r) => ScopeId::Static(format!("{}.{}", self.src(*r), name)),
                    None => path_line(),
                }
            }
            PParent::Casgn => path_line(),
            PParent::BeginWrap { pp_block } => {
                if pp_block {
                    path_line()
                } else {
                    ScopeId::None
                }
            }
            PParent::Lvasgn | PParent::Disallowed => ScopeId::None,
        }
    }

    /// Nearest parser rescue/ensure ancestor for a node starting at `off`.
    fn rescue_scope(&self, off: usize) -> u8 {
        for f in self.frames.iter().rev() {
            match &f.kind {
                Kind::RescueMod => return 1,
                Kind::Begin {
                    has_rescue,
                    has_ensure,
                    ensure_stmts,
                } => {
                    if let Some((s, e)) = ensure_stmts
                        && *s <= off
                        && off < *e
                    {
                        return 2;
                    }
                    if *has_rescue {
                        return 1;
                    }
                    if *has_ensure {
                        return 2;
                    }
                }
                _ => {}
            }
        }
        0
    }

    /// `method_key` prefix from the nearest ancestor def.
    fn key_prefix(&self) -> String {
        self.frames
            .iter()
            .rev()
            .find_map(|f| match &f.kind {
                Kind::Def { name, .. } => Some(format!("{name}.")),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    fn push_event(
        &mut self,
        name: String,
        key_body: String,
        scope_id: ScopeId,
        sexp: Option<(usize, usize)>,
        off: (usize, usize),
        node_start: usize,
    ) {
        let (scope_line, static_suffix) = match scope_id {
            ScopeId::None => (-1i64, None),
            ScopeId::PathLine(l) => (l as i64, None),
            ScopeId::Static(s) => (-1i64, Some(s)),
        };
        let mut key = format!("{}{}", self.key_prefix(), key_body);
        if let Some(s) = static_suffix {
            key.push('@');
            key.push_str(&s);
        }
        let (sexp_start, sexp_end) = match sexp {
            Some((s, e)) => (s as i64, e as i64),
            None => (-1, -1),
        };
        let rescue_scope = self.rescue_scope(node_start);
        let line = self.li.line_of(node_start);
        self.events.push(DupMethodEvent {
            name,
            key,
            sexp_start,
            sexp_end,
            scope_line,
            off_start: off.0,
            off_end: off.1,
            line,
            rescue_scope,
        });
    }

    /// `found_method(node, method_name)` — non-anonymous: key body == name.
    fn found_plain(&mut self, name: String, off: (usize, usize), node_start: usize) {
        self.push_event(name.clone(), name, ScopeId::None, None, off, node_start);
    }

    /// `found_instance_method(node, name)`.
    fn found_instance_method(&mut self, name: &str, off: (usize, usize), node_start: usize) {
        if let Some(scope) = self.pmn(self.frames.len()) {
            let full = format!("{}{}", humanize_scope(&scope), name);
            self.found_plain(full, off, node_start);
        } else if let Some(anon) = self.anonymous_class_block() {
            let base = self.anon_base(anon);
            let scope = if self.sclass_any > 0 {
                format!("#<Class:{base}>")
            } else {
                base
            };
            let full = format!("{}{}", humanize_scope(&scope), name);
            let scope_id = self.anon_scope_id(anon);
            self.push_event(full.clone(), full, scope_id, None, off, node_start);
        } else {
            self.found_sclass_method(name, off, node_start);
        }
    }

    /// `anonymous_class_block(node)` — Some(call frame index) or None.
    fn anonymous_class_block(&self) -> Option<usize> {
        let idx = self.first_block()?;
        let Kind::Call(info) = &self.frames[idx].kind else {
            return None; // lambda literal: never a Class.new block
        };
        if !info.is_com_new_block {
            return None;
        }
        // `first_block.parent&.type?(:lvasgn)`: DIRECT parser parent only.
        if matches!(self.resolve_parent(idx), PParent::Lvasgn) {
            return None;
        }
        if self.sclass_nonself > 0 {
            return None;
        }
        Some(idx)
    }

    /// `qualified_name(anon_block.parent_module_name, nil, 'Object')`.
    fn anon_base(&self, call_idx: usize) -> String {
        qualified_name(self.pmn(call_idx).as_deref(), None, "Object")
    }

    /// `found_sclass_method`.
    fn found_sclass_method(&mut self, name: &str, off: (usize, usize), node_start: usize) {
        let subject = self.frames.iter().rev().find_map(|f| match &f.kind {
            Kind::Sclass { subject_send_name } => Some(subject_send_name.clone()),
            _ => None,
        });
        let Some(Some(send_name)) = subject else {
            return;
        };
        self.found_plain(format!("{send_name}.{name}"), off, node_start);
    }

    fn handle_def(&mut self, def: &ruby_prism::DefNode<'_>) {
        if self.if_depth > 0 {
            return;
        }
        let name = lossy(def.name().as_slice());
        let off = (
            def.def_keyword_loc().start_offset(),
            def.name_loc().end_offset(),
        );
        let node_start = def.location().start_offset();
        match def.receiver() {
            None => self.found_instance_method(&name, off, node_start),
            Some(recv) => {
                if recv.as_self_node().is_some() {
                    // check_self_receiver
                    if let Some(enclosing) = self.pmn(self.frames.len()) {
                        self.found_plain(format!("{enclosing}.{name}"), off, node_start);
                    } else if let Some(anon) = self.anonymous_class_block() {
                        let scope = self.anon_base(anon);
                        let full = format!("{scope}.{name}");
                        let scope_id = self.anon_scope_id(anon);
                        self.push_event(full.clone(), full, scope_id, None, off, node_start);
                    }
                } else if matches!(
                    recv,
                    Node::ConstantReadNode { .. } | Node::ConstantPathNode { .. }
                ) {
                    self.check_const_receiver(def, &recv, &name, off, node_start);
                }
            }
        }
    }

    /// `check_const_receiver`: `lookup_constant` over class/module/casgn
    /// ancestors; on failure fall back to the parser-sexp key (stock's
    /// `each_ancestor` returns `self`).
    fn check_const_receiver(
        &mut self,
        def: &ruby_prism::DefNode<'_>,
        recv: &Node<'_>,
        name: &str,
        off: (usize, usize),
        node_start: usize,
    ) {
        // `_, const_name = *node.receiver` — the SHORT name.
        let short = match recv {
            Node::ConstantReadNode { .. } => {
                lossy(recv.as_constant_read_node().unwrap().name().as_slice())
            }
            Node::ConstantPathNode { .. } => match recv.as_constant_path_node().unwrap().name() {
                Some(n) => lossy(n.as_slice()),
                None => return,
            },
            _ => return,
        };
        for i in (0..self.frames.len()).rev() {
            let chain = match &self.frames[i].kind {
                Kind::ClassMod { chain } => Some(chain),
                Kind::Casgn { chain } => chain.as_ref(),
                _ => None,
            };
            let Some(chain) = chain else { continue };
            for k in (0..chain.names.len()).rev() {
                if chain.names[k] != short {
                    continue;
                }
                let ns: Option<Option<String>> = if k == 0 {
                    match chain.root {
                        Root::Nil => None,
                        // cbase / non-const namespace nodes are present but
                        // have a nil const_name.
                        Root::Cbase | Root::Other => Some(None),
                    }
                } else {
                    let joined = chain.names[..k].join("::");
                    Some(Some(match chain.root {
                        Root::Other => format!("::{joined}"),
                        _ => joined,
                    }))
                };
                let qualified = qualified_name(
                    self.pmn(i).as_deref(),
                    ns.as_ref().map(|o| o.as_deref()),
                    &chain.names[k],
                );
                self.found_plain(format!("{qualified}.{name}"), off, node_start);
                return;
            }
        }
        // lookup_constant returned the node itself: sexp-dump key.
        let loc = def.location();
        let _ = recv;
        self.push_event(
            name.to_string(),
            String::new(),
            ScopeId::None,
            Some((loc.start_offset(), loc.end_offset())),
            off,
            node_start,
        );
    }

    fn handle_alias(&mut self, alias: &ruby_prism::AliasMethodNode<'_>) {
        let (Some(new), Some(old)) = (
            sym_value_node(&alias.new_name()),
            sym_value_node(&alias.old_name()),
        ) else {
            return;
        };
        if new == old {
            return;
        }
        if self.if_depth > 0 {
            return;
        }
        let loc = alias.location();
        let off = (loc.start_offset(), loc.end_offset());
        self.found_instance_method(&new, off, loc.start_offset());
    }

    fn handle_send(&mut self, call: &CallNode<'_>) {
        let name = call.name().as_slice();
        if !RESTRICT_ON_SEND.contains(&name) {
            return;
        }
        if call.is_safe_navigation() || call.receiver().is_some() {
            return;
        }
        // "Virtual" argument list: parser appends `&block_pass` as a
        // trailing argument, prism keeps it in the block slot.
        let mut args: Vec<Node<'_>> = call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        let mut has_literal_block = false;
        if let Some(b) = call.block() {
            if b.as_block_argument_node().is_some() {
                args.push(b);
            } else {
                has_literal_block = true;
            }
        }
        let off = (call.location().start_offset(), send_end(call, &args, has_literal_block));
        let node_start = call.location().start_offset();

        match name {
            b"alias_method" => {
                // (send nil? :alias_method (sym $new) (sym $old))
                if args.len() != 2 {
                    return;
                }
                let (Some(new), Some(old)) = (sym_value(&args[0]), sym_value(&args[1])) else {
                    return;
                };
                if new == old || self.if_depth > 0 {
                    return;
                }
                self.found_instance_method(&new, off, node_start);
            }
            b"attr" | b"attr_reader" | b"attr_writer" | b"attr_accessor" => {
                // attribute_accessor? needs at least one argument.
                if args.is_empty() {
                    return;
                }
                // NOTE: stock has NO condition check on the attr path.
                let (readable, writable) = match name {
                    b"attr" => (
                        true,
                        args.len() == 2 && args[1].as_true_node().is_some(),
                    ),
                    b"attr_reader" => (true, false),
                    b"attr_writer" => (false, true),
                    _ => (true, true),
                };
                let list: Vec<Node<'_>> = if name == b"attr" {
                    vec![args.remove(0)]
                } else {
                    args
                };
                for arg in &list {
                    let Some(n) = sym_value(arg) else { continue };
                    if readable {
                        self.found_instance_method(&n, off, node_start);
                    }
                    if writable {
                        self.found_instance_method(&format!("{n}="), off, node_start);
                    }
                }
            }
            b"delegate" => {
                if !self.cfg.active_support_extensions_enabled {
                    return;
                }
                let Some((names, prefix)) = delegate_names(&args) else {
                    return;
                };
                if self.if_depth > 0 {
                    return;
                }
                for n in names {
                    let full = match &prefix {
                        Some(p) => format!("{p}_{n}"),
                        None => n,
                    };
                    self.found_instance_method(&full, off, node_start);
                }
            }
            b"def_delegator" | b"def_instance_delegator" => {
                // 2 or 3 args, all plain sym/str; the last is the defined name.
                if args.len() != 2 && args.len() != 3 {
                    return;
                }
                let values: Vec<Option<String>> =
                    args.iter().map(sym_or_str_value).collect();
                if values.iter().any(|v| v.is_none()) {
                    return;
                }
                if self.if_depth > 0 {
                    return;
                }
                let defined = values.last().unwrap().clone().unwrap();
                self.found_instance_method(&defined, off, node_start);
            }
            b"def_delegators" | b"def_instance_delegators" => {
                if args.len() < 2 {
                    return;
                }
                let values: Vec<Option<String>> =
                    args.iter().map(sym_or_str_value).collect();
                if values.iter().any(|v| v.is_none()) {
                    return;
                }
                if self.if_depth > 0 {
                    return;
                }
                for v in &values[1..] {
                    self.found_instance_method(v.as_deref().unwrap(), off, node_start);
                }
            }
            _ => {}
        }
    }

    fn make_frame(&mut self, node: &Node<'_>) -> Frame {
        let mut frame = Frame {
            kind: Kind::Other,
            pmn: Pmn::Invisible,
            bump_if: false,
            bump_sclass_any: false,
            bump_sclass_nonself: false,
        };
        match node {
            Node::ProgramNode { .. } => {
                let p = node.as_program_node().unwrap();
                frame.kind = Kind::Program {
                    multi: p.statements().body().iter().count() >= 2,
                };
            }
            Node::StatementsNode { .. } => {
                let s = node.as_statements_node().unwrap();
                frame.kind = Kind::Statements {
                    multi: s.body().iter().count() >= 2,
                    holder: self.frames.len().saturating_sub(1),
                };
            }
            Node::ParenthesesNode { .. } => frame.kind = Kind::Parens,
            Node::BeginNode { .. } => {
                let b = node.as_begin_node().unwrap();
                frame.kind = Kind::Begin {
                    has_rescue: b.rescue_clause().is_some(),
                    has_ensure: b.ensure_clause().is_some(),
                    ensure_stmts: b.ensure_clause().and_then(|e| e.statements()).map(|s| {
                        let l = s.as_node().location();
                        (l.start_offset(), l.end_offset())
                    }),
                };
            }
            Node::RescueModifierNode { .. } => frame.kind = Kind::RescueMod,
            Node::IfNode { .. } | Node::UnlessNode { .. } => {
                frame.kind = Kind::If;
                frame.bump_if = true;
            }
            Node::LambdaNode { .. } => {
                frame.kind = Kind::Lambda;
                frame.pmn = Pmn::Abort;
            }
            Node::BlockNode { .. } => {
                frame.kind = Kind::Block {
                    call: self.frames.len().saturating_sub(1),
                };
            }
            Node::DefNode { .. } => {
                let d = node.as_def_node().unwrap();
                frame.kind = Kind::Def {
                    name: lossy(d.name().as_slice()),
                    recv: d.receiver().map(|r| {
                        let l = r.location();
                        (l.start_offset(), l.end_offset())
                    }),
                };
            }
            Node::ClassNode { .. } | Node::ModuleNode { .. } => {
                let cpath = if let Some(c) = node.as_class_node() {
                    c.constant_path()
                } else {
                    node.as_module_node().unwrap().constant_path()
                };
                match const_chain(&cpath) {
                    Some(chain) => {
                        frame.pmn = Pmn::Part(chain.const_name());
                        frame.kind = Kind::ClassMod { chain };
                    }
                    None => {
                        // Dynamic class name (`class self::B` reaches here
                        // only for non-const cpaths, which cannot parse) —
                        // defensive: contributes nothing.
                        frame.kind = Kind::Other;
                    }
                }
            }
            Node::SingletonClassNode { .. } => {
                let s = node.as_singleton_class_node().unwrap();
                let subject = s.expression();
                frame.bump_sclass_any = true;
                let mut subject_send_name = None;
                if subject.as_self_node().is_some() {
                    let inner = self.pmn(self.frames.len());
                    frame.pmn = Pmn::Part(format!("#<Class:{}>", inner.unwrap_or_default()));
                } else if let Some(chain) = const_chain(&subject) {
                    frame.pmn = Pmn::Part(format!("#<Class:{}>", chain.const_name()));
                    frame.bump_sclass_nonself = true;
                } else {
                    if let Some(c) = subject.as_call_node() {
                        // parser send subject: no safe navigation, no block.
                        if !c.is_safe_navigation() && c.block().is_none() {
                            subject_send_name = Some(lossy(c.name().as_slice()));
                        }
                    }
                    frame.pmn = Pmn::Abort;
                    frame.bump_sclass_nonself = true;
                }
                frame.kind = Kind::Sclass { subject_send_name };
            }
            Node::ConstantWriteNode { .. } | Node::ConstantPathWriteNode { .. } => {
                let (chain, value) = if let Some(w) = node.as_constant_write_node() {
                    (
                        Some(ConstChain {
                            root: Root::Nil,
                            names: vec![lossy(w.name().as_slice())],
                        }),
                        w.value(),
                    )
                } else {
                    let w = node.as_constant_path_write_node().unwrap();
                    (const_chain(&w.target().as_node()), w.value())
                };
                let defined = casgn_defines_module(&value);
                if let Some(chain) = chain {
                    if defined {
                        frame.pmn = Pmn::Part(chain.const_name());
                        frame.kind = Kind::Casgn { chain: Some(chain) };
                    } else {
                        frame.kind = Kind::Casgn { chain: Some(chain) };
                        frame.pmn = Pmn::Invisible;
                    }
                } else {
                    frame.kind = Kind::Casgn { chain: None };
                }
            }
            Node::LocalVariableWriteNode { .. } => frame.kind = Kind::Lvasgn,
            Node::CallNode { .. } => {
                let c = node.as_call_node().unwrap();
                let kind = block_kind(&c);
                let recv = c.receiver();
                let info = CallInfo {
                    name: lossy(c.name().as_slice()),
                    recv: recv.as_ref().map(|r| {
                        let l = r.location();
                        (l.start_offset(), l.end_offset())
                    }),
                    recv_is_com_new_block: recv.as_ref().is_some_and(|r| {
                        r.as_call_node()
                            .is_some_and(|rc| is_com_new_block(&rc, block_kind(&rc)))
                    }),
                    is_com_new_block: is_com_new_block(&c, kind),
                    has_plain_block: kind == BlockKind::Plain,
                    start: c.location().start_offset(),
                };
                if kind == BlockKind::Plain {
                    // parser (block ...) ancestor semantics live here.
                    frame.pmn = self.block_pmn(&c);
                }
                frame.kind = Kind::Call(info);
            }
            _ => {}
        }
        frame
    }

    /// `parent_module_name_for_block` + `new_class_or_module_block?` for a
    /// call carrying a plain literal block.
    fn block_pmn(&self, call: &CallNode<'_>) -> Pmn {
        if call.name().as_slice() == b"class_eval" {
            return match call.receiver() {
                None => Pmn::Invisible, // receiverless class_eval: skip
                Some(r) => match const_chain(&r) {
                    Some(chain) => Pmn::Part(chain.const_name()),
                    None => Pmn::Abort,
                },
            };
        }
        // `^(casgn _ _ (block (send (const _ {:Class :Module}) :new) ...))`
        // — parent casgn, zero args, any const namespace.
        let parent_is_casgn = self
            .frames
            .last()
            .is_some_and(|f| matches!(f.kind, Kind::Casgn { .. }));
        let zero_args = call
            .arguments()
            .is_none_or(|a| a.arguments().iter().count() == 0);
        if parent_is_casgn
            && zero_args
            && call.name().as_slice() == b"new"
            && !call.is_safe_navigation()
            && call
                .receiver()
                .is_some_and(|r| is_any_class_or_module_const(&r))
        {
            return Pmn::Invisible;
        }
        Pmn::Abort
    }
}

/// `alias new old` name nodes must be plain symbols.
fn sym_value_node(node: &Node<'_>) -> Option<String> {
    node.as_symbol_node().map(|s| lossy(s.unescaped()))
}

/// `(casgn _ _ (send #global :new ...))` or the block form: does this casgn
/// value define a module for `defined_module0`?
fn casgn_defines_module(value: &Node<'_>) -> bool {
    let Some(c) = value.as_call_node() else {
        return false;
    };
    if c.name().as_slice() != b"new" || c.is_safe_navigation() {
        return false;
    }
    if !c.receiver().is_some_and(|r| is_global_class_or_module(&r)) {
        return false;
    }
    // send form: no literal block (a block argument `&b` stays a send);
    // block form: a PLAIN literal block (numblock/itblock are not `(block)`).
    match block_kind(&c) {
        BlockKind::None | BlockKind::BlockArg | BlockKind::Plain => true,
        BlockKind::Numbered | BlockKind::It => false,
    }
}

/// parser send `source_range` end: prism's CallNode range includes an
/// attached literal block, parser's `(send)` child does not.
fn send_end(call: &CallNode<'_>, virtual_args: &[Node<'_>], has_literal_block: bool) -> usize {
    if !has_literal_block {
        return call.location().end_offset();
    }
    if let Some(cl) = call.closing_loc() {
        return cl.end_offset();
    }
    virtual_args
        .last()
        .map(|a| a.location().end_offset())
        .or_else(|| call.message_loc().map(|m| m.end_offset()))
        .unwrap_or_else(|| call.location().end_offset())
}

/// `delegate_method?` + `on_delegate`: returns the (possibly prefixed)
/// method names.
fn delegate_names(args: &[Node<'_>]) -> Option<(Vec<String>, Option<String>)> {
    if args.len() < 2 {
        return None;
    }
    let (hash, names_part) = args.split_last().unwrap();
    let mut names = Vec::with_capacity(names_part.len());
    for a in names_part {
        names.push(sym_or_str_value(a)?);
    }
    // Last arg: a hash containing `(pair (sym :to) {sym str})`.
    let elements: Vec<Node<'_>> = if let Some(h) = hash.as_hash_node() {
        h.elements().iter().collect()
    } else if let Some(h) = hash.as_keyword_hash_node() {
        h.elements().iter().collect()
    } else {
        return None;
    };
    let to_ok = elements.iter().any(|e| {
        e.as_assoc_node().is_some_and(|p| {
            p.key()
                .as_symbol_node()
                .is_some_and(|k| k.unescaped() == b"to")
                && sym_or_str_value(&p.value()).is_some()
        })
    });
    if !to_ok {
        return None;
    }
    // `delegate_prefix`: first `prefix:` pair; `true` borrows the first
    // `to:` value, a sym/str is used directly, anything else means no
    // prefix.
    let hash_value = |key: &[u8]| {
        elements.iter().find_map(|e| {
            e.as_assoc_node().and_then(|p| {
                p.key()
                    .as_symbol_node()
                    .filter(|k| k.unescaped() == key)
                    .map(|_| p.value())
            })
        })
    };
    let prefix = hash_value(b"prefix").and_then(|p| {
        if p.as_true_node().is_some() {
            hash_value(b"to").and_then(|t| sym_or_str_value(&t))
        } else {
            sym_or_str_value(&p)
        }
    });
    Some((names, prefix))
}

impl<'s> Rule for DuplicateMethodsRule<'s> {
    fn enter(&mut self, node: &Node<'_>) {
        match node {
            Node::DefNode { .. } => {
                let d = node.as_def_node().unwrap();
                self.handle_def(&d);
            }
            Node::AliasMethodNode { .. } => {
                let a = node.as_alias_method_node().unwrap();
                self.handle_alias(&a);
            }
            Node::CallNode { .. } => {
                let c = node.as_call_node().unwrap();
                self.handle_send(&c);
            }
            _ => {}
        }
        let frame = self.make_frame(node);
        if frame.bump_if {
            self.if_depth += 1;
        }
        if frame.bump_sclass_any {
            self.sclass_any += 1;
        }
        if frame.bump_sclass_nonself {
            self.sclass_nonself += 1;
        }
        self.frames.push(frame);
    }

    fn leave(&mut self) {
        if let Some(f) = self.frames.pop() {
            if f.bump_if {
                self.if_depth -= 1;
            }
            if f.bump_sclass_any {
                self.sclass_any -= 1;
            }
            if f.bump_sclass_nonself {
                self.sclass_nonself -= 1;
            }
        }
    }

    fn interest(&self) -> Interest {
        Interest(Interest::LEAVE | Interest::ENTER_ALL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(src: &str) -> Vec<(String, String, i64, i64, u8)> {
        check_duplicate_methods(
            src.as_bytes(),
            &Config {
                active_support_extensions_enabled: false,
            },
        )
        .into_iter()
        .map(|e| (e.key, e.name, e.scope_line, e.sexp_start, e.rescue_scope))
        .collect()
    }

    fn keys(src: &str) -> Vec<String> {
        events(src).into_iter().map(|e| e.0).collect()
    }

    // Typical: two top-level defs share the key.
    #[test]
    fn top_level_defs() {
        assert_eq!(keys("def foo; end\ndef foo; end\n"), vec!["Object#foo", "Object#foo"]);
    }

    // Class scope.
    #[test]
    fn class_defs() {
        assert_eq!(
            keys("class A\n  def foo; end\n  def foo; end\nend\n"),
            vec!["A#foo", "A#foo"]
        );
    }

    // class << self humanizes to `A.`.
    #[test]
    fn sclass_self() {
        assert_eq!(
            keys("class A\n  class << self\n    def foo; end\n  end\n  def self.foo; end\nend\n"),
            vec!["A.foo", "A.foo"]
        );
    }

    // class << B (const subject) inside a class.
    #[test]
    fn sclass_const() {
        assert_eq!(
            keys("class A\n  class << B\n    def x; end\n  end\nend\n"),
            vec!["A::B.x"]
        );
    }

    // class << expr tracks via the subject's method name.
    #[test]
    fn sclass_expr() {
        assert_eq!(
            keys("class << blah\n  def foo; end\nend\n"),
            vec!["blah.foo"]
        );
    }

    // defs with self receiver inside an sclass: raw pmn, no humanize.
    #[test]
    fn defs_self_inside_sclass() {
        assert_eq!(
            keys("class A\n  class << self\n    def self.x; end\n  end\nend\n"),
            vec!["A::#<Class:A>.x"]
        );
    }

    // Zero-arg casgn Class.new block contributes the casgn name.
    #[test]
    fn casgn_class_new() {
        assert_eq!(
            keys("Foo = Class.new do\n  def x; end\nend\n"),
            vec!["Foo#x"]
        );
    }

    // Args on Class.new break the casgn skip: anonymous path with the
    // casgn-derived base and a path:line scope id.
    #[test]
    fn casgn_class_new_args() {
        let evts = events("Foo = Class.new(Base) do\n  def x; end\nend\n");
        assert_eq!(evts.len(), 1);
        assert_eq!(evts[0].0, "Foo::Object#x");
        assert_eq!(evts[0].2, 1); // @path:1
    }

    // lvasgn Class.new is fully excluded.
    #[test]
    fn lvasgn_excluded() {
        assert!(keys("x = Class.new do\n  def q; end\nend\n").is_empty());
    }

    // ...but parenthesizing the value breaks the lvasgn parent check.
    #[test]
    fn lvasgn_parens_not_excluded() {
        let evts = events("x = (Class.new do\n  def q; end\nend)\n");
        assert_eq!(evts.len(), 1);
        assert_eq!(evts[0].0, "Object#q");
        assert_eq!(evts[0].2, -1);
    }

    // Bare Class.new block at top level: `Object#x`, no scope id.
    #[test]
    fn bare_class_new() {
        let evts = events("Class.new do\n  def x; end\nend\n");
        assert_eq!(evts[0].0, "Object#x");
        assert_eq!(evts[0].2, -1);
    }

    // Class.new inside a describe block: `::Object#x` + path:line id.
    #[test]
    fn class_new_in_describe() {
        let evts = events("describe 'a' do\n  Class.new do\n    def x; end\n  end\nend\n");
        assert_eq!(evts.len(), 1);
        assert_eq!(evts[0].0, "::Object#x");
        assert_eq!(evts[0].2, 2);
    }

    // Chained receiver: `x.foo(Class.new do ... end)` gets a static
    // receiver.method scope id.
    #[test]
    fn anon_in_call_args_with_receiver() {
        let evts = events("x.foo(Class.new do\n  def y; end\nend)\n");
        assert_eq!(evts[0].0, "Object#y@x.foo");
    }

    // Chained off the anon block itself: `::Object` base (the outer tap
    // block aborts the anon's own pmn) + path:line.
    #[test]
    fn anon_chain_tap() {
        let evts = events("Class.new do\n  def x; end\nend.tap { 1 }\n");
        assert_eq!(evts[0].0, "::Object#x");
        assert_eq!(evts[0].1, "::Object#x");
        assert_eq!(evts[0].2, 1);
    }

    // Nested def: method_key prefixes the ancestor def name.
    #[test]
    fn nested_def_key() {
        assert_eq!(
            keys("def outer\n  def inner; end\n  def inner; end\nend\n"),
            vec!["Object#outer", "outer.Object#inner", "outer.Object#inner"]
        );
    }

    // Single-statement def body: anon scope id is path:line; multi-
    // statement wraps in (begin) whose parent is the def -> no scope id.
    #[test]
    fn anon_in_def_body_statement_count() {
        let single = events("def m\n  Class.new do\n    def x; end\n  end\nend\n");
        assert_eq!(single[1].0, "m.Object#x");
        assert_eq!(single[1].2, 2);
        let multi = events("def m\n  y = 1\n  Class.new do\n    def x; end\n  end\nend\n");
        assert_eq!(multi[1].0, "m.Object#x");
        assert_eq!(multi[1].2, -1);
    }

    // rescue / ensure scopes: parser nests (ensure (rescue ...) ...).
    #[test]
    fn rescue_ensure_scopes() {
        let evts = events(
            "def foo; end\nbegin\n  def foo; end\nrescue\n  def foo; end\nensure\n  def foo; end\nend\n",
        );
        let scopes: Vec<u8> = evts.iter().map(|e| e.4).collect();
        assert_eq!(scopes, vec![0, 1, 1, 2]);
    }

    // Ensure-only begin: body and ensure both map to :ensure.
    #[test]
    fn ensure_only() {
        let evts = events("begin\n  def foo; end\nensure\n  def foo; end\nend\n");
        let scopes: Vec<u8> = evts.iter().map(|e| e.4).collect();
        assert_eq!(scopes, vec![2, 2]);
    }

    // Modifier rescue wraps both sides in :rescue.
    #[test]
    fn rescue_modifier() {
        let evts = events("def foo; end rescue nil\n");
        assert_eq!(evts[0].4, 1);
    }

    // if-ancestors suppress defs.
    #[test]
    fn if_suppresses_def() {
        assert_eq!(keys("if a\n  def foo; end\nend\ndef foo; end\n"), vec!["Object#foo"]);
        assert_eq!(keys("def foo; end if a\n"), Vec::<String>::new());
        assert_eq!(keys("x = a ? (def foo; end) : nil\n"), Vec::<String>::new());
    }

    // ...but not attr_*.
    #[test]
    fn if_does_not_suppress_attr() {
        assert_eq!(keys("if a\n  attr_reader :foo\nend\n"), vec!["Object#foo"]);
    }

    // attr family.
    #[test]
    fn attr_family() {
        assert_eq!(
            keys("class A\n  attr_accessor :foo\nend\n"),
            vec!["A#foo", "A#foo="]
        );
        assert_eq!(keys("class A\n  attr :foo, true\nend\n"), vec!["A#foo", "A#foo="]);
        // attr tracks only its first argument.
        assert_eq!(keys("class A\n  attr :a, :b\nend\n"), vec!["A#a"]);
        // Block-pass counts as a trailing parser argument: 3 "args" != 2.
        assert_eq!(keys("class A\n  attr :foo, true, &b\nend\n"), vec!["A#foo"]);
        // Strings are ignored by sym_name.
        assert_eq!(keys("class A\n  attr_reader :a, 'b', :c\nend\n"), vec!["A#a", "A#c"]);
    }

    // alias / alias_method.
    #[test]
    fn alias_forms() {
        assert_eq!(keys("class A\n  alias foo bar\nend\n"), vec!["A#foo"]);
        assert_eq!(keys("class A\n  alias foo foo\nend\n"), Vec::<String>::new());
        assert_eq!(
            keys("class A\n  alias_method :foo, :bar\nend\n"),
            vec!["A#foo"]
        );
        assert_eq!(
            keys("class A\n  alias_method 'foo', 'bar'\nend\n"),
            Vec::<String>::new()
        );
        assert_eq!(
            keys("class A\n  alias_method :foo, :bar, :baz\nend\n"),
            Vec::<String>::new()
        );
    }

    // Forwardable delegators.
    #[test]
    fn delegators() {
        assert_eq!(
            keys("class A\n  def_delegator :t, :foo\nend\n"),
            vec!["A#foo"]
        );
        assert_eq!(
            keys("class A\n  def_delegator :t, :orig, :foo\nend\n"),
            vec!["A#foo"]
        );
        assert_eq!(
            keys("class A\n  def_delegators :t, :foo, :bar\nend\n"),
            vec!["A#foo", "A#bar"]
        );
    }

    // delegate needs ActiveSupportExtensionsEnabled.
    #[test]
    fn delegate_gated() {
        let src = "class A\n  delegate :foo, to: :bar\nend\n";
        assert!(keys(src).is_empty());
        let evts = check_duplicate_methods(
            src.as_bytes(),
            &Config {
                active_support_extensions_enabled: true,
            },
        );
        assert_eq!(evts[0].key, "A#foo");
    }

    // delegate prefix handling.
    #[test]
    fn delegate_prefix() {
        let cfg = Config {
            active_support_extensions_enabled: true,
        };
        let k = |src: &str| {
            check_duplicate_methods(src.as_bytes(), &cfg)
                .into_iter()
                .map(|e| e.key)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            k("class A\n  delegate :foo, to: :bar, prefix: true\nend\n"),
            vec!["A#bar_foo"]
        );
        assert_eq!(
            k("class A\n  delegate :foo, to: :bar, prefix: 'pre'\nend\n"),
            vec!["A#pre_foo"]
        );
        // Splat kwargs break the hash pattern.
        assert_eq!(
            k("class A\n  delegate :foo, **opts\nend\n"),
            Vec::<String>::new()
        );
        // A block-pass arg breaks the trailing-hash shape.
        assert_eq!(
            k("class A\n  delegate :foo, to: :bar, &b\nend\n"),
            Vec::<String>::new()
        );
    }

    // class_eval semantics: const receiver contributes, expr aborts,
    // receiverless skips; only `class_eval` (not module_eval/class_exec).
    #[test]
    fn class_eval_forms() {
        assert_eq!(
            keys("A.class_eval do\n  def foo; end\nend\n"),
            vec!["A#foo"]
        );
        assert!(keys("blah.class_eval do\n  def foo; end\nend\n").is_empty());
        assert_eq!(
            keys("class A\n  class_eval do\n    def foo; end\n  end\nend\n"),
            vec!["A#foo"]
        );
        assert!(keys("A.module_eval do\n  def foo; end\nend\n").is_empty());
        assert!(keys("A.class_exec do\n  def foo; end\nend\n").is_empty());
    }

    // numblock / itblock ancestors are invisible.
    #[test]
    fn numblock_invisible() {
        assert_eq!(
            keys("class A\n  foo { _1; def x; end }\nend\n"),
            vec!["A#x"]
        );
        assert_eq!(
            keys("class A\n  foo { it; def x; end }\nend\n"),
            vec!["A#x"]
        );
        assert!(keys("class A\n  foo { def x; end }\nend\n").is_empty());
    }

    // Struct.new blocks are entirely invisible (not Class/Module).
    #[test]
    fn struct_new_invisible() {
        assert!(keys("Foo = Struct.new(:a) do\n  def x; end\nend\n").is_empty());
    }

    // def on a const receiver: lookup through the enclosing class chain.
    #[test]
    fn defs_const_lookup() {
        assert_eq!(
            keys("class A\n  def A.foo; end\nend\n"),
            vec!["A.foo"]
        );
        assert_eq!(
            keys("module A\n  module B\n    module C\n      def B.foo; end\n    end\n  end\nend\n"),
            vec!["A::B.foo"]
        );
    }

    // Unresolvable const receiver: sexp fallback event.
    #[test]
    fn defs_const_sexp_fallback() {
        let evts = events("def B.foo; end\n");
        assert_eq!(evts.len(), 1);
        assert_eq!(evts[0].1, "foo"); // short name only
        assert_eq!(evts[0].3, 0); // sexp_start = defs node start
        assert_eq!(evts[0].0, ""); // no method_key prefix
    }

    // defs on self at top level.
    #[test]
    fn defs_self_top_level() {
        assert_eq!(keys("def self.foo; end\n"), vec!["Object.foo"]);
    }

    // cbase class name drops the leading `::`.
    #[test]
    fn cbase_class() {
        assert_eq!(keys("class ::A\n  def foo; end\nend\n"), vec!["A#foo"]);
    }

    // `class self::B` interpolates a nil const_name.
    #[test]
    fn self_colon_class() {
        assert_eq!(
            keys("class A\n  class self::B\n    def x; end\n  end\nend\n"),
            vec!["A::::B#x"]
        );
    }

    // Offense location for defs: keyword..name.
    #[test]
    fn def_offense_location() {
        let evts = check_duplicate_methods(
            b"class A\n  def self.foo; end\nend\n",
            &Config {
                active_support_extensions_enabled: false,
            },
        );
        let src = "class A\n  def self.foo; end\nend\n";
        assert_eq!(&src[evts[0].off_start..evts[0].off_end], "def self.foo");
    }

    // Send events exclude an attached literal block from the range.
    #[test]
    fn send_range_excludes_block() {
        let src = "class A\n  alias_method :a, :b\nend\n";
        let evts = check_duplicate_methods(
            src.as_bytes(),
            &Config {
                active_support_extensions_enabled: false,
            },
        );
        assert_eq!(&src[evts[0].off_start..evts[0].off_end], "alias_method :a, :b");
    }
}
