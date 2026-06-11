//! `Metrics/CyclomaticComplexity` + `Metrics/PerceivedComplexity`.
//!
//! Both metrics are computed in a single pass over each method body so the two
//! cops share one re-parse per file.

use std::collections::HashSet;

use ruby_prism::{Node, Visit, visit_call_node, visit_def_node};

/// Per-method complexity result. Both scores are reported; each cop selects the
/// one it needs.
pub struct MethodComplexity {
    pub start_offset: usize,
    pub end_offset: usize,
    /// End of the offense head (method name for `def`, block opening for
    /// `define_method`), used by the LSP location mode.
    pub head_end: usize,
    pub method_name: String,
    pub cyclomatic: usize,
    pub perceived: usize,
}

pub fn check_complexity(source: &[u8]) -> Vec<MethodComplexity> {
    check_complexity_exceeding(source, 0, 0)
}

/// Like [`check_complexity`], but only reports methods whose score exceeds a
/// threshold (`cyclomatic > max_cyclomatic || perceived > max_perceived`), so
/// the Ruby side never marshals the (vastly more numerous) compliant methods.
/// Scores start at 1, so a threshold of `0` means "report everything".
pub fn check_complexity_exceeding(
    source: &[u8],
    max_cyclomatic: usize,
    max_perceived: usize,
) -> Vec<MethodComplexity> {
    let mut finder = build_rule(source, max_cyclomatic, max_perceived);
    super::parse_cache::with_parsed(source, |_source, node| finder.visit(node));
    finder.out
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule(
    source: &[u8],
    max_cyclomatic: usize,
    max_perceived: usize,
) -> MethodFinder<'_> {
    MethodFinder {
        source,
        max_cyclomatic,
        max_perceived,
        out: Vec::new(),
    }
}

/// Method names whose blocks are treated as iterating (`map{}`, `each{}`, ...)
/// and therefore add to complexity. From `Utils::IteratingBlock`.
const ITERATING_METHODS: &[&[u8]] = &[
    b"all?",
    b"any?",
    b"chain",
    b"chunk",
    b"chunk_while",
    b"collect",
    b"collect_concat",
    b"count",
    b"cycle",
    b"detect",
    b"drop",
    b"drop_while",
    b"each",
    b"each_cons",
    b"each_entry",
    b"each_slice",
    b"each_with_index",
    b"each_with_object",
    b"entries",
    b"filter",
    b"filter_map",
    b"find",
    b"find_all",
    b"find_index",
    b"flat_map",
    b"grep",
    b"grep_v",
    b"group_by",
    b"inject",
    b"lazy",
    b"map",
    b"max",
    b"max_by",
    b"min",
    b"min_by",
    b"minmax",
    b"minmax_by",
    b"none?",
    b"one?",
    b"partition",
    b"reduce",
    b"reject",
    b"reverse_each",
    b"select",
    b"slice_after",
    b"slice_before",
    b"slice_when",
    b"sort",
    b"sort_by",
    b"sum",
    b"take",
    b"take_while",
    b"tally",
    b"to_h",
    b"uniq",
    b"zip",
    b"with_index",
    b"with_object",
    b"bsearch",
    b"bsearch_index",
    b"collect!",
    b"combination",
    b"d_permutation",
    b"delete_if",
    b"each_index",
    b"keep_if",
    b"map!",
    b"permutation",
    b"product",
    b"reject!",
    b"repeat",
    b"repeated_combination",
    b"select!",
    b"sort!",
    b"each_key",
    b"each_pair",
    b"each_value",
    b"fetch",
    b"fetch_values",
    b"has_key?",
    b"merge",
    b"merge!",
    b"transform_keys",
    b"transform_keys!",
    b"transform_values",
    b"transform_values!",
];

fn is_iterating(name: &[u8]) -> bool {
    ITERATING_METHODS.contains(&name)
}

// --- Method discovery -------------------------------------------------------

pub(crate) struct MethodFinder<'a> {
    source: &'a [u8],
    max_cyclomatic: usize,
    max_perceived: usize,
    pub(crate) out: Vec<MethodComplexity>,
}

impl MethodFinder<'_> {
    fn record(&mut self, start: usize, end: usize, head_end: usize, name: String, body: &Node<'_>) {
        let (cyclomatic, perceived) = score_body(self.source, body);
        if cyclomatic <= self.max_cyclomatic && perceived <= self.max_perceived {
            return;
        }
        self.out.push(MethodComplexity {
            start_offset: start,
            end_offset: end,
            head_end,
            method_name: name,
            cyclomatic,
            perceived,
        });
    }
}

