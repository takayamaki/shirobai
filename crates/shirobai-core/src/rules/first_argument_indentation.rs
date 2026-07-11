//! `Layout/FirstArgumentIndentation`.
//!
//! Checks the indentation of the first argument of a multi-line method call.
//! Arguments after the first are checked by `Layout/ArgumentAlignment`. Same
//! alignment family / `AlignmentCorrector` division of labour as
//! `Layout/ArgumentAlignment` and `Layout/MultilineMethodCallIndentation`: Rust
//! computes the offense range (the first argument), the `column_delta`, the
//! message, the `within?` autocorrect flag and the range to realign (either the
//! first argument, or the whole receiver chain for
//! `special_for_inner_method_call_in_parentheses`); Ruby applies it via
//! `AlignmentCorrector`.

use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// One misindented first argument. `[start_offset, end_offset)` is the offense
/// range (the first argument). `[correct_start, correct_end)` is the range the
/// Ruby side realigns by `column_delta` (the whole chain when the entire call
/// should be corrected). `autocorrect` is false for offenses nested inside an
/// already-registered offense range.
pub struct FirstArgIndentOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub column_delta: isize,
    pub message: String,
    pub autocorrect: bool,
    pub correct_start: usize,
    pub correct_end: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    /// `special_for_inner_method_call_in_parentheses` (default).
    SpecialInParens,
    Consistent,
    ConsistentRelativeToReceiver,
    /// `special_for_inner_method_call`.
    Special,
}

impl Style {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Style::Consistent,
            2 => Style::ConsistentRelativeToReceiver,
            3 => Style::Special,
            _ => Style::SpecialInParens,
        }
    }
}

pub fn check_first_argument_indentation(
    source: &[u8],
    style: u8,
    indent_width: usize,
    enforce_fixed_with_no_line_break: bool,
) -> Vec<FirstArgIndentOffense> {
    let Some(mut rule) = build_rule(
        source,
        style,
        indent_width,
        enforce_fixed_with_no_line_break,
    ) else {
        return Vec::new();
    };
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle. `None` when
/// the cop is disabled outright (the `enforce_first_argument_with_fixed_indentation`
/// + no-line-break fast path handled on the Ruby side).
pub(crate) fn build_rule(
    source: &[u8],
    style: u8,
    indent_width: usize,
    enforce_fixed_with_no_line_break: bool,
) -> Option<Visitor<'_>> {
    if enforce_fixed_with_no_line_break {
        return None;
    }
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Some(Visitor {
        source,
        line_index: line_index.clone(),
        style: Style::from_u8(style),
        indent: indent_width,
        stack: Vec::new(),
        comment_lines: comment_lines(source, &line_index),
        offenses: Vec::new(),
    })
}

/// Lightweight ancestor frame. Wrapper kinds (`Arguments`/`Statements`/
/// `KeywordHash`) are transparent to `effective parent` lookups, mirroring the
/// nodes parser-gem does not materialise.
enum FrameKind {
    /// A `send` / `csend` / `super`. `name` empty for `super`.
    Call {
        name: Vec<u8>,
        recv: Option<(usize, usize)>,
        parenthesized: bool,
        /// Begin offset of `loc.end` (the closing paren, when present).
        end_loc_start: Option<usize>,
    },
    Splat,
    /// `arguments` / `statements` / braceless `keyword_hash` wrappers.
    Wrapper,
    Other,
}

struct Frame {
    start: usize,
    end: usize,
    kind: FrameKind,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    style: Style,
    indent: usize,
    stack: Vec<Frame>,
    /// 1-based line numbers of lines that are a comment beginning the line.
    comment_lines: Vec<usize>,
    pub(crate) offenses: Vec<FirstArgIndentOffense>,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

/// 1-based line numbers of comments that begin their line.
///
/// Comment positions come from the shared (cached) parse instead of a second
/// full re-parse, and line start / line number are resolved through the shared
/// [`LineIndex`].
fn comment_lines(source: &[u8], line_index: &LineIndex) -> Vec<usize> {
    let mut lines = Vec::new();
    for (start, _end) in super::parse_cache::comment_ranges(source) {
        let line_start = line_index.line_start(start);
        if source[line_start..start]
            .iter()
            .all(|&b| b == b' ' || b == b'\t')
        {
            lines.push(line_index.line_of(start));
        }
    }
    lines
}

impl<'a> Visitor<'a> {
    fn line_start(&self, off: usize) -> usize {
        self.line_index.line_start(off)
    }

