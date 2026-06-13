//! `Layout/IndentationConsistency`.
//!
//! Checks that entities on the same logical depth share the same indentation.
//! The stock cop fires `on_begin` / `on_kwbegin` and runs `check_alignment`
//! (from the `Alignment` mixin) over the begin's children: every child that
//! `begins_its_line?` is realigned to a base column. The base column is the
//! display column of the first child, except in `normal` style when the first
//! child is a bare access modifier (`base_column_for_normal_style`). In
//! `indented_internal_methods` style the children are split into sections
//! delimited by `private` / `protected` and each section is checked
//! independently.
//!
//! Here it is reconstructed over Prism. A parser-gem `(begin ...)` is a Prism
//! `StatementsNode` with at least two children (an implicit statement group, a
//! `(...)` group, or the program root); a parser-gem `(kwbegin ...)` without
//! handlers is a `BeginNode` whose body `StatementsNode` carries the
//! statements, and with handlers the protected / handler bodies are themselves
//! `(begin ...)` groups. Processing every `StatementsNode` with >= 2 children
//! reproduces both `on_begin` and `on_kwbegin` exactly. Ruby applies the
//! realignment via `AlignmentCorrector` (same division of labour as the other
//! indentation cops), so the corrector receives each offending child node and
//! the `column_delta`.

use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// One inconsistent-indentation offense. `[start_offset, end_offset)` is the
/// offending child's full source range (the offense location and the
/// `AlignmentCorrector` target node). `column_delta` is `base - display_column`.
/// `autocorrect` is false when the child's range is `within?` an
/// already-registered offense location in this same investigation
/// (`@current_offenses`), in which case it is reported but not corrected.
pub struct ConsistencyOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub autocorrect: bool,
}

#[derive(Clone, Copy)]
pub struct Config {
    /// `Layout/IndentationConsistency` EnforcedStyle == 'indented_internal_methods'.
    pub indented_internal_methods: bool,
}

pub fn check_indentation_consistency(source: &[u8], config: Config) -> Vec<ConsistencyOffense> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        config,
        offenses: Vec::new(),
        // Walk-state: the kind and start offset of every ancestor node, used to
        // resolve `in_macro_scope?` and the parent column for
        // `base_column_for_normal_style`.
        ancestors: Vec::new(),
        condition_ranges: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    config: Config,
    pub(crate) offenses: Vec<ConsistencyOffense>,
    /// `(kind, start_offset)` for each open ancestor node (pushed on `enter`,
    /// popped on `leave`). The top is the parent of the node currently entering.
    ancestors: Vec<(NodeKind, usize)>,
    /// The predicate range of each open `if` / `unless` ancestor (or `None`),
    /// so a `begin` in the condition can be classified as `IfCondition`.
    condition_ranges: Vec<Option<(usize, usize)>>,
}

/// The ancestor node kinds that `in_macro_scope?` distinguishes.
#[derive(Clone, Copy, PartialEq)]
enum NodeKind {
    /// The synthetic program root (parser-gem's root `(begin)` has no parent).
    Program,
    /// `class` / `module` / `class << self` — establishes a macro scope.
    ClassLike,
    /// A `Class.new` / `Module.new` / `Struct.new` / `Data.define` block
    /// (`class_constructor?`) — establishes a macro scope.
    ClassConstructor,
    /// A wrapper node (`begin` / `kwbegin` / any block / an `if` body): it is in
    /// macro scope iff it is itself in a macro scope.
    Wrapper,
    /// A node sitting in the condition position of an `if` — explicitly excluded
    /// from the wrapper set by the matcher, so it breaks the macro-scope chain.
    IfCondition,
    /// Anything else (`def`, a call's arguments, an array, ...): breaks the chain.
    Other,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

impl<'a> Visitor<'a> {
    fn line_start(&self, off: usize) -> usize {
        self.line_index.line_start(off)
    }

    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    /// `display_column(range)` = unicode display width of `line[0, range.column]`.
    fn display_column(&self, off: usize) -> isize {
        self.line_index.display_column(self.source, off) as isize
    }

