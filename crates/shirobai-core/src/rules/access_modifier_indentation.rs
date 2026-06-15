//! `Layout/AccessModifierIndentation`.
//!
//! Stock's logic (`vendor/rubocop/lib/rubocop/cop/layout/access_modifier_indentation.rb`):
//!
//! - `on_class` / `on_sclass` / `on_module` / `on_block` (a shared alias chain
//!   in stock) inspect `node.body` and bail when it is not a `(begin ...)` —
//!   the `body.begin_type?` guard. So `class A; private; end` (single send
//!   body) and `class A; if ... end; end` (`if` body) trigger nothing, while
//!   `class A; class_attribute :x; private; ... end` does.
//! - From a `begin` body it picks every direct child whose `:send` node is a
//!   `bare_access_modifier?` — a `(send nil? {:public :protected :private
//!   :module_function})` with no arguments — and skips ones on the same line
//!   as the class/module/block header (`same_line?(node, modifier)`).
//! - For each, `column_offset_between(modifier.source_range, node.loc.end)`
//!   gives the modifier's column relative to the construct's `end` keyword's
//!   column (in `effective_column` terms — BOM-adjusted). The expected offset
//!   is `IndentationWidth` under `indent` style, or `0` under `outdent`.
//!   `column_delta = expected - actual`. When zero, `correct_style_detected`;
//!   otherwise the modifier is flagged and `AlignmentCorrector.correct(...,
//!   column_delta)` shifts the line by `column_delta`.
//!
//! Reproduced here in one shared-walk pass: on every class-like / block node,
//! reach into its `(begin ...)` body, walk only its direct statement children
//! for bare access modifiers, compute `column_delta`, and emit one record per
//! modifier (with or without an `Offense`). Same-line modifiers and modifiers
//! whose enclosing body is not a parser-`begin` (prism `StatementsNode` with
//! `>1` statements) are skipped — exactly matching stock's `each_child_node`
//! on a `(begin ...)` body. For a `(kwbegin ...)` (`begin ... end`) body and
//! for single-statement bodies the prism shape is, respectively, `BeginNode`
//! and a bare statement node, neither of which triggers `each_child_node(:send)`
//! on stock — and neither does here.

use std::rc::Rc;

use ruby_prism::{
    BlockNode, ClassNode, LambdaNode, ModuleNode, Node, SingletonClassNode, StatementsNode,
};

use super::line_index::LineIndex;

/// `EnforcedStyle`: 0 = `indent`, 1 = `outdent`.
pub const STYLE_INDENT: u8 = 0;
pub const STYLE_OUTDENT: u8 = 1;

#[derive(Clone, Copy)]
pub struct Config {
    pub style: u8,
    /// `Layout/IndentationWidth` `Width`, or this cop's own `IndentationWidth`
    /// override when configured (the Ruby `configured_indentation_width`).
    pub indentation_width: usize,
}

/// One inspected bare access modifier, in walk order. `column_delta` is
/// `expected - actual` (matches stock's `@column_delta`). `Some(Offense)` when
/// non-zero (the wrapper then emits an offense and an `AlignmentCorrector`
/// shift); `None` means the modifier already aligns with the configured style
/// (the wrapper calls `correct_style_detected`).
pub struct AccessModifierIndentationRecord {
    /// `bare_access_modifier?` send range (offense highlight).
    pub start: usize,
    pub end: usize,
    /// `None` when the modifier matches the configured style.
    pub offense: Option<AccessModifierIndentationOffense>,
}

pub struct AccessModifierIndentationOffense {
    /// Formatted stock `MSG`: `"<Style> access modifiers like `<name>`."`.
    pub message: String,
    /// Signed `expected - actual` column delta for `AlignmentCorrector.correct`.
    pub column_delta: i64,
}

pub fn check_access_modifier_indentation(
    source: &[u8],
    config: Config,
) -> Vec<AccessModifierIndentationRecord> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.records
}

pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        line_index,
        config,
        records: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    config: Config,
    pub(crate) records: Vec<AccessModifierIndentationRecord>,
}

impl<'a> Visitor<'a> {
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn column(&self, off: usize) -> usize {
        self.line_index.column(self.source, off)
    }

    /// `RangeHelp#effective_column`: subtract one when the offset is on line 1
    /// and the source begins with a UTF-8 BOM.
    fn effective_column(&self, off: usize) -> usize {
        let col = self.column(off);
        if self.line_of(off) == 1 && self.source.starts_with(&[0xef, 0xbb, 0xbf]) && col > 0 {
            col - 1
        } else {
            col
        }
    }

    fn modifier_name(&self, start: usize, end: usize) -> &str {
        std::str::from_utf8(&self.source[start..end]).unwrap_or("")
    }

    fn style_word(&self) -> &'static str {
        if self.config.style == STYLE_INDENT {
            "Indent"
        } else {
            "Outdent"
        }
    }

    /// `expected_indent_offset`: `IndentationWidth` under `indent`, `0` under
    /// `outdent`.
    fn expected_indent_offset(&self) -> i64 {
        if self.config.style == STYLE_INDENT {
            self.config.indentation_width as i64
        } else {
            0
        }
    }

    /// Inspect a class-like / block body. `body` is the prism node a stock
    /// `node.body` would yield. `end_anchor` is the offset of the construct's
    /// `end` keyword (or `}` for a brace block / lambda) — `node.loc.end` in
    /// stock — whose `effective_column` is the alignment reference.
    /// `header_line` is the line of the construct's opening (`node`'s start) —
    /// stock's `same_line?(node, modifier)` skips modifiers on it.
    fn check_body(&mut self, body: Option<Node<'_>>, end_anchor: usize, header_line: usize) {
        // `body.begin_type?` in stock — parser only emits `(begin ...)` when
        // the body contains >= 2 statements. Prism, in contrast, *always*
        // wraps a class/module/sclass/block body in `StatementsNode`, even for
        // a single statement. So we must check >= 2 children here to mirror
        // parser's `begin_type?`: a single-statement body is parser-`send`
        // (or whatever the bare type is), where `each_child_node(:send)`
        // never fires. An explicit `begin ... end` is a `(kwbegin (begin ...))`
        // in parser and a `BeginNode` in prism — both skip the body check.
        let Some(body) = body else { return };
        let Some(stmts) = body.as_statements_node() else {
            return;
        };
        if stmts.body().iter().count() < 2 {
            return;
        }
        self.scan_statements(&stmts, end_anchor, header_line);
    }

    fn scan_statements(
        &mut self,
        stmts: &StatementsNode<'_>,
        end_anchor: usize,
        header_line: usize,
    ) {
        let end_col_eff = self.effective_column(end_anchor);
        let expected = self.expected_indent_offset();
        for child in stmts.body().iter() {
            let Some(call) = child.as_call_node() else {
                continue;
            };
            // `bare_access_modifier?` = no receiver, no arguments, no block,
            // and name in {public, protected, private, module_function}. Stock
            // also requires `macro?`, but stock's `each_child_node` is reading
            // the direct child of a class/module/sclass/block body — exactly
            // the macro scope, by definition — so the macro check is
            // redundant here.
            if call.receiver().is_some()
                || call.arguments().is_some()
                || call.block().is_some()
            {
                continue;
            }
            let name = call.name();
            let name_bytes = name.as_slice();
            if !matches!(
                name_bytes,
                b"public" | b"protected" | b"private" | b"module_function"
            ) {
                continue;
            }
            let loc = call.location();
            let start = loc.start_offset();
            let end = loc.end_offset();
            // `same_line?(node, modifier)`: skip when the modifier is on the
            // same line as the class/module/block header.
            if self.line_of(start) == header_line {
                continue;
            }
            let modifier_col_eff = self.effective_column(start) as i64;
            let column_delta = expected - (modifier_col_eff - end_col_eff as i64);
            let offense = if column_delta == 0 {
                None
            } else {
                let message = format!(
                    "{} access modifiers like `{}`.",
                    self.style_word(),
                    self.modifier_name(start, end),
                );
                Some(AccessModifierIndentationOffense {
                    message,
                    column_delta,
                })
            };
            self.records
                .push(AccessModifierIndentationRecord { start, end, offense });
        }
    }
}

