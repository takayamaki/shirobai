//! The four Application* cops (rubocop-rails 2.35.5): `Rails/ApplicationRecord`,
//! `Rails/ApplicationController`, `Rails/ApplicationMailer`,
//! `Rails/ApplicationJob`. Each enforces a specific superclass and shares one
//! implementation (stock's `RuboCop::Cop::EnforceSuperclass` mixin), so one
//! rule feeds all four cops off the single shared walk — the always-active
//! rails origin must stay cheap (table-driven const-name checks, no extra
//! AST pass).
//!
//! Mirrors `vendor/rubocop-rails/lib/rubocop/cop/rails/application_*.rb` and
//! `vendor/rubocop/lib/rubocop/cop/mixin/enforce_superclass.rb`. Every quirk
//! below was probed against stock (rubocop-rails 2.35.5 + rubocop 1.88.0 +
//! railties 7.0.8, `.tmp/2026-07-06` probe):
//!
//! - **`on_class`** (`(class (const _ !:SUPERCLASS) BASE_PATTERN ...)`): a
//!   `class X < ActiveRecord::Base` fires unless the terminal name of `X` is
//!   the cop's `SUPERCLASS` (scope-insensitive: `Foo::ApplicationRecord` is
//!   exempt too). `BASE_PATTERN` is `(const (const {nil? cbase} :RECV) :Base)`
//!   — the superclass must be exactly `RECV::Base` / `::RECV::Base`. Offense
//!   range = the whole superclass const node (leading `::` included);
//!   autocorrect replaces it with the bare `SUPERCLASS` name.
//! - **`on_send`** (`(send (const {nil? cbase} :Class) :new BASE_PATTERN)`):
//!   `Class.new(RECV::Base)` fires. Receiver is `Class` (nil / cbase scope),
//!   selector `new`, EXACTLY one positional argument that is `BASE_PATTERN`
//!   (two args or a block-pass — both parser args — disqualify; a literal
//!   block does not). Offense range = the argument node; autocorrect replaces
//!   it with `SUPERCLASS`.
//! - **send exemption** (`!^(casgn {nil? cbase} :SUPERCLASS ...)` and
//!   `!^^(casgn {nil? cbase} :SUPERCLASS (block ...))`): the call is exempt
//!   when it is the direct value of a `casgn` (nil / cbase scope) whose name
//!   is `SUPERCLASS`. In prism a call-with-block is one `CallNode`, so both
//!   stock branches collapse to "value of a matching `ConstantWrite`" — this
//!   covers `ApplicationRecord = Class.new(ActiveRecord::Base)` and its
//!   `do..end` / `{}` block forms. A namespaced casgn
//!   (`Foo::ApplicationRecord = ...`) is NOT exempt; a cross-cop assignment
//!   (`ApplicationController = Class.new(ActiveRecord::Base)`) still fires
//!   `Rails/ApplicationRecord`.
//!
//! Anonymous forms (`Class.new(ActiveRecord::Base) {}`,
//! `wrap(Class.new(ActiveRecord::Base))`) fire. The message and replacement
//! text are cop constants; the wrapper owns them, so each slot here is just
//! the `(start, end)` offense-highlight-and-replace range.

use std::collections::HashSet;

use ruby_prism::{CallNode, Node, Visit};

/// The four cops in rails origin slot order (0..4). Each is a
/// `(receiver_module, superclass)` pair; the base terminal is always `Base`.
struct CopSpec {
    /// The `RECV` in `RECV::Base` (`ActiveRecord`, `ActionController`, ...).
    receiver: &'static [u8],
    /// The enforced superclass name (`ApplicationRecord`, ...), also the
    /// class-name exemption and the autocorrect replacement.
    superclass: &'static [u8],
}

const COPS: [CopSpec; 4] = [
    CopSpec { receiver: b"ActiveRecord", superclass: b"ApplicationRecord" },
    CopSpec { receiver: b"ActionController", superclass: b"ApplicationController" },
    CopSpec { receiver: b"ActionMailer", superclass: b"ApplicationMailer" },
    CopSpec { receiver: b"ActiveJob", superclass: b"ApplicationJob" },
];

