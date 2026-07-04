//! `Layout/ArrayAlignment`.
//!
//! Checks that the elements of a multi-line array literal are aligned. Two
//! styles: `with_first_element` (align every element under the first one) and
//! `with_fixed_indentation` (one indentation level below the line the array
//! starts on).
//!
//! Ported from the cop + the shared `Alignment` mixin (`check_alignment` /
//! `each_bad_alignment`), like `Layout/ArgumentAlignment`. Rust computes the
//! per-element `column_delta` and the offense range; Ruby realigns via
//! `AlignmentCorrector`. The `within?` nested-offense rule (report-without-
//! autocorrect for offenses already covered by a registered offense range) is
//! replicated here over the pre-order walk.
//!
//! Two prism-vs-parser mapping points need care (probed on stock 1.88):
//!
//! * parser-gem builds an `array` node in exactly three bracket-less spots:
//!   the RHS of a single assignment (`a = 1, 2`, including `foo.bar = 1, 2` /
//!   `foo[0] = 1, 2` setter sends), the RHS of a masgn (`a, b = 1, 2` — the
//!   cop skips those via `parent&.masgn_type?`), and a `rescue` clause's
//!   exception list (`rescue A, B`). prism mirrors the first two as
//!   `ArrayNode`s without `opening_loc` but keeps rescue exceptions as a bare
//!   node list, so the rescue check is driven from the dispatcher's rescue
//!   hook instead of an array visit.
//! * an unbracketed `ArrayNode` is only checked when a parent intercept has
//!   claimed it (assignment value / setter argument). An unclaimed implicit
//!   array has no parser-gem `array` counterpart, so stock never fires there.

use std::rc::Rc;

use ruby_prism::{ArrayNode, Location, Node};

use super::line_index::LineIndex;

/// One misaligned element. `column_delta` is `base_column - actual_column`
/// (display columns). `autocorrect` is false for offenses nested inside an
/// already-registered offense range (the mixin's `within?` rule).
pub struct ArrayAlignOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub autocorrect: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    WithFirstElement,
    WithFixedIndentation,
}

