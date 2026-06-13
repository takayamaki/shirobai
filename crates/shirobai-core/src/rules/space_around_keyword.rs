//! `Layout/SpaceAroundKeyword`.
//!
//! Checks for a missing space before and/or after a keyword. Stock fires a
//! callback per node type (`on_if`, `on_while`, `on_block`, `on_super`, ...),
//! each handing `check` a list of `node.loc` parts (`:keyword`, `:begin`,
//! `:end`, `:else`, `:operator`, `:selector`, and the implicit `then`) plus a
//! `begin_keyword` (default `'do'`). `check` resolves each part to a range and
//! routes it:
//!
//! - `:begin` -> `check_begin`: only when the range text equals `begin_keyword`
//!   (`do` for blocks/loops/`for`, `then` for `if`, and `nil` for `kwbegin`
//!   meaning "always"). Then `check_keyword` (before + after).
//! - `:end` -> `check_end`: before-space only. Skipped when `begin_keyword == DO`
//!   and the construct is not a `do...end` form (`do?`), so a `{}` block / a
//!   `while`-modifier never flag their (absent) `end`. `if` / `kwbegin` use a
//!   non-`DO` `begin_keyword`, so their `end` is always before-checked.
//! - everything else -> `check_keyword` (before + after).
//!
//! `check_keyword` flags a missing space *before* (unless the preceding char is
//! one of `[\s(|{\[;,*=]`, or the keyword is `preceded_by_operator?`) and a
//! missing space *after* (unless the following char is one of `[\s;,#\\)}\].]`,
//! or an accepted opening delimiter / safe-navigation / namespace operator).
//!
//! Reconstructed over Prism in one ancestor-stack walk. The stock cop reads the
//! *parser-gem* AST, so each parser `on_xxx` maps to a Prism node + location:
//! `IfNode`/`UnlessNode` (if/elsif/unless, modifier and guard forms all reduce
//! to the same keyword/then/else/end checks), `WhileNode`/`UntilNode`,
//! `CaseNode`/`CaseMatchNode`, `ForNode`, `BlockNode`, `BeginNode` (kwbegin),
//! `ElseNode` (case/if/rescue `else`), `RescueNode` (`resbody` keyword),
//! `EnsureNode`, `WhenNode`, `InNode`, `Pre`/`PostExecutionNode`,
//! `SuperNode`/`ForwardingSuperNode`, `YieldNode`, `ReturnNode`, `BreakNode`,
//! `NextNode`, `DefinedNode`, `AndNode`/`OrNode` (only the `and`/`or` keyword
//! form, not `&&`/`||`), and the prefix-`!` `CallNode`.
//!
//! `preceded_by_operator?` climbs the ancestor stack: an `and`/`or`/`&&`/`||`
//! (`AndNode`/`OrNode`) or a range (`RangeNode`) ancestor means "preceded by an
//! operator" (suppress the before-space offense); a non-operator method call
//! (`CallNode` whose name is not an operator) is climbed past (dotted calls
//! bind tighter than operators); an operator-method `CallNode` means true; any
//! other ancestor means false.

use ruby_prism::{CallNode, Location, Node};

/// One missing-space offense. `(start, end)` is the keyword range (the offense
/// highlight). `before` is true for a missing space *before* the keyword
/// (autocorrect inserts a space before the range) and false for a missing space
/// *after* (inserts a space after).
pub struct SpaceAroundKeywordOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub before: bool,
}

/// Classification of an ancestor for `preceded_by_operator?`.
#[derive(Clone, Copy, PartialEq)]
enum Anc {
    /// `and` / `or` / `&&` / `||` (`operator_keyword?`) or a range (`..`/`...`).
    OperatorOrRange,
    /// A `CallNode` whose method name is an operator (`operator_method?`).
    OperatorCall,
    /// A `CallNode` whose method name is a regular (non-operator) method.
    RegularSend,
    /// Anything else.
    Other,
}