/// Per-file offense ranges for the four Application* cops, indexed like
/// [`COPS`] (rails origin slots 0..4). Each range is both the offense
/// highlight and the autocorrect replace target.
///
/// The last two vecs are NOT final offenses but Architecture-B *candidate*
/// ranges (rails origin slots 6..8): each is the parser send-node range of a
/// send the Rust prefilter thinks might be an offense. The Ruby wrapper
/// relocates the real parser node at that range and runs stock's detection +
/// autocorrect verbatim, so these are a cheap superset — false positives are
/// filtered on the Ruby side (`NodeLocator` + the stock matchers).
#[derive(Debug, Default, Clone)]
pub struct RailsAppResult {
    pub application_record: Vec<(usize, usize)>,
    pub application_controller: Vec<(usize, usize)>,
    pub application_mailer: Vec<(usize, usize)>,
    pub application_job: Vec<(usize, usize)>,
    /// `Rails/HttpPositionalArguments` candidate send ranges (parser send
    /// range, document order). The wrapper re-runs stock's `http_request?` /
    /// `needs_conversion?` / routing-block / rack-test guards.
    pub http_positional_arguments: Vec<(usize, usize)>,
    /// `Rails/DeprecatedActiveModelErrorsMethods` candidate send ranges
    /// (parser send range, document order). The wrapper re-runs stock's five
    /// errors-chain matchers, the model-file receiver rule and the version
    /// gate.
    pub deprecated_active_model_errors_methods: Vec<(usize, usize)>,
    /// `Rails/IndexBy` / `Rails/IndexWith` candidate node ranges (parser
    /// block/send node range, document order). ONE list feeds both cops: the
    /// four shapes (`each_with_object`, `to_h { }`, `map { }.to_h`,
    /// `Hash[map { }]`) are identical for IndexBy and IndexWith — only which of
    /// key/value is the identity element differs, and that is decided by each
    /// cop's own stock matcher on the relocated parser node. Emitted in
    /// pre-order so the wrapper's per-cop `ignore_node` (nested transforms)
    /// sees an outer node before any node it contains. See
    /// `collect_index_method`.
    pub index_method: Vec<(usize, usize)>,
}

impl RailsAppResult {
    fn push(&mut self, cop: usize, range: (usize, usize)) {
        match cop {
            0 => self.application_record.push(range),
            1 => self.application_controller.push(range),
            2 => self.application_mailer.push(range),
            _ => self.application_job.push(range),
        }
    }
}

/// Standalone entry point used by the per-cop fallback (non-bundle-eligible
/// files). Runs the shared rule over one parse and returns all four slots.
pub fn check_rails_app(source: &[u8]) -> RailsAppResult {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.finish()
}

/// Standalone entry point for the `Rails/HttpPositionalArguments` candidate
/// prefilter (non-bundle-eligible files).
pub fn check_rails_http_positional_arguments(source: &[u8]) -> Vec<(usize, usize)> {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.finish().http_positional_arguments
}

/// Standalone entry point for the
/// `Rails/DeprecatedActiveModelErrorsMethods` candidate prefilter.
pub fn check_rails_deprecated_active_model_errors_methods(source: &[u8]) -> Vec<(usize, usize)> {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.finish().deprecated_active_model_errors_methods
}

/// Standalone entry point for the `Rails/IndexBy` / `Rails/IndexWith` candidate
/// prefilter (non-bundle-eligible files). Both cops read the same candidate
/// list; the wrapper picks its own offenses with the stock matcher.
pub fn check_rails_index_method(source: &[u8]) -> Vec<(usize, usize)> {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.finish().index_method
}

/// Build the rule for use standalone or in the shared-walk bundle.
pub(crate) fn build_rule() -> RailsAppVisitor {
    RailsAppVisitor {
        result: RailsAppResult::default(),
        exempt_calls: HashSet::new(),
    }
}

pub(crate) struct RailsAppVisitor {
    result: RailsAppResult,
    /// Start offsets of `Class.new(BASE)` calls that are the direct value of a
    /// matching `SUPERCLASS =` constant write (pre-marked when the write node
    /// is entered, before the child call is visited).
    exempt_calls: HashSet<usize>,
}

impl RailsAppVisitor {
    pub(crate) fn finish(self) -> RailsAppResult {
        self.result
    }

    /// `on_class`: flag `class X < RECV::Base` unless `X`'s terminal name is
    /// the matched cop's `SUPERCLASS`.
    fn check_class(&mut self, node: &ruby_prism::ClassNode<'_>) {
        let Some(superclass) = node.superclass() else {
            return;
        };
        let Some(cop) = matching_base_cop(&superclass) else {
            return;
        };
        // Class-name exemption (`!:SUPERCLASS`): terminal const name of the
        // class being defined.
        if terminal_const_name(&node.constant_path()) == Some(COPS[cop].superclass) {
            return;
        }
        let loc = superclass.location();
        self.result
            .push(cop, (loc.start_offset(), loc.end_offset()));
    }

    /// Architecture-B candidate prefilter for the two send-shaped Rails cops.
    /// Rides the same `CallNode` entry as the Application `Class.new` check —
    /// cheap, table-driven, no extra walk. Emits parser send-node ranges; the
    /// Ruby wrapper relocates and runs stock detection + autocorrect verbatim.
    fn collect_arch_b(&mut self, node: &CallNode<'_>) {
        if http_positional_candidate(node) {
            self.result.http_positional_arguments.push(send_range(node));
        }
        if deprecated_errors_candidate(node) {
            self.result
                .deprecated_active_model_errors_methods
                .push(send_range(node));
        }
        self.collect_index_method(node);
    }

