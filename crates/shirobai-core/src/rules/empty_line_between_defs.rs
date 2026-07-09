//! `Layout/EmptyLineBetweenDefs`.
//!
//! Checks that class / module / method (and `DefLikeMacros`) definitions are
//! separated by the configured number of empty lines. The stock cop fires
//! `on_begin` and walks `node.children.each_cons(2)`, checking every adjacent
//! pair where both members are "candidates" (a method def, a class, a module,
//! or a configured def-like macro). For each offending pair it reports the
//! second member's `def_location` and, on autocorrect, inserts or removes empty
//! lines at the newline after the first member's `end`.
//!
//! Here it is reconstructed over Prism. A parser-gem `(begin ...)` is a Prism
//! `StatementsNode` with at least two children (an implicit statement group, a
//! `(...)` group, the program root, or an explicit `begin`/`kwbegin` body) and
//! a parser-gem `(kwbegin ...)` with handlers exposes its protected / handler
//! bodies as further `(begin ...)` groups. Processing every `StatementsNode`
//! body owned by each node (plus the `RescueNode` handler bodies) reproduces
//! `on_begin` exactly. A call-with-block (`foo do ... end`) is a Prism
//! `CallNode` carrying a `BlockNode`, mirroring parser's `(block (send ...))`;
//! the candidate node is that whole `CallNode`.
//!
//! `macro?` (`!receiver && in_macro_scope?`) is resolved with the same
//! ancestor-stack macro-scope chain `Layout/IndentationConsistency` uses: a
//! macro candidate qualifies iff the enclosing begin is in macro scope.

use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// One missing/extra-empty-line offense. `[start_offset, end_offset)` is the
/// second member's `def_location` (the reported offense range). `message` is the
/// fully formatted stock message. The autocorrect mirrors stock's `autocorrect`:
/// `insert` selects the arm (`true` = `insert_after(range_between(pos, pos+1),
/// "\n" * n)`, `false` = `remove(range_between(pos, pos + n))`), where `pos` is
/// the computed `newline_pos`.
pub struct EmptyLineBetweenDefsOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: String,
    pub insert: bool,
    pub pos: usize,
    pub n: usize,
}

#[derive(Clone)]
pub struct Config {
    pub method_defs: bool,
    pub class_defs: bool,
    pub module_defs: bool,
    pub allow_adjacent_one_line_defs: bool,
    /// `NumberOfEmptyLines` as `[min, max]` (a scalar `k` packs as `[k, k]`).
    pub minimum_empty_lines: usize,
    pub maximum_empty_lines: usize,
    /// `DefLikeMacros` method names, compared verbatim against the call name
    /// (stock does `fetch('DefLikeMacros', []).map(&:to_sym)`, no case folding).
    pub def_like_macros: Vec<String>,
}

pub fn check_empty_line_between_defs(
    source: &[u8],
    config: Config,
) -> Vec<EmptyLineBetweenDefsOffense> {
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
        ancestors: Vec::new(),
        condition_ranges: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    config: Config,
    pub(crate) offenses: Vec<EmptyLineBetweenDefsOffense>,
    /// `(kind, start_offset)` for each open ancestor node (pushed on `enter`,
    /// popped on `leave`); the top is the parent of the node currently entering.
    ancestors: Vec<(NodeKind, usize)>,
    /// The predicate range of each open `if` / `unless` ancestor (or `None`).
    condition_ranges: Vec<Option<(usize, usize)>>,
}

/// Ancestor node kinds for the `in_macro_scope?` chain (identical semantics to
/// `Layout/IndentationConsistency`).
#[derive(Clone, Copy, PartialEq)]
enum NodeKind {
    Program,
    ClassLike,
    ClassConstructor,
    Wrapper,
    IfCondition,
    Other,
}

/// What kind of definition a candidate is, for the message `type` and the
/// `single_line?` / location handling.
#[derive(Clone, Copy, PartialEq)]
enum CandKind {
    Method,
    Class,
    Module,
    Block,
    Send,
}