    /// `Unicode::DisplayWidth.of(line[0, column])`.
    fn display_column(&self, off: usize) -> usize {
        self.line_index.display_column(self.source, off)
    }

    /// 1-based line number of `off`.
    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn begins_its_line(&self, off: usize) -> bool {
        let ls = self.line_start(off);
        self.source[ls..off]
            .iter()
            .all(|&b| b == b' ' || b == b'\t')
    }

    fn text(&self, s: usize, e: usize) -> &'a str {
        std::str::from_utf8(&self.source[s..e]).unwrap_or("")
    }

    /// The source line text for 1-based `line_number` (without trailing
    /// newline), or `None` past EOF.
    fn line_text(&self, line_number: usize) -> Option<&'a str> {
        if line_number == 0 {
            return None;
        }
        let mut start = 0;
        let mut cur = 1;
        let mut i = 0;
        while i < self.source.len() {
            if cur == line_number {
                let end = self.source[i..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map(|p| i + p)
                    .unwrap_or(self.source.len());
                return std::str::from_utf8(&self.source[start..end]).ok();
            }
            if self.source[i] == b'\n' {
                cur += 1;
                start = i + 1;
            }
            i += 1;
        }
        if cur == line_number {
            return std::str::from_utf8(&self.source[start..]).ok();
        }
        None
    }

    /// `previous_code_line(line_number) =~ /\S/`: the indentation column of the
    /// nearest *preceding* non-blank, non-comment line. Mirrors the cop's
    /// decrement-then-read loop (the starting line itself is never returned).
    fn previous_code_line_indent(&self, line_number: usize) -> usize {
        let mut ln = line_number;
        let mut line = String::new();
        while line.trim().is_empty() || self.comment_lines.contains(&ln) {
            if ln == 0 {
                return 0;
            }
            ln -= 1;
            line = self.line_text(ln).unwrap_or("").to_string();
        }
        line.find(|c: char| !c.is_whitespace()).unwrap_or(0)
    }

    /// `comment_line?(line)`: whether `line` (its content) is a comment line.
    fn is_comment_text(text: &str) -> bool {
        text.trim_start().starts_with('#')
    }

    /// `node.parent`: the effective parent of the call currently being processed
    /// (whose frame is not yet on the stack). The nearest ancestor parser-gem
    /// materialises — `arguments`/`statements`/`keyword_hash` wrappers are skipped.
    fn effective_parent(&self) -> Option<&Frame> {
        self.stack
            .iter()
            .rev()
            .find(|f| !matches!(f.kind, FrameKind::Wrapper))
    }