    /// `Rails/IndexBy` / `Rails/IndexWith` candidate prefilter (shared list).
    ///
    /// Emits the parser node range for each of stock's four shapes so the
    /// wrapper can relocate the real parser node and run stock's matcher +
    /// autocorrect verbatim:
    ///
    /// - `each_with_object` / `to_h` WITH a literal block -> the parser
    ///   `block`/`numblock`/`itblock` node (receiver start .. block close).
    ///   Covers csend heads too (`x&.to_h { }`); stock's `on_bad_*` matchers
    ///   accept a csend via `call`.
    /// - `map { } .to_h` (send or csend outer) -> the outer `to_h` send node.
    /// - `Hash[map { }]` (send only) -> the `[]` send node.
    ///
    /// The block candidate is pushed before the map-to-h candidate for the same
    /// `to_h` node, mirroring stock's `on_block`-before-`on_send` visit order
    /// (the block node contains the send node), so per-cop `ignore_node` on a
    /// nested transform sees the container first.
    fn collect_index_method(&mut self, node: &CallNode<'_>) {
        if let Some(range) = index_block_candidate(node) {
            self.result.index_method.push(range);
        }
        if index_map_to_h_candidate(node) {
            self.result.index_method.push(send_range(node));
        }
        if index_hash_brackets_candidate(node) {
            self.result.index_method.push(send_range(node));
        }
    }

    /// `on_send`: flag `Class.new(RECV::Base)` unless the call is an exempt
    /// `SUPERCLASS =` definition.
    fn check_call(&mut self, node: &CallNode<'_>) {
        self.collect_arch_b(node);
        let Some((cop, arg)) = class_new_base_arg(node) else {
            return;
        };
        if self.exempt_calls.contains(&node.location().start_offset()) {
            return;
        }
        // Re-confirm the cop from the argument (class_new_base_arg already
        // resolved it) and push the argument range.
        let loc = arg.location();
        self.result
            .push(cop, (loc.start_offset(), loc.end_offset()));
    }

    /// Pre-mark the exemption when entering a constant write: if the write's
    /// terminal name (nil / cbase scope) is a cop's `SUPERCLASS` and its
    /// direct value is that cop's `Class.new(BASE)`, exempt the call.
    fn note_const_write(&mut self, value: Option<Node<'_>>, name: Option<&[u8]>) {
        let Some(name) = name else { return };
        // Only nil/cbase-scope writes to a SUPERCLASS name matter.
        if !COPS.iter().any(|c| c.superclass == name) {
            return;
        }
        let Some(value) = value else { return };
        let Some(call) = value.as_call_node() else {
            return;
        };
        let Some((cop, _)) = class_new_base_arg(&call) else {
            return;
        };
        // The exemption only applies when the casgn name matches the SAME
        // cop's superclass (a cross-cop assignment stays a live offense).
        if COPS[cop].superclass == name {
            self.exempt_calls.insert(call.location().start_offset());
        }
    }

    fn enter_node(&mut self, node: &Node<'_>) {
        if let Some(class) = node.as_class_node() {
            self.check_class(&class);
        } else if let Some(call) = node.as_call_node() {
            self.check_call(&call);
        } else if let Some(w) = node.as_constant_write_node() {
            self.note_const_write(Some(w.value()), Some(w.name().as_slice()));
        } else if let Some(w) = node.as_constant_path_write_node() {
            // Only cbase (`::App = ...`) counts as `{nil? cbase}`; a
            // namespaced target (`Foo::App = ...`) is not exempt.
            let target = w.target();
            let name = if target.parent().is_none() {
                target.name().map(|n| n.as_slice())
            } else {
                None
            };
            self.note_const_write(Some(w.value()), name);
        }
    }
}

/// Terminal (short) constant name of a name node: `Foo` -> `Foo`,
/// `A::B::Foo` -> `Foo`. Only `ConstantReadNode` / `ConstantPathNode` carry a
/// name; anything else yields `None`.
fn terminal_const_name<'a>(node: &Node<'a>) -> Option<&'a [u8]> {
    if let Some(read) = node.as_constant_read_node() {
        return Some(read.name().as_slice());
    }
    if let Some(path) = node.as_constant_path_node() {
        return path.name().map(|n| n.as_slice());
    }
    None
}

/// `(const {nil? cbase} :NAME)`: a bare `NAME` (`ConstantReadNode`, always
/// nil-scope) or `::NAME` (a parent-less `ConstantPathNode`, cbase).
fn top_level_const_name<'a>(node: &Node<'a>) -> Option<&'a [u8]> {
    if let Some(read) = node.as_constant_read_node() {
        return Some(read.name().as_slice());
    }
    if let Some(path) = node.as_constant_path_node()
        && path.parent().is_none()
    {
        return path.name().map(|n| n.as_slice());
    }
    None
}

/// If `node` matches `(const (const {nil? cbase} :RECV) :Base)` for some cop,
/// return that cop index. `RECV::Base` / `::RECV::Base`, nothing else.
fn matching_base_cop(node: &Node<'_>) -> Option<usize> {
    let path = node.as_constant_path_node()?;
    if path.name().map(|n| n.as_slice()) != Some(b"Base") {
        return None;
    }
    let parent = path.parent()?;
    let recv = top_level_const_name(&parent)?;
    COPS.iter().position(|c| c.receiver == recv)
}

