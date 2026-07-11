//! `Metrics/AbcSize`.
//!
//! Mirrors `RuboCop::Cop::Metrics::Utils::AbcSizeCalculator` (plus its
//! `RepeatedCsendDiscount` / `RepeatedAttributeDiscount` mixins) over the prism
//! AST. The calculator is a post-order (`visit_depth_last`) walk of each method
//! body over the *parser-gem* AST, so this module emulates the parser node
//! sequence where prism's shape differs (operator writes, multiple assignment,
//! `unless` clause order, blocks, lambdas, `=~` with named captures, ...). The
//! ordering matters: the csend and repeated-attribute discounts are stateful,
//! so e.g. a call with a block must be counted *before* the block body (the
//! parser nests the send inside the block node), and an `unless` else-clause
//! before its body. Every counting rule below was verified against a stock
//! probe (`.tmp/2026-06-13/abc-size/probe*.rb`); the quirks are deliberate.
//!
//! Scoring rules (per method body, A/B/C):
//!
//! - A (assignments): variable assignments (`x =`, `@x =`, `X =`, multiple
//!   assignment targets, `for` loop and `rescue =>` references), attribute
//!   writes (`x.foo =`, `x[0] =`), the lhs of compound assignments, `for`
//!   itself, and block/def arguments. Local names starting with `_` never
//!   count.
//! - B (branches): method calls (`send`/`csend`), yields, and `->` lambda
//!   literals (the parser gem emits them as a `lambda` send). Comparison
//!   operators count as C instead.
//! - C (conditions): `if`/`unless`/ternary (+1 more for a literal `else`
//!   keyword), `while`/`until` (but *not* `begin..end while` post-loops),
//!   `for`, `when`, `in`, `rescue`, `and`/`or`, `||=`/`&&=`, comparison
//!   operators, safe navigation (deduplicated per untouched local receiver),
//!   and iterating blocks (`.map {}`; numbered/`it` blocks are *not* counted —
//!   `numblock`/`itblock` are missing from `CyclomaticComplexity::COUNTED_NODES`).
//!
//! With `CountRepeatedAttributes: false`, repeated no-argument call chains on
//! the same root (`var.foo.bar` twice, bare `foo` twice, even repeated `->`
//! lambdas) are discounted, with setter/variable writes invalidating their
//! chain subtrees (`RepeatedAttributeDiscount`).

use std::collections::{HashMap, HashSet};

use ruby_prism::{Node, Visit, visit_call_node, visit_def_node};

use super::complexity::{define_method_info, is_iterating};

/// Per-method ABC result. The Ruby side derives the score
/// (`Math.sqrt(a**2 + b**2 + c**2).round(2)`), the vector string and the
/// offense message, so floats never cross the FFI boundary.
pub struct AbcMethod {
    pub start_offset: usize,
    pub end_offset: usize,
    /// End of the offense head (method name for `def`, block opening for
    /// `define_method`), used by the LSP location mode.
    pub head_end: usize,
    pub method_name: String,
    pub assignments: u64,
    pub branches: u64,
    pub conditions: u64,
}

/// Standalone entry point. Reports methods whose squared vector exceeds
/// `max_floor**2` (`max_floor` is `Max.floor` — conservative for a float
/// `Max`, exact for an integer one; the Ruby side re-applies the exact
/// `complexity > Max` filter). A negative `max_floor` reports every method.
pub fn check_abc_size(source: &[u8], max_floor: i64, discount_repeated: bool) -> Vec<AbcMethod> {
    let mut finder = build_rule(source, max_floor, discount_repeated);
    super::parse_cache::with_parsed(source, |_source, node| finder.visit(node));
    finder.out
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule(source: &[u8], max_floor: i64, discount_repeated: bool) -> AbcFinder<'_> {
    AbcFinder {
        source,
        max_floor,
        discount_repeated,
        out: Vec::new(),
    }
}

// --- Method discovery (same shape as `complexity::MethodFinder`) ------------

pub(crate) struct AbcFinder<'a> {
    source: &'a [u8],
    max_floor: i64,
    discount_repeated: bool,
    pub(crate) out: Vec<AbcMethod>,
}

impl AbcFinder<'_> {
    fn record(&mut self, start: usize, end: usize, head_end: usize, name: String, body: &Node<'_>) {
        let (a, b, c) = score_body(self.source, body, self.discount_repeated);
        if self.max_floor >= 0 {
            let floor = self.max_floor as u64;
            if a * a + b * b + c * c <= floor * floor {
                return;
            }
        }
        self.out.push(AbcMethod {
            start_offset: start,
            end_offset: end,
            head_end,
            method_name: name,
            assignments: a,
            branches: b,
            conditions: c,
        });
    }

    fn process_def(&mut self, node: &ruby_prism::DefNode<'_>) {
        if let Some(body) = node.body() {
            let loc = node.location();
            let name = String::from_utf8_lossy(node.name().as_slice()).into_owned();
            self.record(
                loc.start_offset(),
                loc.end_offset(),
                node.name_loc().end_offset(),
                name,
                &body,
            );
        }
    }

    fn process_call(&mut self, node: &ruby_prism::CallNode<'_>) {
        if let Some((name, body, head_end)) = define_method_info(node) {
            let loc = node.location();
            self.record(loc.start_offset(), loc.end_offset(), head_end, name, &body);
        }
    }
}

impl<'pr> Visit<'pr> for AbcFinder<'_> {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        self.process_def(node);
        visit_def_node(self, node);
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.process_call(node);
        visit_call_node(self, node);
    }
}

/// Shared-walk driver. Same `MatchWriteNode` caveat as the complexity finder:
/// the `=~` call reached through that typed field is never a `define_method`
/// block, so missing it is harmless.
impl super::dispatch::Rule for AbcFinder<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_CALL
                    | Interest::ENTER_DEF,
        )
    }
    
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(def) = node.as_def_node() {
            self.process_def(&def);
        } else if let Some(call) = node.as_call_node() {
            self.process_call(&call);
        }
    }

    fn leave(&mut self) {}
}

// --- Scoring ----------------------------------------------------------------

const COMPARISON_METHODS: &[&[u8]] = &[b"==", b"===", b"!=", b"<=", b">=", b">", b"<"];

fn is_comparison(name: &[u8]) -> bool {
    COMPARISON_METHODS.contains(&name)
}

