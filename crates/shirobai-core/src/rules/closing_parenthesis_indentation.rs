//! `Layout/ClosingParenthesisIndentation`.
//!
//! Checks the indentation of hanging closing parentheses (a `)` preceded by a
//! line break) in method calls, method definitions and grouped expressions.
//! Same `AlignmentCorrector` division of labour as the other alignment cops:
//! Rust computes the offense range (the `)` token), the `column_delta` and the
//! message; Ruby applies the realignment via `AlignmentCorrector` over the same
//! range, exactly like stock (`autocorrect` passes `right_paren` itself).
//!
//! Columns are parser-gem columns: character counts from the line start (`)`
//! and `(` come from `Source::Range#column`, indents from
//! `ProcessedSource#line_indentation`), not display width.

use std::rc::Rc;

use ruby_prism::Node;

use super::line_index::LineIndex;

/// One misindented hanging `)`. `[start_offset, end_offset)` is the closing
/// paren token (the offense range and the range Ruby realigns by
/// `column_delta`).
pub struct ClosingParenIndentOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub message: String,
}

pub fn check_closing_parenthesis_indentation(
    source: &[u8],
    indent_width: usize,
) -> Vec<ClosingParenIndentOffense> {
    let mut rule = build_rule(source, indent_width);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle. The check is
/// stateless per node (no ancestor stack), so the plain full walk fits.
pub(crate) fn build_rule(source: &[u8], indent_width: usize) -> Visitor<'_> {
    Visitor {
        source,
        line_index: super::line_index::with_line_index(source, |li| li.clone()),
        indent: indent_width,
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    indent: usize,
    pub(crate) offenses: Vec<ClosingParenIndentOffense>,
}

/// One element between the parens: an argument, a def parameter or a grouped
/// statement. `hash_child_starts` carries the start offsets of the children
/// (pairs / kwsplats) when the element is a hash, for the
/// `all_elements_aligned?` special case on the first element.
struct Element {
    start: usize,
    hash_child_starts: Option<Vec<usize>>,
}

fn element_of(node: &Node<'_>) -> Element {
    let hash_child_starts = if let Some(h) = node.as_hash_node() {
        Some(
            h.elements()
                .iter()
                .map(|e| e.location().start_offset())
                .collect(),
        )
    } else {
        node.as_keyword_hash_node().map(|h| {
            h.elements()
                .iter()
                .map(|e| e.location().start_offset())
                .collect()
        })
    };
    Element {
        start: node.location().start_offset(),
        hash_child_starts,
    }
}

/// Ruby regex `\s` (the line-local subset; `\n` cannot occur inside a line).
fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\x0b' | b'\x0c' | b'\r')
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_ALL,
        )
    }
    
    fn enter(&mut self, node: &Node<'_>) {
        self.dispatch(node);
    }

    fn leave(&mut self) {}
}