/// If `node` is `Class.new(RECV::Base)` for some cop, return
/// `(cop_index, base_arg_node)`. Receiver `Class` (nil / cbase scope),
/// selector `new`, exactly one positional argument that is `BASE_PATTERN`,
/// and no block-pass argument (a literal block is allowed).
fn class_new_base_arg<'a>(node: &CallNode<'a>) -> Option<(usize, Node<'a>)> {
    // `(send ...)` head only — never `&.` (`on_csend` is not aliased for
    // this pattern).
    if node
        .call_operator_loc()
        .is_some_and(|l| l.as_slice() == b"&.")
    {
        return None;
    }
    if node.name().as_slice() != b"new" {
        return None;
    }
    let receiver = node.receiver()?;
    if top_level_const_name(&receiver) != Some(b"Class") {
        return None;
    }
    // A block-pass is a parser argument: `Class.new(Base, &blk)` breaks the
    // exact-one-arg pattern. A literal block does not.
    if matches!(node.block(), Some(b) if b.as_block_argument_node().is_some()) {
        return None;
    }
    let args = node.arguments()?;
    let mut it = args.arguments().iter();
    let arg = it.next()?;
    if it.next().is_some() {
        return None; // more than one argument
    }
    let cop = matching_base_cop(&arg)?;
    Some((cop, arg))
}

// --- Architecture-B candidate prefilters ---------------------------------

/// The six HTTP verbs `Rails/HttpPositionalArguments` restricts to
/// (`RESTRICT_ON_SEND`).
const HTTP_VERBS: [&[u8]; 6] = [b"get", b"post", b"put", b"patch", b"delete", b"head"];

/// True for a `&.` (csend) call — none of the stock `(send ...)` patterns
/// match csend, so csend calls are never candidates.
fn is_csend(node: &CallNode<'_>) -> bool {
    node.call_operator_loc()
        .is_some_and(|l| l.as_slice() == b"&.")
}

/// The receiver of `node` as a plain (non-csend) `CallNode`, if any.
fn plain_receiver_call<'a>(node: &CallNode<'a>) -> Option<CallNode<'a>> {
    let recv = node.receiver()?;
    let call = recv.as_call_node()?;
    if is_csend(&call) {
        return None;
    }
    Some(call)
}

/// `(send _ :errors)` — any receiver (the wrapper applies the nil/send/ivar/
/// lvar + model-file receiver rule); csend excluded.
fn is_errors_call(call: &CallNode<'_>) -> bool {
    !is_csend(call) && call.name().as_slice() == b"errors"
}

/// `(send (send _ :errors) {:messages :details})`.
fn is_messages_details_on_errors(call: &CallNode<'_>) -> bool {
    if is_csend(call) {
        return false;
    }
    let name = call.name().as_slice();
    if name != b"messages" && name != b"details" {
        return false;
    }
    plain_receiver_call(call).is_some_and(|r| is_errors_call(&r))
}

/// `MANIPULATIVE_METHODS` (rubocop-rails 2.35.5), verbatim.
fn is_manipulative(m: &[u8]) -> bool {
    matches!(
        m,
        b"<<" | b"append"
            | b"clear"
            | b"collect!"
            | b"compact!"
            | b"concat"
            | b"delete"
            | b"delete_at"
            | b"delete_if"
            | b"drop"
            | b"drop_while"
            | b"fill"
            | b"filter!"
            | b"keep_if"
            | b"flatten!"
            | b"insert"
            | b"map!"
            | b"pop"
            | b"prepend"
            | b"push"
            | b"reject!"
            | b"replace"
            | b"reverse!"
            | b"rotate!"
            | b"select!"
            | b"shift"
            | b"shuffle!"
            | b"slice!"
            | b"sort!"
            | b"sort_by!"
            | b"uniq!"
            | b"unshift"
    )
}

/// `Rails/HttpPositionalArguments` candidate: a bare (nil-receiver) send of an
/// HTTP verb with at least two arguments (`http_request?` needs a non-nil
/// action plus a captured data arg). Superset of `http_request?` — the
/// wrapper re-checks the `!nil?` action, `needs_conversion?`, routing-block
/// and rack-test guards.
fn http_positional_candidate(node: &CallNode<'_>) -> bool {
    if is_csend(node) || node.receiver().is_some() {
        return false;
    }
    let name = node.name().as_slice();
    if !HTTP_VERBS.contains(&name) {
        return false;
    }
    // `http_request?` needs an action arg plus a captured data arg. In the
    // parser AST a block-pass (`&blk`) is an argument, so stock fires on
    // `get :x, &blk`; prism keeps it in `block()`, so count it separately.
    let argc = node
        .arguments()
        .map(|a| a.arguments().iter().count())
        .unwrap_or(0);
    let block_pass = matches!(node.block(), Some(b) if b.as_block_argument_node().is_some());
    argc + usize::from(block_pass) >= 2
}