/// `capturing_variable?`: a non-`_`-prefixed name captures.
fn capturing(name: &[u8]) -> bool {
    !name.starts_with(b"_")
}

/// Root of an attribute chain, mirroring `RepeatedAttributeDiscount`'s
/// `root_node?` keys. `self` and no-receiver share one bucket (stock seeds the
/// same hash under both keys); constants key by structural identity.
#[derive(Hash, PartialEq, Eq)]
enum RootKey {
    SelfNil,
    Lvar(Vec<u8>),
    Ivar(Vec<u8>),
    Cvar(Vec<u8>),
    Gvar(Vec<u8>),
    Const(Vec<u8>),
}

/// The `known_attributes` trie: per root, a nested map of no-arg method names.
#[derive(Default)]
struct AttrTrie {
    roots: HashMap<RootKey, usize>,
    nodes: Vec<HashMap<Vec<u8>, usize>>,
}

impl AttrTrie {
    fn push_node(&mut self) -> usize {
        self.nodes.push(HashMap::new());
        self.nodes.len() - 1
    }

    /// `discount_repeated_attribute?`: walks root + methods, creating missing
    /// components. Returns true (repeated) only when nothing was missing.
    ///
    /// Stock pre-seeds the self/nil bucket, but the distinction is
    /// unobservable: a root miss already implies the final component was
    /// missing too, and the created-on-miss bucket behaves identically after.
    fn lookup_insert(&mut self, root: RootKey, methods: &[&[u8]]) -> bool {
        let mut missing = false;
        let mut idx = match self.roots.get(&root) {
            Some(&idx) => idx,
            None => {
                missing = true;
                let idx = self.push_node();
                self.roots.insert(root, idx);
                idx
            }
        };
        for &m in methods {
            idx = match self.nodes[idx].get(m) {
                Some(&next) => next,
                None => {
                    missing = true;
                    let next = self.push_node();
                    self.nodes[idx].insert(m.to_vec(), next);
                    next
                }
            };
        }
        !missing
    }

    /// `update_repeated_attribute` for a setter: resolve the receiver path
    /// without creating anything, then drop `method` (and its subtree).
    fn delete(&mut self, root: &RootKey, methods: &[&[u8]], method: &[u8]) {
        let Some(&idx) = self.roots.get(root) else {
            return;
        };
        let mut idx = idx;
        for &m in methods {
            match self.nodes[idx].get(m) {
                Some(&next) => idx = next,
                None => return,
            }
        }
        self.nodes[idx].remove(method);
    }

    /// `update_repeated_attribute` for a variable write: clear the root's
    /// subtree if the root is known.
    fn clear_root(&mut self, root: &RootKey) {
        if let Some(&idx) = self.roots.get(root) {
            self.nodes[idx].clear();
        }
    }
}

fn score_body(source: &[u8], body: &Node<'_>, discount_repeated: bool) -> (u64, u64, u64) {
    let mut scorer = AbcScorer {
        source,
        a: 0,
        b: 0,
        c: 0,
        csend_vars: HashSet::new(),
        trie: discount_repeated.then(AttrTrie::default),
        in_pattern: false,
        masgn_depth: 0,
    };
    scorer.visit(body);
    (scorer.a, scorer.b, scorer.c)
}

struct AbcScorer<'a> {
    source: &'a [u8],
    a: u64,
    b: u64,
    c: u64,
    /// Local variables that already had a counted `&.` since their last
    /// assignment (`RepeatedCsendDiscount`).
    csend_vars: HashSet<Vec<u8>>,
    /// `Some` iff `CountRepeatedAttributes: false`.
    trie: Option<AttrTrie>,
    /// Inside a pattern (`in`/`=>`): variable targets are `match_var`s in the
    /// parser gem, which count nothing.
    in_pattern: bool,
    /// Inside the target list of a `masgn`: call/index targets are counted as
    /// assignments by `compound_assignment` (their setter form is undetectable
    /// without an operator location).
    masgn_depth: u32,
}

/// The pieces of a (possibly virtual) parser `send`/`csend` needed to count it.
struct CallView<'n, 'pr> {
    receiver: Option<&'n Node<'pr>>,
    name: &'pr [u8],
    /// `setter_method?` (assignment form; parser's `loc?(:operator)` ⟺
    /// prism's `ATTRIBUTE_WRITE` flag — verified for `x.y = 1`, `x.y=(1)` and
    /// `x[0] = 1`).
    attribute_write: bool,
    safe_nav: bool,
    /// `(call _receiver _method)` with no arguments — a block-pass counts as
    /// an argument, an attached block node does not.
    is_attribute: bool,
}

/// Resolves a receiver as an attribute-chain prefix: the root key plus the
/// method names from the root outward, or `None` when the chain breaks
/// (mirrors `root_node?` + the `attribute_call?` recursion: only no-arg,
/// no-block calls link the chain — in the parser gem a receiver with any kind
/// of block is the `block` node or has a `block_pass` argument, breaking it).
fn attr_chain<'pr>(node: Option<&Node<'pr>>, methods: &mut Vec<&'pr [u8]>) -> Option<RootKey> {
    let Some(node) = node else {
        return Some(RootKey::SelfNil);
    };
    if node.as_self_node().is_some() {
        return Some(RootKey::SelfNil);
    }
    if let Some(v) = node.as_local_variable_read_node() {
        return Some(RootKey::Lvar(v.name().as_slice().to_vec()));
    }
    if let Some(v) = node.as_instance_variable_read_node() {
        return Some(RootKey::Ivar(v.name().as_slice().to_vec()));
    }
    if let Some(v) = node.as_class_variable_read_node() {
        return Some(RootKey::Cvar(v.name().as_slice().to_vec()));
    }
    if let Some(v) = node.as_global_variable_read_node() {
        return Some(RootKey::Gvar(v.name().as_slice().to_vec()));
    }
    if node.as_constant_read_node().is_some() || node.as_constant_path_node().is_some() {
        return Some(RootKey::Const(const_key(node)));
    }
    if let Some(call) = node.as_call_node()
        && call.arguments().is_none()
        && call.block().is_none()
    {
        let receiver = call.receiver();
        let root = attr_chain(receiver.as_ref(), methods)?;
        methods.push(call.name().as_slice());
        return Some(root);
    }
    None
}