pub fn check_space_around_keyword(source: &[u8]) -> Vec<SpaceAroundKeywordOffense> {
    let mut rule = build_rule(source);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8]) -> Visitor<'_> {
    Visitor {
        source,
        offenses: Vec::new(),
        ancestors: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    pub(crate) offenses: Vec<SpaceAroundKeywordOffense>,
    /// Classification of every open ancestor; top = parent of the entering
    /// node. Drives `preceded_by_operator?`.
    ancestors: Vec<Anc>,
}

/// `DO` keyword text.
const DO: &[u8] = b"do";
/// `super` only: a `::` namespace operator after the keyword is accepted.
const ACCEPT_NAMESPACE_OPERATOR: &[u8] = b"super";

impl<'a> Visitor<'a> {
    /// Classify `node` for the ancestor stack (`preceded_by_operator?`).
    fn classify(node: &Node<'_>) -> Anc {
        if node.as_and_node().is_some()
            || node.as_or_node().is_some()
            || node.as_range_node().is_some()
        {
            Anc::OperatorOrRange
        } else if let Some(call) = node.as_call_node() {
            if is_operator_method(&call) {
                Anc::OperatorCall
            } else {
                Anc::RegularSend
            }
        } else {
            Anc::Other
        }
    }

    /// `preceded_by_operator?`: climb the ancestor stack (top first = nearest
    /// ancestor). Returns true at the first operator/range ancestor or
    /// operator-method send; climbs past regular sends; false at any other
    /// ancestor (or when the stack runs out).
    fn preceded_by_operator(&self) -> bool {
        for anc in self.ancestors.iter().rev() {
            match anc {
                Anc::OperatorOrRange | Anc::OperatorCall => return true,
                Anc::RegularSend => continue,
                Anc::Other => return false,
            }
        }
        false
    }

    /// `check_keyword`: emit a missing-space-before offense (unless the char
    /// before is an accepted one or the keyword is preceded by an operator) and
    /// a missing-space-after offense (unless the char after is accepted).
    fn check_keyword(&mut self, range: Location<'_>) {
        self.check_keyword_range(range.start_offset(), range.end_offset());
    }

    /// `check_keyword` on an explicit `[start, end)` byte range (used for a bare
    /// `super`, whose `ForwardingSuperNode` has no keyword loc — the node's own
    /// location spans any trailing `{}` block, so the keyword is the literal
    /// `super` at the node start).
    fn check_keyword_range(&mut self, start: usize, end: usize) {
        if self.space_before_missing(start) && !self.preceded_by_operator() {
            self.offenses.push(SpaceAroundKeywordOffense {
                start_offset: start,
                end_offset: end,
                before: true,
            });
        }
        if self.space_after_missing(start, end) {
            self.offenses.push(SpaceAroundKeywordOffense {
                start_offset: start,
                end_offset: end,
                before: false,
            });
        }
    }

    /// `check_end`: before-space only. `do_form` is `do?(node)`; when false and
    /// the construct uses the default `DO` begin_keyword, the (absent) `end` is
    /// not checked.
    fn check_end(&mut self, range: Location<'_>, begin_keyword_is_do: bool, do_form: bool) {
        if begin_keyword_is_do && !do_form {
            return;
        }
        let start = range.start_offset();
        if self.space_before_missing(start) {
            self.offenses.push(SpaceAroundKeywordOffense {
                start_offset: start,
                end_offset: range.end_offset(),
                before: true,
            });
        }
    }

    /// `check_begin`: only when the range text equals `begin_keyword`. `None`
    /// `begin_keyword` (kwbegin) means "always". Then `check_keyword`.
    fn check_begin(&mut self, range: Location<'_>, begin_keyword: Option<&[u8]>) {
        if let Some(kw) = begin_keyword {
            let text = &self.source[range.start_offset()..range.end_offset()];
            if text != kw {
                return;
            }
        }
        self.check_keyword(range);
    }

