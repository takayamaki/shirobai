//! `Layout/MultilineOperationIndentation`.
//!
//! Checks the indentation of the right-hand operand of binary operations
//! (`+`, `<<`, `&&`, `||`, ...) that span more than one line. Ports the shared
//! `MultilineExpressionIndentation` mixin logic (the subset the operation cop
//! exercises) to Rust: ancestor-stack bookkeeping replaces parser-gem's
//! `each_ancestor`, since Prism nodes carry no parent pointer.

use ruby_prism::{CallNode, Location, Node, Visit};

/// One misindented operand. `column_delta` is `correct_column - actual_column`
/// (positive => the operand must move right). `message` is the fully formatted
/// RuboCop offense message.
pub struct OperationIndentOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub message: String,
}

/// Enforced indentation style.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Aligned,
    Indented,
}

impl Style {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Style::Indented,
            _ => Style::Aligned,
        }
    }
}

pub fn check_multiline_operation_indentation(
    source: &[u8],
    style: u8,
    indent_width: usize,
    base_indent_width: usize,
) -> Vec<OperationIndentOffense> {
    super::parse_cache::with_parsed(source, |source, node| {
        let mut visitor = Visitor {
            source,
            style: Style::from_u8(style),
            indent: indent_width,
            base: base_indent_width,
            stack: Vec::new(),
            offenses: Vec::new(),
        };
        visitor.visit(node);
        visitor.offenses
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KwKind {
    If,
    Unless,
    While,
    Until,
    For,
    Return,
}

impl KwKind {
    fn keyword(self) -> &'static str {
        match self {
            KwKind::If => "if",
            KwKind::Unless => "unless",
            KwKind::While => "while",
            KwKind::Until => "until",
            KwKind::For => "for",
            KwKind::Return => "return",
        }
    }
}

#[derive(Clone, Copy)]
struct KwInfo {
    kind: KwKind,
    is_modifier: bool,
}

/// The parser-gem-equivalent ancestor of a node, holding only what the mixin's
/// ancestor predicates need. Nodes we never inspect (`StatementsNode`,
/// `ArgumentsNode`, ...) become `Other` and are transparently skipped.
enum FrameKind {
    /// `if` / `unless` / `while` / `until` / `for` / `return`. Also one of the
    /// `UNALIGNED_RHS_TYPES`, so it disqualifies an assignment-rhs search.
    Keyword {
        kind: KwKind,
        /// `indented_keyword_expression`: the condition / collection / return
        /// value. `None` for a bare `return`. Absent for a ternary `if`, which
        /// is skipped by `kw_node_with_special_indentation`.
        expr: Option<(usize, usize)>,
        is_modifier: bool,
        is_ternary: bool,
    },
    /// An `lvasgn` / `op_asgn` / ... node. `rhs` is `extract_rhs` (its value).
    Assignment {
        rhs: (usize, usize),
    },
    Send {
        setter: bool,
        /// `(` .. `)` span when the call is parenthesized (`loc.end == ')'`).
        paren: Option<(usize, usize)>,
        args: Vec<(usize, usize)>,
        def_modifier: bool,
    },
    Block {
        body: Option<(usize, usize)>,
    },
    /// A parenthesized grouping `( ... )` (`grouped_expression?`).
    Paren,
    /// `array` / `kwbegin`: an `UNALIGNED_RHS_TYPE` that is not a keyword.
    Unaligned,
    Other,
}

struct Visitor<'a> {
    source: &'a [u8],
    style: Style,
    indent: usize,
    base: usize,
    stack: Vec<FrameKind>,
    offenses: Vec<OperationIndentOffense>,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

fn within((is, ie): (usize, usize), (os, oe): (usize, usize)) -> bool {
    is >= os && ie <= oe
}

impl<'a> Visitor<'a> {
    /// Start offset of the line containing `off`.
    fn line_start(&self, off: usize) -> usize {
        match self.source[..off].iter().rposition(|&b| b == b'\n') {
            Some(i) => i + 1,
            None => 0,
        }
    }

    /// Column (codepoint count from line start) of `off`.
    fn col(&self, off: usize) -> usize {
        let ls = self.line_start(off);
        std::str::from_utf8(&self.source[ls..off])
            .map(|s| s.chars().count())
            .unwrap_or(off - ls)
    }

    /// `indentation(node)`: column of the first non-whitespace char of the line
    /// containing `off` (i.e. the leading-whitespace width).
    fn indent_col(&self, off: usize) -> usize {
        let ls = self.line_start(off);
        self.source[ls..]
            .iter()
            .take_while(|&&b| b == b' ' || b == b'\t')
            .count()
    }

    /// `begins_its_line?`: everything before `off` on its line is whitespace.
    fn begins_its_line(&self, off: usize) -> bool {
        let ls = self.line_start(off);
        self.source[ls..off]
            .iter()
            .all(|&b| b == b' ' || b == b'\t')
    }

    /// `not_for_this_cop?`: the operation sits inside a grouped expression or
    /// inside the parenthesized argument list of a method call.
    fn not_for_this_cop(&self, op: (usize, usize)) -> bool {
        self.stack.iter().rev().any(|f| match f {
            FrameKind::Paren => true,
            FrameKind::Send {
                paren: Some((pb, pe)),
                ..
            } => op.0 > *pb && op.1 < *pe,
            _ => false,
        })
    }

    /// `kw_node_with_special_indentation`: nearest non-ternary keyword ancestor
    /// whose indented expression contains the operation.
    fn kw_special(&self, op: (usize, usize)) -> Option<KwInfo> {
        for f in self.stack.iter().rev() {
            if let FrameKind::Keyword {
                kind,
                expr,
                is_modifier,
                is_ternary,
            } = *f
            {
                if is_ternary {
                    continue;
                }
                if let Some(e) = expr
                    && within(op, e)
                {
                    return Some(KwInfo { kind, is_modifier });
                }
            }
        }
        None
    }

    /// `part_of_assignment_rhs`: walking up from the operation, the nearest
    /// assignment/setter whose rhs contains `candidate`, or `None` if an
    /// `UNALIGNED_RHS_TYPE` (or enclosing block body) is hit first. Returns the
    /// `extract_rhs` range of that assignment.
    fn part_of_assignment_rhs(&self, candidate: (usize, usize)) -> Option<(usize, usize)> {
        for f in self.stack.iter().rev() {
            match f {
                FrameKind::Keyword { .. } | FrameKind::Unaligned => return None,
                FrameKind::Block { body } => {
                    if body.is_some_and(|b| within(candidate, b)) {
                        return None;
                    }
                }
                FrameKind::Assignment { rhs } => {
                    if within(candidate, *rhs) {
                        return Some(*rhs);
                    }
                }
                FrameKind::Send { setter, args, .. } => {
                    if *setter
                        && let Some(last) = args.last()
                        && within(candidate, *last)
                    {
                        return Some(*last);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// `argument_in_method_call(node, :with_or_without_parentheses)`: returns
    /// `Some(def_modifier?)` of the enclosing call whose argument list contains
    /// the operation, or `None` (also `None` if a block is reached first).
    fn argument_in_method_call(&self, op: (usize, usize)) -> Option<bool> {
        for f in self.stack.iter().rev() {
            match f {
                FrameKind::Block { .. } => return None,
                FrameKind::Send {
                    setter,
                    args,
                    def_modifier,
                    ..
                } => {
                    if *setter {
                        continue;
                    }
                    if args.iter().any(|a| within(op, *a)) {
                        return Some(*def_modifier);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn should_align(
        &self,
        op: (usize, usize),
        kw: Option<KwInfo>,
        assign_rhs: Option<(usize, usize)>,
    ) -> bool {
        if let Some(arhs) = assign_rhs
            && self.begins_its_line(arhs.0)
        {
            return true;
        }
        if self.style != Style::Aligned {
            return false;
        }
        if kw.is_some() || assign_rhs.is_some() {
            return true;
        }
        match self.argument_in_method_call(op) {
            Some(def_modifier) => !def_modifier,
            None => false,
        }
    }

    fn operation_description(
        &self,
        kw: Option<KwInfo>,
        assign_rhs: Option<(usize, usize)>,
    ) -> String {
        if let Some(k) = kw {
            let keyword = k.kind.keyword();
            let kind = if k.kind == KwKind::For {
                "collection"
            } else {
                "condition"
            };
            let article = if keyword.starts_with('i') || keyword.starts_with('u') {
                "an"
            } else {
                "a"
            };
            format!("a {kind} in {article} `{keyword}` statement")
        } else if assign_rhs.is_some() {
            "an expression in an assignment".to_string()
        } else {
            "an expression".to_string()
        }
    }

    fn correct_indentation(&self, kw: Option<KwInfo>) -> usize {
        let special = kw.is_some_and(|k| !k.is_modifier);
        if special {
            self.indent + self.base
        } else {
            self.indent
        }
    }

    /// The shared `offending_range` + `check` for both operands (`on_and`/`on_or`
    /// and the binary-operator `on_send`).
    fn handle(&mut self, op: (usize, usize), lhs_start: usize, rhs: (usize, usize)) {
        if !self.begins_its_line(rhs.0) {
            return;
        }
        if self.not_for_this_cop(op) {
            return;
        }

        let kw = self.kw_special(op);
        let assign_rhs = self.part_of_assignment_rhs(rhs);
        let align = self.should_align(op, kw, assign_rhs);

        let correct_column = if align {
            self.col(op.0) as isize
        } else {
            (self.indent_col(lhs_start) + self.correct_indentation(kw)) as isize
        };
        let column_delta = correct_column - self.col(rhs.0) as isize;
        if column_delta == 0 {
            return;
        }

        let what = self.operation_description(kw, assign_rhs);
        let message = if align {
            format!("Align the operands of {what} spanning multiple lines.")
        } else {
            let used = self.col(rhs.0) as isize - self.indent_col(lhs_start) as isize;
            let expected = self.correct_indentation(kw);
            format!(
                "Use {expected} (not {used}) spaces for indenting {what} spanning multiple lines."
            )
        };

        self.offenses.push(OperationIndentOffense {
            start_offset: rhs.0,
            end_offset: rhs.1,
            column_delta,
            message,
        });
    }

    /// The operation cop's `on_send`: binary operator calls (`a + b`), excluding
    /// `.`-calls, `[]`, and unary operators.
    fn process_send(&mut self, call: &CallNode<'_>) {
        let Some(receiver) = call.receiver() else {
            return;
        };
        if call.call_operator_loc().is_some() {
            return; // has a dot: handled by the method-call cop.
        }
        if call.name().as_slice() == b"[]" {
            return;
        }
        let Some(args) = call.arguments() else {
            return;
        };
        let Some(first_arg) = args.arguments().iter().next() else {
            return; // unary operator: no right-hand side.
        };
        let op = loc(&call.as_node().location());
        let lhs_start = receiver.location().start_offset();
        self.handle(op, lhs_start, loc(&first_arg.location()));
    }

    fn frame_for(&self, node: &Node<'_>) -> FrameKind {
        if let Some(n) = node.as_if_node() {
            let is_ternary = n.if_keyword_loc().is_none();
            let is_modifier = !is_ternary && n.end_keyword_loc().is_none();
            return FrameKind::Keyword {
                kind: KwKind::If,
                expr: Some(loc(&n.predicate().location())),
                is_modifier,
                is_ternary,
            };
        }
        if let Some(n) = node.as_unless_node() {
            return FrameKind::Keyword {
                kind: KwKind::Unless,
                expr: Some(loc(&n.predicate().location())),
                is_modifier: n.end_keyword_loc().is_none(),
                is_ternary: false,
            };
        }
        if let Some(n) = node.as_while_node() {
            return FrameKind::Keyword {
                kind: KwKind::While,
                expr: Some(loc(&n.predicate().location())),
                is_modifier: false,
                is_ternary: false,
            };
        }
        if let Some(n) = node.as_until_node() {
            return FrameKind::Keyword {
                kind: KwKind::Until,
                expr: Some(loc(&n.predicate().location())),
                is_modifier: false,
                is_ternary: false,
            };
        }
        if let Some(n) = node.as_for_node() {
            return FrameKind::Keyword {
                kind: KwKind::For,
                expr: Some(loc(&n.collection().location())),
                is_modifier: false,
                is_ternary: false,
            };
        }
        if let Some(n) = node.as_return_node() {
            let expr = n
                .arguments()
                .and_then(|a| a.arguments().iter().next().map(|f| loc(&f.location())));
            return FrameKind::Keyword {
                kind: KwKind::Return,
                expr,
                is_modifier: false,
                is_ternary: false,
            };
        }
        if let Some(n) = node.as_parentheses_node() {
            let _ = n;
            return FrameKind::Paren;
        }
        if node.as_array_node().is_some() {
            return FrameKind::Unaligned;
        }
        if let Some(n) = node.as_begin_node() {
            if n.begin_keyword_loc().is_some() {
                return FrameKind::Unaligned; // kwbegin
            }
            return FrameKind::Other;
        }
        if let Some(n) = node.as_block_node() {
            return FrameKind::Block {
                body: n.body().map(|b| loc(&b.location())),
            };
        }
        if let Some(rhs) = assignment_value(node) {
            return FrameKind::Assignment { rhs };
        }
        if let Some(c) = node.as_call_node() {
            let paren = match c.closing_loc() {
                Some(close) if close.as_slice() == b")" => c
                    .opening_loc()
                    .map(|o| (o.start_offset(), close.end_offset())),
                _ => None,
            };
            let args = c
                .arguments()
                .map(|a| {
                    a.arguments()
                        .iter()
                        .map(|arg| loc(&arg.location()))
                        .collect()
                })
                .unwrap_or_default();
            return FrameKind::Send {
                setter: c.is_attribute_write(),
                paren,
                args,
                def_modifier: is_def_modifier(node),
            };
        }
        FrameKind::Other
    }
}

/// `extract_rhs` for assignment nodes (`assignment?` types): the `value` of any
/// equals/shorthand write node.
fn assignment_value(node: &Node<'_>) -> Option<(usize, usize)> {
    macro_rules! try_value {
        ($($m:ident),* $(,)?) => {
            $(if let Some(n) = node.$m() { return Some(loc(&n.value().location())); })*
        };
    }
    try_value!(
        as_local_variable_write_node,
        as_instance_variable_write_node,
        as_class_variable_write_node,
        as_global_variable_write_node,
        as_constant_write_node,
        as_constant_path_write_node,
        as_multi_write_node,
        as_local_variable_operator_write_node,
        as_local_variable_or_write_node,
        as_local_variable_and_write_node,
        as_instance_variable_operator_write_node,
        as_instance_variable_or_write_node,
        as_instance_variable_and_write_node,
        as_class_variable_operator_write_node,
        as_class_variable_or_write_node,
        as_class_variable_and_write_node,
        as_global_variable_operator_write_node,
        as_global_variable_or_write_node,
        as_global_variable_and_write_node,
        as_constant_operator_write_node,
        as_constant_or_write_node,
        as_constant_and_write_node,
        as_constant_path_operator_write_node,
        as_constant_path_or_write_node,
        as_constant_path_and_write_node,
        as_index_operator_write_node,
        as_index_or_write_node,
        as_index_and_write_node,
        as_call_operator_write_node,
        as_call_or_write_node,
        as_call_and_write_node,
    );
    None
}

/// `def_modifier?`: a receiver-less command whose argument is a `def`/`defs`, or
/// recursively another such command (`private def foo`).
fn is_def_modifier(node: &Node<'_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    if call.receiver().is_some() {
        return false;
    }
    let Some(args) = call.arguments() else {
        return false;
    };
    let Some(first) = args.arguments().iter().next() else {
        return false;
    };
    if first.as_def_node().is_some() {
        return true;
    }
    is_def_modifier(&first)
}

impl<'a> Visit<'a> for Visitor<'a> {
    fn visit_branch_node_enter(&mut self, node: Node<'a>) {
        if let Some(n) = node.as_and_node() {
            let op = loc(&node.location());
            self.handle(
                op,
                n.left().location().start_offset(),
                loc(&n.right().location()),
            );
        } else if let Some(n) = node.as_or_node() {
            let op = loc(&node.location());
            self.handle(
                op,
                n.left().location().start_offset(),
                loc(&n.right().location()),
            );
        } else if let Some(c) = node.as_call_node() {
            self.process_send(&c);
        }

        let kind = self.frame_for(&node);
        self.stack.push(kind);
    }

    fn visit_branch_node_leave(&mut self) {
        self.stack.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: Style) -> Vec<(usize, usize, isize, String)> {
        let s = match style {
            Style::Aligned => 0,
            Style::Indented => 1,
        };
        check_multiline_operation_indentation(source.as_bytes(), s, 2, 2)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.message))
            .collect()
    }

    #[test]
    fn no_indentation_of_second_line() {
        let got = run("a +\nb\n", Style::Aligned);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, 2);
        assert_eq!(
            got[0].3,
            "Use 2 (not 0) spaces for indenting an expression spanning multiple lines."
        );
    }

    #[test]
    fn accepts_indented_operand() {
        assert!(run("a +\n  b\n", Style::Aligned).is_empty());
    }

    #[test]
    fn aligned_if_condition() {
        let got = run("if a +\n    b\n  something\nend\n", Style::Aligned);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].3,
            "Align the operands of a condition in an `if` statement spanning multiple lines."
        );
    }

    #[test]
    fn accepts_grouped_expression() {
        assert!(run("(a +\n b)\n", Style::Aligned).is_empty());
    }

    #[test]
    fn does_not_check_dot_calls() {
        assert!(run("Foo\n.a\n  .b\n", Style::Aligned).is_empty());
    }

    #[test]
    fn assignment_alignment() {
        let got = run("a = b +\n  c\n", Style::Aligned);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].3,
            "Align the operands of an expression in an assignment spanning multiple lines."
        );
    }
}
