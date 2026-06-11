//! `Metrics/BlockNesting`.
//!
//! Counts how deeply conditional and looping constructs nest. RuboCop walks the
//! AST tracking a running nesting level, increments it on each "counted" node
//! and reports the first node that exceeds `Max` (descendants of a reported node
//! are suppressed). The `CountBlocks` / `CountModifierForms` options decide
//! whether blocks and modifier `if`/`unless`/`while`/`until` forms add a level.
//!
//! Prism has no parent pointers and `Node` is not `Clone`, so instead of the
//! recursive descent RuboCop uses we walk with the `Visit` trait and maintain
//! the running level (and the "inside an already-reported subtree" flag) on a
//! stack, pushing on branch enter and unwinding on branch leave.

use ruby_prism::{Node, Visit};

/// One reportable offense: the byte range of the offending node. The highlight
/// is truncated to the first line by RuboCop's offense formatter, so only the
/// start offset (caret position) and first-line span matter.
pub struct BlockNestingOffense {
    pub start_offset: usize,
    pub end_offset: usize,
}

/// Returns the reportable offenses plus the deepest nesting level observed at
/// any node that exceeded `max` (`0` when nothing exceeded). The deepest level
/// feeds `ExcludeLimit`'s `Max` bookkeeping on the Ruby side, mirroring
/// RuboCop's per-node `self.max = current_level`.
pub fn check_block_nesting(
    source: &[u8],
    max: usize,
    count_blocks: bool,
    count_modifier_forms: bool,
) -> (Vec<BlockNestingOffense>, usize) {
    let mut checker = build_rule(source, max, count_blocks, count_modifier_forms);
    super::parse_cache::with_parsed(source, |_source, node| checker.visit(node));
    (checker.out, checker.deepest)
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule(
    source: &[u8],
    max: usize,
    count_blocks: bool,
    count_modifier_forms: bool,
) -> Checker<'_> {
    Checker {
        source,
        max,
        count_blocks,
        count_modifier_forms,
        out: Vec::new(),
        deepest: 0,
        current_level: 0,
        ignore_depth: 0,
        stack: Vec::new(),
    }
}

/// What a considered node contributes, mirroring RuboCop's `count_if_block?`.
enum Counted {
    /// Not a nesting construct at all (`consider_node?` is false).
    No,
    /// A nesting construct that always adds a level.
    Yes,
    /// An `if`/`unless`/`while`/`until` in modifier form: adds a level only when
    /// `CountModifierForms` is enabled.
    Modifier,
    /// An `elsif` clause: a nesting construct that never adds a level.
    Elsif,
}

/// What each entered branch node pushed, so `leave` can unwind it.
struct Frame {
    /// `1` if this node incremented `current_level`, else `0`.
    delta: usize,
    /// `true` if this node was the one that activated ignore-suppression.
    opened_ignore: bool,
}

pub(crate) struct Checker<'a> {
    source: &'a [u8],
    max: usize,
    count_blocks: bool,
    count_modifier_forms: bool,
    pub(crate) out: Vec<BlockNestingOffense>,
    pub(crate) deepest: usize,
    current_level: usize,
    /// `0` when not suppressing; otherwise the stack length at which the
    /// suppressing ancestor was entered (its subtree is ignored).
    ignore_depth: usize,
    stack: Vec<Frame>,
}