// Per-node dispatch. Each class-like / block node passes its body and end
// anchor through `check_body`; we never recurse into anything ourselves because
// the shared walker drives `enter` on every branch node — nested classes /
// blocks are handled by their own `enter` event.
impl<'a> Visitor<'a> {
    fn handle_class(&mut self, n: &ClassNode<'_>) {
        let header_line = self.line_of(n.location().start_offset());
        let end = n.end_keyword_loc().start_offset();
        self.check_body(n.body(), end, header_line);
    }

    fn handle_module(&mut self, n: &ModuleNode<'_>) {
        let header_line = self.line_of(n.location().start_offset());
        let end = n.end_keyword_loc().start_offset();
        self.check_body(n.body(), end, header_line);
    }

    fn handle_sclass(&mut self, n: &SingletonClassNode<'_>) {
        let header_line = self.line_of(n.location().start_offset());
        let end = n.end_keyword_loc().start_offset();
        self.check_body(n.body(), end, header_line);
    }

    fn handle_block(&mut self, n: &BlockNode<'_>) {
        // `node.loc.end` for parser blocks is the `end` or `}` token. Prism
        // gives us `opening_loc` and `closing_loc`; the closer is what we want.
        let opener = n
            .parameters()
            .as_ref()
            .map(|p| p.location().start_offset())
            .unwrap_or(n.opening_loc().start_offset());
        // `same_line?(node, modifier)` in parser refers to the call expression
        // the block decorates — which spans the call *and* the block's
        // opening. The block node's own start in prism is the opener (`do` /
        // `{`), so using the opener line is the closer match. But stock keys
        // off the parser block node's start (call expression start). To stay
        // faithful, we look at the parent (the parser block node).
        // Pragmatic call: use the opener line. With multi-line call chains the
        // call expression and the opener share the same line in real-world
        // source overwhelmingly often; if they ever diverge stock's
        // `same_line?` check is itself ambiguous (parser block node's line is
        // the call's line, not the opener's). We keep this consistent and
        // re-evaluate if a corpus diverges.
        let _ = opener; // currently unused; documents the consideration.
        let header_line = self.line_of(n.location().start_offset());
        let end = n.closing_loc().start_offset();
        self.check_body(n.body(), end, header_line);
    }

