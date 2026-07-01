//! `Layout/DotPosition`.
//!
//! Checks the `.`/`&.` position in multi-line method calls (`leading` vs
//! `trailing` style).

use std::rc::Rc;

use ruby_prism::{CallNode, Node, Visit};

use super::line_index::LineIndex;

/// One misplaced dot. `(start, end)` is the dot range (the offense highlight).
/// `(remove_start, remove_end)` is the range autocorrect deletes (the dot, or
/// its whole line when the dot stands alone). `insert_pos` is where the dot text
/// is re-inserted (before the selector for `leading`, after the receiver for
/// `trailing`).
pub struct DotPositionOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub remove_start: usize,
    pub remove_end: usize,
    pub insert_pos: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Leading,
    Trailing,
}

pub fn check_dot_position(source: &[u8], style: u8) -> Vec<DotPositionOffense> {
    let mut visitor = build_rule(source, style);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule(source: &[u8], style: u8) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        style: if style == 1 {
            Style::Trailing
        } else {
            Style::Leading
        },
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    style: Style,
    pub(crate) offenses: Vec<DotPositionOffense>,
}

impl<'a> Visitor<'a> {
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn line_start(&self, off: usize) -> usize {
        self.line_index.line_start(off)
    }

    fn process_send(&mut self, call: &CallNode<'_>) {
        let Some(dot) = call.call_operator_loc() else {
            return;
        };
        let dot_text = &self.source[dot.start_offset()..dot.end_offset()];
        // `node.dot? || node.safe_navigation?`: only `.` and `&.`, not `::`.
        if dot_text != b"." && dot_text != b"&." {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };

        // selector_range: the message, or the opening paren for `l.(1)`.
        let Some(selector_start) = call
            .message_loc()
            .map(|m| m.start_offset())
            .or_else(|| call.opening_loc().map(|o| o.start_offset()))
        else {
            return;
        };

        if !self.is_offense(&receiver, selector_start, dot.start_offset()) {
            return;
        }

        // Autocorrect ranges.
        let remove = self.remove_range(dot.start_offset(), dot.end_offset());
        let insert_pos = match self.style {
            Style::Leading => selector_start,
            Style::Trailing => receiver.location().end_offset(),
        };

        self.offenses.push(DotPositionOffense {
            start_offset: dot.start_offset(),
            end_offset: dot.end_offset(),
            remove_start: remove.0,
            remove_end: remove.1,
            insert_pos,
        });
    }

    /// `proper_dot_position?` negated.
    fn is_offense(&self, receiver: &Node<'_>, selector_start: usize, dot_start: usize) -> bool {
        let receiver_end = receiver.location().end_offset();
        let selector_line = self.line_of(selector_start);

        // same_line?(selector, receiver.end): a single-line call.
        if selector_line == self.line_of(receiver_end) {
            return false;
        }

        // `receiver_end_line`: a heredoc receiver/argument ends on its terminator
        // line, which is below the opening token.
        let receiver_line = self
            .last_heredoc_line(receiver)
            .unwrap_or_else(|| self.line_of(receiver_end));
        let dot_line = self.line_of(dot_start);

        // A blank line / comment between the receiver-or-dot and the selector.
        if selector_line.saturating_sub(receiver_line.max(dot_line)) > 1 {
            return false;
        }

        // correct_dot_position_style? negated.
        match self.style {
            Style::Leading => dot_line != selector_line,
            Style::Trailing => dot_line == selector_line,
        }
    }

    /// `last_heredoc_line`: the terminator line of the last heredoc on `node` —
    /// the heredoc itself, or the highest among a call's heredoc arguments.
    fn last_heredoc_line(&self, node: &Node<'_>) -> Option<usize> {
        if let Some(c) = node.as_call_node() {
            return c.arguments().and_then(|args| {
                args.arguments()
                    .iter()
                    .filter_map(|a| self.heredoc_end_line(&a))
                    .max()
            });
        }
        self.heredoc_end_line(node)
    }

    /// The terminator line of `node` when it is a heredoc string, else `None`.
    fn heredoc_end_line(&self, node: &Node<'_>) -> Option<usize> {
        let (opening, closing) = if let Some(s) = node.as_string_node() {
            (s.opening_loc(), s.closing_loc())
        } else if let Some(s) = node.as_interpolated_string_node() {
            (s.opening_loc(), s.closing_loc())
        } else if let Some(s) = node.as_x_string_node() {
            (Some(s.opening_loc()), Some(s.closing_loc()))
        } else if let Some(s) = node.as_interpolated_x_string_node() {
            (Some(s.opening_loc()), Some(s.closing_loc()))
        } else {
            return None;
        };
        let opening = opening?;
        let closing = closing?;
        if self.source[opening.start_offset()..].starts_with(b"<<") {
            Some(self.line_of(closing.end_offset()))
        } else {
            None
        }
    }

    /// `dot_range`: the whole line (incl. trailing newline) when the dot stands
    /// alone on it, otherwise just the dot.
    fn remove_range(&self, dot_start: usize, dot_end: usize) -> (usize, usize) {
        let ls = self.line_start(dot_start);
        let line_end = match self.source[dot_end..].iter().position(|&b| b == b'\n') {
            Some(i) => dot_end + i,
            None => self.source.len(),
        };
        let line = &self.source[ls..line_end];
        let stripped: &[u8] = {
            let start = line
                .iter()
                .position(|&b| b != b' ' && b != b'\t')
                .unwrap_or(line.len());
            let end = line
                .iter()
                .rposition(|&b| b != b' ' && b != b'\t')
                .map(|i| i + 1)
                .unwrap_or(0);
            if start <= end { &line[start..end] } else { b"" }
        };
        if stripped == b"." {
            // Whole line including the final newline.
            let end = (line_end + 1).min(self.source.len());
            (ls, end)
        } else {
            (dot_start, dot_end)
        }
    }
}

impl<'pr> Visit<'pr> for Visitor<'_> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        self.process_send(node);
        ruby_prism::visit_call_node(self, node);
    }
}

/// Shared-walk driver. The generic branch hook fires for every `CallNode` the
/// typed `visit_call_node` sees except the one reached through
/// `MatchWriteNode`'s concretely-typed `call` field — an `=~` operator call,
/// which has no `.`/`&.` operator and is a no-op for `process_send` anyway.
impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_CALL,
        )
    }
    
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.process_send(&call);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: Style) -> Vec<(usize, usize)> {
        let s = match style {
            Style::Leading => 0,
            Style::Trailing => 1,
        };
        check_dot_position(source.as_bytes(), s)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset))
            .collect()
    }

    #[test]
    fn leading_flags_trailing_dot() {
        let got = run("something.\n  method\n", Style::Leading);
        assert_eq!(got.len(), 1);
        assert_eq!(&"something.\n  method\n"[got[0].0..got[0].1], ".");
    }

    #[test]
    fn leading_accepts_leading_dot() {
        assert!(run("something\n  .method\n", Style::Leading).is_empty());
    }

    #[test]
    fn trailing_flags_leading_dot() {
        let got = run("something\n  .method\n", Style::Trailing);
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn accepts_single_line() {
        assert!(run("something.method\n", Style::Leading).is_empty());
    }

    #[test]
    fn ignores_scope_resolution() {
        assert!(run("Foo::\n  Bar\n", Style::Leading).is_empty());
    }
}
