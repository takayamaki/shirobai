//! `Lint/SafeNavigationChain`.

use ruby_prism::{CallNode, Node};

/// An ordinary method call chained after a safe-navigation call.
#[derive(Debug)]
pub struct SafeNavChainOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    /// Replacement text for `[start_offset, end_offset)`. Empty means the
    /// offense has no safe autocorrection.
    pub replacement: String,
    /// Optional range to wrap in parentheses (`wrap_end > wrap_start`).
    pub wrap_start: usize,
    pub wrap_end: usize,
}

/// `MethodIdentifierPredicates::OPERATOR_METHODS`.
const OPERATOR_METHODS: &[&[u8]] = &[
    b"|", b"^", b"&", b"<=>", b"==", b"===", b"=~", b">", b">=", b"<", b"<=", b"<<", b">>",
    b"+", b"-", b"*", b"/", b"%", b"**", b"~", b"+@", b"-@", b"!@", b"~@", b"[]", b"[]=", b"!",
    b"!=", b"!~", b"`",
];

pub fn check_safe_navigation_chain(
    source: &[u8],
    nil_methods: &[String],
) -> Vec<SafeNavChainOffense> {
    let mut rule = build_rule(source, nil_methods);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule<'a>(source: &'a [u8], nil_methods: &'a [String]) -> Visitor<'a> {
    Visitor {
        source,
        nil_methods,
        stack: Vec::new(),
        offenses: Vec::new(),
    }
}

enum TernaryKind {
    Normal,
    IfBranch,
    ElseBranch,
}

type Range = (usize, usize);

fn range_of(node: &Node<'_>) -> Range {
    let l = node.location();
    (l.start_offset(), l.end_offset())
}

fn contains(outer: Option<Range>, inner: Range) -> bool {
    outer.is_some_and(|(s, e)| s <= inner.0 && inner.1 <= e)
}

/// The parser-gem-equivalent parent of a node, with the data the cop's parent
/// predicates need. Structural Prism wrappers (`ArgumentsNode`, a
/// one-statement `StatementsNode`, `ElseNode`, `ProgramNode`) are
/// `Transparent` and skipped.
enum Frame {
    Transparent,
    /// parser `begin` / `kwbegin` / `rescue` / `ensure`: parentheses,
    /// interpolation, a body of two or more statements, `begin ... end`.
    Begin,
    /// `block` / `numblock` / `itblock` / `when` / `in_pattern` / `class` /
    /// `module` / `sclass`: only the body is a statement.
    Body {
        body: Option<Range>,
    },
    /// `in_pattern`: like `Body`, and its pattern position tells an `if`
    /// guard apart from an `if` in the body.
    In {
        body: Option<Range>,
        pattern: Range,
    },
    While {
        predicate: Range,
    },
    If {
        ternary: bool,
        predicate: Range,
        then_start: Option<usize>,
        else_start: Option<usize>,
    },
    Case {
        predicate: Option<Range>,
    },
    Def {
        endless: bool,
    },
    Resbody {
        body: Option<Range>,
    },
    /// A rescue modifier: its expression sits in the parser `rescue` node,
    /// its rescue value in a modifier `resbody`.
    RescueMod {
        expression: Range,
    },
    /// parser `for` is not a body parent; only a body of two or more
    /// statements (a parser `begin`) is a statement.
    For {
        multi_body: Option<Range>,
    },
    And {
        semantic: bool,
        left_receiver: Option<Range>,
        right_start: usize,
    },
    Or {
        semantic: bool,
    },
    /// `argument_taking?`: a non-operator, non-setter send, or
    /// `yield` / `super` / `return` / `break` / `next`.
    Args {
        only_arg: Option<Range>,
        paren_multiline: bool,
    },
    /// An assignment (including a setter send): only its value counts.
    Assign {
        own: Range,
        value: Option<Range>,
    },
    Opaque,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    nil_methods: &'a [String],
    stack: Vec<Frame>,
    pub(crate) offenses: Vec<SafeNavChainOffense>,
}

impl<'a> Visitor<'a> {
    fn text(&self, start: usize, end: usize) -> &'a str {
        std::str::from_utf8(&self.source[start..end]).unwrap_or("")
    }

    fn node_source(&self, node: &Node<'_>) -> &'a str {
        let loc = node.location();
        self.text(loc.start_offset(), loc.end_offset())
    }

    fn is_csend(node: &Node<'_>) -> bool {
        node.as_call_node().is_some_and(|c| c.is_safe_navigation())
    }

    fn is_csend_or_parenthesized_csend(node: &Node<'_>) -> bool {
        if Self::is_csend(node) {
            return true;
        }
        if let Some(paren) = node.as_parentheses_node()
            && let Some(body) = paren.body()
            && let Some(stmts) = body.as_statements_node()
        {
            let children: Vec<_> = stmts.body().iter().collect();
            if children.len() == 1 {
                return Self::is_csend(&children[0]);
            }
        }
        false
    }

    /// Stack indices of the parser-gem ancestors, nearest first.
    fn effective(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.stack.len())
            .rev()
            .filter(|&i| !matches!(self.stack[i], Frame::Transparent))
    }

    /// The parser-gem parent: the nearest stacked frame that is not a structural
    /// Prism wrapper.
    fn effective_parent(&self) -> Option<&Frame> {
        self.effective().next().map(|i| &self.stack[i])
    }

    fn has_newline(&self, start: usize, end: usize) -> bool {
        self.source[start..end].contains(&b'\n')
    }

    /// The `Args` frame of a call-like node: the sole parser argument (a
    /// `&blk` counts as one) and whether it is a multiline parenthesized call.
    fn args_frame(
        &self,
        start: usize,
        args: Option<ruby_prism::ArgumentsNode<'_>>,
        block_arg: bool,
        rparen_end: Option<usize>,
    ) -> Frame {
        let list: Vec<Node<'_>> = args.map(|a| a.arguments().iter().collect()).unwrap_or_default();
        let only_arg = (list.len() == 1 && !block_arg).then(|| range_of(&list[0]));
        Frame::Args {
            only_arg,
            paren_multiline: rparen_end.is_some_and(|e| self.has_newline(start, e)),
        }
    }

    /// Frame describing `node` for its children.
    fn frame_for(&self, node: &Node<'_>) -> Frame {
        match node {
            Node::ArgumentsNode { .. } | Node::ElseNode { .. } | Node::ProgramNode { .. } => {
                Frame::Transparent
            }
            Node::StatementsNode { .. } => {
                if node.as_statements_node().unwrap().body().len() >= 2 {
                    Frame::Begin
                } else {
                    Frame::Transparent
                }
            }
            Node::ParenthesesNode { .. }
            | Node::EmbeddedStatementsNode { .. }
            | Node::BeginNode { .. }
            | Node::EnsureNode { .. } => Frame::Begin,
            Node::AndNode { .. } => {
                let a = node.as_and_node().unwrap();
                let op = a.operator_loc();
                let left_receiver = a
                    .left()
                    .as_call_node()
                    .and_then(|c| c.receiver())
                    .map(|r| range_of(&r));
                Frame::And {
                    semantic: self.text(op.start_offset(), op.end_offset()) == "and",
                    left_receiver,
                    right_start: a.right().location().start_offset(),
                }
            }
            Node::OrNode { .. } => {
                let op = node.as_or_node().unwrap().operator_loc();
                Frame::Or {
                    semantic: self.text(op.start_offset(), op.end_offset()) == "or",
                }
            }
            Node::IfNode { .. } | Node::UnlessNode { .. } => {
                // `in pattern if guard`: prism nests the pattern in an
                // `IfNode`/`UnlessNode`; parser has an `if_guard` instead.
                if let Some(Frame::In { pattern, .. }) = self.stack.last()
                    && *pattern == range_of(node)
                {
                    return Frame::Opaque;
                }
                if let Some(if_node) = node.as_if_node() {
                    let ternary = if_node.if_keyword_loc().is_none();
                    Frame::If {
                        ternary,
                        predicate: range_of(&if_node.predicate()),
                        then_start: if_node.statements().map(|s| s.location().start_offset()),
                        else_start: if_node
                            .subsequent()
                            .and_then(|e| e.as_else_node())
                            .and_then(|e| e.statements())
                            .map(|s| s.location().start_offset()),
                    }
                } else {
                    let unless_node = node.as_unless_node().unwrap();
                    Frame::If {
                        ternary: false,
                        predicate: range_of(&unless_node.predicate()),
                        then_start: None,
                        else_start: None,
                    }
                }
            }
            Node::WhileNode { .. } => Frame::While {
                predicate: range_of(&node.as_while_node().unwrap().predicate()),
            },
            Node::UntilNode { .. } => Frame::While {
                predicate: range_of(&node.as_until_node().unwrap().predicate()),
            },
            Node::CaseNode { .. } => Frame::Case {
                predicate: node.as_case_node().unwrap().predicate().map(|p| range_of(&p)),
            },
            Node::CaseMatchNode { .. } => Frame::Case {
                predicate: node
                    .as_case_match_node()
                    .unwrap()
                    .predicate()
                    .map(|p| range_of(&p)),
            },
            Node::WhenNode { .. } => Frame::Body {
                body: node
                    .as_when_node()
                    .unwrap()
                    .statements()
                    .map(|s| range_of(&s.as_node())),
            },
            Node::InNode { .. } => {
                let in_node = node.as_in_node().unwrap();
                Frame::In {
                    body: in_node.statements().map(|s| range_of(&s.as_node())),
                    pattern: range_of(&in_node.pattern()),
                }
            }
            Node::BlockNode { .. } => Frame::Body {
                body: node.as_block_node().unwrap().body().map(|b| range_of(&b)),
            },
            Node::LambdaNode { .. } => Frame::Body {
                body: node.as_lambda_node().unwrap().body().map(|b| range_of(&b)),
            },
            Node::ClassNode { .. } => Frame::Body {
                body: node.as_class_node().unwrap().body().map(|b| range_of(&b)),
            },
            Node::ModuleNode { .. } => Frame::Body {
                body: node.as_module_node().unwrap().body().map(|b| range_of(&b)),
            },
            Node::SingletonClassNode { .. } => Frame::Body {
                body: node
                    .as_singleton_class_node()
                    .unwrap()
                    .body()
                    .map(|b| range_of(&b)),
            },
            Node::DefNode { .. } => Frame::Def {
                endless: node.as_def_node().unwrap().equal_loc().is_some(),
            },
            Node::RescueModifierNode { .. } => Frame::RescueMod {
                expression: range_of(&node.as_rescue_modifier_node().unwrap().expression()),
            },
            Node::ForNode { .. } => Frame::For {
                multi_body: node
                    .as_for_node()
                    .unwrap()
                    .statements()
                    .filter(|s| s.body().len() >= 2)
                    .map(|s| range_of(&s.as_node())),
            },
            Node::CallNode { .. } => {
                let c = node.as_call_node().unwrap();
                let name = c.name().as_slice();
                if c.equal_loc().is_some() {
                    let value = c
                        .arguments()
                        .and_then(|a| a.arguments().iter().last())
                        .map(|v| range_of(&v));
                    return Frame::Assign {
                        own: range_of(node),
                        value,
                    };
                }
                if OPERATOR_METHODS.contains(&name) {
                    return Frame::Opaque;
                }
                let block_arg = c.block().is_some_and(|b| b.as_block_argument_node().is_some());
                let rparen = c
                    .closing_loc()
                    .filter(|l| self.text(l.start_offset(), l.end_offset()) == ")")
                    .map(|l| l.end_offset());
                self.args_frame(node.location().start_offset(), c.arguments(), block_arg, rparen)
            }
            Node::SuperNode { .. } => {
                let s = node.as_super_node().unwrap();
                let block_arg = s.block().is_some_and(|b| b.as_block_argument_node().is_some());
                let rparen = s.rparen_loc().map(|l| l.end_offset());
                self.args_frame(node.location().start_offset(), s.arguments(), block_arg, rparen)
            }
            Node::YieldNode { .. } => {
                let y = node.as_yield_node().unwrap();
                let rparen = y.rparen_loc().map(|l| l.end_offset());
                self.args_frame(node.location().start_offset(), y.arguments(), false, rparen)
            }
            Node::ReturnNode { .. } => {
                let args = node.as_return_node().unwrap().arguments();
                self.args_frame(0, args, false, None)
            }
            Node::BreakNode { .. } => {
                let args = node.as_break_node().unwrap().arguments();
                self.args_frame(0, args, false, None)
            }
            Node::NextNode { .. } => {
                let args = node.as_next_node().unwrap().arguments();
                self.args_frame(0, args, false, None)
            }
            _ => match assignment_value(node) {
                Some(value) => Frame::Assign {
                    own: range_of(node),
                    value: Some(range_of(&value)),
                },
                None => Frame::Opaque,
            },
        }
    }

    /// `require_safe_navigation?`: suppress the `x&.foo && x.foo` idiom.
    fn require_safe_navigation(&self, call: &CallNode<'_>) -> bool {
        let Some(Frame::And {
            left_receiver,
            right_start,
            ..
        }) = self.effective_parent()
        else {
            return true;
        };
        if *right_start != call.location().start_offset() {
            return true;
        }
        let node_receiver = call.receiver().map(|r| self.node_source(&r));
        let left = left_receiver.map(|(s, e)| self.text(s, e));
        left != node_receiver
    }

    /// Classifies a node that sits in a ternary whose condition is the safe-nav
    /// receiver: the `if` branch is not an offense, the `else` branch is an
    /// offense without a safe correction.
    fn ternary(&self, call: &CallNode<'_>, safe_nav: &Node<'_>) -> TernaryKind {
        let Some(Frame::If {
            ternary: true,
            predicate,
            then_start,
            else_start,
        }) = self.effective_parent()
        else {
            return TernaryKind::Normal;
        };
        if self.text(predicate.0, predicate.1) != self.node_source(safe_nav) {
            return TernaryKind::Normal;
        }
        let start = call.location().start_offset();
        if *then_start == Some(start) {
            TernaryKind::IfBranch
        } else if *else_start == Some(start) {
            TernaryKind::ElseBranch
        } else {
            TernaryKind::Normal
        }
    }

    /// 1.92.3 `command_call_allowed?` (rubocop#16406).
    fn command_call_allowed(&self, node: Range) -> bool {
        let Some(idx) = self.effective().next() else {
            return true;
        };
        if let Frame::Args {
            only_arg,
            paren_multiline,
        } = &self.stack[idx]
        {
            return *only_arg == Some(node) && !*paren_multiline && !self.comma_follows(node.1);
        }
        let condition = match &self.stack[idx] {
            Frame::And { semantic, .. } | Frame::Or { semantic } => *semantic,
            Frame::If {
                ternary, predicate, ..
            } => !*ternary && *predicate == node,
            Frame::While { predicate } => *predicate == node,
            Frame::Case { predicate } => *predicate == Some(node),
            _ => false,
        };
        condition || self.statement(node, idx)
    }

    /// `/\G\s*,/` at `pos`.
    fn comma_follows(&self, pos: usize) -> bool {
        self.source[pos..]
            .iter()
            .find(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r'))
            == Some(&b',')
    }

    /// `statement?(node, parent)` with `parent` at stack index `idx`.
    fn statement(&self, node: Range, idx: usize) -> bool {
        match &self.stack[idx] {
            Frame::Begin => true,
            Frame::Body { body } | Frame::In { body, .. } | Frame::Resbody { body } => {
                contains(*body, node)
            }
            Frame::While { predicate } => *predicate != node,
            Frame::If {
                ternary, predicate, ..
            } => !*ternary && *predicate != node,
            Frame::Case { predicate } => *predicate != Some(node),
            Frame::Def { endless } => !*endless,
            Frame::RescueMod { expression } => *expression == node,
            Frame::For { multi_body } => contains(*multi_body, node),
            Frame::Assign { own, value } => {
                if *value != Some(node) {
                    return false;
                }
                match self.effective().find(|&i| i < idx) {
                    Some(grandparent) => self.statement(*own, grandparent),
                    None => true,
                }
            }
            _ => false,
        }
    }

    fn process_call(&mut self, call: &CallNode<'_>) {
        if call.is_safe_navigation() {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        if !Self::is_csend_or_parenthesized_csend(&receiver) {
            return;
        }
        let method = call.name().as_slice();
        if method == b"+@"
            || method == b"-@"
            || self.nil_methods.iter().any(|m| m.as_bytes() == method)
        {
            return;
        }
        if !self.require_safe_navigation(call) {
            return;
        }

        let ternary = self.ternary(call, &receiver);
        if matches!(ternary, TernaryKind::IfBranch) {
            return;
        }

        let dot = call.call_operator_loc();
        let has_dot = dot.is_some();
        let brackets = method == b"[]" || method == b"[]=";
        let begin = dot
            .map(|d| d.start_offset())
            .unwrap_or_else(|| receiver.location().end_offset());
        let end = call.location().end_offset();

        let replacement = if matches!(ternary, TernaryKind::ElseBranch) {
            String::new()
        } else {
            self.build_replacement(call, method, begin, end)
        };

        // 1.92.3 `require_parentheses?`: an operator send without a dot is
        // wrapped unless its parent allows a command call there.
        let own = range_of(&call.as_node());
        let (wrap_start, wrap_end) = if !replacement.is_empty()
            && !has_dot
            && !brackets
            && !self.command_call_allowed(own)
        {
            own
        } else {
            (0, 0)
        };

        self.offenses.push(SafeNavChainOffense {
            start_offset: begin,
            end_offset: end,
            replacement,
            wrap_start,
            wrap_end,
        });
    }

    fn build_replacement(
        &self,
        call: &CallNode<'_>,
        method: &[u8],
        begin: usize,
        end: usize,
    ) -> String {
        if method == b"[]" || method == b"[]=" {
            let args = call
                .arguments()
                .map(|a| {
                    a.arguments()
                        .iter()
                        .map(|arg| self.node_source(&arg))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            return format!("&.{}({})", String::from_utf8_lossy(method), args);
        }
        let src = self.text(begin, end);
        if src.starts_with('.') {
            format!("&{src}")
        } else {
            format!("&.{src}")
        }
    }
}

/// The value of an assignment node (parser `children.last`), or `None` when
/// `node` is not an assignment.
fn assignment_value<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    macro_rules! value_of {
        ($($as:ident),* $(,)?) => {
            $(if let Some(n) = node.$as() { return Some(n.value()); })*
        };
    }
    value_of!(
        as_local_variable_write_node,
        as_local_variable_or_write_node,
        as_local_variable_and_write_node,
        as_local_variable_operator_write_node,
        as_instance_variable_write_node,
        as_instance_variable_or_write_node,
        as_instance_variable_and_write_node,
        as_instance_variable_operator_write_node,
        as_class_variable_write_node,
        as_class_variable_or_write_node,
        as_class_variable_and_write_node,
        as_class_variable_operator_write_node,
        as_global_variable_write_node,
        as_global_variable_or_write_node,
        as_global_variable_and_write_node,
        as_global_variable_operator_write_node,
        as_constant_write_node,
        as_constant_or_write_node,
        as_constant_and_write_node,
        as_constant_operator_write_node,
        as_constant_path_write_node,
        as_constant_path_or_write_node,
        as_constant_path_and_write_node,
        as_constant_path_operator_write_node,
        as_call_or_write_node,
        as_call_and_write_node,
        as_call_operator_write_node,
        as_index_or_write_node,
        as_index_and_write_node,
        as_index_operator_write_node,
        as_multi_write_node,
    );
    None
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.process_call(&call);
        }
        let frame = self.frame_for(node);
        self.stack.push(frame);
    }

    fn leave(&mut self) {
        self.stack.pop();
    }

    // `RescueNode` bypasses the branch hooks (see `dispatch`); its parser
    // `resbody` has only the body as a statement.
    fn enter_rescue(&mut self, node: &Node<'_>) {
        let body = node
            .as_rescue_node()
            .and_then(|r| r.statements())
            .map(|s| range_of(&s.as_node()));
        self.stack.push(Frame::Resbody { body });
    }

    fn leave_rescue(&mut self) {
        self.stack.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<(usize, usize, String)> {
        check_safe_navigation_chain(
            source.as_bytes(),
            &["nil?".to_string(), "present?".to_string()],
        )
        .into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.replacement))
        .collect()
    }

    #[test]
    fn basic_chain() {
        let got = run("x&.foo.bar");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, "&.bar");
        assert_eq!(&"x&.foo.bar"[got[0].0..got[0].1], ".bar");
    }

    #[test]
    fn accepts_safe_chain() {
        assert!(run("x&.foo&.bar").is_empty());
    }

    #[test]
    fn accepts_nil_method() {
        assert!(run("x&.foo.nil?").is_empty());
    }

    #[test]
    fn operator_correction() {
        let got = run("x&.foo < bar");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, "&. < bar");
    }

    #[test]
    fn brackets_correction() {
        let got = run("x&.foo[bar]");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, "&.[](bar)");
    }
}