    /// `space_before_missing?`: the char before the range is not one of
    /// `[\s(|{\[;,*=]`. At the start of the buffer there is no missing space.
    fn space_before_missing(&self, start: usize) -> bool {
        if start == 0 {
            return false;
        }
        let c = self.source[start - 1];
        !(c.is_ascii_whitespace()
            || matches!(c, b'(' | b'|' | b'{' | b'[' | b';' | b',' | b'*' | b'='))
    }

    /// `space_after_missing?`: the char after the range is not accepted.
    fn space_after_missing(&self, start: usize, end: usize) -> bool {
        let keyword = &self.source[start..end];
        let next = self.source.get(end).copied();

        if self.accepted_opening_delimiter(keyword, next) {
            return false;
        }
        if self.safe_navigation_call(end) {
            return false;
        }
        if keyword == ACCEPT_NAMESPACE_OPERATOR && self.namespace_operator(end) {
            return false;
        }
        match next {
            None => false,
            Some(c) => !(c.is_ascii_whitespace()
                || matches!(c, b';' | b',' | b'#' | b'\\' | b')' | b'}' | b']' | b'.')),
        }
    }

    /// `accepted_opening_delimiter?`: end-of-buffer accepts; `[` after a
    /// `super`/`yield`; `(` after a break/defined?/next/not/rescue/super/yield.
    fn accepted_opening_delimiter(&self, keyword: &[u8], next: Option<u8>) -> bool {
        let Some(c) = next else {
            return true;
        };
        (accept_left_square_bracket(keyword) && c == b'[')
            || (accept_left_parenthesis(keyword) && c == b'(')
    }

    /// `safe_navigation_call?`: the two chars after the keyword start with `&.`.
    fn safe_navigation_call(&self, end: usize) -> bool {
        self.source.get(end..end + 2) == Some(b"&." as &[u8])
    }

    /// `namespace_operator?`: the two chars after the keyword start with `::`.
    fn namespace_operator(&self, end: usize) -> bool {
        self.source.get(end..end + 2) == Some(b"::" as &[u8])
    }

    // --- Per-node-type dispatch (one `on_xxx` each). ---

    /// `on_if` (and `on_if_guard`/`on_unless_guard` — guards reduce to the
    /// keyword check). `if`/`elsif`/`unless`: keyword, then (`begin` with
    /// begin_keyword `then`), else, end (non-`DO` begin_keyword -> always
    /// before-checked). `else`/`end` are handled on the `ElseNode` and (for
    /// `end`) here via `end_keyword_loc`.
    fn on_if(
        &mut self,
        kw: Option<Location<'_>>,
        then_loc: Option<Location<'_>>,
        end: Option<Location<'_>>,
    ) {
        // A ternary (`a ? b : c`) is an `IfNode` with no `if` keyword loc; stock
        // (`node.loc?(:keyword)`) and parser never see a keyword there.
        if let Some(kw) = kw {
            self.check_keyword(kw);
        }
        if let Some(then_loc) = then_loc {
            // `:begin` with begin_keyword `then` -> only fires for a literal
            // `then` (prism only has `then_keyword_loc` when `then` is written).
            self.check_begin(then_loc, Some(b"then"));
        }
        if let Some(end) = end {
            // begin_keyword is `then` (non-DO) -> `end` always before-checked.
            self.check_end(end, false, false);
        }
    }

    /// `on_while` / `on_until`: keyword, begin (`do`), end (only `do?`).
    fn on_while_until(
        &mut self,
        kw: Location<'_>,
        do_loc: Option<Location<'_>>,
        end: Option<Location<'_>>,
    ) {
        self.check_keyword(kw);
        let do_form = do_loc.is_some();
        if let Some(do_loc) = do_loc {
            self.check_begin(do_loc, Some(DO));
        }
        if let Some(end) = end {
            self.check_end(end, true, do_form);
        }
    }