    /// `on_send` entry. `node_range` is the call's full range, `first_arg` the
    /// first argument, `name` the method name (empty for `super`), `has_dot`
    /// whether it is a `.`/`&.` call, `end_loc_start` the begin of `loc.end` (the
    /// closing paren, for the entire-chain correction check).
    #[allow(clippy::too_many_arguments)]
    fn process_call(
        &mut self,
        node_range: (usize, usize),
        first_arg: (usize, usize),
        name: &[u8],
        is_attribute_write: bool,
        is_operator: bool,
        has_dot: bool,
        end_loc_start: Option<usize>,
    ) {
        // should_check?: arguments? (guaranteed by caller) && !bare_operator? &&
        // !setter_method?.
        if (is_operator && !has_dot) || is_attribute_write {
            return;
        }
        // same_line?(node, first_argument).
        if self.line_of(node_range.0) == self.line_of(first_arg.0) {
            return;
        }

        let special = self.special_inner_call_indentation(node_range, name);
        let base_indent = if special {
            self.column_of(self.base_range(node_range, first_arg))
        } else {
            self.previous_code_line_indent(self.line_of(first_arg.0))
        };
        let indent = base_indent + self.indent;

        // check_alignment([first_argument], indent): a single item, on its own
        // line.
        if !self.begins_its_line(first_arg.0) {
            return;
        }
        let column_delta = indent as isize - self.display_column(first_arg.0) as isize;
        if column_delta == 0 {
            return;
        }

        // within? a previously-registered offense range -> report without
        // autocorrect, with the generic message.
        let within_prior = self
            .offenses
            .iter()
            .any(|o| first_arg.0 >= o.start_offset && first_arg.1 <= o.end_offset);

        let (message, correct, autocorrect) = if within_prior {
            (
                "Bad indentation of the first argument.".to_string(),
                first_arg,
                false,
            )
        } else {
            let msg = self.message(node_range, first_arg, special);
            let correct = self.correct_range(node_range, first_arg, column_delta, end_loc_start);
            (msg, correct, true)
        };

        self.offenses.push(FirstArgIndentOffense {
            start_offset: first_arg.0,
            end_offset: first_arg.1,
            column_delta,
            message,
            autocorrect,
            correct_start: correct.0,
            correct_end: correct.1,
        });
    }

    /// `special_inner_call_indentation?(node)`.
    fn special_inner_call_indentation(&self, node_range: (usize, usize), _name: &[u8]) -> bool {
        match self.style {
            Style::Consistent => return false,
            Style::ConsistentRelativeToReceiver => return true,
            _ => {}
        }

        let Some(parent) = self.effective_parent() else {
            return false;
        };
        // eligible_method_call?: (send _ !:[]= ...). The receiver position is a
        // wildcard, so only the method name matters.
        let FrameKind::Call {
            name: pname,
            parenthesized: pparen,
            ..
        } = &parent.kind
        else {
            return false;
        };
        // `super` is not a `send` in this matcher; also exclude `[]=`.
        if pname.is_empty() || pname.as_slice() == b"[]=" {
            return false;
        }
        if !*pparen && self.style == Style::SpecialInParens {
            return false;
        }
        // node must begin inside the parent (otherwise it is the first part of a
        // chained method call).
        node_range.0 > parent.start
    }

    /// `base_range(send_node, arg_node)`: from the start of `send_node` (or its
    /// splat/kwsplat parent) to the start of the argument.
    fn base_range(&self, node_range: (usize, usize), first_arg: (usize, usize)) -> (usize, usize) {
        let start = match self.effective_parent() {
            Some(f) if matches!(f.kind, FrameKind::Splat) => f.start,
            _ => node_range.0,
        };
        (start, first_arg.0)
    }

    /// `column_of(range)`.
    fn column_of(&self, range: (usize, usize)) -> usize {
        let source = self.text(range.0, range.1).trim();
        if source.contains('\n') {
            let line = self.line_of(range.0) + source.matches('\n').count() + 1;
            self.previous_code_line_indent(line)
        } else {
            self.display_column(range.0)
        }
    }

    /// `message(arg_node)`.
    fn message(
        &self,
        node_range: (usize, usize),
        first_arg: (usize, usize),
        special: bool,
    ) -> String {
        let br = self.base_range(node_range, first_arg);
        let raw = self.text(br.0, br.1);
        let text = raw.trim();
        let base = if !text.contains('\n') && special {
            format!("`{text}`")
        } else {
            // comment_line?(text.lines.reverse_each.first): the last line of the
            // stripped base range.
            let last_line = text.lines().next_back().unwrap_or("");
            if Self::is_comment_text(last_line) {
                "the start of the previous line (not counting the comment)".to_string()
            } else {
                "the start of the previous line".to_string()
            }
        };
        format!("Indent the first argument one step more than {base}.")
    }