    fn handle_lambda(&mut self, n: &LambdaNode<'_>) {
        // Stock aliases `on_block` for `on_block`-equivalents. parser-gem
        // surfaces `-> { ... }` as a `block` node (an `each_child_node(:send)`
        // matches it), so we need to handle prism's separate `LambdaNode` too.
        let header_line = self.line_of(n.location().start_offset());
        let end = n.closing_loc().start_offset();
        self.check_body(n.body(), end, header_line);
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(n) = node.as_class_node() {
            self.handle_class(&n);
        } else if let Some(n) = node.as_module_node() {
            self.handle_module(&n);
        } else if let Some(n) = node.as_singleton_class_node() {
            self.handle_sclass(&n);
        } else if let Some(n) = node.as_block_node() {
            self.handle_block(&n);
        } else if let Some(n) = node.as_lambda_node() {
            self.handle_lambda(&n);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8, width: usize) -> Vec<AccessModifierIndentationRecord> {
        check_access_modifier_indentation(
            source.as_bytes(),
            Config {
                style,
                indentation_width: width,
            },
        )
    }

    #[test]
    fn indent_correctly_indented_no_offense() {
        let r = run("class A\n  X = 1\n  private\nend\n", STYLE_INDENT, 2);
        assert_eq!(r.len(), 1);
        assert!(r[0].offense.is_none());
    }

    #[test]
    fn indent_misaligned_private_zero_column() {
        let r = run("class A\n  X = 1\nprivate\nend\n", STYLE_INDENT, 2);
        assert_eq!(r.len(), 1);
        let off = r[0].offense.as_ref().unwrap();
        assert_eq!(off.column_delta, 2);
        assert!(off.message.contains("Indent access modifiers like `private`"));
    }

    #[test]
    fn outdent_misaligned_private_at_indent() {
        let r = run(
            "class A\n  X = 1\n  private\nend\n",
            STYLE_OUTDENT,
            2,
        );
        assert_eq!(r.len(), 1);
        let off = r[0].offense.as_ref().unwrap();
        assert_eq!(off.column_delta, -2);
        assert!(off.message.contains("Outdent access modifiers like `private`"));
    }

    #[test]
    fn same_line_modifier_skipped() {
        let r = run("class A; private; X = 1\nend\n", STYLE_INDENT, 2);
        // `same_line?` skips modifiers on the header line.
        assert!(r.is_empty());
    }

    #[test]
    fn single_statement_body_skipped() {
        // body is a bare send, not a begin → nothing emitted.
        let r = run("class A\nprivate\nend\n", STYLE_INDENT, 2);
        assert!(r.is_empty());
    }

    #[test]
    fn kwbegin_body_skipped() {
        // `class A; begin; private; def x; end; end; end`: body is a
        // `BeginNode`, not a `StatementsNode` directly.
        let r = run(
            "class A\n  begin\n  private\n    def x; end\n  end\nend\n",
            STYLE_INDENT,
            2,
        );
        assert!(r.is_empty());
    }

    #[test]
    fn modifier_with_argument_skipped() {
        let r = run(
            "class A\n  X = 1\n  private :foo\nend\n",
            STYLE_INDENT,
            2,
        );
        assert!(r.is_empty());
    }

    #[test]
    fn block_body_inspected() {
        // `Test = Class.new do; private; ...; end` — block.
        let r = run(
            "Test = Class.new do\n  X = 1\nprivate\n  def x; end\nend\n",
            STYLE_INDENT,
            2,
        );
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].offense.as_ref().unwrap().column_delta, 2);
    }

    #[test]
    fn module_function() {
        let r = run(
            "module M\n  X = 1\n  module_function\nend\n",
            STYLE_INDENT,
            2,
        );
        assert_eq!(r.len(), 1);
        assert!(r[0].offense.is_none());
    }

    #[test]
    fn singleton_class() {
        let r = run(
            "class << self\n  X = 1\nprivate\n  def x; end\nend\n",
            STYLE_INDENT,
            2,
        );
        assert_eq!(r.len(), 1);
        let off = r[0].offense.as_ref().unwrap();
        assert_eq!(off.column_delta, 2);
    }

    #[test]
    fn nested_classes_each_independent() {
        let src = "class Outer\n  class Inner\n  private\n    def x; end\n  end\nend\n";
        let r = run(src, STYLE_INDENT, 2);
        assert_eq!(r.len(), 1);
        let off = r[0].offense.as_ref().unwrap();
        // `class Inner` (col 2) `end` (col 2) — modifier `private` at col 2,
        // offset = 0, expected = 2 → delta = 2.
        assert_eq!(off.column_delta, 2);
    }

    #[test]
    fn override_indentation_width() {
        // outdented `private` is acceptable under outdent with width=4.
        let r = run(
            "class A\n  X = 1\nprivate\n    def x; end\nend\n",
            STYLE_OUTDENT,
            4,
        );
        assert_eq!(r.len(), 1);
        assert!(r[0].offense.is_none());
    }
}