/// Returns `(name, body, head_end)` for a `define_method :name do ... end`
/// block, mirroring RuboCop's `define_method?` matcher.
fn define_method_info<'a>(call: &ruby_prism::CallNode<'a>) -> Option<(String, Node<'a>, usize)> {
    if call.name().as_slice() != b"define_method" || call.receiver().is_some() {
        return None;
    }
    let block = call.block()?.as_block_node()?;
    let first = call.arguments()?.arguments().iter().next()?;
    let name = if let Some(sym) = first.as_symbol_node() {
        String::from_utf8_lossy(sym.unescaped()).into_owned()
    } else if let Some(str_node) = first.as_string_node() {
        String::from_utf8_lossy(str_node.unescaped()).into_owned()
    } else {
        return None;
    };
    let body = block.body()?;
    Some((name, body, block.opening_loc().end_offset()))
}

impl MethodFinder<'_> {
    /// Score a `def`'s body (a `defs` included).
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

    /// Score a `define_method :name do ... end` body.
    fn process_call(&mut self, node: &ruby_prism::CallNode<'_>) {
        if let Some((name, body, head_end)) = define_method_info(node) {
            let loc = node.location();
            self.record(loc.start_offset(), loc.end_offset(), head_end, name, &body);
        }
    }
}

impl<'pr> Visit<'pr> for MethodFinder<'_> {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        self.process_def(node);
        visit_def_node(self, node);
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.process_call(node);
        visit_call_node(self, node);
    }
}

/// Shared-walk driver. The generic branch hook fires for every `DefNode` and
/// for every `CallNode` the typed visits see except the one reached through
/// `MatchWriteNode`'s concretely-typed `call` field — an `=~` operator call,
/// which is never a `define_method` block, so `process_call` rejects it anyway.
impl super::dispatch::Rule for MethodFinder<'_> {
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

fn score_body(source: &[u8], body: &Node<'_>) -> (usize, usize) {
    let mut scorer = Scorer {
        source,
        csend_vars: HashSet::new(),
        cyclomatic: 1,
        perceived: 1,
    };
    scorer.visit(body);
    (scorer.cyclomatic, scorer.perceived)
}

struct Scorer<'a> {
    source: &'a [u8],
    /// Local variables that already had a counted `&.` since their last
    /// assignment (repeated `&.` on the same variable is discounted).
    csend_vars: HashSet<Vec<u8>>,
    cyclomatic: usize,
    perceived: usize,
}