/// `Rails/DeprecatedActiveModelErrorsMethods` candidate: the outer send of one
/// of stock's five errors-chain shapes. Structural superset (the errors-chain
/// receiver types and version gate are re-checked by the wrapper).
fn deprecated_errors_candidate(node: &CallNode<'_>) -> bool {
    if is_csend(node) {
        return false;
    }
    let m = node.name().as_slice();
    let Some(recv) = plain_receiver_call(node) else {
        return false;
    };
    // `errors[...] = v` / `errors.messages[...] = v` (`:[]=`).
    if m == b"[]=" {
        return is_errors_call(&recv) || is_messages_details_on_errors(&recv);
    }
    // `errors.{keys,values,to_h,to_xml}`.
    if matches!(m, b"keys" | b"values" | b"to_h" | b"to_xml") {
        return is_errors_call(&recv);
    }
    // Manipulative call on `errors[...]` / `errors.{messages,details}[...]`.
    if is_manipulative(m) && recv.name().as_slice() == b"[]" {
        return plain_receiver_call(&recv)
            .is_some_and(|r| is_errors_call(&r) || is_messages_details_on_errors(&r));
    }
    false
}

// --- Rails/IndexBy + Rails/IndexWith candidate prefilters ----------------

/// The literal block (`{ }` / `do..end`) of `node`, if any. A block-pass
/// (`&blk`) is NOT a literal block (it lives in `block()` as a
/// `BlockArgumentNode`), so it is excluded.
fn literal_block<'a>(node: &CallNode<'a>) -> Option<ruby_prism::BlockNode<'a>> {
    node.block()?.as_block_node()
}

/// A `map` / `collect` call carrying a literal block (any head — send or
/// csend), i.e. the receiver shape of `map { }.to_h` / `Hash[map { }]`. Stock's
/// `on_bad_map_to_h` / `on_bad_hash_brackets_map` accept `block` / `numblock`
/// / `itblock` here, all of which are a call-with-literal-block in prism.
fn is_map_collect_block(node: &CallNode<'_>) -> bool {
    let name = node.name().as_slice();
    (name == b"map" || name == b"collect") && literal_block(node).is_some()
}

/// Block-shaped candidate: `each_with_object` / `to_h` carrying a literal
/// block. Returns the parser block-node range: receiver start
/// (`node.location().start`) to block close (`block.location().end`). This is
/// the expression range of the `block` / `numblock` / `itblock` parser node the
/// wrapper relocates. The empty-hash arg, block arity and body shape are all
/// re-checked by the stock matcher, so this is a cheap superset.
fn index_block_candidate(node: &CallNode<'_>) -> Option<(usize, usize)> {
    let block = literal_block(node)?;
    let name = node.name().as_slice();
    if name == b"each_with_object" || name == b"to_h" {
        Some((node.location().start_offset(), block.location().end_offset()))
    } else {
        None
    }
}

/// `map { }.to_h` candidate: the outer `to_h` (send OR csend) whose receiver is
/// a `map` / `collect` call with a literal block. The candidate node is the
/// outer send; `send_range` excludes any literal block on the `to_h` itself
/// (the `map { }.to_h { |k, v| ... }` form still fires here — stock's `on_send`
/// matches the `to_h` send regardless of its own block).
fn index_map_to_h_candidate(node: &CallNode<'_>) -> bool {
    if node.name().as_slice() != b"to_h" {
        return false;
    }
    let Some(recv) = node.receiver() else {
        return false;
    };
    recv.as_call_node()
        .is_some_and(|r| is_map_collect_block(&r))
}

/// `Hash[map { }]` candidate: a send `[]` on a top-level `Hash` (nil / cbase
/// scope, never csend, never `Foo::Hash`), with exactly one argument that is a
/// `map` / `collect` call with a literal block. The candidate node is the `[]`
/// send (range ends at `]`).
fn index_hash_brackets_candidate(node: &CallNode<'_>) -> bool {
    if is_csend(node) || node.name().as_slice() != b"[]" {
        return false;
    }
    let Some(recv) = node.receiver() else {
        return false;
    };
    if top_level_const_name(&recv) != Some(b"Hash") {
        return false;
    }
    let Some(args) = node.arguments() else {
        return false;
    };
    let mut it = args.arguments().iter();
    let Some(first) = it.next() else {
        return false;
    };
    if it.next().is_some() {
        return false; // more than one argument
    }
    first
        .as_call_node()
        .is_some_and(|c| is_map_collect_block(&c))
}

/// Parser send-node range for `node` (the range `NodeLocator` matches on the
/// parser AST). Equals prism `location()` for paren-less no-block calls, but
/// index brackets (`[]` / `[]=`) put `closing_loc` at `]` while the value
/// follows, and a literal `do`/`{}` block is a separate parser node outside
/// the send — so the end is the max of message / closing-paren / last
/// argument / block-pass, never a literal block. Verified against parser-gem
/// on heredoc, multiline, index-assign and block forms.
fn send_range(node: &CallNode<'_>) -> (usize, usize) {
    let start = node.location().start_offset();
    let mut end = node
        .message_loc()
        .map(|m| m.end_offset())
        .unwrap_or_else(|| node.location().end_offset());
    if let Some(cl) = node.closing_loc() {
        end = end.max(cl.end_offset());
    }
    if let Some(args) = node.arguments()
        && let Some(last) = args.arguments().iter().last()
    {
        end = end.max(last.location().end_offset());
    }
    if let Some(block) = node.block()
        && let Some(bp) = block.as_block_argument_node()
    {
        end = end.max(bp.location().end_offset());
    }
    (start, end)
}