/// A resolved candidate: the node range `[start, end)` and its kind plus the
/// `def_location` and `def_start` / `def_end` line numbers.
struct Candidate {
    /// Full source range of the candidate node (for `single_line?` and the
    /// `begin_pos` used in the same-line adjustment).
    node_start: usize,
    node_end: usize,
    /// `def_location` range (the reported offense location).
    loc_start: usize,
    loc_end: usize,
    /// `def_start(node)` 1-based line.
    start_line: usize,
    /// `def_end(node)` 1-based line.
    end_line: usize,
    /// `end_loc(node).end_pos` (the `end`/body end offset), for `newline_pos`.
    end_pos: usize,
    kind: CandKind,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

impl<'a> Visitor<'a> {
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    /// The `in_macro_scope?` chain evaluated at the enclosing begin (its parent
    /// is `ancestors.last()`): true iff the parent is class-like /
    /// class-constructor / program, or a wrapper that is itself in macro scope.
    fn begin_in_macro_scope(&self) -> bool {
        for &(kind, _) in self.ancestors.iter().rev() {
            match kind {
                NodeKind::Program | NodeKind::ClassLike | NodeKind::ClassConstructor => return true,
                NodeKind::Wrapper => continue,
                NodeKind::IfCondition | NodeKind::Other => return false,
            }
        }
        false
    }

    /// Whether `name` is a bare command (`!receiver`, no args needed here) that
    /// is a configured def-like macro and sits in macro scope.
    fn is_macro_name(&self, name: &[u8]) -> bool {
        if self.config.def_like_macros.is_empty() {
            return false;
        }
        let Ok(s) = std::str::from_utf8(name) else {
            return false;
        };
        self.config.def_like_macros.iter().any(|m| m == s)
    }

    /// Resolve a top-level statement node into a candidate, or `None` if it is
    /// not a def-like definition under the current config.
    fn candidate(&self, node: &Node<'_>) -> Option<Candidate> {
        // Method: `def` / `defs` (DefNode covers both `def m` and `def self.m`).
        if let Some(d) = node.as_def_node() {
            if !self.config.method_defs {
                return None;
            }
            let (ns, ne) = loc(&node.location());
            let loc_start = d.def_keyword_loc().start_offset();
            let loc_end = d.name_loc().end_offset();
            return Some(Candidate {
                node_start: ns,
                node_end: ne,
                loc_start,
                loc_end,
                // def_start = def keyword line; def_end = node end line.
                start_line: self.line_of(loc_start),
                end_line: self.line_of(ne),
                end_pos: ne,
                kind: CandKind::Method,
            });
        }
        if let Some(c) = node.as_class_node() {
            if !self.config.class_defs {
                return None;
            }
            let (ns, ne) = loc(&node.location());
            // `loc.keyword.join(loc.name)`.
            let loc_start = c.class_keyword_loc().start_offset();
            let loc_end = c.constant_path().location().end_offset();
            return Some(Candidate {
                node_start: ns,
                node_end: ne,
                loc_start,
                loc_end,
                start_line: self.line_of(loc_start),
                end_line: self.line_of(ne),
                end_pos: ne,
                kind: CandKind::Class,
            });
        }
        if let Some(m) = node.as_module_node() {
            if !self.config.module_defs {
                return None;
            }
            let (ns, ne) = loc(&node.location());
            let loc_start = m.module_keyword_loc().start_offset();
            let loc_end = m.constant_path().location().end_offset();
            return Some(Candidate {
                node_start: ns,
                node_end: ne,
                loc_start,
                loc_end,
                start_line: self.line_of(loc_start),
                end_line: self.line_of(ne),
                end_pos: ne,
                kind: CandKind::Module,
            });
        }
        // Macro: a call (optionally carrying a block). The send node (the call
        // minus the block) supplies `macro?` and `def_start`.
        if let Some(call) = node.as_call_node() {
            // The macro candidate is the call itself; a `&.`-receiver or an
            // explicit receiver disqualifies it (`!receiver`).
            if call.receiver().is_some() {
                return None;
            }
            if !self.is_macro_name(call.name().as_slice()) {
                return None;
            }
            if !self.begin_in_macro_scope() {
                return None;
            }
            let block = call.block().filter(|b| b.as_block_node().is_some());
            let (ns, ne) = loc(&node.location());
            if block.is_some() {
                // any_block_type: location = source_range.join(send.source_range)
                // = the whole call-with-block range (the send is contained, so the
                // join is the call's own range). def_start = the call's line.
                return Some(Candidate {
                    node_start: ns,
                    node_end: ne,
                    loc_start: ns,
                    loc_end: ne,
                    start_line: self.line_of(ns),
                    end_line: self.line_of(ne),
                    end_pos: ne,
                    kind: CandKind::Block,
                });
            }
            // send_type: location = source_range.
            return Some(Candidate {
                node_start: ns,
                node_end: ne,
                loc_start: ns,
                loc_end: ne,
                start_line: self.line_of(ns),
                end_line: self.line_of(ne),
                end_pos: ne,
                kind: CandKind::Send,
            });
        }
        None
    }