    /// The range `AlignmentCorrector` realigns: `node_to_correct.source_range`.
    fn correct_range(
        &self,
        node_range: (usize, usize),
        first_arg: (usize, usize),
        column_delta: isize,
        node_end_loc_start: Option<usize>,
    ) -> (usize, usize) {
        // send_node = first_argument.parent = the call (node_range).
        let (top, top_end_loc) = self.find_top_level_send(node_range, node_end_loc_start);
        if self.should_correct_entire_chain(node_range, top, top_end_loc, column_delta) {
            top
        } else {
            // node_to_correct = node = first_argument. AlignmentCorrector over the
            // argument's range realigns its line(s).
            first_arg
        }
    }

    /// `find_top_level_send(send_node)`: climb up `.`-chained calls where the
    /// current node is the receiver. Returns the outermost such call's range and
    /// the begin offset of its `loc.end`.
    fn find_top_level_send(
        &self,
        send_range: (usize, usize),
        send_end_loc: Option<usize>,
    ) -> ((usize, usize), Option<usize>) {
        let mut top = send_range;
        let mut top_end_loc = send_end_loc;
        for f in self.stack.iter().rev() {
            match &f.kind {
                FrameKind::Wrapper => continue,
                FrameKind::Call {
                    recv,
                    end_loc_start,
                    ..
                } => {
                    // parent.receiver == top && parent.loc.dot
                    if recv.map(|r| r == top).unwrap_or(false) && self.frame_has_dot(f) {
                        top = (f.start, f.end);
                        top_end_loc = *end_loc_start;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        (top, top_end_loc)
    }

    fn frame_has_dot(&self, f: &Frame) -> bool {
        // A `.`/`&.` call has a receiver and a dot between the receiver end and
        // the selector.
        if let FrameKind::Call { recv: Some(r), .. } = &f.kind {
            let between = self.text(r.1, f.end);
            between.trim_start().starts_with('.') || between.trim_start().starts_with("&.")
        } else {
            false
        }
    }

    /// `should_correct_entire_chain?(send_node, top_level_send)`.
    fn should_correct_entire_chain(
        &self,
        send_range: (usize, usize),
        top: (usize, usize),
        top_end_loc: Option<usize>,
        column_delta: isize,
    ) -> bool {
        if self.style != Style::SpecialInParens {
            return false;
        }
        if !self.inner_call(top) {
            return false;
        }
        if (self.display_column(send_range.0) as isize) >= column_delta.abs() {
            return false;
        }
        // top_level_send != send_node || begins_its_line?(top_level_send.loc.end)
        top != send_range || self.begins_its_line(top_end_loc.unwrap_or(top.1))
    }

    /// `inner_call?(top_level_send)`: `top_level_send.parent` is a parenthesized
    /// send.
    fn inner_call(&self, top: (usize, usize)) -> bool {
        // The parent of the top-level send: nearest enclosing real frame that
        // contains `top` strictly.
        for f in self.stack.iter().rev() {
            if matches!(f.kind, FrameKind::Wrapper) {
                continue;
            }
            // Skip frames that are part of the chain itself (== top or inside).
            if f.start == top.0 && f.end == top.1 {
                continue;
            }
            return matches!(
                &f.kind,
                FrameKind::Call { parenthesized: true, name, .. } if !name.is_empty()
            );
        }
        false
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        self.dispatch(node);
        self.stack.push(self.make_frame(node));
    }

    fn leave(&mut self) {
        self.stack.pop();
    }
}

impl<'a> Visitor<'a> {
    fn dispatch(&mut self, node: &Node<'_>) {
        if let Some(c) = node.as_call_node() {
            // parser-gem's `send.arguments` includes the block-pass argument
            // (`&blk`, bare `&`) as the last argument, so a call whose ONLY
            // argument is a block-pass still has a first argument. prism
            // keeps it in `block()`, outside `arguments()`.
            let first = c
                .arguments()
                .and_then(|args| args.arguments().iter().next())
                .or_else(|| c.block().filter(|b| b.as_block_argument_node().is_some()));
            let Some(first) = first else { return };
            // `node.dot?`: a `.`/`&.` operator call.
            let has_dot = c.call_operator_loc().is_some();
            self.process_call(
                loc(&c.as_node().location()),
                loc(&first.location()),
                c.name().as_slice(),
                c.is_attribute_write(),
                is_operator_name(c.name().as_slice()),
                has_dot,
                c.closing_loc().map(|cl| cl.start_offset()),
            );
        } else if let Some(s) = node.as_super_node() {
            // Same block-pass mapping as calls: `super(&blk)`.
            let first = s
                .arguments()
                .and_then(|args| args.arguments().iter().next())
                .or_else(|| s.block().filter(|b| b.as_block_argument_node().is_some()));
            let Some(first) = first else { return };
            self.process_call(
                loc(&s.as_node().location()),
                loc(&first.location()),
                b"",
                false,
                false,
                false,
                s.rparen_loc().map(|cl| cl.start_offset()),
            );
        }
    }

    fn make_frame(&self, node: &Node<'_>) -> Frame {
        let l = node.location();
        let kind = if let Some(c) = node.as_call_node() {
            let paren = c
                .opening_loc()
                .map(|o| o.as_slice() == b"(")
                .unwrap_or(false);
            FrameKind::Call {
                name: c.name().as_slice().to_vec(),
                recv: c.receiver().map(|r| loc(&r.location())),
                parenthesized: paren,
                end_loc_start: c.closing_loc().map(|cl| cl.start_offset()),
            }
        } else if let Some(s) = node.as_super_node() {
            FrameKind::Call {
                name: Vec::new(),
                recv: None,
                parenthesized: s.lparen_loc().is_some(),
                end_loc_start: s.rparen_loc().map(|cl| cl.start_offset()),
            }
        } else if node.as_splat_node().is_some() || node.as_assoc_splat_node().is_some() {
            FrameKind::Splat
        } else if node.as_arguments_node().is_some()
            || node.as_statements_node().is_some()
            || node.as_keyword_hash_node().is_some()
        {
            FrameKind::Wrapper
        } else {
            FrameKind::Other
        };
        Frame {
            start: l.start_offset(),
            end: l.end_offset(),
            kind,
        }
    }
}

/// `operator_method?`: the method name is in stock's `OPERATOR_METHODS`
/// (rubocop-ast `method_identifier_predicates.rb`). That set includes `[]` and
/// `[]=`, so braceless index reads (`x[:foo]`) are bare operators that
/// `should_check?` filters out. Excluding `[]` here would cause shirobai to
/// false-positive on multi-line `x[\n  :sym\n]` patterns that stock silently
/// passes (Discourse `color_scheme.rb:436` modifier-if + index, etc.).
fn is_operator_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name
            .iter()
            .all(|&b| !b.is_ascii_alphanumeric() && b != b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Vec<(usize, usize, isize, String, bool)> {
        check_first_argument_indentation(source.as_bytes(), style, 2, false)
            .into_iter()
            .map(|o| {
                (
                    o.start_offset,
                    o.end_offset,
                    o.column_delta,
                    o.message,
                    o.autocorrect,
                )
            })
            .collect()
    }

    #[test]
    fn over_indented_first_argument() {
        // `:foo` at col 4 should be at col 2 (previous line `run(` at col 0 +2).
        let got = run("run(\n    :foo,\n    bar: 3\n)\n", 0);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, -2);
        assert!(
            got[0].3.contains("start of the previous line"),
            "{}",
            got[0].3
        );
    }

    #[test]
    fn accepts_first_arg_on_same_line() {
        assert!(run("run :foo,\n    bar: 3\n", 0).is_empty());
    }

    #[test]
    fn special_inner_call_in_parens() {
        // run(:foo, defaults.merge(\n   bar: 3))  -> base is `defaults.merge(`.
        let got = run(
            "run(:foo, defaults.merge(\n                        bar: 3))\n",
            0,
        );
        assert_eq!(got.len(), 1);
        assert!(got[0].3.contains("`defaults.merge(`"), "{}", got[0].3);
    }
}