/// Serialization of a constant root for structural identity (`(const nil :A)`
/// vs `(const (cbase) :A)` vs `(const (const nil :B) :A)` are distinct hash
/// keys in stock). A non-constant parent falls back to its source offset,
/// which under-discounts only structurally-equal-but-distinct occurrences of
/// an expression-scoped constant — a shape the corpus never exercises.
fn const_key(node: &Node<'_>) -> Vec<u8> {
    if let Some(c) = node.as_constant_read_node() {
        return c.name().as_slice().to_vec();
    }
    if let Some(path) = node.as_constant_path_node() {
        let mut key = match path.parent() {
            Some(parent) => const_key(&parent),
            None => Vec::new(), // `::A` (cbase)
        };
        key.extend_from_slice(b"::");
        if let Some(name) = path.name() {
            key.extend_from_slice(name.as_slice());
        }
        return key;
    }
    let loc = node.location();
    let mut key = b"expr:".to_vec();
    key.extend_from_slice(&loc.start_offset().to_ne_bytes());
    key
}

impl AbcScorer<'_> {
    // --- Attribute-trie operations (`RepeatedAttributeDiscount`) ------------

    fn trie_lookup(&mut self, receiver: Option<&Node<'_>>, name: &[u8]) -> bool {
        let mut methods: Vec<&[u8]> = Vec::new();
        let Some(root) = attr_chain(receiver, &mut methods) else {
            return false;
        };
        methods.push(name);
        self.trie
            .as_mut()
            .expect("caller checked")
            .lookup_insert(root, &methods)
    }

    /// Setter invalidation: `calls.delete(getter_name)` resolved through the
    /// receiver chain (no creation on missing components).
    fn trie_delete(&mut self, receiver: Option<&Node<'_>>, getter_name: &[u8]) {
        if self.trie.is_none() {
            return;
        }
        let mut methods: Vec<&[u8]> = Vec::new();
        let Some(root) = attr_chain(receiver, &mut methods) else {
            return;
        };
        self.trie
            .as_mut()
            .expect("checked above")
            .delete(&root, &methods, getter_name);
    }

    fn trie_clear(&mut self, root: RootKey) {
        if let Some(trie) = self.trie.as_mut() {
            trie.clear_root(&root);
        }
    }

    // --- Shared counting steps ----------------------------------------------

    /// `RepeatedCsendDiscount#discount_for_repeated_csend?`: only untouched
    /// local-variable receivers discount.
    fn csend_contribution(&mut self, receiver: Option<&Node<'_>>) -> u64 {
        if let Some(recv) = receiver
            && let Some(lvar) = recv.as_local_variable_read_node()
        {
            let name = lvar.name().as_slice().to_vec();
            if self.csend_vars.contains(&name) {
                return 0;
            }
            self.csend_vars.insert(name);
        }
        1
    }

    /// `calculate_node` for a send/csend: the prepended attribute update, the
    /// assignment count and `evaluate_branch_nodes`.
    fn count_call(&mut self, view: CallView<'_, '_>) {
        // update_repeated_attribute (prepended): a setter deletes its getter
        // from the receiver's chain.
        if view.attribute_write {
            let getter = &view.name[..view.name.len() - 1];
            self.trie_delete(view.receiver, getter);
        }
        // assignment?: setter_method?
        if view.attribute_write {
            self.a += 1;
        }
        // evaluate_branch_nodes; RepeatedAttributeDiscount prepends the
        // repeated-attribute early return (skipping B *and* the csend C).
        if self.trie.is_some()
            && view.is_attribute
            && self.trie_lookup(view.receiver, view.name)
        {
            return;
        }
        if is_comparison(view.name) {
            self.c += 1;
        } else {
            self.b += 1;
            if view.safe_nav {
                self.c += self.csend_contribution(view.receiver);
            }
        }
    }

    /// Effects of a parser `lvasgn` node (standalone, in a shorthand lhs, in a
    /// `masgn`/`for`/`rescue =>` target position): attribute invalidation,
    /// csend-discount reset, and an assignment when the name captures.
    fn lvasgn_effects(&mut self, name: &[u8]) {
        self.trie_clear(RootKey::Lvar(name.to_vec()));
        self.csend_vars.remove(name);
        if capturing(name) {
            self.a += 1;
        }
    }

    /// Effects of `ivasgn`/`cvasgn`/`gvasgn`: invalidation + assignment
    /// (always counted; the `_` rule is for locals only).
    fn var_effects(&mut self, root: RootKey) {
        self.trie_clear(root);
        self.a += 1;
    }

    /// `compound_assignment`'s contribution for an op-/or-/and-asgn (and the
    /// shorthand `call`/`index` forms): the parser counts each child that
    /// `respond_to?(:setter_method?) && !setter_method?`. The target arm is a
    /// bare `(lvasgn ...)` etc. (no `setter_method?`), so only the *value*
    /// matters: it counts iff it is a `send`/`csend` (and not a setter). In the
    /// parser gem a send with a literal `{ }`/`do..end` block is a `block` node
    /// (no `setter_method?`, so it does *not* count), but a send with a
    /// block-PASS (`&:to_sym`) stays a `send` (block-pass is an argument), so it
    /// does count. In prism both attach via `block()`: a `BlockNode`
    /// disqualifies, a `BlockArgumentNode` (block-pass) does not. Non-call
    /// values (literals, `if`, `and`, arrays, ...) never count.
    fn compound_value_assignment(&mut self, value: &Node<'_>) {
        if let Some(call) = value.as_call_node()
            && !call.is_attribute_write()
            && call
                .block()
                .is_none_or(|b| b.as_block_node().is_none())
        {
            self.a += 1;
        }
    }

    /// An `ElseNode` whose keyword is literally `else` (ternaries carry `:`).
    fn literal_else(&self, subsequent: Option<&Node<'_>>) -> bool {
        let Some(sub) = subsequent else { return false };
        let Some(else_node) = sub.as_else_node() else {
            return false;
        };
        let loc = else_node.else_keyword_loc();
        &self.source[loc.start_offset()..loc.end_offset()] == b"else"
    }
}

impl<'pr> AbcScorer<'_> {
    /// Visit a block attached to a call (parser order: the send is counted
    /// before the block's parameters and body), then count the block node:
    /// +1 C for an iterating method, except `numblock`/`itblock` forms which
    /// are missing from `COUNTED_NODES`.
    fn visit_attached_block(&mut self, block: &ruby_prism::BlockNode<'pr>, method_name: &[u8]) {
        let mut numbered = false;
        if let Some(params) = block.parameters() {
            numbered = params.as_numbered_parameters_node().is_some()
                || params.as_it_parameters_node().is_some();
            self.visit(&params);
        }
        if let Some(body) = block.body() {
            self.visit(&body);
        }
        if !numbered && is_iterating(method_name) {
            self.c += 1;
        }
    }