impl Visitor<'_> {
    fn dispatch(&mut self, node: &Node<'_>) {
        if let Some(c) = node.as_call_node() {
            // `on_send` / `on_csend`: only parenthesized calls have `loc.begin`.
            // The `(` check excludes index sends (`a[1]` is `:index` under
            // rubocop-ast's `emit_index`, never dispatched to `on_send`).
            let Some(open) = c.opening_loc() else { return };
            if open.as_slice() != b"(" {
                return;
            }
            let Some(close) = c.closing_loc() else { return };
            // `node.arguments`: parser includes a `block_pass` (`&blk`) as the
            // last argument; Prism keeps it in the separate `block` field.
            let mut elements: Vec<Element> = c
                .arguments()
                .map(|args| args.arguments().iter().map(|a| element_of(&a)).collect())
                .unwrap_or_default();
            if let Some(block) = c.block()
                && block.as_block_argument_node().is_some()
            {
                elements.push(element_of(&block));
            }
            let node_start = c.as_node().location().start_offset();
            self.check(
                node_start,
                open.start_offset(),
                close.start_offset(),
                close.end_offset(),
                &elements,
            );
        } else if let Some(d) = node.as_def_node() {
            // `on_def` / `on_defs` check the parameter list's parens. Stock
            // passes the `args` node, whose source range starts at `(`, so the
            // empty-parens candidate column equals the `(` column.
            let (Some(open), Some(close)) = (d.lparen_loc(), d.rparen_loc()) else {
                return;
            };
            let mut elements: Vec<Element> = Vec::new();
            if let Some(params) = d.parameters() {
                for p in &params.requireds() {
                    elements.push(element_of(&p));
                }
                for p in &params.optionals() {
                    elements.push(element_of(&p));
                }
                if let Some(p) = params.rest() {
                    elements.push(element_of(&p));
                }
                for p in &params.posts() {
                    elements.push(element_of(&p));
                }
                for p in &params.keywords() {
                    elements.push(element_of(&p));
                }
                if let Some(p) = params.keyword_rest() {
                    elements.push(element_of(&p));
                }
                if let Some(p) = params.block() {
                    elements.push(element_of(&p.as_node()));
                }
                // Source order (parser's `args` children are positional).
                elements.sort_by_key(|e| e.start);
            }
            self.check(
                open.start_offset(),
                open.start_offset(),
                close.start_offset(),
                close.end_offset(),
                &elements,
            );
        } else if let Some(p) = node.as_parentheses_node() {
            // `on_begin`: grouped expressions. parser materialises a `begin`
            // node for round parens only (keyword `begin` is `kwbegin`, multiple
            // assignment targets are `mlhs`), which is exactly Prism's
            // `ParenthesesNode`.
            let elements: Vec<Element> = match p.body() {
                None => Vec::new(),
                Some(b) => match b.as_statements_node() {
                    Some(st) => st.body().iter().map(|n| element_of(&n)).collect(),
                    None => vec![element_of(&b)],
                },
            };
            let node_start = p.as_node().location().start_offset();
            self.check(
                node_start,
                p.opening_loc().start_offset(),
                p.closing_loc().start_offset(),
                p.closing_loc().end_offset(),
                &elements,
            );
        } else if let Some(e) = node.as_embedded_statements_node() {
            // `on_begin` also fires for parser's `:begin` node materialised
            // around a string/regexp/symbol interpolation `#{...}` — same node
            // type as a parenthesised expression, with `loc.begin == "#{"` and
            // `loc.end == "}"`. Stock checks indentation of the closing `}`
            // exactly like a hanging `)` (message hard-codes `)` even so), and
            // its `AlignmentCorrector` realigns the `}` token. Prism keeps the
            // interpolation as a separate `EmbeddedStatementsNode`, but the
            // opening/closing locs map to parser's `loc.begin` / `loc.end`
            // verbatim and the inner statements map to `node.children`. This
            // is what Redmine `redcloth3.rb:775` trips on.
            let elements: Vec<Element> = e
                .statements()
                .map(|st| st.body().iter().map(|n| element_of(&n)).collect())
                .unwrap_or_default();
            let node_start = e.as_node().location().start_offset();
            self.check(
                node_start,
                e.opening_loc().start_offset(),
                e.closing_loc().start_offset(),
                e.closing_loc().end_offset(),
                &elements,
            );
        }
    }

    /// `check(node, elements)` over resolved paren offsets. `node_start` is the
    /// start of the node whose `loc.column` feeds the no-elements candidates.
    fn check(
        &mut self,
        node_start: usize,
        lp: usize,
        rp: usize,
        rp_end: usize,
        elements: &[Element],
    ) {
        if !self.begins_its_line(rp) {
            return;
        }
        let rp_col = self.column(rp);
        let lp_col = self.column(lp);

        let correct_column = if elements.is_empty() {
            // `correct_column_candidates`; the first is the specified correction.
            let candidates = [
                self.line_indentation_at(lp),
                lp_col,
                self.column(node_start),
            ];
            if candidates.contains(&rp_col) {
                return;
            }
            candidates[0]
        } else {
            self.expected_column(lp, lp_col, elements)
        };

        let column_delta = correct_column as isize - rp_col as isize;
        if column_delta == 0 {
            return;
        }

        let message = if correct_column == lp_col {
            "Align `)` with `(`.".to_string()
        } else {
            format!("Indent `)` to column {correct_column} (not {rp_col})")
        };
        self.offenses.push(ClosingParenIndentOffense {
            start_offset: rp,
            end_offset: rp_end,
            column_delta,
            message,
        });
    }

    /// `expected_column(left_paren, elements)`.
    fn expected_column(&self, lp: usize, lp_col: usize, elements: &[Element]) -> usize {
        let first = &elements[0];
        if self.line_index.line_of(first.start) > self.line_index.line_of(lp) {
            // Line break after `(`: outdent by the configured width (min 0).
            self.line_indentation_at(first.start)
                .saturating_sub(self.indent)
        } else if self.all_elements_aligned(elements) {
            lp_col
        } else {
            self.line_indentation_at(first.start)
        }
    }

    /// `all_elements_aligned?`: every column identical (`uniq.one?`; an empty
    /// column list — a first-element hash without children — is not aligned).
    fn all_elements_aligned(&self, elements: &[Element]) -> bool {
        let cols: Vec<usize> = if let Some(child_starts) = &elements[0].hash_child_starts {
            child_starts.iter().map(|&s| self.column(s)).collect()
        } else {
            elements.iter().map(|e| self.column(e.start)).collect()
        };
        match cols.split_first() {
            Some((first, rest)) => rest.iter().all(|c| c == first),
            None => false,
        }
    }

    /// `begins_its_line?`: only whitespace precedes `off` on its line.
    fn begins_its_line(&self, off: usize) -> bool {
        let ls = self.line_index.line_start(off);
        self.source[ls..off].iter().all(|&b| is_ws(b))
    }

    /// `processed_source.line_indentation(line)` for the line containing `off`:
    /// the length of the leading whitespace run (in characters; the run is
    /// ASCII, so bytes == chars).
    fn line_indentation_at(&self, off: usize) -> usize {
        let ls = self.line_index.line_start(off);
        self.source[ls..].iter().take_while(|&&b| is_ws(b)).count()
    }

    /// parser-gem `Source::Range#column`: character column within the line.
    fn column(&self, off: usize) -> usize {
        self.line_index.column(self.source, off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<(usize, usize, isize, String)> {
        check_closing_parenthesis_indentation(source.as_bytes(), 2)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.message))
            .collect()
    }

    #[test]
    fn method_call_with_line_break_after_paren() {
        // `)` must outdent to column 0 (first arg's indent 2 minus width 2).
        let got = run("some_method(\n  a\n  )\n");
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].2, got[0].3.as_str()),
            (-2, "Indent `)` to column 0 (not 2)")
        );
    }

    #[test]
    fn method_call_aligned_elements_align_with_paren() {
        let got = run("some_method(a\n)\n");
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].2, got[0].3.as_str()), (11, "Align `)` with `(`."));
    }

    #[test]
    fn accepts_correctly_indented_paren() {
        assert!(run("some_method(\n  a\n)\n").is_empty());
        assert!(run("some_method(a\n           )\n").is_empty());
        assert!(run("some_method()\n").is_empty());
    }

    #[test]
    fn keyword_hash_first_argument_uses_pair_columns() {
        // Pairs not aligned -> expected is the first argument line's indent (0).
        assert!(run("some_method(x: 1,\n  y: 2,\n  z: 3\n)\n").is_empty());
        let got = run("some_method(x: 1,\n  y: 2\n  )\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].3, "Indent `)` to column 0 (not 2)");
    }

    #[test]
    fn block_pass_is_an_element() {
        let got = run("foo(&blk\n  )\n");
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].2, got[0].3.as_str()), (1, "Align `)` with `(`."));
    }

    #[test]
    fn def_parameters() {
        let got = run("def some_method(a\n)\nend\n");
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].2, got[0].3.as_str()), (15, "Align `)` with `(`."));
        assert!(run("def some_method(\n  a\n)\nend\n").is_empty());
        assert!(run("def some_method()\nend\n").is_empty());
    }

    #[test]
    fn grouped_expression() {
        let got = run("w = x * (\n  y + z\n    )\n");
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].2, got[0].3.as_str()),
            (-4, "Indent `)` to column 0 (not 4)")
        );
        assert!(run("w = x * (y + z +\n        a)\n").is_empty());
    }

    #[test]
    fn empty_parens_candidates() {
        // Candidates: line indent of `(` line (0), `(` column, node column.
        assert!(run("foo = some_method(\n)\n").is_empty());
        assert!(run("foo = some_method(\n                 )\n").is_empty());
        assert!(run("foo = some_method(\n      )\n").is_empty());
        let got = run("foo = some_method(\n  )\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].3, "Indent `)` to column 0 (not 2)");
    }

    #[test]
    fn string_interpolation_closing_brace() {
        // Stock's `on_begin` also matches parser's `:begin` node around a
        // string/regexp interpolation `#{...}`, treating the `}` as a hanging
        // closing paren. Redmine `redcloth3.rb:775` form: a regexp with an
        // interpolation whose statements line is indented 8 from BOL while the
        // closing `}` is at column 4 — expected column = 8 - 2 = 6.
        let got = run("    re = /^(#{\n        x.join('|')\n    })$/\n");
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].2, got[0].3.as_str()), (2, "Indent `)` to column 6 (not 4)"));
        // No offense when the closing `}` already sits at the outdented column.
        assert!(run("    re = /^(#{\n        x.join('|')\n      })$/\n").is_empty());
    }

    #[test]
    fn index_send_and_super_are_not_checked() {
        assert!(run("foo[\n  1\n  ]\n").is_empty());
        assert!(run("super(\n  a\n  )\n").is_empty());
        assert!(run("yield(\n  a\n  )\n").is_empty());
        assert!(run("defined?(\n  a\n  )\n").is_empty());
    }
}