impl Scorer<'_> {
    fn add_both(&mut self, n: usize) {
        self.cyclomatic += n;
        self.perceived += n;
    }

    fn keyword_is_elsif(&self, if_node: &ruby_prism::IfNode<'_>) -> bool {
        match if_node.if_keyword_loc() {
            Some(loc) => &self.source[loc.start_offset()..loc.end_offset()] == b"elsif",
            None => false,
        }
    }

    /// RuboCop perceived score for an `if`: `else? && !elsif? ? 2 : 1`, where
    /// `else?` is `!ternary && has-a-subsequent-clause`.
    fn perceived_if_score(&self, if_node: &ruby_prism::IfNode<'_>) -> usize {
        let ternary = if_node.if_keyword_loc().is_none();
        let has_else = !ternary && if_node.subsequent().is_some();
        if has_else && !self.keyword_is_elsif(if_node) {
            2
        } else {
            1
        }
    }

    fn case_score(&self, case_node: &ruby_prism::CaseNode<'_>) -> usize {
        let branches =
            case_node.conditions().len() + usize::from(case_node.else_clause().is_some());
        if case_node.predicate().is_none() {
            branches
        } else {
            ((branches as f64) * 0.2 + 0.8).round() as usize
        }
    }

    fn csend_contribution(&mut self, call: &ruby_prism::CallNode<'_>) -> usize {
        if let Some(recv) = call.receiver()
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

    fn score_call(&mut self, call: &ruby_prism::CallNode<'_>) {
        if call.is_safe_navigation() {
            let c = self.csend_contribution(call);
            self.add_both(c);
        }
        if call.block().is_some() {
            self.add_both(usize::from(is_iterating(call.name().as_slice())));
        }
    }

    fn score_node(&mut self, node: &Node<'_>) {
        if let Some(write) = node.as_local_variable_write_node() {
            self.csend_vars.remove(write.name().as_slice());
        } else if let Some(call) = node.as_call_node() {
            self.score_call(&call);
        } else if let Some(if_node) = node.as_if_node() {
            self.cyclomatic += 1;
            self.perceived += self.perceived_if_score(&if_node);
        } else if let Some(unless_node) = node.as_unless_node() {
            self.cyclomatic += 1;
            self.perceived += usize::from(unless_node.else_clause().is_some()) + 1;
        } else if let Some(case_node) = node.as_case_node() {
            self.perceived += self.case_score(&case_node);
        } else if let Some(begin_node) = node.as_begin_node() {
            if begin_node.rescue_clause().is_some() {
                self.add_both(1);
            }
        } else {
            self.score_simple(node);
        }
    }

    fn score_simple(&mut self, node: &Node<'_>) {
        match node {
            Node::WhileNode { .. }
            | Node::UntilNode { .. }
            | Node::ForNode { .. }
            | Node::InNode { .. }
            | Node::AndNode { .. }
            | Node::OrNode { .. }
            | Node::RescueModifierNode { .. }
            | Node::CallOrWriteNode { .. }
            | Node::ClassVariableOrWriteNode { .. }
            | Node::ConstantOrWriteNode { .. }
            | Node::ConstantPathOrWriteNode { .. }
            | Node::GlobalVariableOrWriteNode { .. }
            | Node::IndexOrWriteNode { .. }
            | Node::InstanceVariableOrWriteNode { .. }
            | Node::LocalVariableOrWriteNode { .. }
            | Node::CallAndWriteNode { .. }
            | Node::ClassVariableAndWriteNode { .. }
            | Node::ConstantAndWriteNode { .. }
            | Node::ConstantPathAndWriteNode { .. }
            | Node::GlobalVariableAndWriteNode { .. }
            | Node::IndexAndWriteNode { .. }
            | Node::InstanceVariableAndWriteNode { .. }
            | Node::LocalVariableAndWriteNode { .. } => self.add_both(1),
            // Perceived counts the `case`, cyclomatic counts the `when`s.
            Node::WhenNode { .. } => self.cyclomatic += 1,
            _ => {}
        }
    }
}