impl<'pr> Visit<'pr> for RailsAppVisitor {
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        self.check_class(node);
        ruby_prism::visit_class_node(self, node);
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_constant_write_node(&mut self, node: &ruby_prism::ConstantWriteNode<'pr>) {
        self.note_const_write(Some(node.value()), Some(node.name().as_slice()));
        ruby_prism::visit_constant_write_node(self, node);
    }

    fn visit_constant_path_write_node(&mut self, node: &ruby_prism::ConstantPathWriteNode<'pr>) {
        let target = node.target();
        let name = if target.parent().is_none() {
            target.name().map(|n| n.as_slice())
        } else {
            None
        };
        self.note_const_write(Some(node.value()), name);
        ruby_prism::visit_constant_path_write_node(self, node);
    }
}

impl super::dispatch::Rule for RailsAppVisitor {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        // ClassNode (on_class), CallNode (on_send), and the constant-write
        // classes (exemption pre-marking). No stack, so no LEAVE.
        Interest(Interest::ENTER_CLASS_MOD | Interest::ENTER_CALL | Interest::ENTER_WRITE)
    }

    fn enter(&mut self, node: &Node<'_>) {
        self.enter_node(node);
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(src: &str) -> Vec<(usize, usize)> {
        check_rails_app(src.as_bytes()).application_record
    }

    #[test]
    fn flags_class_active_record_base() {
        // `class Foo < ActiveRecord::Base` — offense on `ActiveRecord::Base`.
        let off = rec("class Foo < ActiveRecord::Base\nend\n");
        assert_eq!(off, vec![(12, 30)]);
    }

    #[test]
    fn flags_cbase_active_record_base() {
        // `::ActiveRecord::Base` — range includes the leading `::`.
        let off = rec("class Bar < ::ActiveRecord::Base\nend\n");
        assert_eq!(off, vec![(12, 32)]);
    }

    #[test]
    fn exempts_application_record_itself() {
        assert!(rec("class ApplicationRecord < ActiveRecord::Base\nend\n").is_empty());
    }

    #[test]
    fn exempts_namespaced_application_record_name() {
        assert!(rec("class Foo::ApplicationRecord < ActiveRecord::Base\nend\n").is_empty());
    }

    #[test]
    fn flags_namespaced_model() {
        let off = rec("class Nested::MyModel < ActiveRecord::Base\nend\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_class_new() {
        // `Baz = Class.new(ActiveRecord::Base)` — offense on the arg.
        let off = rec("Baz = Class.new(ActiveRecord::Base)\n");
        assert_eq!(off, vec![(16, 34)]);
    }

    #[test]
    fn exempts_application_record_class_new() {
        assert!(rec("ApplicationRecord = Class.new(ActiveRecord::Base)\n").is_empty());
    }

    #[test]
    fn exempts_application_record_class_new_block() {
        assert!(rec("ApplicationRecord = Class.new(ActiveRecord::Base) do\nend\n").is_empty());
        assert!(rec("ApplicationRecord = Class.new(ActiveRecord::Base) { def x; end }\n").is_empty());
    }

    #[test]
    fn exempts_cbase_application_record_class_new() {
        assert!(rec("::ApplicationRecord = Class.new(ActiveRecord::Base)\n").is_empty());
    }

    #[test]
    fn flags_namespaced_casgn() {
        // `Foo::ApplicationRecord = ...` is NOT exempt (scope not nil/cbase).
        let off = rec("Foo::ApplicationRecord = Class.new(ActiveRecord::Base)\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_named_class_new_block() {
        // `Foo = Class.new(...) do..end` — name Foo != ApplicationRecord.
        let off = rec("Foo = Class.new(ActiveRecord::Base) do\n  def x; end\nend\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_anonymous_class_new() {
        assert_eq!(rec("Class.new(ActiveRecord::Base) do\nend\n").len(), 1);
        assert_eq!(rec("wrap(Class.new(ActiveRecord::Base))\n").len(), 1);
    }

    #[test]
    fn cross_cop_assignment_still_fires_record() {
        // casgn name ApplicationController != ApplicationRecord -> fires.
        let off = rec("ApplicationController = Class.new(ActiveRecord::Base)\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn accepts_two_arg_class_new() {
        assert!(rec("A = Class.new(ActiveRecord::Base, foo)\n").is_empty());
    }

    #[test]
    fn accepts_block_pass_class_new() {
        assert!(rec("C = Class.new(ActiveRecord::Base, &blk)\n").is_empty());
    }

    #[test]
    fn flags_cbase_class_receiver() {
        assert_eq!(rec("B = ::Class.new(ActiveRecord::Base)\n").len(), 1);
    }

    #[test]
    fn accepts_wrong_receiver() {
        assert!(rec("D = Klass.new(ActiveRecord::Base)\n").is_empty());
        assert!(rec("class Other < SomeOther::Base\nend\n").is_empty());
        assert!(rec("class Mig < ActiveRecord::Migration[7.0]\nend\n").is_empty());
    }

    #[test]
    fn other_cops_dispatch_to_their_slots() {
        let r = check_rails_app(
            "class C < ActionController::Base\nend\n\
             class M < ActionMailer::Base\nend\n\
             class J < ActiveJob::Base\nend\n\
             J2 = Class.new(ActiveJob::Base)\n"
                .as_bytes(),
        );
        assert!(r.application_record.is_empty());
        assert_eq!(r.application_controller.len(), 1);
        assert_eq!(r.application_mailer.len(), 1);
        assert_eq!(r.application_job.len(), 2);
    }

    #[test]
    fn bundle_rule_matches_standalone() {
        let src = "class Foo < ActiveRecord::Base\nend\n\
                   ApplicationRecord = Class.new(ActiveRecord::Base)\n\
                   Baz = Class.new(ActiveRecord::Base)\n";
        let alone = check_rails_app(src.as_bytes());
        let mut rule = build_rule();
        super::super::dispatch::run(src.as_bytes(), &mut [&mut rule]);
        let bundled = rule.finish();
        assert_eq!(bundled.application_record, alone.application_record);
        assert_eq!(bundled.application_record.len(), 2);
    }

    // --- Architecture-B candidate prefilters ---

    fn http(src: &str) -> Vec<(usize, usize)> {
        check_rails_http_positional_arguments(src.as_bytes())
    }

    fn dep(src: &str) -> Vec<(usize, usize)> {
        check_rails_deprecated_active_model_errors_methods(src.as_bytes())
    }

    #[test]
    fn http_candidate_basic_verbs() {
        // Each verb with >= 2 args -> one candidate covering the whole send.
        assert_eq!(http("get :new, user_id: @user.id\n"), vec![(0, 27)]);
        assert_eq!(http("post(:user_attrs, id: 1)\n"), vec![(0, 24)]);
        assert_eq!(http("head :create, user_id: 1\n").len(), 1);
    }

    #[test]
    fn http_candidate_arity_and_receiver() {
        // One arg -> not a candidate (`http_request?` needs action + data).
        assert!(http("get :new\n").is_empty());
        // Explicit receiver -> not a bare send.
        assert!(http("@user.get.id = ''\n").is_empty());
        assert!(http("obj.get :new, id: 1\n").is_empty());
        // Non-verb selector.
        assert!(http("puts :create, user_id: 1\n").is_empty());
        assert!(http("process :new, params: { id: 1 }\n").is_empty());
    }

    #[test]
    fn http_candidate_superset_forms() {
        // The wrapper filters these; the prefilter still emits them (superset).
        assert_eq!(http("get :nothing, **args\n").len(), 1); // kwsplat
        assert_eq!(http("get(:list, ...)\n").len(), 1); // forwarded args
        assert_eq!(http("get :new, params: { id: 1 }\n").len(), 1); // already kw
    }

    #[test]
    fn http_candidate_block_range_excludes_literal_block() {
        // `get :x, foo do end` -> send range is `get :x, foo` (block excluded).
        assert_eq!(http("get :x, foo do end\n"), vec![(0, 11)]);
        // A block-pass counts as the data arg (stock fires) and is part of the
        // send range.
        assert_eq!(http("get :x, &blk\n"), vec![(0, 12)]);
        // Only a block-pass and no data arg -> not enough args.
        assert!(http("get &blk\n").is_empty());
    }

    #[test]
    fn dep_root_forms() {
        assert_eq!(dep("user.errors[:name] << 'msg'\n"), vec![(0, 27)]);
        assert_eq!(dep("user.errors[:name] = []\n"), vec![(0, 23)]);
        assert_eq!(dep("user.errors[:name].clear\n"), vec![(0, 24)]);
    }

    #[test]
    fn dep_errors_deprecated_forms() {
        // The `.keys` node is the offense, not the outer `.include?`.
        assert_eq!(dep("user.errors.keys.include?(:name)\n"), vec![(0, 16)]);
        assert_eq!(dep("user.errors.values\n"), vec![(0, 18)]);
        assert_eq!(dep("user.errors.to_h\n"), vec![(0, 16)]);
        assert_eq!(dep("user.errors.to_xml\n"), vec![(0, 18)]);
    }

    #[test]
    fn dep_messages_details_forms() {
        assert_eq!(dep("user.errors.messages[:name] << 'msg'\n"), vec![(0, 36)]);
        assert_eq!(dep("user.errors.messages[:name] = []\n"), vec![(0, 32)]);
        assert_eq!(dep("user.errors.details[:name] << {}\n"), vec![(0, 32)]);
        assert_eq!(dep("errors.details[:name].clear\n"), vec![(0, 27)]);
    }

    #[test]
    fn dep_nil_receiver_and_model_forms() {
        // Bare `errors` (only valid in model files; the wrapper decides).
        assert_eq!(dep("errors[:name] << 'msg'\n"), vec![(0, 22)]);
        assert_eq!(dep("errors.keys\n"), vec![(0, 11)]);
    }

    #[test]
    fn dep_non_candidates() {
        // Non-manipulative / wrong shape -> no candidate.
        assert!(dep("errors[:name].present?\n").is_empty());
        assert!(dep("errors.messages[:name].keys\n").is_empty());
        assert!(dep("errors.messages[:name].present?\n").is_empty());
        assert!(dep("user.foo[:name] << 'msg'\n").is_empty());
        assert!(dep("user.errors.attribute_names\n").is_empty());
        // csend is never a candidate.
        assert!(dep("user&.errors&.keys\n").is_empty());
    }

    #[test]
    fn arch_b_candidates_ride_the_shared_walk() {
        let src = "get :new, id: 1\nuser.errors[:name] << 'x'\n";
        let mut rule = build_rule();
        super::super::dispatch::run(src.as_bytes(), &mut [&mut rule]);
        let bundled = rule.finish();
        assert_eq!(
            bundled.http_positional_arguments,
            check_rails_http_positional_arguments(src.as_bytes())
        );
        assert_eq!(
            bundled.deprecated_active_model_errors_methods,
            check_rails_deprecated_active_model_errors_methods(src.as_bytes())
        );
        assert_eq!(bundled.http_positional_arguments.len(), 1);
        assert_eq!(bundled.deprecated_active_model_errors_methods.len(), 1);
    }

    // --- Rails/IndexBy + Rails/IndexWith candidate prefilter ---

    fn idx(src: &str) -> Vec<(usize, usize)> {
        check_rails_index_method(src.as_bytes())
    }

    #[test]
    fn index_each_with_object_block_candidate() {
        // Block node range: receiver start through the closing brace.
        assert_eq!(idx("x.each_with_object({}) { |el, h| h[foo(el)] = el }\n"), vec![(0, 50)]);
        // do..end block: through `end`.
        let src = "x.each_with_object({}) do |el, memo|\n  memo[el] = el\nend\n";
        let off = idx(src);
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].0, 0);
        assert_eq!(&src.as_bytes()[off[0].1 - 3..off[0].1], b"end");
        // Safe-navigation head is still a candidate (stock fires on it).
        assert_eq!(idx("x&.each_with_object({}) { |el, h| h[foo(el)] = el }\n").len(), 1);
    }

    #[test]
    fn index_to_h_block_candidate() {
        // `to_h { ... }` -> block node range (whole call + block).
        assert_eq!(idx("x.to_h { |el| [el.to_sym, el] }\n"), vec![(0, 31)]);
        // numbered / it params still carry a literal block.
        assert_eq!(idx("x.to_h { [_1.to_sym, _1] }\n").len(), 1);
    }

    #[test]
    fn index_map_to_h_candidate_shapes() {
        // `map { }.to_h` -> outer send node (ends at `to_h`, excludes any
        // trailing block on to_h).
        assert_eq!(idx("x.map { |el| [el.to_sym, el] }.to_h\n"), vec![(0, 35)]);
        assert_eq!(idx("x.collect { |el| [el.to_sym, el] }.to_h\n").len(), 1);
        // `map { }.to_h { |k, v| ... }` -> BOTH the to_h block candidate and
        // the map-to-h send candidate (the send is where stock fires).
        let off = idx("x.map { |el| [el.to_sym, el] }.to_h { |k, v| [v, k] }\n");
        assert_eq!(off.len(), 2);
        // Block candidate (container) first, send candidate second.
        assert!(off[0].1 > off[1].1);
        // csend outer to_h is a candidate.
        assert_eq!(idx("x.map { |el| [el, el] }&.to_h\n").len(), 1);
    }

    #[test]
    fn index_hash_brackets_candidate_shapes() {
        assert_eq!(idx("Hash[x.map { |el| [el.to_sym, el] }]\n"), vec![(0, 36)]);
        assert_eq!(idx("::Hash[x.map { |el| [el.to_sym, el] }]\n").len(), 1);
        // Namespaced Hash is not a candidate.
        assert!(idx("Foo::Hash[x.map { |el| [el.to_sym, el] }]\n").is_empty());
        // Hash[collect { }] too.
        assert_eq!(idx("Hash[x.collect { [_1.to_sym, _1] }]\n").len(), 1);
    }

    #[test]
    fn index_non_candidates() {
        // to_h / map without a literal block.
        assert!(idx("x.to_h\n").is_empty());
        assert!(idx("x.map { |el| el }.to_h\n").len() == 1); // map.to_h IS a candidate shape (wrapper filters body)
        assert!(idx("x.map(&:to_s).to_h\n").is_empty()); // map has no literal block
        // each_with_object without a block.
        assert!(idx("x.each_with_object({})\n").is_empty());
        // Hash[...] whose arg is not a map/collect block.
        assert!(idx("Hash[pairs]\n").is_empty());
        assert!(idx("Hash[x.map { |el| [el, el] }, y]\n").is_empty()); // two args
    }

    #[test]
    fn index_candidates_ride_the_shared_walk() {
        let src = "x.each_with_object({}) { |el, h| h[foo(el)] = el }\n\
                   Hash[y.map { |el| [el.to_sym, el] }]\n";
        let mut rule = build_rule();
        super::super::dispatch::run(src.as_bytes(), &mut [&mut rule]);
        let bundled = rule.finish();
        assert_eq!(bundled.index_method, check_rails_index_method(src.as_bytes()));
        assert_eq!(bundled.index_method.len(), 2);
    }
}