    /// Visit a pattern subtree (`in` / `=>`): parser emits `match_var`s there,
    /// so variable targets are inert, while embedded code (pins, interpolation)
    /// still counts.
    fn visit_pattern(&mut self, pattern: &Node<'pr>) {
        let saved = self.in_pattern;
        self.in_pattern = true;
        self.visit(pattern);
        self.in_pattern = saved;
    }

    /// `recv.attr op= v`: parser visits the getter send (full branch
    /// treatment), then the op-asgn node counts one compound assignment and
    /// deletes the getter from the receiver chain (`||=`/`&&=` add a
    /// condition; plain `op=` does not).
    fn call_shorthand(
        &mut self,
        receiver: Option<Node<'pr>>,
        read_name: &'pr [u8],
        safe_nav: bool,
        value: &Node<'pr>,
        condition: bool,
    ) {
        if let Some(recv) = &receiver {
            self.visit(recv);
        }
        self.count_call(CallView {
            receiver: receiver.as_ref(),
            name: read_name,
            attribute_write: false,
            safe_nav,
            is_attribute: true,
        });
        self.visit(value);
        self.trie_delete(receiver.as_ref(), read_name);
        self.a += 1;
        self.compound_value_assignment(value);
        if condition {
            self.c += 1;
        }
    }

    /// `recv[idx] op= v`: like `call_shorthand` with the `[]` getter; the
    /// shorthand update destructures `[recv, :[], idx...]` into `[recv, :[]]`.
    fn index_shorthand(
        &mut self,
        receiver: Option<Node<'pr>>,
        arguments: Option<ruby_prism::ArgumentsNode<'pr>>,
        block: Option<ruby_prism::BlockArgumentNode<'pr>>,
        value: &Node<'pr>,
        condition: bool,
    ) {
        if let Some(recv) = &receiver {
            self.visit(recv);
        }
        let mut has_args = false;
        if let Some(args) = arguments {
            for arg in &args.arguments() {
                has_args = true;
                self.visit(&arg);
            }
        }
        if let Some(block_arg) = block {
            has_args = true;
            if let Some(expr) = block_arg.expression() {
                self.visit(&expr);
            }
        }
        self.count_call(CallView {
            receiver: receiver.as_ref(),
            name: b"[]",
            attribute_write: false,
            safe_nav: false,
            is_attribute: !has_args,
        });
        self.visit(value);
        self.trie_delete(receiver.as_ref(), b"[]");
        self.a += 1;
        self.compound_value_assignment(value);
        if condition {
            self.c += 1;
        }
    }

    fn constant_path_shorthand_delete(&mut self, target: &ruby_prism::ConstantPathNode<'pr>) {
        // `(casgn scope :B)` destructures into `[scope, :B]`: delete `B` from
        // the scope bucket. A cbase scope (`::B`) is not a root node in stock —
        // no deletion.
        let Some(name) = target.name() else { return };
        if let Some(parent) = target.parent() {
            self.trie_delete(Some(&parent), name.as_slice());
        }
    }
}