    /// `on_block` (and numblock / itblock): begin (`do`), end (only `do?`).
    fn on_block(&mut self, opening: Location<'_>, closing: Option<Location<'_>>) {
        let opening_text = &self.source[opening.start_offset()..opening.end_offset()];
        let do_form = opening_text == DO;
        self.check_begin(opening, Some(DO));
        if let Some(closing) = closing {
            self.check_end(closing, true, do_form);
        }
    }

    /// `on_for`: begin (`do`), end (only `do?`).
    fn on_for(&mut self, do_loc: Option<Location<'_>>, end: Location<'_>) {
        let do_form = do_loc.is_some();
        if let Some(do_loc) = do_loc {
            self.check_begin(do_loc, Some(DO));
        }
        self.check_end(end, true, do_form);
    }

    /// `on_kwbegin`: begin (`begin`, begin_keyword `nil` -> always), end
    /// (begin_keyword `nil` -> always before-checked).
    fn on_kwbegin(&mut self, begin: Location<'_>, end: Location<'_>) {
        self.check_begin(begin, None);
        self.check_end(end, false, false);
    }

    /// `on_case` / `on_case_match`: keyword (the `else` is on the `ElseNode`).
    fn on_case_keyword(&mut self, kw: Location<'_>) {
        self.check_keyword(kw);
    }

    /// case/if/rescue `else`. A ternary's "else" is an `ElseNode` whose keyword
    /// is `:`, which parser never represents as an `else` keyword, so only a
    /// literal `else` is checked.
    fn on_else(&mut self, kw: Location<'_>) {
        if &self.source[kw.start_offset()..kw.end_offset()] == b"else" {
            self.check_keyword(kw);
        }
    }
}

/// `operator_method?`: a `CallNode` whose method name is an operator. RuboCop's
/// `Node#operator_method?` is `OPERATOR_METHODS.include?(method_name)`; here we
/// recognize the same set by inspecting the selector text (the `name` bytes).
fn is_operator_method(call: &CallNode<'_>) -> bool {
    let name = call.name();
    let n = name.as_slice();
    matches!(
        n,
        b"+" | b"-"
            | b"*"
            | b"/"
            | b"%"
            | b"**"
            | b"=="
            | b"==="
            | b"!="
            | b"<"
            | b">"
            | b"<="
            | b">="
            | b"<=>"
            | b"<<"
            | b">>"
            | b"&"
            | b"|"
            | b"^"
            | b"~"
            | b"!"
            | b"=~"
            | b"!~"
            | b"[]"
            | b"[]="
            | b"+@"
            | b"-@"
            | b"`"
    )
}

/// `ACCEPT_LEFT_PAREN`.
fn accept_left_parenthesis(keyword: &[u8]) -> bool {
    matches!(
        keyword,
        b"break" | b"defined?" | b"next" | b"not" | b"rescue" | b"super" | b"yield"
    )
}

/// `ACCEPT_LEFT_SQUARE_BRACKET`.
fn accept_left_square_bracket(keyword: &[u8]) -> bool {
    matches!(keyword, b"super" | b"yield")
}