    /// `begins_its_line?(range)`: only whitespace precedes `off` on its line.
    fn begins_its_line(&self, off: usize) -> bool {
        let ls = self.line_start(off);
        self.source[ls..off]
            .iter()
            .all(|&b| b == b' ' || b == b'\t')
    }

    /// `in_macro_scope?` for a begin whose parent kind is at `ancestors.last()`:
    /// the begin's parent must be class-like / class-constructor / program, or a
    /// wrapper that is itself in macro scope. The ancestor stack at the point a
    /// `StatementsNode` is processed ends with the begin's parent node kind.
    fn begin_in_macro_scope(&self) -> bool {
        // Walk the ancestor stack from the begin's parent upward. A run of
        // wrapper frames is transparent; the chain terminates at the first
        // non-wrapper, which decides the result.
        for &(kind, _) in self.ancestors.iter().rev() {
            match kind {
                NodeKind::Program | NodeKind::ClassLike | NodeKind::ClassConstructor => return true,
                NodeKind::Wrapper => continue,
                NodeKind::IfCondition | NodeKind::Other => return false,
            }
        }
        false
    }

    /// Whether the begin's parent is the program root (parser's root `(begin)`
    /// has no parent, so `base_column_for_normal_style` returns the access
    /// modifier indent unconditionally).
    fn begin_parent_is_root(&self) -> bool {
        matches!(self.ancestors.last(), Some((NodeKind::Program, _)))
    }

    /// The start offset of the begin's parent node (top of the ancestor stack),
    /// for the `access_modifier_indent > module_indent` comparison.
    fn parent_start(&self) -> Option<usize> {
        self.ancestors.last().map(|&(_, start)| start)
    }