impl<'pr> Visit<'pr> for Scorer<'_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        self.score_node(&node);
    }

    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        self.score_node(&node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scores(source: &str) -> Vec<(String, usize, usize)> {
        check_complexity(source.as_bytes())
            .into_iter()
            .map(|m| (m.method_name, m.cyclomatic, m.perceived))
            .collect()
    }

    fn one(source: &str) -> (usize, usize) {
        let s = scores(source);
        assert_eq!(
            s.len(),
            1,
            "expected exactly one method in {source:?}, got {s:?}"
        );
        (s[0].1, s[0].2)
    }

    // Base score is 1 for a method with no decision points.
    #[test]
    fn no_decision_points() {
        assert_eq!(one("def m\n  call_foo\nend"), (1, 1));
    }

    // if modifier: +1 both.
    #[test]
    fn if_modifier() {
        assert_eq!(one("def m\n  call_foo if c\nend"), (2, 2));
    }

    // unless modifier: +1 both.
    #[test]
    fn unless_modifier() {
        assert_eq!(one("def m\n  call_foo unless c\nend"), (2, 2));
    }

    // if/elsif/else: cyclomatic 3 (two ifs), perceived 4 (outer else=2, elsif=1).
    #[test]
    fn if_elsif_else() {
        let src = "def m\n  if a\n    x\n  elsif b\n    y\n  else\n    z\n  end\nend";
        assert_eq!(one(src), (3, 4));
    }

    // ternary: +1 both (no else keyword).
    #[test]
    fn ternary() {
        assert_eq!(one("def m\n  v = c ? 1 : 2\nend"), (2, 2));
    }

    // while/until/for: +1 both.
    #[test]
    fn loops() {
        assert_eq!(one("def m\n  while c do x end\nend"), (2, 2));
        assert_eq!(one("def m\n  until c do x end\nend"), (2, 2));
        assert_eq!(one("def m\n  for i in 1..2 do x end\nend"), (2, 2));
    }

    // rescue: +1 both (container, regardless of clause count).
    #[test]
    fn rescue_block() {
        let src = "def m\n  begin\n    a\n  rescue E1\n    b\n  rescue E2\n    c\n  end\nend";
        assert_eq!(one(src), (2, 2));
    }

    // case/when: cyclomatic counts whens (+2), perceived scores the case.
    #[test]
    fn case_when_with_expr() {
        let src = "def m\n  case v\n  when 1 then a\n  when 2 then b\n  when 3 then c\n  when 4 then d\n  end\nend";
        // cyclomatic: 1 + 4 whens = 5; perceived: 1 + round(4*0.2+0.8)=1+2 = 3
        assert_eq!(one(src), (5, 3));
    }

    // case without expression: perceived counts each when as a branch.
    #[test]
    fn case_without_expr() {
        let src = "def m\n  case\n  when a then x\n  when b then y\n  end\nend";
        // cyclomatic: 1 + 2 whens = 3; perceived: 1 + 2 branches = 3
        assert_eq!(one(src), (3, 3));
    }

    // case without expression and else: perceived counts the else branch too.
    #[test]
    fn case_without_expr_with_else() {
        let src = "def m\n  case\n  when a then x\n  when b then y\n  else z\n  end\nend";
        assert_eq!(one(src), (3, 4));
    }

    // && / || / and / or: +1 both.
    #[test]
    fn boolean_operators() {
        assert_eq!(one("def m\n  a && b\nend"), (2, 2));
        assert_eq!(one("def m\n  a || b\nend"), (2, 2));
        assert_eq!(one("def m\n  a and b\nend"), (2, 2));
        assert_eq!(one("def m\n  a or b\nend"), (2, 2));
    }

    // ||= / &&=: +1 both.
    #[test]
    fn or_and_asgn() {
        assert_eq!(one("def m\n  foo = nil\n  foo ||= 42\nend"), (2, 2));
        assert_eq!(one("def m\n  foo = nil\n  foo &&= 42\nend"), (2, 2));
    }

    // Repeated &. on the same untouched local variable counts once.
    #[test]
    fn repeated_csend_discount() {
        let src =
            "def m\n  var = 1\n  var&.foo\n  var&.bar\n  var = 2\n  var&.baz\n  var&.qux\nend";
        // var&.foo (+1), var&.bar (discount 0), reset, var&.baz (+1), var&.qux (0)
        assert_eq!(one(src), (3, 3));
    }

    // Iterating blocks add 1; non-iterating blocks add 0.
    #[test]
    fn iterating_blocks() {
        assert_eq!(one("def m\n  [].map { |x| x }\nend"), (2, 2));
        assert_eq!(one("def m\n  [].map(&:to_s)\nend"), (2, 2));
        assert_eq!(one("def m\n  foo { bar }\nend"), (1, 1));
    }

    // define_method is treated as a method definition.
    #[test]
    fn define_method() {
        let s = scores("define_method :foo do\n  call if c\nend");
        assert_eq!(s, vec![("foo".to_string(), 2, 2)]);
    }

    // Each method is scored separately.
    #[test]
    fn separate_methods() {
        let s = scores("def a\n  x if c\nend\ndef b\n  y if d\nend");
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].1, s[0].2), (2, 2));
        assert_eq!((s[1].1, s[1].2), (2, 2));
    }

    // Empty methods are not reported.
    #[test]
    fn empty_method() {
        assert!(scores("def m\nend").is_empty());
    }

    // The threshold filter keeps a method when either score exceeds its Max.
    #[test]
    fn exceeding_filters_on_either_threshold() {
        // cyclomatic 3, perceived 4 (if/elsif/else).
        let src = "def m\n  if a\n    x\n  elsif b\n    y\n  else\n    z\n  end\nend";
        let kept = |max_c, max_p| check_complexity_exceeding(src.as_bytes(), max_c, max_p).len();
        assert_eq!(kept(2, 3), 1); // both exceed
        assert_eq!(kept(3, 3), 1); // only perceived exceeds
        assert_eq!(kept(2, 4), 1); // only cyclomatic exceeds
        assert_eq!(kept(3, 4), 0); // neither exceeds (boundary: > , not >=)
    }

    // A threshold of 0 reports every method (scores start at 1).
    #[test]
    fn exceeding_zero_thresholds_report_everything() {
        let src = "def a\n  x\nend\ndef b\n  y if c\nend";
        assert_eq!(check_complexity_exceeding(src.as_bytes(), 0, 0).len(), 2);
        assert_eq!(scores(src).len(), 2);
    }
}