/// Shared-walk driver. The ancestor stack is maintained by `enter`/`leave`
/// (every branch node pushes its classification); the per-node keyword checks
/// run on `enter`. `RescueNode` is reached through `BeginNode`'s typed
/// `rescue_clause` field, so it never hits the generic branch hook — it is
/// handled in `enter_rescue` instead (mirroring stock's `on_resbody`).
impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(n) = node.as_if_node() {
            self.on_if(n.if_keyword_loc(), n.then_keyword_loc(), n.end_keyword_loc());
        } else if let Some(n) = node.as_unless_node() {
            self.on_if(
                Some(n.keyword_loc()),
                n.then_keyword_loc(),
                n.end_keyword_loc(),
            );
        } else if let Some(n) = node.as_while_node() {
            self.on_while_until(n.keyword_loc(), n.do_keyword_loc(), n.closing_loc());
        } else if let Some(n) = node.as_until_node() {
            self.on_while_until(n.keyword_loc(), n.do_keyword_loc(), n.closing_loc());
        } else if let Some(n) = node.as_block_node() {
            self.on_block(n.opening_loc(), Some(n.closing_loc()));
        } else if let Some(n) = node.as_for_node() {
            self.on_for(n.do_keyword_loc(), n.end_keyword_loc());
        } else if let Some(n) = node.as_begin_node() {
            // `on_kwbegin` only for an explicit `begin ... end`.
            if let (Some(begin), Some(end)) = (n.begin_keyword_loc(), n.end_keyword_loc()) {
                self.on_kwbegin(begin, end);
            }
            // A `begin`'s `else` / `ensure` clauses are reached through typed
            // `else_clause` / `ensure_clause` fields, bypassing the generic
            // branch hook, so they are checked here from the owning node
            // (`on_rescue`'s `:else` and `on_ensure`). The `rescue` clause is
            // handled in `enter_rescue`.
            if let Some(els) = n.else_clause() {
                self.on_else(els.else_keyword_loc());
            }
            if let Some(ens) = n.ensure_clause() {
                self.check_keyword(ens.ensure_keyword_loc());
            }
        } else if let Some(n) = node.as_case_node() {
            self.on_case_keyword(n.case_keyword_loc());
            // `case`'s `else` is a typed `else_clause`, bypassing the hook.
            if let Some(els) = n.else_clause() {
                self.on_else(els.else_keyword_loc());
            }
        } else if let Some(n) = node.as_case_match_node() {
            self.on_case_keyword(n.case_keyword_loc());
            if let Some(els) = n.else_clause() {
                self.on_else(els.else_keyword_loc());
            }
        } else if let Some(n) = node.as_else_node() {
            // Reached via the generic hook only as an `if`/`elsif` `subsequent`
            // (`if ... else ... end`); `case` / `begin` `else` is handled from
            // the owning node above.
            self.on_else(n.else_keyword_loc());
        } else if let Some(n) = node.as_when_node() {
            self.check_keyword(n.keyword_loc());
        } else if let Some(n) = node.as_in_node() {
            self.check_keyword(n.in_loc());
        } else if let Some(n) = node.as_match_predicate_node() {
            // `on_match_pattern_p`: one-line `expr in pattern` (`in` operator).
            self.check_keyword(n.operator_loc());
        } else if let Some(n) = node.as_rescue_modifier_node() {
            // `on_resbody`: the `rescue` of a modifier `expr rescue expr`.
            self.check_keyword(n.keyword_loc());
        } else if let Some(n) = node.as_pre_execution_node() {
            self.check_keyword(n.keyword_loc());
        } else if let Some(n) = node.as_post_execution_node() {
            self.check_keyword(n.keyword_loc());
        } else if let Some(n) = node.as_super_node() {
            self.check_keyword(n.keyword_loc());
        } else if let Some(n) = node.as_forwarding_super_node() {
            let start = n.location().start_offset();
            self.check_keyword_range(start, start + b"super".len());
        } else if let Some(n) = node.as_yield_node() {
            self.check_keyword(n.keyword_loc());
        } else if let Some(n) = node.as_return_node() {
            self.check_keyword(n.keyword_loc());
        } else if let Some(n) = node.as_break_node() {
            self.check_keyword(n.keyword_loc());
        } else if let Some(n) = node.as_next_node() {
            self.check_keyword(n.keyword_loc());
        } else if let Some(n) = node.as_defined_node() {
            self.check_keyword(n.keyword_loc());
        } else if let Some(n) = node.as_and_node() {
            if n.operator_loc().as_slice() == b"and" {
                self.check_keyword(n.operator_loc());
            }
        } else if let Some(n) = node.as_or_node() {
            if n.operator_loc().as_slice() == b"or" {
                self.check_keyword(n.operator_loc());
            }
        } else if let Some(n) = node.as_call_node()
            && n.name().as_slice() == b"!"
            && n.receiver().is_some()
            && n.opening_loc().is_none()
            && let Some(msg) = n.message_loc()
            && &self.source[msg.start_offset()..msg.end_offset()] == b"not"
        {
            // `on_send` with `prefix_not?`: a `not expr` prefix. (The `!expr`
            // form is the `!` operator, not this cop's `:selector`.)
            self.check_keyword(msg);
        }

        self.ancestors.push(Self::classify(node));
    }

    fn leave(&mut self) {
        self.ancestors.pop();
    }

    fn enter_rescue(&mut self, node: &Node<'_>) {
        if let Some(n) = node.as_rescue_node() {
            self.check_keyword(n.keyword_loc());
        }
        self.ancestors.push(Self::classify(node));
    }

    fn leave_rescue(&mut self) {
        self.ancestors.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<(usize, usize, char)> {
        check_space_around_keyword(source.as_bytes())
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset, if o.before { 'B' } else { 'A' }))
            .collect()
    }

    #[test]
    fn after_pre_execution() {
        assert_eq!(run("BEGIN{}\n"), vec![(0, 5, 'A')]);
    }

    #[test]
    fn after_post_execution() {
        assert_eq!(run("END{}\n"), vec![(0, 3, 'A')]);
    }

    #[test]
    fn before_and_keyword() {
        assert_eq!(run("1and 2\n"), vec![(1, 4, 'B')]);
    }

    #[test]
    fn after_and_keyword() {
        assert_eq!(run("1 and(2)\n"), vec![(2, 5, 'A')]);
    }

    #[test]
    fn does_not_flag_double_ampersand() {
        assert!(run("1&&2\n").is_empty());
    }

    #[test]
    fn after_kwbegin() {
        assert_eq!(run("begin\"\" end\n"), vec![(0, 5, 'A')]);
    }

    #[test]
    fn after_case() {
        assert_eq!(run("case\"\" when 1; end\n"), vec![(0, 4, 'A')]);
    }

    #[test]
    fn before_do_in_block() {
        assert_eq!(run("a \"b\"do end\n"), vec![(5, 7, 'B')]);
    }

    #[test]
    fn after_do_in_block() {
        assert_eq!(run("a do|x| end\n"), vec![(2, 4, 'A')]);
    }

    #[test]
    fn before_do_in_while() {
        assert_eq!(run("while 1do end\n"), vec![(7, 9, 'B')]);
    }

    #[test]
    fn after_do_in_while() {
        assert_eq!(run("while 1 do\"x\" end\n"), vec![(8, 10, 'A')]);
    }

    #[test]
    fn before_do_in_for() {
        assert_eq!(run("for x in []do end\n"), vec![(11, 13, 'B')]);
    }

    #[test]
    fn before_end_in_kwbegin() {
        assert_eq!(run("begin \"a\"end\n"), vec![(9, 12, 'B')]);
    }

    #[test]
    fn before_end_in_if() {
        assert_eq!(run("if a; \"b\"end\n"), vec![(9, 12, 'B')]);
    }

    #[test]
    fn before_end_in_do_block() {
        assert_eq!(run("a do \"a\"end\n"), vec![(8, 11, 'B')]);
    }

    #[test]
    fn before_else_in_if() {
        assert_eq!(run("if a; \"\"else end\n"), vec![(8, 12, 'B')]);
    }

    #[test]
    fn after_else_in_if() {
        assert_eq!(run("if a; else\"\" end\n"), vec![(6, 10, 'A')]);
    }

    #[test]
    fn before_else_in_rescue() {
        assert_eq!(run("begin rescue; \"\"else end\n"), vec![(16, 20, 'B')]);
    }

    #[test]
    fn before_else_in_case() {
        assert_eq!(run("case a; when b; \"\"else end\n"), vec![(18, 22, 'B')]);
    }

    #[test]
    fn before_elsif() {
        assert_eq!(run("if a; \"\"elsif b; end\n"), vec![(8, 13, 'B')]);
    }

    #[test]
    fn after_elsif() {
        assert_eq!(run("if a; elsif\"\"; end\n"), vec![(6, 11, 'A')]);
    }

    #[test]
    fn before_ensure() {
        assert_eq!(run("begin \"\"ensure end\n"), vec![(8, 14, 'B')]);
    }

    #[test]
    fn after_ensure() {
        assert_eq!(run("begin ensure\"\" end\n"), vec![(6, 12, 'A')]);
    }

    #[test]
    fn after_if() {
        assert_eq!(run("if\"\"; end\n"), vec![(0, 2, 'A')]);
    }

    #[test]
    fn after_not() {
        assert_eq!(run("not\"\"\n"), vec![(0, 3, 'A')]);
    }

    #[test]
    fn accepts_not_paren() {
        assert!(run("not(1)\n").is_empty());
    }

    #[test]
    fn before_rescue_modifier() {
        assert_eq!(run("\"\"rescue a\n"), vec![(2, 8, 'B')]);
    }

    #[test]
    fn after_rescue_modifier() {
        assert_eq!(run("a rescue\"\"\n"), vec![(2, 8, 'A')]);
    }

    #[test]
    fn accepts_rescue_paren() {
        assert!(run("begin; rescue(Error); end\n").is_empty());
    }

    #[test]
    fn after_return_string() {
        assert_eq!(run("return\"\"\n"), vec![(0, 6, 'A')]);
    }

    #[test]
    fn after_return_paren() {
        assert_eq!(run("return(1)\n"), vec![(0, 6, 'A')]);
    }

    #[test]
    fn after_super_string() {
        assert_eq!(run("super\"\"\n"), vec![(0, 5, 'A')]);
    }

    #[test]
    fn accepts_super_paren() {
        assert!(run("super(1)\n").is_empty());
    }

    #[test]
    fn after_super_brace() {
        assert_eq!(run("super{}\n"), vec![(0, 5, 'A')]);
    }

    #[test]
    fn accepts_defined_paren() {
        assert!(run("defined?(1)\n").is_empty());
    }

    #[test]
    fn after_defined() {
        assert_eq!(run("defined?1\n"), vec![(0, 8, 'A')]);
    }

    #[test]
    fn before_then() {
        assert_eq!(run("if \"\"then a end\n"), vec![(5, 9, 'B')]);
    }

    #[test]
    fn after_then() {
        assert_eq!(run("if a then\"\" end\n"), vec![(5, 9, 'A')]);
    }

    #[test]
    fn after_unless() {
        assert_eq!(run("unless\"\"; end\n"), vec![(0, 6, 'A')]);
    }

    #[test]
    fn before_until_modifier() {
        assert_eq!(run("1until \"\"\n"), vec![(1, 6, 'B')]);
    }

    #[test]
    fn after_until_modifier() {
        assert_eq!(run("1 until\"\"\n"), vec![(2, 7, 'A')]);
    }

    #[test]
    fn before_when() {
        assert_eq!(run("case \"\"when a; end\n"), vec![(7, 11, 'B')]);
    }

    #[test]
    fn after_when() {
        assert_eq!(run("case a when\"\"; end\n"), vec![(7, 11, 'A')]);
    }

    #[test]
    fn before_while_modifier() {
        assert_eq!(run("1while \"\"\n"), vec![(1, 6, 'B')]);
    }

    #[test]
    fn after_yield() {
        assert_eq!(run("yield\"\"\n"), vec![(0, 5, 'A')]);
    }

    #[test]
    fn accepts_yield_paren() {
        assert!(run("yield(1)\n").is_empty());
    }

    #[test]
    fn accepts_operator_before_begin() {
        assert!(run("+begin end\n").is_empty());
    }

    #[test]
    fn after_begin_plus() {
        assert_eq!(run("begin+1 end\n"), vec![(0, 5, 'A')]);
    }

    #[test]
    fn accepts_dot_after_begin_end() {
        assert!(run("begin end.inspect\n").is_empty());
    }

    #[test]
    fn accepts_bang_before_yield() {
        assert!(run("!yield\n").is_empty());
    }

    #[test]
    fn accepts_dot_after_yield() {
        assert!(run("yield.method\n").is_empty());
    }

    #[test]
    fn accepts_bang_yield_method() {
        assert!(run("!yield.method\n").is_empty());
    }

    #[test]
    fn accepts_bang_super_method() {
        assert!(run("!super.method\n").is_empty());
    }

    #[test]
    fn accepts_super_namespace() {
        assert!(run("super::ModuleName\n").is_empty());
    }

    #[test]
    fn accepts_super_safe_nav() {
        assert!(run("super&.foo\n").is_empty());
    }

    #[test]
    fn accepts_yield_safe_nav() {
        assert!(run("yield&.foo\n").is_empty());
    }

    #[test]
    fn accepts_super_bracket() {
        assert!(run("super[1]\n").is_empty());
    }

    #[test]
    fn accepts_yield_bracket() {
        assert!(run("yield[1]\n").is_empty());
    }

    #[test]
    fn accepts_pipe_before_break() {
        assert!(run("loop { |x|break }\n").is_empty());
    }

    #[test]
    fn accepts_range_before_super() {
        assert!(run("1..super.size\n").is_empty());
    }

    #[test]
    fn accepts_erange_before_super() {
        assert!(run("1...super.size\n").is_empty());
    }

    #[test]
    fn accepts_operators_before_begin() {
        assert!(run("a=begin end\n").is_empty());
        assert!(run("a==begin end\n").is_empty());
        assert!(run("a+begin end\n").is_empty());
        assert!(run("a+begin; end.method\n").is_empty());
    }

    #[test]
    fn clean_constructs() {
        assert!(run("loop{}\n").is_empty());
        assert!(run("a 1,foo,1\n").is_empty());
        assert!(run("test do;end\n").is_empty());
        assert!(run("[begin end]\n").is_empty());
        assert!(run("loop {next}\n").is_empty());
        assert!(run("{a: begin end}\n").is_empty());
        assert!(run("a[begin end]\n").is_empty());
        assert!(run("\"#{begin end}\"\n").is_empty());
    }

    #[test]
    fn after_in_pattern() {
        assert_eq!(run("case a; in\"\"; end\n"), vec![(8, 10, 'A')]);
    }

    #[test]
    fn after_in_one_line_pattern() {
        assert_eq!(run("a in\"\"\n"), vec![(2, 4, 'A')]);
    }

    #[test]
    fn before_in_one_line_pattern() {
        assert_eq!(run("\"\"in a\n"), vec![(2, 4, 'B')]);
    }

    #[test]
    fn before_else_in_case_match() {
        assert_eq!(run("case a; in b; \"\"else end\n"), vec![(16, 20, 'B')]);
    }

    #[test]
    fn after_else_in_case_match() {
        assert_eq!(run("case a; in b; else\"\" end\n"), vec![(14, 18, 'A')]);
    }

    #[test]
    fn clean_case_match_no_offense() {
        assert!(run("case \"\"; in 1; end\n").is_empty());
    }

    #[test]
    fn before_if_guard_in_pattern() {
        assert_eq!(
            run("case a; in \"pattern\"if \"condition\"; else \"\" end\n"),
            vec![(20, 22, 'B')]
        );
    }

    #[test]
    fn after_if_guard_in_pattern() {
        assert_eq!(
            run("case a; in \"pattern\" if\"condition\"; else \"\" end\n"),
            vec![(21, 23, 'A')]
        );
    }
}