pub fn check_array_alignment(
    source: &[u8],
    style: u8,
    indent_width: usize,
) -> Vec<ArrayAlignOffense> {
    let mut rule = build_rule(source, style, indent_width);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule(source: &[u8], style: u8, indent_width: usize) -> Visitor<'_> {
    let style = if style == 1 {
        Style::WithFixedIndentation
    } else {
        Style::WithFirstElement
    };
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        style,
        indent: indent_width,
        masgn_values: Vec::new(),
        implicit_claims: Vec::new(),
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    style: Style,
    indent: usize,
    /// Start offsets of arrays that are a `MultiWriteNode`'s value — the
    /// stock cop's `return if node.parent&.masgn_type?`.
    masgn_values: Vec<usize>,
    /// `(array start offset, parent start offset)` claims for unbracketed
    /// arrays, recorded by the assignment / setter-send parent intercepts.
    /// The parent offset feeds `target_method_lineno` (the stock cop uses
    /// `node.parent.loc.line` when the array has no brackets).
    implicit_claims: Vec<(usize, usize)>,
    pub(crate) offenses: Vec<ArrayAlignOffense>,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

impl Visitor<'_> {
    fn line_start(&self, off: usize) -> usize {
        self.line_index.line_start(off)
    }

    /// `Unicode::DisplayWidth.of(line[0, column])`: the display column of `off`
    /// (East-Asian wide characters count as two).
    fn display_column(&self, off: usize) -> usize {
        self.line_index.display_column(self.source, off)
    }

    /// 1-based line number of `off`.
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    /// The indentation (`/\S.*/.match(line).begin(0)`) of the line `off` is on.
    fn indentation_of_line(&self, off: usize) -> usize {
        let ls = self.line_start(off);
        self.source[ls..]
            .iter()
            .take_while(|&&b| b == b' ' || b == b'\t')
            .count()
    }

    fn begins_its_line(&self, off: usize) -> bool {
        let ls = self.line_start(off);
        self.source[ls..off]
            .iter()
            .all(|&b| b == b' ' || b == b'\t')
    }

    /// If `value` is an unbracketed array, claim it for `parent_off` (the
    /// assignment / setter send the parser-gem AST makes its parent).
    fn claim_implicit(&mut self, value: &Node<'_>, parent_off: usize) {
        if let Some(arr) = value.as_array_node()
            && arr.opening_loc().is_none()
        {
            self.implicit_claims
                .push((arr.location().start_offset(), parent_off));
        }
    }

    /// `on_array`.
    fn process_array(&mut self, arr: &ArrayNode<'_>) {
        let start = arr.location().start_offset();
        // `return if node.parent&.masgn_type?`
        if let Some(pos) = self.masgn_values.iter().position(|&o| o == start) {
            self.masgn_values.swap_remove(pos);
            return;
        }
        // For an unbracketed array, `target_method_lineno` needs the parser
        // parent's start; unclaimed unbracketed arrays have no parser-gem
        // `array` counterpart at all, so stock never visits them.
        let claimed_parent = if arr.opening_loc().is_none() {
            let Some(pos) = self.implicit_claims.iter().position(|&(o, _)| o == start) else {
                return;
            };
            Some(self.implicit_claims.swap_remove(pos).1)
        } else {
            None
        };
        let elements = arr.elements();
        if elements.len() < 2 {
            return; // `return if node.children.size < 2`
        }
        let items: Vec<(usize, usize)> = elements.iter().map(|e| loc(&e.location())).collect();
        let base_column = self.base_column(claimed_parent.unwrap_or(start), &items);
        self.check_alignment(&items, base_column);
    }

    /// `base_column(node, args)`. `target_off` sits on `target_method_lineno`:
    /// the array's own start for a bracketed array (`node.loc.line`), the
    /// claimed parent's start otherwise (`node.parent.loc.line`).
    fn base_column(&self, target_off: usize, items: &[(usize, usize)]) -> usize {
        match self.style {
            Style::WithFixedIndentation => self.indentation_of_line(target_off) + self.indent,
            Style::WithFirstElement => self.display_column(items[0].0),
        }
    }

    /// `check_alignment` + `each_bad_alignment` (same port as
    /// `argument_alignment.rs`).
    fn check_alignment(&mut self, items: &[(usize, usize)], base_column: usize) {
        let mut prev_line: isize = -1;
        for &item in items {
            let line = self.line_of(item.0) as isize;
            if line > prev_line && self.begins_its_line(item.0) {
                let column_delta = base_column as isize - self.display_column(item.0) as isize;
                if column_delta != 0 {
                    // within? any already-registered offense range -> no autocorrect.
                    let autocorrect = !self
                        .offenses
                        .iter()
                        .any(|o| item.0 >= o.start_offset && item.1 <= o.end_offset);
                    self.offenses.push(ArrayAlignOffense {
                        start_offset: item.0,
                        end_offset: item.1,
                        column_delta,
                        autocorrect,
                    });
                }
            }
            prev_line = line;
        }
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(Interest::ENTER_LITERAL | Interest::ENTER_WRITE | Interest::ENTER_CALL | Interest::RESCUE)
    }

    fn enter(&mut self, node: &Node<'_>) {
        match node {
            Node::ArrayNode { .. } => {
                let arr = node.as_array_node().unwrap();
                self.process_array(&arr);
            }
            // Parent intercepts for parser-gem's three bracket-less array
            // spots (see the module doc). Claims are recorded before the
            // walk descends into the value, so the array's own visit finds
            // them.
            Node::MultiWriteNode { .. } => {
                let mw = node.as_multi_write_node().unwrap();
                if let Some(arr) = mw.value().as_array_node() {
                    self.masgn_values.push(arr.location().start_offset());
                }
            }
            Node::LocalVariableWriteNode { .. } => {
                let w = node.as_local_variable_write_node().unwrap();
                self.claim_implicit(&w.value(), node.location().start_offset());
            }
            Node::InstanceVariableWriteNode { .. } => {
                let w = node.as_instance_variable_write_node().unwrap();
                self.claim_implicit(&w.value(), node.location().start_offset());
            }
            Node::ClassVariableWriteNode { .. } => {
                let w = node.as_class_variable_write_node().unwrap();
                self.claim_implicit(&w.value(), node.location().start_offset());
            }
            Node::GlobalVariableWriteNode { .. } => {
                let w = node.as_global_variable_write_node().unwrap();
                self.claim_implicit(&w.value(), node.location().start_offset());
            }
            Node::ConstantWriteNode { .. } => {
                let w = node.as_constant_write_node().unwrap();
                self.claim_implicit(&w.value(), node.location().start_offset());
            }
            Node::ConstantPathWriteNode { .. } => {
                let w = node.as_constant_path_write_node().unwrap();
                self.claim_implicit(&w.value(), node.location().start_offset());
            }
            // Setter sends (`foo.bar = 1, 2` / `foo[0] = 1, 2`, safe
            // navigation included): the implicit array is the last argument.
            // Or-write / operator-write forms reject a bare list, so plain
            // setters are the only call shape that can carry one.
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                if call.name().as_slice().ends_with(b"=")
                    && let Some(args) = call.arguments()
                    && let Some(last) = args.arguments().iter().last()
                {
                    self.claim_implicit(&last, node.location().start_offset());
                }
            }
            _ => {}
        }
    }

    fn leave(&mut self) {}

    // parser-gem wraps a `rescue` clause's exception list (2+ entries) in a
    // bracket-less `array` node whose parent is the `resbody`, so the stock
    // cop checks it like any other array. prism keeps the exceptions as a
    // bare node list; replay the check from the rescue hook (which fires
    // before the exceptions are visited — the same pre-order position the
    // parser array node occupies).
    fn enter_rescue(&mut self, node: &Node<'_>) {
        let Some(rescue) = node.as_rescue_node() else {
            return;
        };
        let items: Vec<(usize, usize)> = rescue
            .exceptions()
            .iter()
            .map(|e| loc(&e.location()))
            .collect();
        if items.len() < 2 {
            return;
        }
        let base_column = match self.style {
            // `resbody` starts at the `rescue` keyword, so
            // `node.parent.loc.line` is the keyword's line.
            Style::WithFixedIndentation => {
                self.indentation_of_line(rescue.keyword_loc().start_offset()) + self.indent
            }
            Style::WithFirstElement => self.display_column(items[0].0),
        };
        self.check_alignment(&items, base_column);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Vec<(usize, usize, isize, bool)> {
        check_array_alignment(source.as_bytes(), style, 2)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.autocorrect))
            .collect()
    }

    #[test]
    fn with_first_element_misaligned() {
        // `b` and `c` under-indented relative to `a` (col 9).
        let got = run("array = [a,\n   b,\n  c]\n", 0);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], (15, 16, 6, true)); // `b` at col 3 -> +6
        assert_eq!(got[1], (20, 21, 7, true)); // `c` at col 2 -> +7
    }

    #[test]
    fn with_first_element_aligned_is_clean() {
        assert!(run("array = [a,\n         b,\n         c]\n", 0).is_empty());
    }

    #[test]
    fn fixed_indentation() {
        // Elements aligned with the first element (col 9) but fixed wants col 2.
        let got = run("array = [a,\n         b,\n         c]\n", 1);
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|o| o.2 == -7));
        assert!(run("array = [a,\n  b,\n  c]\n", 1).is_empty());
    }

    #[test]
    fn fixed_indentation_checks_first_element_on_its_own_line() {
        // `[` alone on line 1: the first element itself is realigned to col 2.
        let got = run("x = [\n    1,\n  2]\n", 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, -2);
    }

    #[test]
    fn masgn_rhs_is_skipped() {
        assert!(run("a, b = 1,\n        2\n", 0).is_empty());
        assert!(run("a, b = [1,\n        2]\n", 0).is_empty());
        assert!(run("a, b = 1,\n        2\n", 1).is_empty());
    }

    #[test]
    fn implicit_assignment_array_is_checked() {
        let got = run("a = 1,\n  2\n", 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], (9, 10, 2, true)); // `2` at col 2 -> col 4

        let got = run("foo.bar = 1,\n   2\n", 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, -1); // fixed: col 3 -> col 2
    }

    #[test]
    fn rescue_exception_list_is_checked() {
        let src = "begin\n  x\nrescue FooError,\n    BarError => e\n  y\nend\n";
        let got = run(src, 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], (31, 39, 3, true)); // `BarError` col 4 -> col 7

        let got = run(src, 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, -2); // fixed: col 4 -> col 2
    }

    #[test]
    fn percent_array_is_checked() {
        let got = run("x = %w[aa\n    bb]\n", 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], (14, 16, 3, true)); // `bb` col 4 -> col 7
    }

    #[test]
    fn nested_offense_loses_autocorrect() {
        // `[3,\n 4]` is realigned as an element of the outer array; the inner
        // `4` offense is within that range -> report without autocorrect.
        let got = run("x = [[1,\n   2],\n  [3,\n 4]]\n", 0);
        assert_eq!(got.len(), 3);
        assert!(got[0].3 && got[1].3);
        assert!(!got[2].3);
    }

    #[test]
    fn single_element_and_single_line_are_clean() {
        assert!(run("x = [a, b, c]\n", 0).is_empty());
        assert!(run("x = [foo: 1,\n     bar: 2]\n", 0).is_empty()); // one hash child
        assert!(run("a = *b\n", 0).is_empty());
    }
}