    fn check_statements(&mut self, children: &[Node<'_>]) {
        if children.len() < 2 {
            return;
        }
        // Resolve every child to a candidate-or-None once, then walk each_cons(2).
        let cands: Vec<Option<Candidate>> = children.iter().map(|c| self.candidate(c)).collect();
        for win in cands.windows(2) {
            let (Some(prev), Some(node)) = (&win[0], &win[1]) else {
                continue;
            };
            self.check_defs(prev, node);
        }
    }

    fn check_defs(&mut self, prev: &Candidate, node: &Candidate) {
        let count = self.blank_lines_count_between(prev, node);

        if self.line_count_allowed(count) {
            return;
        }
        if self.multiple_blank_lines_groups(prev, node) {
            return;
        }
        if self.config.allow_adjacent_one_line_defs
            && self.single_line(prev)
            && self.single_line(node)
        {
            return;
        }

        let message = self.message(node, count);
        let (insert, pos, n) = self.autocorrect(prev, node, count);
        self.offenses.push(EmptyLineBetweenDefsOffense {
            start_offset: node.loc_start,
            end_offset: node.loc_end,
            message,
            insert,
            pos,
            n,
        });
    }

    /// `single_line?`: the candidate node spans a single source line.
    fn single_line(&self, c: &Candidate) -> bool {
        self.line_of(c.node_start) == self.line_of(c.node_end)
    }

    fn line_count_allowed(&self, count: usize) -> bool {
        count >= self.config.minimum_empty_lines && count <= self.config.maximum_empty_lines
    }

    /// `blank_lines_count_between`: count blank lines in the window strictly
    /// between the two defs (1-based lines `def_end(prev)+1 ..= def_start(node)-1`).
    fn blank_lines_count_between(&self, prev: &Candidate, node: &Candidate) -> usize {
        self.lines_between(prev, node)
            .iter()
            .filter(|&&line| self.line_is_blank(line))
            .count()
    }

    /// The 1-based line numbers strictly between the two defs, mirroring
    /// `lines_between_defs`: `processed_source.lines[def_end(prev) ..= def_start(node)-2]`
    /// (0-based array indices) = 1-based lines `def_end+1 ..= def_start-1`.
    fn lines_between(&self, prev: &Candidate, node: &Candidate) -> Vec<usize> {
        let begin = prev.end_line; // 0-based array index = 1-based (begin+1)
        // end_line_num = def_start(node) - 2 (0-based array index). Negative => [].
        let def_start = node.start_line;
        if def_start < 2 {
            return Vec::new();
        }
        let end_idx = def_start - 2; // 0-based array index
        if end_idx < begin {
            return Vec::new();
        }
        // 0-based array index k corresponds to 1-based line k+1.
        (begin..=end_idx).map(|k| k + 1).collect()
    }

    /// Whether the 1-based `line` is blank (empty or whitespace only), matching
    /// `String#blank?` on `processed_source.lines[line - 1]` (the newline is
    /// already stripped from each element).
    fn line_is_blank(&self, line: usize) -> bool {
        // line content = source[line_start(line) .. line_end(line)] without `\n`.
        let Some((ls, le)) = self.line_byte_range(line) else {
            return true;
        };
        self.source[ls..le]
            .iter()
            .all(|&b| b == b' ' || b == b'\t' || b == b'\r' || b == 0x0b || b == 0x0c)
    }

    /// Byte range `[start, end)` of the content of 1-based `line` (excluding the
    /// trailing `\n`). `None` when the line is past the end of source (stock's
    /// `lines` would not include it, but a blank tail is harmless).
    fn line_byte_range(&self, line: usize) -> Option<(usize, usize)> {
        if line == 0 {
            return None;
        }
        // The line starts at the offset just past the (line-1)-th `\n`.
        let starts = self.line_index.line_starts();
        let idx = line - 1;
        if idx >= starts.len() {
            return None;
        }
        let start = starts[idx];
        let end = if idx + 1 < starts.len() {
            // next line start minus the `\n` (and a preceding `\r` if present).
            let mut e = starts[idx + 1] - 1;
            if e > start && self.source[e - 1] == b'\r' {
                e -= 1;
            }
            e
        } else {
            self.source.len()
        };
        Some((start, end))
    }

    /// `multiple_blank_lines_groups?`: with `lines` the window between defs,
    /// `blank_start = max{i | lines[i].blank?}`, `non_blank_end = min{i |
    /// !lines[i].blank?}`; true iff both exist and `blank_start > non_blank_end`.
    fn multiple_blank_lines_groups(&self, prev: &Candidate, node: &Candidate) -> bool {
        let lines = self.lines_between(prev, node);
        let mut blank_start: Option<usize> = None;
        let mut non_blank_end: Option<usize> = None;
        for (i, &line) in lines.iter().enumerate() {
            if self.line_is_blank(line) {
                blank_start = Some(i); // max blank index (last wins)
            } else if non_blank_end.is_none() {
                non_blank_end = Some(i); // min non-blank index (first wins)
            }
        }
        match (blank_start, non_blank_end) {
            (Some(b), Some(n)) => b > n,
            _ => false,
        }
    }

    /// Format the stock `MSG` for the second member.
    fn message(&self, node: &Candidate, count: usize) -> String {
        let type_str = match node.kind {
            CandKind::Method => "method",
            CandKind::Class => "class",
            CandKind::Module => "module",
            CandKind::Block => "block",
            CandKind::Send => "send",
        };
        format!(
            "Expected {} between {} definitions; found {}.",
            self.expected_lines(),
            type_str,
            count
        )
    }

    fn expected_lines(&self) -> String {
        if self.config.minimum_empty_lines != self.config.maximum_empty_lines {
            format!(
                "{}..{} empty lines",
                self.config.minimum_empty_lines, self.config.maximum_empty_lines
            )
        } else {
            let lines = if self.config.maximum_empty_lines == 1 {
                "line"
            } else {
                "lines"
            };
            format!("{} empty {}", self.config.maximum_empty_lines, lines)
        }
    }

    /// `autocorrect`: returns `(insert, pos, n)`. `pos` is `newline_pos`; `n` is
    /// the `difference`. `insert` true => insert `"\n" * n` after
    /// `range_between(pos, pos + 1)`; false => remove `range_between(pos, pos + n)`.
    fn autocorrect(&self, prev: &Candidate, node: &Candidate, count: usize) -> (bool, usize, usize) {
        // newline_pos = source.index("\n", end_loc(prev).end_pos).
        let end_pos = prev.end_pos;
        let mut newline_pos = self.source[end_pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|i| end_pos + i)
            // No trailing newline at all: stock would get nil and raise; in
            // practice every file the corrector touches has one. Fall back to
            // end of source to stay in range.
            .unwrap_or(self.source.len());

        // Same-line one-liners: newline_pos = begin_pos - 1 if newline_pos > begin_pos.
        let begin_pos = node.node_start;
        if newline_pos > begin_pos {
            newline_pos = begin_pos - 1;
        }

        if count > self.config.maximum_empty_lines {
            let difference = count - self.config.maximum_empty_lines;
            (false, newline_pos, difference)
        } else {
            let difference = self.config.minimum_empty_lines - count;
            (true, newline_pos, difference)
        }
    }
}

impl<'a> Visitor<'a> {
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

