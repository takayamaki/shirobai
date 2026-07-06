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
#[derive(Debug, Default, Clone)]
pub struct RailsAppResult {
    pub application_record: Vec<(usize, usize)>,
    pub application_controller: Vec<(usize, usize)>,
    pub application_mailer: Vec<(usize, usize)>,
    pub application_job: Vec<(usize, usize)>,
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

    /// `on_send`: flag `Class.new(RECV::Base)` unless the call is an exempt
    /// `SUPERCLASS =` definition.
    fn check_call(&mut self, node: &CallNode<'_>) {
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
}