impl<'pr> Visit<'pr> for AbcScorer<'_> {
    // --- Calls (send/csend) with their attached blocks -----------------------

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let receiver = node.receiver();
        if let Some(recv) = &receiver {
            self.visit(recv);
        }
        let mut has_args = false;
        if let Some(args) = node.arguments() {
            for arg in &args.arguments() {
                has_args = true;
                self.visit(&arg);
            }
        }
        let block = node.block();
        let mut has_block_pass = false;
        if let Some(block_arg) = block.as_ref().and_then(|b| b.as_block_argument_node()) {
            // A block-pass is an argument child of the send in parser: visited
            // (and its `block_pass` condition counted) before the send itself.
            has_block_pass = true;
            if let Some(expr) = block_arg.expression() {
                self.visit(&expr);
            }
            if is_iterating(node.name().as_slice()) {
                self.c += 1;
            }
        }
        // A static regexp literal on the lhs of `=~` is `match_with_lvasgn` in
        // the parser gem — with or without a named capture, `/x/o` included —
        // so it is no send and counts no branch; only the operands (visited
        // above) count. prism keeps this a plain `CallNode` when there is no
        // named capture (the named-capture form is a `MatchWriteNode`, handled
        // by `visit_match_write_node`), so skip the branch count here. A
        // dynamic regexp (`/#{x}/ =~ y`), `!~`, a parenthesized regexp
        // (`(/x/) =~ y`) and a regexp on the rhs (`y =~ /x/`) all stay real
        // sends in the parser gem and count as usual.
        let regexp_match_lhs = node.name().as_slice() == b"=~"
            && receiver
                .as_ref()
                .is_some_and(|r| r.as_regular_expression_node().is_some());
        if !regexp_match_lhs {
            self.count_call(CallView {
                receiver: receiver.as_ref(),
                name: node.name().as_slice(),
                attribute_write: node.is_attribute_write(),
                safe_nav: node.is_safe_navigation(),
                is_attribute: !has_args && !has_block_pass,
            });
        }
        if let Some(block_node) = block.as_ref().and_then(|b| b.as_block_node()) {
            self.visit_attached_block(&block_node, node.name().as_slice());
        }
    }

    /// `->` literals are `(block (send nil :lambda) ...)` in the parser gem: a
    /// counted (and attribute-discountable) `lambda` send plus a
    /// never-iterating block.
    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        self.count_call(CallView {
            receiver: None,
            name: b"lambda",
            attribute_write: false,
            safe_nav: false,
            is_attribute: true,
        });
        if let Some(params) = node.parameters() {
            self.visit(&params);
        }
        if let Some(body) = node.body() {
            self.visit(&body);
        }
    }

    fn visit_yield_node(&mut self, node: &ruby_prism::YieldNode<'pr>) {
        ruby_prism::visit_yield_node(self, node);
        self.b += 1;
    }

    /// `/(?<g>x)/ =~ str` is `match_with_lvasgn` in the parser gem: no send,
    /// no assignments — only the operands count.
    fn visit_match_write_node(&mut self, node: &ruby_prism::MatchWriteNode<'pr>) {
        let call = node.call();
        if let Some(receiver) = call.receiver() {
            self.visit(&receiver);
        }
        if let Some(args) = call.arguments() {
            for arg in &args.arguments() {
                self.visit(&arg);
            }
        }
    }

    // --- Variable writes -----------------------------------------------------

    fn visit_local_variable_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableWriteNode<'pr>,
    ) {
        self.visit(&node.value());
        self.lvasgn_effects(node.name().as_slice());
    }

    fn visit_instance_variable_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableWriteNode<'pr>,
    ) {
        self.visit(&node.value());
        self.var_effects(RootKey::Ivar(node.name().as_slice().to_vec()));
    }

    fn visit_class_variable_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableWriteNode<'pr>,
    ) {
        self.visit(&node.value());
        self.var_effects(RootKey::Cvar(node.name().as_slice().to_vec()));
    }

    fn visit_global_variable_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableWriteNode<'pr>,
    ) {
        self.visit(&node.value());
        self.var_effects(RootKey::Gvar(node.name().as_slice().to_vec()));
    }

    /// `casgn`: an assignment, but never an attribute invalidation
    /// (`VAR_SETTER_TO_GETTER` has no `casgn` entry).
    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'pr>) {
        self.visit(&node.value());
        self.a += 1;
    }

    fn visit_constant_path_write_node(&mut self, node: &ruby_prism::ConstantPathWriteNode<'pr>) {
        if let Some(parent) = node.target().parent() {
            self.visit(&parent);
        }
        self.visit(&node.value());
        self.a += 1;
    }

    // --- Assignment targets (masgn / for / rescue-ref; inert in patterns) ----

    fn visit_local_variable_target_node(
        &mut self,
        node: &ruby_prism::LocalVariableTargetNode<'pr>,
    ) {
        if !self.in_pattern {
            self.lvasgn_effects(node.name().as_slice());
        }
    }

    fn visit_instance_variable_target_node(
        &mut self,
        node: &ruby_prism::InstanceVariableTargetNode<'pr>,
    ) {
        if !self.in_pattern {
            self.var_effects(RootKey::Ivar(node.name().as_slice().to_vec()));
        }
    }

    fn visit_class_variable_target_node(
        &mut self,
        node: &ruby_prism::ClassVariableTargetNode<'pr>,
    ) {
        if !self.in_pattern {
            self.var_effects(RootKey::Cvar(node.name().as_slice().to_vec()));
        }
    }

    fn visit_global_variable_target_node(
        &mut self,
        node: &ruby_prism::GlobalVariableTargetNode<'pr>,
    ) {
        if !self.in_pattern {
            self.var_effects(RootKey::Gvar(node.name().as_slice().to_vec()));
        }
    }

    fn visit_constant_target_node(&mut self, _node: &ruby_prism::ConstantTargetNode<'pr>) {
        if !self.in_pattern {
            self.a += 1;
        }
    }

    fn visit_constant_path_target_node(
        &mut self,
        node: &ruby_prism::ConstantPathTargetNode<'pr>,
    ) {
        if let Some(parent) = node.parent() {
            self.visit(&parent);
        }
        if !self.in_pattern {
            self.a += 1;
        }
    }

    /// `self.a, ... =` target: a setter send *without* an operator location in
    /// the parser gem, so `setter_method?` is false — no individual assignment
    /// and no invalidation; `compound_assignment` counts it instead (the
    /// `masgn_depth` guard), and it is a regular branch (attribute-eligible,
    /// method name keeping its `=` suffix).
    fn visit_call_target_node(&mut self, node: &ruby_prism::CallTargetNode<'pr>) {
        let receiver = node.receiver();
        self.visit(&receiver);
        if self.masgn_depth > 0 {
            self.a += 1;
        }
        self.count_call(CallView {
            receiver: Some(&receiver),
            name: node.name().as_slice(),
            attribute_write: false,
            safe_nav: node.is_safe_navigation(),
            is_attribute: true,
        });
    }

    fn visit_index_target_node(&mut self, node: &ruby_prism::IndexTargetNode<'pr>) {
        let receiver = node.receiver();
        self.visit(&receiver);
        let mut has_args = false;
        if let Some(args) = node.arguments() {
            for arg in &args.arguments() {
                has_args = true;
                self.visit(&arg);
            }
        }
        if let Some(block_arg) = node.block() {
            has_args = true;
            if let Some(expr) = block_arg.expression() {
                self.visit(&expr);
            }
        }
        if self.masgn_depth > 0 {
            self.a += 1;
        }
        self.count_call(CallView {
            receiver: Some(&receiver),
            name: b"[]=",
            attribute_write: false,
            safe_nav: false,
            is_attribute: !has_args,
        });
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        self.masgn_depth += 1;
        for left in &node.lefts() {
            self.visit(&left);
        }
        if let Some(rest) = node.rest() {
            self.visit(&rest);
        }
        for right in &node.rights() {
            self.visit(&right);
        }
        self.masgn_depth -= 1;
        self.visit(&node.value());
        // The masgn node itself adds nothing further: `compound_assignment`'s
        // per-target counts are folded into the target visits above, and
        // `masgn` neither resets nor invalidates.
    }

    // --- Operator / or- / and-assignments ------------------------------------

    fn visit_local_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        // Parser shape: `(op-asgn (lvasgn :x) :op value)` — the lvasgn child
        // (visited first) carries the assignment; `compound_assignment` then
        // counts the value when it is a non-setter send. The shorthand update
        // destructures into a bare symbol, a no-op.
        self.lvasgn_effects(node.name().as_slice());
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
    }

    fn visit_local_variable_or_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        self.lvasgn_effects(node.name().as_slice());
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.c += 1;
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        self.lvasgn_effects(node.name().as_slice());
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.c += 1;
    }

    fn visit_instance_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOperatorWriteNode<'pr>,
    ) {
        self.var_effects(RootKey::Ivar(node.name().as_slice().to_vec()));
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
    }

    fn visit_instance_variable_or_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableOrWriteNode<'pr>,
    ) {
        self.var_effects(RootKey::Ivar(node.name().as_slice().to_vec()));
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.c += 1;
    }

    fn visit_instance_variable_and_write_node(
        &mut self,
        node: &ruby_prism::InstanceVariableAndWriteNode<'pr>,
    ) {
        self.var_effects(RootKey::Ivar(node.name().as_slice().to_vec()));
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.c += 1;
    }

    fn visit_class_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOperatorWriteNode<'pr>,
    ) {
        self.var_effects(RootKey::Cvar(node.name().as_slice().to_vec()));
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
    }

    fn visit_class_variable_or_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableOrWriteNode<'pr>,
    ) {
        self.var_effects(RootKey::Cvar(node.name().as_slice().to_vec()));
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.c += 1;
    }

    fn visit_class_variable_and_write_node(
        &mut self,
        node: &ruby_prism::ClassVariableAndWriteNode<'pr>,
    ) {
        self.var_effects(RootKey::Cvar(node.name().as_slice().to_vec()));
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.c += 1;
    }

    fn visit_global_variable_operator_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOperatorWriteNode<'pr>,
    ) {
        self.var_effects(RootKey::Gvar(node.name().as_slice().to_vec()));
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
    }

    fn visit_global_variable_or_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableOrWriteNode<'pr>,
    ) {
        self.var_effects(RootKey::Gvar(node.name().as_slice().to_vec()));
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.c += 1;
    }

    fn visit_global_variable_and_write_node(
        &mut self,
        node: &ruby_prism::GlobalVariableAndWriteNode<'pr>,
    ) {
        self.var_effects(RootKey::Gvar(node.name().as_slice().to_vec()));
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.c += 1;
    }

    /// `X op= v`: the casgn child counts an assignment; the shorthand update
    /// destructures `(casgn nil :X)` into `[nil, :X]` and deletes `X` from the
    /// self/nil attribute bucket.
    fn visit_constant_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantOperatorWriteNode<'pr>,
    ) {
        self.a += 1;
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.trie_delete(None, node.name().as_slice());
    }

    fn visit_constant_or_write_node(&mut self, node: &ruby_prism::ConstantOrWriteNode<'pr>) {
        self.a += 1;
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.trie_delete(None, node.name().as_slice());
        self.c += 1;
    }

    fn visit_constant_and_write_node(&mut self, node: &ruby_prism::ConstantAndWriteNode<'pr>) {
        self.a += 1;
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.trie_delete(None, node.name().as_slice());
        self.c += 1;
    }

    fn visit_constant_path_operator_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOperatorWriteNode<'pr>,
    ) {
        let target = node.target();
        if let Some(parent) = target.parent() {
            self.visit(&parent);
        }
        self.a += 1;
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.constant_path_shorthand_delete(&target);
    }

    fn visit_constant_path_or_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathOrWriteNode<'pr>,
    ) {
        let target = node.target();
        if let Some(parent) = target.parent() {
            self.visit(&parent);
        }
        self.a += 1;
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.constant_path_shorthand_delete(&target);
        self.c += 1;
    }

    fn visit_constant_path_and_write_node(
        &mut self,
        node: &ruby_prism::ConstantPathAndWriteNode<'pr>,
    ) {
        let target = node.target();
        if let Some(parent) = target.parent() {
            self.visit(&parent);
        }
        self.a += 1;
        let value = node.value();
        self.visit(&value);
        self.compound_value_assignment(&value);
        self.constant_path_shorthand_delete(&target);
        self.c += 1;
    }

    fn visit_call_operator_write_node(
        &mut self,
        node: &ruby_prism::CallOperatorWriteNode<'pr>,
    ) {
        self.call_shorthand(
            node.receiver(),
            node.read_name().as_slice(),
            node.is_safe_navigation(),
            &node.value(),
            false,
        );
    }

    fn visit_call_or_write_node(&mut self, node: &ruby_prism::CallOrWriteNode<'pr>) {
        self.call_shorthand(
            node.receiver(),
            node.read_name().as_slice(),
            node.is_safe_navigation(),
            &node.value(),
            true,
        );
    }

    fn visit_call_and_write_node(&mut self, node: &ruby_prism::CallAndWriteNode<'pr>) {
        self.call_shorthand(
            node.receiver(),
            node.read_name().as_slice(),
            node.is_safe_navigation(),
            &node.value(),
            true,
        );
    }

    fn visit_index_operator_write_node(
        &mut self,
        node: &ruby_prism::IndexOperatorWriteNode<'pr>,
    ) {
        self.index_shorthand(
            node.receiver(),
            node.arguments(),
            node.block(),
            &node.value(),
            false,
        );
    }

    fn visit_index_or_write_node(&mut self, node: &ruby_prism::IndexOrWriteNode<'pr>) {
        self.index_shorthand(
            node.receiver(),
            node.arguments(),
            node.block(),
            &node.value(),
            true,
        );
    }

    fn visit_index_and_write_node(&mut self, node: &ruby_prism::IndexAndWriteNode<'pr>) {
        self.index_shorthand(
            node.receiver(),
            node.arguments(),
            node.block(),
            &node.value(),
            true,
        );
    }

    // --- Conditionals ---------------------------------------------------------

    fn visit_if_node(&mut self, node: &ruby_prism::IfNode<'pr>) {
        self.visit(&node.predicate());
        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
        let subsequent = node.subsequent();
        if let Some(sub) = &subsequent {
            self.visit(sub);
        }
        self.c += 1;
        if self.literal_else(subsequent.as_ref()) {
            self.c += 1;
        }
    }

    /// Parser represents `unless c; THEN; else; ELSE; end` as
    /// `(if c ELSE THEN)`: the else clause is visited *before* the body.
    fn visit_unless_node(&mut self, node: &ruby_prism::UnlessNode<'pr>) {
        self.visit(&node.predicate());
        let else_clause = node.else_clause();
        if let Some(else_node) = &else_clause {
            self.visit_else_node(else_node);
        }
        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
        self.c += 1;
        if else_clause.is_some() {
            self.c += 1;
        }
    }

    fn visit_and_node(&mut self, node: &ruby_prism::AndNode<'pr>) {
        ruby_prism::visit_and_node(self, node);
        self.c += 1;
    }

    fn visit_or_node(&mut self, node: &ruby_prism::OrNode<'pr>) {
        ruby_prism::visit_or_node(self, node);
        self.c += 1;
    }

    fn visit_while_node(&mut self, node: &ruby_prism::WhileNode<'pr>) {
        ruby_prism::visit_while_node(self, node);
        // `begin..end while` is `while_post` in the parser gem, which is not
        // in `COUNTED_NODES`.
        if !node.is_begin_modifier() {
            self.c += 1;
        }
    }

    fn visit_until_node(&mut self, node: &ruby_prism::UntilNode<'pr>) {
        ruby_prism::visit_until_node(self, node);
        if !node.is_begin_modifier() {
            self.c += 1;
        }
    }

    fn visit_for_node(&mut self, node: &ruby_prism::ForNode<'pr>) {
        ruby_prism::visit_for_node(self, node);
        // `for` is itself an assignment (`for_type?`) and a condition; its
        // index targets count via the target arms.
        self.a += 1;
        self.c += 1;
    }

    fn visit_when_node(&mut self, node: &ruby_prism::WhenNode<'pr>) {
        ruby_prism::visit_when_node(self, node);
        // `case` itself is not counted (not in `COUNTED_NODES`) and neither is
        // a case `else`; each `when` adds one.
        self.c += 1;
    }

    fn visit_begin_node(&mut self, node: &ruby_prism::BeginNode<'pr>) {
        ruby_prism::visit_begin_node(self, node);
        // One `rescue` node per begin-with-rescue, regardless of clause count;
        // `else`/`ensure` add nothing.
        if node.rescue_clause().is_some() {
            self.c += 1;
        }
    }

    fn visit_rescue_modifier_node(&mut self, node: &ruby_prism::RescueModifierNode<'pr>) {
        ruby_prism::visit_rescue_modifier_node(self, node);
        self.c += 1;
    }

    // --- Pattern matching -------------------------------------------------------

    fn visit_in_node(&mut self, node: &ruby_prism::InNode<'pr>) {
        let pattern = node.pattern();
        // A guard is an If/Unless wrapping the pattern in prism; the parser
        // gem's `if_guard`/`unless_guard` is not a counted condition. The
        // pattern is its `statements`, the guard expression its `predicate`,
        // visited in that (parser) order.
        if let Some(guard) = pattern.as_if_node() {
            if let Some(statements) = guard.statements() {
                for stmt in &statements.body() {
                    self.visit_pattern(&stmt);
                }
            }
            self.visit(&guard.predicate());
        } else if let Some(guard) = pattern.as_unless_node() {
            if let Some(statements) = guard.statements() {
                for stmt in &statements.body() {
                    self.visit_pattern(&stmt);
                }
            }
            self.visit(&guard.predicate());
        } else {
            self.visit_pattern(&pattern);
        }
        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
        self.c += 1;
    }

    fn visit_match_predicate_node(&mut self, node: &ruby_prism::MatchPredicateNode<'pr>) {
        self.visit(&node.value());
        self.visit_pattern(&node.pattern());
    }

    fn visit_match_required_node(&mut self, node: &ruby_prism::MatchRequiredNode<'pr>) {
        self.visit(&node.value());
        self.visit_pattern(&node.pattern());
    }

    /// `^(expr)` re-enters code mode inside a pattern.
    fn visit_pinned_expression_node(&mut self, node: &ruby_prism::PinnedExpressionNode<'pr>) {
        let saved = self.in_pattern;
        self.in_pattern = false;
        self.visit(&node.expression());
        self.in_pattern = saved;
    }

    // --- Arguments (block/def parameters; `argument?` counting) ----------------

    fn visit_required_parameter_node(&mut self, node: &ruby_prism::RequiredParameterNode<'pr>) {
        if capturing(node.name().as_slice()) {
            self.a += 1;
        }
    }

    fn visit_optional_parameter_node(&mut self, node: &ruby_prism::OptionalParameterNode<'pr>) {
        if capturing(node.name().as_slice()) {
            self.a += 1;
        }
        self.visit(&node.value());
    }

    fn visit_rest_parameter_node(&mut self, node: &ruby_prism::RestParameterNode<'pr>) {
        if let Some(name) = node.name()
            && capturing(name.as_slice())
        {
            self.a += 1;
        }
    }

    fn visit_required_keyword_parameter_node(
        &mut self,
        node: &ruby_prism::RequiredKeywordParameterNode<'pr>,
    ) {
        if capturing(node.name().as_slice()) {
            self.a += 1;
        }
    }

    fn visit_optional_keyword_parameter_node(
        &mut self,
        node: &ruby_prism::OptionalKeywordParameterNode<'pr>,
    ) {
        if capturing(node.name().as_slice()) {
            self.a += 1;
        }
        self.visit(&node.value());
    }

    fn visit_keyword_rest_parameter_node(
        &mut self,
        node: &ruby_prism::KeywordRestParameterNode<'pr>,
    ) {
        if let Some(name) = node.name()
            && capturing(name.as_slice())
        {
            self.a += 1;
        }
    }

    fn visit_block_parameter_node(&mut self, node: &ruby_prism::BlockParameterNode<'pr>) {
        if let Some(name) = node.name()
            && capturing(name.as_slice())
        {
            self.a += 1;
        }
    }

    /// Block-local variables (`|a; b|`) are `shadowarg`s — counted arguments.
    fn visit_block_local_variable_node(
        &mut self,
        node: &ruby_prism::BlockLocalVariableNode<'pr>,
    ) {
        if capturing(node.name().as_slice()) {
            self.a += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(a, b, c)` for the single method in `source`, with the discount on or
    /// off. Ground-truth vectors come from the stock probe
    /// (`.tmp/2026-06-13/abc-size/probe*.rb`).
    fn abc(source: &str, discount: bool) -> (u64, u64, u64) {
        let methods = check_abc_size(source.as_bytes(), -1, discount);
        assert_eq!(
            methods.len(),
            1,
            "expected exactly one method in {source:?}, got {}",
            methods.len()
        );
        let m = &methods[0];
        (m.assignments, m.branches, m.conditions)
    }

    /// Discount-off (default `CountRepeatedAttributes: true`) cases.
    #[test]
    fn discount_off_vectors() {
        let cases: &[(&str, (u64, u64, u64))] = &[
            ("def m\n  [1].each { |x| x }\nend", (1, 1, 1)),
            ("def m\n  [1].each { _1 }\nend", (0, 1, 0)),
            // `it` is an it-block parameter under prism's grammar (Ruby >= 3.4),
            // so `it` is inert and only the iterating `each` counts. Under an
            // older parser-gem target `it` would be a method send (B+1, plus the
            // iterating block C+1) — that target-version skew is the only place
            // prism and parser-gem disagree, exercised by the corpus parity run.
            ("def m\n  [1].each { it }\nend", (0, 1, 0)),
            ("def m\n  begin\n    foo\n  end while bar\nend", (0, 2, 0)),
            ("def m\n  begin\n    foo\n  end until bar\nend", (0, 2, 0)),
            ("def m\n  foo while bar\nend", (0, 2, 1)),
            ("def m\n  case x\n  when 1 then a\n  else\n    b\n  end\nend", (0, 3, 1)),
            ("def m\n  case x\n  when 1 then a\n  end\nend", (0, 2, 1)),
            (
                "def m\n  case x\n  in 1 then a\n  in 2 then b\n  else\n    c\n  end\nend",
                (0, 4, 2),
            ),
            ("def m\n  x => y\nend", (0, 1, 0)),
            ("def m\n  x in y\nend", (0, 1, 0)),
            ("def m\n  /(?<g>a)/ =~ str\nend", (0, 1, 0)),
            // A static regexp literal on the lhs of `=~` is `match_with_lvasgn`
            // even without a named capture (`/x/o` included): the `=~` is not a
            // branch, so only the rhs `str` send counts.
            ("def m\n  /x(.)/ =~ str\nend", (0, 1, 0)),
            ("def m\n  /x/o =~ str\nend", (0, 1, 0)),
            // A non-regexp lhs, an interpolated regexp, and `!~` stay real sends.
            ("def m\n  str =~ /x/\nend", (0, 2, 0)),
            ("def m\n  /x#{y}/ =~ str\nend", (0, 3, 0)),
            ("def m\n  /x/ !~ str\nend", (0, 2, 0)),
            ("def m\n  re =~ str\nend", (0, 3, 0)),
            ("def m\n  super\nend", (0, 0, 0)),
            ("def m\n  super(1)\nend", (0, 0, 0)),
            ("def m\n  super { foo }\nend", (0, 1, 0)),
            ("def m\n  ->(x) { x }\nend", (1, 1, 0)),
            ("def m\n  lambda { foo }\nend", (0, 2, 0)),
            ("def m\n  a <=> b\nend", (0, 3, 0)),
            ("def m\n  a&.==(b)\nend", (0, 2, 1)),
            ("def m\n  foo\nrescue Err => e\n  bar\nend", (1, 2, 1)),
            (
                "def m\n  begin\n    a\n  rescue X\n    b\n  rescue Y\n    c\n  else\n    d\n  ensure\n    e\n  end\nend",
                (0, 5, 1),
            ),
            ("def m\n  begin\n    a\n  end\nend", (0, 1, 0)),
            ("def m\n  defined?(foo)\nend", (0, 1, 0)),
            ("def m\n  foo { |a; b| bar }\nend", (2, 2, 0)),
            ("def m\n  @x += 1\nend", (1, 0, 0)),
            ("def m\n  x = []\n  x[0] += 1\nend", (2, 1, 0)),
            ("def m\n  x = []\n  x[0] ||= 1\nend", (2, 1, 1)),
            ("def m\n  foo&.bar ||= 1\nend", (1, 2, 2)),
            ("def m\n  self.a, foo.b, bar[42] = nil\nend", (3, 5, 0)),
            ("def m\n  (a, b), c = d\nend", (3, 1, 0)),
            ("def m\n  *, a = d\nend", (1, 1, 0)),
            ("def m\n  for _i in 0..5\n    x\n  end\nend", (1, 1, 1)),
            ("def m\n  for a, b in pairs\n    x\n  end\nend", (3, 2, 1)),
            ("def m\n  foo.bar=(1)\nend", (1, 2, 0)),
            ("def m\n  self.foo &&= 1\nend", (1, 1, 1)),
            ("def m\n  x.map(&:to_s)\nend", (0, 2, 1)),
            ("def m\n  super(&blk)\nend", (0, 1, 0)),
            ("def m\n  foo(&blk)\nend", (0, 2, 0)),
            ("def m\n  <<~X\n    a#{foo}b\n  X\nend", (0, 1, 0)),
            ("def m\n  $1\nend", (0, 0, 0)),
            ("def m\n  a ? b : c\nend", (0, 3, 1)),
            ("def m\n  unless a\n    b\n  else\n    c\n  end\nend", (0, 3, 2)),
            // `compound_assignment`: an op-/or-/and-asgn whose value is a bare
            // non-setter send counts an extra A. The value send's own branch
            // counts too, and a send-with-block value does *not* add the A.
            ("def m\n  d ||= foo\nend", (2, 1, 1)),
            ("def m\n  d ||= 1\nend", (1, 0, 1)),
            ("def m\n  d ||= foo { 1 }\nend", (1, 1, 1)),
            // Block-PASS value stays a parser `send`, so compound counts it
            // (and the iterating `map` block-pass adds a condition).
            ("def m\n  d -= exclude.map(&:to_sym)\nend", (2, 2, 1)),
            ("def m\n  d += foo\nend", (2, 1, 0)),
            ("def m\n  @d ||= foo\nend", (2, 1, 1)),
            ("def m\n  x = []\n  x[0] += foo\nend", (3, 2, 0)),
            ("def m\n  x.y ||= foo\nend", (2, 3, 1)),
            ("def m\n  d ||= a.b.c\nend", (2, 3, 1)),
        ];
        for (src, expected) in cases {
            assert_eq!(abc(src, false), *expected, "discount-off case {src:?}");
        }
    }

    /// Discount-on (`CountRepeatedAttributes: false`) cases.
    #[test]
    fn discount_on_vectors() {
        let cases: &[(&str, (u64, u64, u64))] = &[
            ("def m\n  x = 1\n  x += 1\nend", (2, 0, 0)),
            ("def m\n  x = 1\n  x ||= 1\nend", (2, 0, 1)),
            ("def m(var)\n  var.foo { 1 }\n  var.foo { 2 }\nend", (0, 1, 0)),
            ("def m(foo)\n  foo.b\n  self.x, foo.b = 1, 2\n  foo.b\nend", (2, 3, 0)),
            ("def m\n  self.X\n  self::X ||= 1\n  self.X\nend", (1, 2, 1)),
            ("def m(var)\n  var&.foo\n  var&.foo\nend", (0, 1, 1)),
            ("def m\n  x = []\n  x[0] += 1\nend", (2, 1, 0)),
            ("def m\n  Foo.bar\n  Foo.bar\nend", (0, 1, 0)),
            ("def m\n  $g.bar\n  $g.bar\n  $g = 1\n  $g.bar\nend", (1, 2, 0)),
        ];
        for (src, expected) in cases {
            assert_eq!(abc(src, true), *expected, "discount-on case {src:?}");
        }
    }
}
