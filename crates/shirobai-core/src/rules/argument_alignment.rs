//! `Layout/ArgumentAlignment`.
//!
//! Checks that the arguments of a multi-line method call are aligned. Two
//! styles: `with_first_argument` (align every argument under the first one) and
//! `with_fixed_indentation` (one indentation level below the method line).
//!
//! Ported from the cop + the shared `Alignment` mixin (`check_alignment` /
//! `each_bad_alignment`). Rust computes the per-argument `column_delta` and the
//! offense range; Ruby realigns via `AlignmentCorrector`. The `within?`
//! nested-offense rule (report-without-autocorrect for offenses already covered
//! by a registered offense range) is replicated here over the pre-order walk.

use ruby_prism::{CallNode, Location, Node};

/// One misaligned argument. `column_delta` is `base_column - actual_column`
/// (display columns). `autocorrect` is false for offenses nested inside an
/// already-registered offense range (the mixin's `within?` rule).
pub struct ArgAlignOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub autocorrect: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    WithFirstArgument,
    WithFixedIndentation,
}

pub fn check_argument_alignment(
    source: &[u8],
    style: u8,
    indent_width: usize,
    incompatible: bool,
) -> Vec<ArgAlignOffense> {
    let style = if style == 1 {
        Style::WithFixedIndentation
    } else {
        Style::WithFirstArgument
    };
    // `autocorrect_incompatible_with_other_cops?`: disables the cop entirely.
    if incompatible && style == Style::WithFirstArgument {
        return Vec::new();
    }
    let mut rule = Visitor {
        source,
        style,
        indent: indent_width,
        offenses: Vec::new(),
    };
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

struct Visitor<'a> {
    source: &'a [u8],
    style: Style,
    indent: usize,
    offenses: Vec<ArgAlignOffense>,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

impl Visitor<'_> {
    fn line_start(&self, off: usize) -> usize {
        match self.source[..off].iter().rposition(|&b| b == b'\n') {
            Some(i) => i + 1,
            None => 0,
        }
    }

    /// `Unicode::DisplayWidth.of(line[0, column])`: the display column of `off`
    /// (East-Asian wide characters count as two).
    fn display_column(&self, off: usize) -> usize {
        let ls = self.line_start(off);
        std::str::from_utf8(&self.source[ls..off])
            .map(unicode_width::UnicodeWidthStr::width)
            .unwrap_or(off - ls)
    }

    /// 1-based line number of `off`.
    fn line_of(&self, off: usize) -> usize {
        self.source[..off].iter().filter(|&&b| b == b'\n').count() + 1
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

    /// `on_send`: the cop's entry point for every (c)send node.
    fn process_send(&mut self, call: &CallNode<'_>) {
        let has_receiver = call.receiver().is_some();
        // `node.call_type? && node.method?(:[]=)`
        if has_receiver && call.name().as_slice() == b"[]=" {
            return;
        }

        let Some(items) = self.flattened_arguments(call) else {
            return; // `!multiple_arguments?`
        };
        if items.is_empty() {
            return;
        }
        let base_column = self.base_column(call, items[0]);
        self.check_alignment(&items, base_column);
    }

    /// `multiple_arguments?` + `flattened_arguments`: the list of item ranges to
    /// align, or `None` when the call does not qualify.
    fn flattened_arguments(&self, call: &CallNode<'_>) -> Option<Vec<(usize, usize)>> {
        let args: Vec<Node<'_>> = call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();

        // multiple_arguments?
        let qualifies = args.len() >= 2
            || args
                .first()
                .map(|f| braceless_hash_pairs(f).is_some_and(|p| p.len() >= 2))
                .unwrap_or(false);
        if !qualifies {
            return None;
        }

        let items = match self.style {
            Style::WithFixedIndentation => {
                // arguments[0..-2] + (last braceless hash -> pairs, else last)
                let mut items: Vec<(usize, usize)> = args[..args.len() - 1]
                    .iter()
                    .map(|n| loc(&n.location()))
                    .collect();
                let last = args.last().unwrap();
                match braceless_hash_pairs(last) {
                    Some(pairs) => items.extend(pairs),
                    None => items.push(loc(&last.location())),
                }
                items
            }
            Style::WithFirstArgument => match braceless_hash_pairs(&args[0]) {
                Some(pairs) => pairs,
                None => args.iter().map(|n| loc(&n.location())).collect(),
            },
        };
        Some(items)
    }

    /// `base_column(node, first_argument)`.
    fn base_column(&self, call: &CallNode<'_>, first_item: (usize, usize)) -> usize {
        if self.style == Style::WithFixedIndentation {
            let method_off = self.target_method_offset(call);
            self.indentation_of_line(method_off) + self.indent
        } else {
            self.display_column(first_item.0)
        }
    }

    /// `target_method_lineno`: the selector's offset, or the opening paren's when
    /// there is no selector (`l.(1)`). For `[]`/`[]=` the message loc spans the
    /// whole bracket; its start still sits on the receiver line.
    fn target_method_offset(&self, call: &CallNode<'_>) -> usize {
        if let Some(sel) = call.message_loc() {
            sel.start_offset()
        } else if let Some(open) = call.opening_loc() {
            open.start_offset()
        } else {
            call.location().start_offset()
        }
    }

    /// `check_alignment` + `each_bad_alignment`.
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
                    self.offenses.push(ArgAlignOffense {
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

/// If `node` is a braceless hash (Prism `KeywordHashNode`), its `pairs`
/// (`AssocNode` ranges; kwsplats excluded). `None` for anything else.
fn braceless_hash_pairs(node: &Node<'_>) -> Option<Vec<(usize, usize)>> {
    let kh = node.as_keyword_hash_node()?;
    Some(
        kh.elements()
            .iter()
            .filter_map(|e| e.as_assoc_node().map(|a| loc(&a.as_node().location())))
            .collect(),
    )
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(c) = node.as_call_node() {
            self.process_send(&c);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Vec<(usize, usize, isize, bool)> {
        check_argument_alignment(source.as_bytes(), style, 2, false)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.autocorrect))
            .collect()
    }

    #[test]
    fn with_first_argument_misaligned() {
        // `b` under-indented relative to `a` (col 5).
        let got = run("func(a,\n  b,\nc)\n", 0);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].2, 3); // `b` at col 2 -> +3
        assert_eq!(got[1].2, 5); // `c` at col 0 -> +5
        assert!(got.iter().all(|o| o.3));
    }

    #[test]
    fn with_first_argument_aligned_is_clean() {
        assert!(run("func(a,\n     b,\n     c)\n", 0).is_empty());
    }

    #[test]
    fn fixed_indentation_outdent() {
        // args aligned with first arg (col 7) but fixed wants col 2.
        let got = run("create :a, :b,\n       account: x,\n       price: y\n", 1);
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|o| o.2 == -5));
    }

    #[test]
    fn skips_index_setter() {
        assert!(run("Test.config[\"x\"] =\n true\n", 0).is_empty());
    }

    #[test]
    fn braceless_hash_first_argument() {
        let got = run("func(foo: 'foo',\n  bar: 'bar')\n", 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, 3); // `bar` (col 2) aligns under `foo` (col 5)
    }
}
