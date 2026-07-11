//! `Layout/EmptyLinesAroundArguments`.
//!
//! Checks for empty lines around the arguments of a method invocation. The stock
//! cop fires `on_send` / `on_csend` and, for every multi-line call with at least
//! one argument whose receiver and selector share a line, scans each argument's
//! `source_range.begin` and (if present) the closing `)`/`]` for a preceding run
//! of whitespace that spans a full empty line; each such run is one offense and,
//! on autocorrect, is removed.
//!
//! Reconstructed over Prism. A parser-gem `(send ...)` / `(csend ...)` is a Prism
//! `CallNode`; `node.arguments` is its `ArgumentsNode`, `node.loc.end` is its
//! `closing_loc`, `node.loc.selector` is its `message_loc`, and the receiver is
//! `receiver()`. Each starting point is checked with the same line arithmetic as
//! stock's `range_with_surrounding_space(start, whitespace: true, side: :left)`
//! followed by `line_range(last_line - 1).adjust(end_pos: 1)`.
//!
//! Faithfulness notes (verified against stock probes):
//!
//! - `single_line?`: the call's whole source range is on one line.
//! - `receiver_and_method_call_on_different_lines?`: a `receiver` exists and its
//!   last line differs from the selector line. When the selector is absent (a
//!   `foo.(...)` call), `selector&.line` is `nil`, so the comparison is always
//!   "different" and the call is skipped (stock quirk).
//! - The whitespace scan moves left over `[ \t]`, then `\n`, then all `\s`,
//!   stopping at the first non-whitespace byte. The run spans an empty line iff
//!   `last_line - first_line > 1` (where `last_line` is the starting point's
//!   line and `first_line` is the line of the run's left boundary).
//! - The offense range is the **entire** line `last_line - 1` (its whitespace
//!   content included) plus its trailing `\n`: `[line_start(L), nl_pos(L) + 1)`.
//!   For a zero-length empty line this is just the `\n`; for a line with spaces
//!   or tabs it is those bytes and the `\n`.
//! - Multiple consecutive empty lines before one starting point still yield a
//!   single offense (the last empty line); the autocorrect loop removes the
//!   surplus lines over successive passes.

use ruby_prism::Node;

use super::line_index::LineIndex;

/// One offense. `[start_offset, end_offset)` is the offense line range (whole
/// line `last_line - 1` plus its `\n`), which is also exactly the range the
/// autocorrect removes.
pub struct EmptyLinesAroundArgumentsOffense {
    pub start_offset: usize,
    pub end_offset: usize,
}

pub fn check_empty_lines_around_arguments(
    source: &[u8],
) -> Vec<EmptyLinesAroundArgumentsOffense> {
    let mut rule = build_rule(source);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle. The check is
/// stateless per node (no ancestor stack), so the plain full walk fits.
pub(crate) fn build_rule(source: &[u8]) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: std::rc::Rc<LineIndex>,
    pub(crate) offenses: Vec<EmptyLinesAroundArgumentsOffense>,
}

impl<'a> Visitor<'a> {
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn on_call(&mut self, call: &ruby_prism::CallNode<'_>, node: &Node<'_>) {
        // `node.single_line?`
        let (node_start, node_end) = (
            node.location().start_offset(),
            node.location().end_offset(),
        );
        if self.line_of(node_start) == self.line_of(node_end) {
            return;
        }
        // `node.arguments.empty?` — parser-gem's `send.arguments` includes
        // the block-pass argument (`&blk`, bare `&`) as the last argument;
        // prism keeps it in `block()`, outside `arguments()`.
        let mut args: Vec<Node<'_>> = call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        if let Some(block) = call.block()
            && block.as_block_argument_node().is_some()
        {
            args.push(block);
        }
        if args.is_empty() {
            return;
        }
        // `receiver_and_method_call_on_different_lines?`
        if self.receiver_and_method_call_on_different_lines(call) {
            return;
        }

        for arg in &args {
            self.empty_range_for_starting_point(arg.location().start_offset());
        }
        if let Some(end) = call.closing_loc() {
            self.empty_range_for_starting_point(end.start_offset());
        }
    }

    /// `node.receiver && node.receiver.loc.last_line != node.loc.selector&.line`.
    fn receiver_and_method_call_on_different_lines(
        &self,
        call: &ruby_prism::CallNode<'_>,
    ) -> bool {
        let Some(receiver) = call.receiver() else {
            return false;
        };
        let recv_last_line = self.line_of(receiver.location().end_offset());
        match call.message_loc() {
            // `last_line != selector.line`
            Some(sel) => recv_last_line != self.line_of(sel.start_offset()),
            // `last_line != nil` is always true.
            None => true,
        }
    }

    /// `empty_range_for_starting_point`: scan left from `start` over whitespace;
    /// if the run spans a full empty line, push the offense for line
    /// `last_line - 1`.
    fn empty_range_for_starting_point(&mut self, start: usize) {
        let begin = self.surrounding_space_left(start);
        let last_line = self.line_of(start);
        let first_line = self.line_of(begin);
        if last_line - first_line <= 1 {
            return;
        }
        // `line_range(last_line - 1).adjust(end_pos: 1)`: the whole line
        // `last_line - 1` plus its trailing `\n`.
        let l = last_line - 1;
        let (ls, nl) = self.line_with_newline(l);
        self.offenses.push(EmptyLinesAroundArgumentsOffense {
            start_offset: ls,
            end_offset: nl,
        });
    }