    fn in_condition_position(&self, node: &Node<'_>) -> bool {
        let Some(&Some((s, e))) = self.condition_ranges.last() else {
            return false;
        };
        node.location().start_offset() == s && node.location().end_offset() == e
    }
}

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
    let Some(cr) = recv.as_constant_read_node() else {
        return false;
    };
    matches!(
        (cr.name().as_slice(), call.name().as_slice()),
        (b"Class" | b"Module" | b"Struct", b"new") | (b"Data", b"define")
    )
}

impl<'a> Visitor<'a> {
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

    fn cfg() -> Config {
        Config {
            method_defs: true,
            class_defs: true,
            module_defs: true,
            allow_adjacent_one_line_defs: false,
            minimum_empty_lines: 1,
            maximum_empty_lines: 1,
            def_like_macros: Vec::new(),
        }
    }

    fn run(source: &str) -> Vec<EmptyLineBetweenDefsOffense> {
        check_empty_line_between_defs(source.as_bytes(), cfg())
    }

    #[test]
    fn adjacent_method_defs() {
        let got = run("def a\nend\ndef b\nend\n");
        assert_eq!(got.len(), 1);
        assert!(got[0].insert);
        assert!(got[0].message.contains("method definitions; found 0"));
    }

    #[test]
    fn separated_defs_no_offense() {
        assert!(run("def a\nend\n\ndef b\nend\n").is_empty());
    }