impl Checker<'_> {
    /// Classifies a node the way RuboCop's `consider_node?` + `count_if_block?`
    /// do, returning whether and how it contributes to the nesting level.
    fn classify(&self, node: &Node<'_>) -> Counted {
        match node {
            Node::CaseNode { .. }
            | Node::CaseMatchNode { .. }
            | Node::ForNode { .. }
            | Node::RescueNode { .. }
            | Node::WhileNode { .. }
            | Node::UntilNode { .. } => Counted::Yes,
            Node::IfNode { .. } => self.classify_if(&node.as_if_node().unwrap()),
            Node::UnlessNode { .. } => {
                let u = node.as_unless_node().unwrap();
                if u.end_keyword_loc().is_none() {
                    Counted::Modifier
                } else {
                    Counted::Yes
                }
            }
            // Blocks (`each do ... end`, `each { ... }`, numbered/`it` params)
            // are CallNodes carrying a `BlockNode`. Only counted when CountBlocks.
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if self.count_blocks && call.block().is_some() {
                    Counted::Yes
                } else {
                    Counted::No
                }
            }
            _ => Counted::No,
        }
    }

    fn classify_if(&self, if_node: &ruby_prism::IfNode<'_>) -> Counted {
        match if_node.if_keyword_loc() {
            // Ternary (`a ? b : c`): no `if` keyword. RuboCop counts it.
            None => Counted::Yes,
            Some(loc) => {
                if &self.source[loc.start_offset()..loc.end_offset()] == b"elsif" {
                    Counted::Elsif
                } else if if_node.end_keyword_loc().is_none() {
                    // `x if c` modifier form.
                    Counted::Modifier
                } else {
                    Counted::Yes
                }
            }
        }
    }

    fn enter(&mut self, node: &Node<'_>) {
        let counted = self.classify(node);
        let mut delta = 0;
        let mut opened_ignore = false;
        if !matches!(counted, Counted::No) {
            let increments = match counted {
                Counted::Yes => true,
                Counted::Modifier => self.count_modifier_forms,
                Counted::Elsif | Counted::No => false,
            };
            if increments {
                self.current_level += 1;
                delta = 1;
            }
            if self.current_level > self.max {
                self.deepest = self.deepest.max(self.current_level);
                if self.ignore_depth == 0 {
                    let loc = node.location();
                    self.out.push(BlockNestingOffense {
                        start_offset: loc.start_offset(),
                        end_offset: loc.end_offset(),
                    });
                    // Suppress reporting for this node's descendants.
                    self.ignore_depth = self.stack.len() + 1;
                    opened_ignore = true;
                }
            }
        }
        self.stack.push(Frame {
            delta,
            opened_ignore,
        });
    }

    fn leave(&mut self) {
        if let Some(frame) = self.stack.pop() {
            self.current_level -= frame.delta;
            if frame.opened_ignore {
                self.ignore_depth = 0;
            }
        }
    }
}

/// Shared-walk driver. The rescue hooks mirror the standalone
/// `visit_rescue_node` override below: each `rescue` clause is a counted level
/// around its own children, while the chained `subsequent` clause is a sibling
/// outside the frame (the walker visits it after `leave_rescue`).
impl super::dispatch::Rule for Checker<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        Checker::enter(self, node);
    }

    fn leave(&mut self) {
        Checker::leave(self);
    }

    fn enter_rescue(&mut self, node: &Node<'_>) {
        Checker::enter(self, node);
    }

    fn leave_rescue(&mut self) {
        Checker::leave(self);
    }
}

