//! `Lint/SafeNavigationChain`.

use ruby_prism::{CallNode, Node};

/// An ordinary method call chained after a safe-navigation call.
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

const COMPARISON_OPERATORS: &[&[u8]] = &[b"==", b"===", b"!=", b"<=>", b"<", b">", b"<=", b">="];

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

/// The parser-gem-equivalent parent of a node, with the data the cop's parent
/// predicates need. Structural Prism wrappers (`ArgumentsNode`,
/// `StatementsNode`, `ElseNode`) are `Transparent` and skipped.
enum Frame {
    Transparent,
    And {
        left_receiver: Option<(usize, usize)>,
        right_start: usize,
        symbolic: bool,
    },
    Or {
        symbolic: bool,
    },
    Ternary {
        predicate: (usize, usize),
        then_start: Option<usize>,
        else_start: Option<usize>,
    },
    CollectionLiteral,
    Comparison,
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

    fn is_comparison(call: &CallNode<'_>) -> bool {
        COMPARISON_OPERATORS.contains(&call.name().as_slice())
    }

    /// The parser-gem parent: the nearest stacked frame that is not a structural
    /// Prism wrapper.
    fn effective_parent(&self) -> Option<&Frame> {
        self.stack
            .iter()
            .rev()
            .find(|f| !matches!(f, Frame::Transparent))
    }

    /// Frame describing `node` for its children.
    fn frame_for(&self, node: &Node<'_>) -> Frame {
        if node.as_arguments_node().is_some()
            || node.as_statements_node().is_some()
            || node.as_else_node().is_some()
        {
            return Frame::Transparent;
        }
        if let Some(a) = node.as_and_node() {
            let op = a.operator_loc();
            let left_receiver = a.left().as_call_node().and_then(|c| c.receiver()).map(|r| {
                let l = r.location();
                (l.start_offset(), l.end_offset())
            });
            return Frame::And {
                left_receiver,
                right_start: a.right().location().start_offset(),
                symbolic: self.text(op.start_offset(), op.end_offset()) == "&&",
            };
        }
        if let Some(o) = node.as_or_node() {
            let op = o.operator_loc();
            return Frame::Or {
                symbolic: self.text(op.start_offset(), op.end_offset()) == "||",
            };
        }
        if let Some(if_node) = node.as_if_node() {
            if if_node.if_keyword_loc().is_none() {
                let p = if_node.predicate().location();
                let then_start = if_node.statements().map(|s| s.location().start_offset());
                let else_start = if_node
                    .subsequent()
                    .and_then(|e| e.as_else_node())
                    .and_then(|e| e.statements())
                    .map(|s| s.location().start_offset());
                return Frame::Ternary {
                    predicate: (p.start_offset(), p.end_offset()),
                    then_start,
                    else_start,
                };
            }
            return Frame::Opaque;
        }
        if node.as_array_node().is_some() || node.as_assoc_node().is_some() {
            return Frame::CollectionLiteral;
        }
        if let Some(c) = node.as_call_node()
            && Self::is_comparison(&c)
        {
            return Frame::Comparison;
        }
        Frame::Opaque
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
        let Some(Frame::Ternary {
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

    fn require_parentheses(&self, call: &CallNode<'_>, has_dot: bool) -> bool {
        let parent = self.effective_parent();
        if !has_dot && matches!(parent, Some(Frame::CollectionLiteral)) {
            return true;
        }
        if !Self::is_comparison(call) {
            return false;
        }
        matches!(
            parent,
            Some(
                Frame::And { symbolic: true, .. }
                    | Frame::Or { symbolic: true }
                    | Frame::Comparison
            )
        )
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
        let begin = dot
            .map(|d| d.start_offset())
            .unwrap_or_else(|| receiver.location().end_offset());
        let end = call.location().end_offset();

        let replacement = if matches!(ternary, TernaryKind::ElseBranch) {
            String::new()
        } else {
            self.build_replacement(call, method, begin, end)
        };

        let (wrap_start, wrap_end) =
            if !replacement.is_empty() && self.require_parentheses(call, has_dot) {
                let loc = call.location();
                (loc.start_offset(), loc.end_offset())
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