    #[test]
    fn single_def_no_offense() {
        assert!(run("x = 0\ndef m\nend\n").is_empty());
    }

    #[test]
    fn nested_single_def_no_offense() {
        assert!(run("def f\n  Class.new do\n    def g\n    end\n  end\nend\n").is_empty());
    }

    #[test]
    fn too_many_empty_lines_removes() {
        let got = run("def a; end\n\n\n\ndef b; end\n");
        assert_eq!(got.len(), 1);
        assert!(!got[0].insert);
        assert!(got[0].message.contains("found 3"));
    }

    #[test]
    fn class_defs() {
        let got = run("class Foo\nend\nclass Baz\nend\n");
        assert_eq!(got.len(), 1);
        assert!(got[0].message.contains("class definitions"));
    }

    #[test]
    fn whitespace_line_is_blank() {
        assert!(run("class J\n  def n\n  end\n  \n  def o\n  end\nend\n").is_empty());
    }

    #[test]
    fn comment_between_still_offends() {
        let got = run("def n\nend\n# c\ndef o\nend\n");
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn multiple_blank_line_groups_suppressed() {
        // comment then two blanks: blank group after the non-blank => suppressed.
        assert!(run("def a\nend\n# c\n\n\ndef b\nend\n").is_empty());
    }

    #[test]
    fn allow_adjacent_one_line_defs() {
        let mut c = cfg();
        c.allow_adjacent_one_line_defs = true;
        let got = check_empty_line_between_defs(b"def a; end\ndef b; end\n", c);
        assert!(got.is_empty());
    }
}