impl<'pr> Visit<'pr> for Checker<'_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        self.enter(&node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.leave();
    }

    // `RescueNode` is reached through `BeginNode`'s concretely-typed
    // `rescue_clause` field, which the generated dispatcher visits directly —
    // bypassing `visit_branch_node_enter`. Override it so rescue clauses still
    // get the enter/leave bookkeeping (each `rescue` is a counted level, like
    // parser's `:resbody`).
    //
    // Chained clauses (`rescue A; rescue B`) are *siblings* at the same level in
    // parser, not nested, so the `subsequent` clause is visited at the parent
    // level rather than inside this clause's incremented frame.
    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        self.enter(&node.as_node());
        for exception in &node.exceptions() {
            self.visit(&exception);
        }
        if let Some(reference) = node.reference() {
            self.visit(&reference);
        }
        if let Some(statements) = node.statements() {
            self.visit_statements_node(&statements);
        }
        self.leave();
        if let Some(subsequent) = node.subsequent() {
            self.visit_rescue_node(&subsequent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns the offense start offsets plus the deepest exceeding level.
    fn run(source: &str, max: usize, blocks: bool, mods: bool) -> (Vec<usize>, usize) {
        let (offenses, deepest) = check_block_nesting(source.as_bytes(), max, blocks, mods);
        (
            offenses.into_iter().map(|o| o.start_offset).collect(),
            deepest,
        )
    }

    // Up to `Max` levels of nesting are accepted.
    #[test]
    fn accepts_max_levels() {
        let src = "if a\n  if b\n    puts b\n  end\nend\n";
        assert_eq!(run(src, 2, false, false), (vec![], 0));
    }

    // `Max + 1` levels of `if` nesting: one offense at the third `if`, deepest 3.
    #[test]
    fn if_over_by_one() {
        let src = "if a\n  if b\n    if c\n      puts c\n    end\n  end\nend\n";
        let third_if = src.find("if c").unwrap();
        assert_eq!(run(src, 2, false, false), (vec![third_if], 3));
    }

    // `Max + 2`: still one offense (descendants suppressed), deepest tracks 4.
    #[test]
    fn if_over_by_two_suppresses_descendant() {
        let src = "if a\n  if b\n    if c\n      if d\n        x\n      end\n    end\n  end\nend\n";
        let third_if = src.find("if c").unwrap();
        assert_eq!(run(src, 2, false, false), (vec![third_if], 4));
    }

    // Two separate nested `if`s at the same level each report.
    #[test]
    fn sibling_nests_report_separately() {
        let src = "if a\n  if b\n    if c\n      x\n    end\n  end\n  if d\n    if e\n      y\n    end\n  end\nend\n";
        let (starts, deepest) = run(src, 2, false, false);
        assert_eq!(
            starts,
            vec![src.find("if c").unwrap(), src.find("if e").unwrap()]
        );
        assert_eq!(deepest, 3);
    }

    // `elsif` clauses never add a level.
    #[test]
    fn elsif_does_not_nest() {
        let src = "if a\nelsif b\nelsif c\nelsif d\nend\n";
        assert_eq!(run(src, 2, false, false), (vec![], 0));
    }

    // `case`, `while`, `until`, `for`, `rescue` all add a level.
    #[test]
    fn loop_and_case_constructs_nest() {
        let case_src = "if a\n  if b\n    case c\n    when C\n      puts C\n    end\n  end\nend\n";
        assert_eq!(
            run(case_src, 2, false, false).0,
            vec![case_src.find("case c").unwrap()]
        );

        let while_src = "if a\n  if b\n    while c\n      x\n    end\n  end\nend\n";
        assert_eq!(
            run(while_src, 2, false, false).0,
            vec![while_src.find("while c").unwrap()]
        );

        let for_src = "if a\n  if b\n    for c in [1,2] do\n      x\n    end\n  end\nend\n";
        assert_eq!(
            run(for_src, 2, false, false).0,
            vec![for_src.find("for c").unwrap()]
        );
    }

    // `case` pattern matching (`case/in`) nests.
    #[test]
    fn case_match_nests() {
        let src = "if a\n  if b\n    case c\n    in C\n      puts C\n    end\n  end\nend\n";
        assert_eq!(
            run(src, 2, false, false).0,
            vec![src.find("case c").unwrap()]
        );
    }

    // A modifier `while` reports at the `begin` keyword.
    #[test]
    fn modifier_while_reports_at_begin() {
        let src = "if a\n  if b\n    begin\n      x\n    end while c\n  end\nend\n";
        assert_eq!(
            run(src, 2, false, false).0,
            vec![src.find("begin").unwrap()]
        );
    }

    // A `rescue` clause reports at the `rescue` keyword.
    #[test]
    fn rescue_reports_at_keyword() {
        let src = "if a\n  if b\n    begin\n      x\n    rescue\n      y\n    end\n  end\nend\n";
        assert_eq!(
            run(src, 2, false, false).0,
            vec![src.find("rescue").unwrap()]
        );
    }

    // Chained `rescue` clauses are siblings at the same level, not nested.
    #[test]
    fn chained_rescues_are_siblings() {
        let src = "if a\n  if b\n    begin\n      x\n    rescue E1\n      y\n    rescue E2\n      z\n    end\n  end\nend\n";
        let (starts, deepest) = run(src, 2, false, false);
        assert_eq!(
            starts,
            vec![
                src.find("rescue E1").unwrap(),
                src.find("rescue E2").unwrap()
            ]
        );
        assert_eq!(deepest, 3);
    }

    // Blocks are ignored unless `CountBlocks` is set.
    #[test]
    fn blocks_only_count_when_enabled() {
        let src = "if a\n  if b\n    [1, 2].each do |c|\n      puts c\n    end\n  end\nend\n";
        assert_eq!(run(src, 2, false, false), (vec![], 0));
        assert_eq!(
            run(src, 2, true, false).0,
            vec![src.find("[1, 2]").unwrap()]
        );
    }

    // Modifier forms only count when `CountModifierForms` is set.
    #[test]
    fn modifier_forms_only_count_when_enabled() {
        let src = "if a\n  if b\n    puts 'hello' if c\n  end\nend\n";
        assert_eq!(run(src, 2, false, false), (vec![], 0));
        assert_eq!(
            run(src, 2, false, true).0,
            vec![src.find("puts 'hello' if c").unwrap()]
        );
    }
}