    fn check_statements(&mut self, children: &[Node<'_>]) {
        if children.len() < 2 {
            return;
        }
        if self.config.indented_internal_methods {
            self.check_indented_internal(children);
        } else {
            self.check_normal(children);
        }
    }

    fn is_bare_access_modifier(&self, node: &Node<'_>) -> bool {
        let Some(c) = node.as_call_node() else {
            return false;
        };
        if c.receiver().is_some() || c.arguments().is_some() || c.block().is_some() {
            return false;
        }
        if !matches!(
            c.name().as_slice(),
            b"public" | b"protected" | b"private" | b"module_function"
        ) {
            return false;
        }
        self.begin_in_macro_scope()
    }

    fn is_special_modifier(&self, node: &Node<'_>) -> bool {
        let Some(c) = node.as_call_node() else {
            return false;
        };
        if c.receiver().is_some() || c.arguments().is_some() || c.block().is_some() {
            return false;
        }
        if !matches!(c.name().as_slice(), b"private" | b"protected") {
            return false;
        }
        self.begin_in_macro_scope()
    }

    /// `check_normal_style`: reject bare access modifiers, then `check_alignment`
    /// with `base_column_for_normal_style`.
    fn check_normal(&mut self, children: &[Node<'_>]) {
        let base = self.base_column_for_normal_style(children);
        let items: Vec<&Node<'_>> = children
            .iter()
            .filter(|c| !self.is_bare_access_modifier(c))
            .collect();
        self.check_alignment(&items, base);
    }

    /// `base_column_for_normal_style(node)`. Returns `Some(col)` to force the
    /// base, or `None` to let `check_alignment` derive it from the first item.
    fn base_column_for_normal_style(&self, children: &[Node<'_>]) -> Option<isize> {
        let first = children.first()?;
        if !self.is_bare_access_modifier(first) {
            return None;
        }
        let access_modifier_indent = self.display_column(first.location().start_offset());
        // `return access_modifier_indent unless node.parent` (root begin).
        if self.begin_parent_is_root() {
            return Some(access_modifier_indent);
        }
        let module_indent = match self.parent_start() {
            Some(off) => self.display_column(off),
            None => return Some(access_modifier_indent),
        };
        if access_modifier_indent > module_indent {
            Some(access_modifier_indent)
        } else {
            None
        }
    }

    /// `check_indented_internal_methods_style`: split children into sections
    /// delimited by bare `private` / `protected` and check each section.
    fn check_indented_internal(&mut self, children: &[Node<'_>]) {
        let mut sections: Vec<Vec<&Node<'_>>> = vec![Vec::new()];
        for child in children {
            if self.is_special_modifier(child) {
                sections.push(Vec::new());
            } else {
                sections.last_mut().unwrap().push(child);
            }
        }
        for group in &sections {
            self.check_alignment(group, None);
        }
    }

    /// `check_alignment(items, base_column)`. `base_column` defaults to the
    /// display column of the first item.
    fn check_alignment(&mut self, items: &[&Node<'_>], base_column: Option<isize>) {
        let base = match base_column {
            Some(b) => b,
            None => match items.first() {
                Some(first) => self.display_column(first.location().start_offset()),
                None => return,
            },
        };

        let mut prev_line: isize = -1;
        for current in items {
            let expr = loc(&current.location());
            let line = self.line_of(expr.0) as isize;
            if line > prev_line && self.begins_its_line(expr.0) {
                let column_delta = base - self.display_column(expr.0);
                if column_delta != 0 {
                    self.register(expr, column_delta);
                }
            }
            prev_line = line;
        }
    }

    /// `register_offense`: an offense whose range is `within?` a previously
    /// registered offense location is reported but not autocorrected (two
    /// rewrites in the same area by the same cop cannot be handled in one pass).
    fn register(&mut self, expr: (usize, usize), column_delta: isize) {
        let within_prior = self
            .offenses
            .iter()
            .any(|o| expr.0 >= o.start_offset && expr.1 <= o.end_offset);
        self.offenses.push(ConsistencyOffense {
            start_offset: expr.0,
            end_offset: expr.1,
            column_delta,
            autocorrect: !within_prior,
        });
    }
}

impl<'a> Visitor<'a> {
    /// Classify a node for the macro-scope chain. `node` is being entered; the
    /// current ancestor top is its parent. A node that sits in the condition
    /// position of its parent `if` is `IfCondition` (the matcher excludes it
    /// from the wrapper set).
    fn classify(&self, node: &Node<'_>) -> NodeKind {
        if node.as_program_node().is_some() {
            return NodeKind::Program;
        }
        if self.in_condition_position(node) {
            return NodeKind::IfCondition;
        }
        if node.as_class_node().is_some()
            || node.as_module_node().is_some()
            || node.as_singleton_class_node().is_some()
        {
            return NodeKind::ClassLike;
        }
        if is_class_constructor(node) {
            return NodeKind::ClassConstructor;
        }
        if node.as_begin_node().is_some()
            || node.as_parentheses_node().is_some()
            || node.as_block_node().is_some()
            || node.as_if_node().is_some()
            || node.as_unless_node().is_some()
        {
            return NodeKind::Wrapper;
        }
        NodeKind::Other
    }

    /// Whether `node` is the predicate (condition) of the immediately-enclosing
    /// `if` / `unless`. Recorded so a `begin` in the condition breaks the
    /// macro-scope chain (`(if _condition <%0 _>)`).
    fn in_condition_position(&self, node: &Node<'_>) -> bool {
        let Some(&Some((s, e))) = self.condition_ranges.last() else {
            return false;
        };
        node.location().start_offset() == s && node.location().end_offset() == e
    }
}

/// `class_constructor?`: the `Class.new` / `Module.new` / `Struct.new` /
/// `Data.define` send that owns a block. The block's body is a macro scope;
/// since the block is a transparent wrapper in the macro-scope chain, marking
/// the owning call as the constructor (equivalent to marking the block) lets
/// the chain resolve. We require the call to own a block (the matcher's body
/// form), since the bare-send form has no body to contain modifiers.
fn is_class_constructor(node: &Node<'_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    if call.block().is_none() {
        return false;
    }
    let Some(recv) = call.receiver() else {
        return false;
    };
    // The receiver must be a bare top-level constant (`global_const?`): a
    // `ConstantReadNode` (no constant path qualifier).
    let Some(cr) = recv.as_constant_read_node() else {
        return false;
    };
    matches!(
        (cr.name().as_slice(), call.name().as_slice()),
        (b"Class" | b"Module" | b"Struct", b"new") | (b"Data", b"define")
    )
}

impl<'a> Visitor<'a> {
    /// Push `node`'s frame, then check every begin (`StatementsNode` with >= 2
    /// children) the node directly owns as a body. The ruby-prism dispatcher
    /// does not fire the branch hooks for `StatementsNode`, so each begin is
    /// reached from its owning parent here. With the frame already pushed, the
    /// begin's parent context is `ancestors.last()` (= this node).
    fn enter_node(&mut self, node: &Node<'_>) {
        let kind = self.classify(node);
        self.ancestors.push((kind, node.location().start_offset()));
        let cond = if let Some(n) = node.as_if_node() {
            Some(loc(&n.predicate().location()))
        } else {
            node.as_unless_node().map(|n| loc(&n.predicate().location()))
        };
        self.condition_ranges.push(cond);

        for st in self.owned_statement_bodies(node) {
            let children: Vec<Node<'_>> = st.body().iter().collect();
            if children.len() >= 2 {
                self.check_statements(&children);
            }
        }
    }

    /// The `StatementsNode` bodies directly owned by `node` (each a parser-gem
    /// `begin` candidate). A `body()` that is an implicit `BeginNode` (rescue /
    /// ensure with no `begin` keyword) is *not* unwrapped here: that BeginNode is
    /// entered on its own and contributes its protected / handler statements.
    fn owned_statement_bodies<'pr>(
        &self,
        node: &Node<'pr>,
    ) -> Vec<ruby_prism::StatementsNode<'pr>> {
        let mut out = Vec::new();
        let mut push_opt = |s: Option<ruby_prism::StatementsNode<'pr>>| {
            if let Some(s) = s {
                out.push(s);
            }
        };
        // The `else` clause of `unless` / `case` / `case`-`in` / `begin` is a
        // typed field whose `ElseNode` the ruby-prism dispatcher does not enter
        // (unlike an `if`'s `else`, which is reached as `subsequent`). Pull
        // those else bodies in from the owning node directly.
        if let Some(n) = node.as_program_node() {
            push_opt(Some(n.statements()));
        } else if let Some(n) = node.as_begin_node() {
            push_opt(n.statements());
            push_opt(n.else_clause().and_then(|e| e.statements()));
        } else if let Some(n) = node.as_if_node() {
            push_opt(n.statements());
        } else if let Some(n) = node.as_unless_node() {
            push_opt(n.statements());
            push_opt(n.else_clause().and_then(|e| e.statements()));
        } else if let Some(n) = node.as_case_node() {
            push_opt(n.else_clause().and_then(|e| e.statements()));
        } else if let Some(n) = node.as_case_match_node() {
            push_opt(n.else_clause().and_then(|e| e.statements()));
        } else if let Some(n) = node.as_else_node() {
            push_opt(n.statements());
        } else if let Some(n) = node.as_while_node() {
            push_opt(n.statements());
        } else if let Some(n) = node.as_until_node() {
            push_opt(n.statements());
        } else if let Some(n) = node.as_for_node() {
            push_opt(n.statements());
        } else if let Some(n) = node.as_when_node() {
            push_opt(n.statements());
        } else if let Some(n) = node.as_in_node() {
            push_opt(n.statements());
        } else if let Some(n) = node.as_ensure_node() {
            push_opt(n.statements());
        } else if let Some(n) = node.as_embedded_statements_node() {
            push_opt(n.statements());
        } else if let Some(n) = node.as_def_node() {
            push_opt(n.body().and_then(|b| b.as_statements_node()));
        } else if let Some(n) = node.as_class_node() {
            push_opt(n.body().and_then(|b| b.as_statements_node()));
        } else if let Some(n) = node.as_module_node() {
            push_opt(n.body().and_then(|b| b.as_statements_node()));
        } else if let Some(n) = node.as_singleton_class_node() {
            push_opt(n.body().and_then(|b| b.as_statements_node()));
        } else if let Some(n) = node.as_block_node() {
            push_opt(n.body().and_then(|b| b.as_statements_node()));
        } else if let Some(n) = node.as_lambda_node() {
            push_opt(n.body().and_then(|b| b.as_statements_node()));
        } else if let Some(n) = node.as_parentheses_node() {
            push_opt(n.body().and_then(|b| b.as_statements_node()));
        }
        out
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        self.enter_node(node);
    }

    fn leave(&mut self) {
        self.ancestors.pop();
        self.condition_ranges.pop();
    }

    // A `RescueNode` is reached through `BeginNode`'s typed `rescue_clause`
    // field, bypassing the branch hooks. Its handler body is a parser-gem
    // `(begin)` (a `resbody` whose body is a multi-statement group). Push a
    // wrapper frame so a bare access modifier inside the handler resolves its
    // macro scope, and check the handler statements.
    fn enter_rescue(&mut self, node: &Node<'_>) {
        let kind = self.classify(node);
        self.ancestors.push((kind, node.location().start_offset()));
        self.condition_ranges.push(None);
        if let Some(n) = node.as_rescue_node()
            && let Some(st) = n.statements()
        {
            let children: Vec<Node<'_>> = st.body().iter().collect();
            if children.len() >= 2 {
                self.check_statements(&children);
            }
        }
    }

    fn leave_rescue(&mut self) {
        self.ancestors.pop();
        self.condition_ranges.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<ConsistencyOffense> {
        check_indentation_consistency(
            source.as_bytes(),
            Config {
                indented_internal_methods: false,
            },
        )
    }

    fn run_internal(source: &str) -> Vec<ConsistencyOffense> {
        check_indentation_consistency(
            source.as_bytes(),
            Config {
                indented_internal_methods: true,
            },
        )
    }

    #[test]
    fn if_body_inconsistent() {
        // `func` at col 1, second `func` at col 2: base is the first (col 1),
        // second offends with delta -1.
        let got = run("if cond\n func\n  func\nend\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].column_delta, -1);
        assert!(got[0].autocorrect);
    }

    #[test]
    fn correctly_indented_no_offense() {
        assert!(run("def test\n  a\n  b\nend\n").is_empty());
    }

    #[test]
    fn single_statement_body_not_checked() {
        assert!(run("def test\n  a\nend\n").is_empty());
    }

    #[test]
    fn class_body_inconsistent_defs() {
        // `def func1` at col 4, `def func2` at col 2: base col 4, delta +2.
        let got = run("class Test\n    def func1\n    end\n  def func2\n  end\nend\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].column_delta, 2);
    }

    #[test]
    fn top_level_access_modifier_base() {
        // `public` at col 0 sets base 0 (root begin, no parent); `def foo` at
        // col 2 offends with delta -2.
        let got = run("public\n\n  def foo\n  end\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].column_delta, -2);
    }

    #[test]
    fn outdented_access_modifier_falls_back_to_first_member() {
        // `public`/`protected`/`private` outdented to module level: base derived
        // from the first non-modifier member (col 2 defs). Only `def g` at col 1
        // offends.
        let src = "class Test\npublic\n\n  def e\n  end\n\nprotected\n\n  def f\n  end\n\nprivate\n\n def g\n end\nend\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].column_delta, 1);
    }

    #[test]
    fn indented_internal_methods_sections_independent() {
        // Public section aligned at col 2, private section aligned (indented) at
        // col 4: no offense in indented_internal_methods style.
        let src = "class A\n  def test\n    a\n    b\n  end\n\n  private\n\n    def foo\n    end\nend\n";
        assert!(run_internal(src).is_empty());
    }

    #[test]
    fn nested_offense_within_offense_not_corrected() {
        // The inner describe block's offense range is within the outer block's
        // offense range: reported but not autocorrected.
        let src = "describe A do\n  render_views\n    describe B do\n            it C do\n            end\n        describe D do\n             before do\n            end\n        end\n    end\nend\n";
        let got = run(src);
        assert_eq!(got.len(), 2);
        assert!(got[0].autocorrect);
        assert!(!got[1].autocorrect);
    }

    #[test]
    fn kwbegin_with_rescue_bodies_checked() {
        // The protected body and the rescue body are separate `(begin)` groups;
        // each misaligned second statement offends.
        let got = run("begin\n  1\n   2\nrescue\n  3\n   4\nend\n");
        assert_eq!(got.len(), 2);
    }
}