    /// `range_with_surrounding_space(start, whitespace: true, side: :left)`'s
    /// `begin_pos`: move left over `[ \t]`, then `\n`, then all `\s`, stopping at
    /// the first non-whitespace byte. The three stock passes collapse to "skip
    /// every ASCII whitespace byte to the left" (each stock predicate is a subset
    /// of `\s`, applied in sequence; the final `\s` pass subsumes the rest).
    fn surrounding_space_left(&self, start: usize) -> usize {
        let mut pos = start;
        while pos > 0 && is_space(self.source[pos - 1]) {
            pos -= 1;
        }
        pos
    }

    /// For 1-based `line`, return `(line_start, newline_pos + 1)` where
    /// `newline_pos` is the offset of the line's terminating `\n`. This is
    /// `line_range(line).adjust(end_pos: 1)`: the whole line content plus the
    /// `\n`. The line is guaranteed to have a terminating `\n` (it lies strictly
    /// before the starting point, which is on a later line).
    fn line_with_newline(&self, line: usize) -> (usize, usize) {
        let starts = self.line_index.line_starts();
        let idx = line - 1;
        let ls = starts[idx];
        // The terminating `\n` is at `starts[idx + 1] - 1` (the next line's start
        // minus one). Every line we report has a following line, so `idx + 1` is
        // in range.
        let nl_plus_one = starts[idx + 1];
        (ls, nl_plus_one)
    }
}

/// Ruby's `/\s/` over a single byte: ASCII space, tab, newline, carriage return,
/// vertical tab and form feed.
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_CALL,
        )
    }
    
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.on_call(&call, node);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<EmptyLinesAroundArgumentsOffense> {
        check_empty_lines_around_arguments(source.as_bytes())
    }

    fn ranges(source: &str) -> Vec<(usize, usize)> {
        run(source).iter().map(|o| (o.start_offset, o.end_offset)).collect()
    }

    #[test]
    fn empty_line_before_arg() {
        assert_eq!(ranges("foo(\n\n  bar\n)\n"), vec![(5, 6)]);
    }

    #[test]
    fn empty_line_between_args() {
        assert_eq!(ranges("foo(\n  baz,\n\n  qux: 0\n)\n"), vec![(12, 13)]);
    }

    #[test]
    fn empty_line_after_arg() {
        assert_eq!(ranges("bar(\n  [baz]\n\n)\n"), vec![(13, 14)]);
    }

    #[test]
    fn args_on_definition_line() {
        assert_eq!(ranges("foo(biz,\n\n    baz: 0)\n"), vec![(9, 10)]);
    }

    #[test]
    fn multiple_empties_one_offense() {
        // Two blank lines between args produce a single offense (the last).
        assert_eq!(ranges("foo(\n  baz,\n\n\n  qux\n)\n"), vec![(13, 14)]);
    }

    #[test]
    fn whitespace_line_covered_fully() {
        // The offense covers the whole whitespace line plus its `\n`.
        assert_eq!(ranges("foo(\n  \n  bar\n)\n"), vec![(5, 8)]);
    }

    #[test]
    fn multiple_args_each_offend() {
        let r = ranges("foo(\n  baz,\n\n  qux,\n\n  biz,\n\n)\n");
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn single_line_call_no_offense() {
        assert!(run("foo(bar)\n").is_empty());
    }

    #[test]
    fn no_args_no_offense() {
        assert!(run("foo(\n\n)\n").is_empty());
    }

    #[test]
    fn clean_multiline_no_offense() {
        assert!(run("foo(\n  bar,\n  baz\n)\n").is_empty());
    }

    #[test]
    fn csend_offends() {
        assert_eq!(ranges("receiver&.foo(\n\n  bar\n)\n"), vec![(15, 16)]);
    }

    #[test]
    fn receiver_diff_line_skipped() {
        assert!(run("foo.\n\n  bar(arg)\n").is_empty());
    }

    #[test]
    fn dot_call_without_selector_skipped() {
        assert!(run("foo.(\n  arg\n)\n").is_empty());
    }

    #[test]
    fn blank_inside_string_arg_no_offense() {
        assert!(run("format('%d\n\n', 1)[0]\n").is_empty());
    }

    #[test]
    fn blank_inside_array_arg_no_offense() {
        assert!(run("foo(:bar, [1,\n\n           2]\n)\n").is_empty());
    }

    #[test]
    fn no_paren_args_offend() {
        assert_eq!(ranges("puts foo,\n\n  bar\n").len(), 1);
    }

    #[test]
    fn block_arg_between_offends() {
        let r = ranges("Foo.prepend(\n  a,\n\n  Module.new do\n    def x; end\n  end\n)\n");
        assert_eq!(r.len(), 1);
    }
}
