//! `Layout/SpaceBeforeFirstArg`.
//!
//! Checks that exactly one space separates the selector of a parenless call
//! from its first argument, unless the argument lines up with something on a
//! nearby line (`AllowForAlignment`, default on).
//!
//! Stock's `on_send` / `on_csend` is AST-shaped except for the
//! `PrecedingFollowingAlignment` mixin. Probed facts pinned here:
//!
//! - the whitespace run is `range_with_surrounding_space(side: :left)` —
//!   `[ \t]` then `\n`s — and ANY single character passes (`foo\tx` is fine);
//! - a glued argument (`something'x'`) is an offense with an empty range,
//!   skipping the alignment check entirely;
//! - the argument must sit on the NODE's first line (a receiver on an
//!   earlier line silences the cop);
//! - parser-gem counts a block-pass as an argument (`foo  &b` is an
//!   offense), while prism keeps it in `block`;
//! - operator methods (`x +  1`, `not  x`) and attribute writes
//!   (`a.b =  1`) are skipped;
//! - the alignment scan looks at the nearest non-blank line per direction
//!   whose first line is not a line-starting comment (comment and blank
//!   lines are transparent), first with no indent filter and then only at
//!   lines with the argument line's indentation;
//! - `aligned_words?` is pure line text: a `\s\S` boundary at the argument's
//!   column, or the argument's own text at that column (char columns);
//! - `aligned_equals_operator?` only matters when the argument's source ends
//!   with `=` (a `:sym=` first argument) or is `<<`: the first
//!   assignment-or-comparison TOKEN on the candidate line is found with a
//!   longest-match operator scan over unmasked bytes (`<=>` and `=~` are not
//!   in the set; `=`s inside strings do not count — probed), and its last
//!   column must equal the argument's.
//!
//! The opaque mask for that rare token scan is built lazily with its own
//! mini-walk; the shared-walk hook only collects candidates.

use ruby_prism::{Node, Visit};

use super::line_index::LineIndex;
use super::space_scan::surrounding_space_left;

/// Config for `Layout/SpaceBeforeFirstArg`.
#[derive(Clone, Copy)]
pub struct Config {
    pub allow_for_alignment: bool,
}

/// One offense: the whitespace run before the first argument (possibly
/// empty); the corrector replaces it with a single space.
pub type SpaceBeforeFirstArgOffense = (usize, usize);

pub fn check_space_before_first_arg(
    source: &[u8],
    config: Config,
) -> Vec<SpaceBeforeFirstArgOffense> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.into_offenses()
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    let comments = super::parse_cache::comment_ranges(source);
    let data_start = super::parse_cache::data_start(source);
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    Visitor {
        source,
        config,
        comments,
        data_start,
        line_index,
        candidates: Vec::new(),
    }
}

/// A call that failed the one-space test; `check_alignment` is false for the
/// glued (`no_space_between…`) and `AllowForAlignment: false` shapes, which
/// skip the alignment scan.
struct Candidate {
    space_start: usize,
    arg_start: usize,
    arg_end: usize,
    check_alignment: bool,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    config: Config,
    comments: Vec<(usize, usize)>,
    data_start: Option<usize>,
    line_index: std::rc::Rc<LineIndex>,
    candidates: Vec<Candidate>,
}

/// `OPERATOR_METHODS` from rubocop-ast (method names never checked).
fn is_operator_method(name: &[u8]) -> bool {
    matches!(
        name,
        b"|" | b"^"
            | b"&"
            | b"<=>"
            | b"=="
            | b"==="
            | b"=~"
            | b">"
            | b">="
            | b"<"
            | b"<="
            | b"<<"
            | b">>"
            | b"+"
            | b"-"
            | b"*"
            | b"/"
            | b"%"
            | b"**"
            | b"~"
            | b"+@"
            | b"-@"
            | b"!@"
            | b"~@"
            | b"[]"
            | b"[]="
            | b"!"
            | b"!="
            | b"!~"
            | b"`"
    )
}

impl<'a> Visitor<'a> {
    fn check_call(&mut self, call: &ruby_prism::CallNode<'_>, node_start: usize) {
        // `return if node.parenthesized?`
        if call.opening_loc().is_some() {
            return;
        }
        let Some(message) = call.message_loc() else {
            return;
        };
        // `regular_method_call_with_arguments?`: has arguments (parser-gem
        // counts a block-pass), not an operator method, not a setter.
        if is_operator_method(call.name().as_slice()) || call.is_attribute_write() {
            return;
        }
        let first = if let Some(first) = call
            .arguments()
            .and_then(|args| args.arguments().iter().next())
        {
            first.location()
        } else if let Some(block @ Node::BlockArgumentNode { .. }) = call.block() {
            block.location()
        } else {
            return;
        };
        let (arg_start, arg_end) = (first.start_offset(), first.end_offset());
        // `range_with_surrounding_space(side: :left)`.
        let space_start = surrounding_space_left(self.source, arg_start);
        if arg_start - space_start == 1 {
            return;
        }
        if message.end_offset() == arg_start {
            // `no_space_between_method_name_and_first_argument?`: offense
            // with an empty range; the alignment check is skipped.
            self.candidates.push(Candidate {
                space_start,
                arg_start,
                arg_end,
                check_alignment: false,
            });
            return;
        }
        // `same_line?(first_arg, node)` — the node's FIRST line.
        if self.line_index.line_of(node_start) != self.line_index.line_of(arg_start) {
            return;
        }
        self.candidates.push(Candidate {
            space_start,
            arg_start,
            arg_end,
            check_alignment: self.config.allow_for_alignment,
        });
    }

    pub(crate) fn into_offenses(self) -> Vec<SpaceBeforeFirstArgOffense> {
        let Visitor {
            source,
            config: _,
            comments,
            data_start,
            line_index,
            candidates,
        } = self;
        if candidates.is_empty() {
            return Vec::new();
        }
        let mut aligner = Aligner {
            source,
            line_index: &line_index,
            comment_lines: comment_begin_lines(source, &comments, &line_index),
            // `ProcessedSource#lines` stops at the `__END__` line.
            line_limit: match data_start {
                Some(ds) => line_index.line_of(ds).saturating_sub(1),
                None => line_index.line_starts().len(),
            },
            masks: None,
            comments,
            data_start,
        };
        candidates
            .into_iter()
            .filter(|c| !(c.check_alignment && aligner.aligned_with_something(c)))
            .map(|c| (c.space_start, c.arg_start))
            .collect()
    }
}

/// 1-based lines whose first non-whitespace content is a comment start
/// (stock's `aligned_comment_lines`).
fn comment_begin_lines(
    source: &[u8],
    comments: &[(usize, usize)],
    line_index: &LineIndex,
) -> std::collections::HashSet<usize> {
    comments
        .iter()
        .filter(|&&(cs, _)| super::trailing_comma::begins_its_line(source, line_index, cs))
        .map(|&(cs, _)| line_index.line_of(cs))
        .collect()
}

struct Aligner<'a> {
    source: &'a [u8],
    line_index: &'a LineIndex,
    comment_lines: std::collections::HashSet<usize>,
    /// Number of lines stock's `processed_source.lines` exposes.
    line_limit: usize,
    /// Opaque mask for the assignment-token scan, built on first use.
    masks: Option<Vec<(usize, usize)>>,
    comments: Vec<(usize, usize)>,
    data_start: Option<usize>,
}

/// The `ASSIGNMENT_OR_COMPARISON_TOKENS` kinds that matter downstream.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AsgnKind {
    /// `tEQL` / `tOP_ASGN` (both are `equal_sign?`).
    EqualSign,
    /// `tEQ` / `tEQQ` / `tNEQ` / `tLEQ` / `tGEQ` (comparisons).
    Comparison,
    /// `tLSHFT`.
    Lshft,
}

impl<'a> Aligner<'a> {
    /// `aligned_with_something?(first_arg.source_range)`.
    fn aligned_with_something(&mut self, c: &Candidate) -> bool {
        let arg_line = self.line_index.line_of(c.arg_start);
        // Second pass: only lines indented like the argument's line.
        let own_start = self.line_index.line_start(c.arg_start);
        let own_end = self.line_end(arg_line);
        let base_indent = self.source[own_start..own_end]
            .iter()
            .position(|&b| !is_ruby_space_no_nl(b))
            .unwrap_or(own_end - own_start);
        for indent in [None, Some(base_indent)] {
            // `pre`: nearest line above; `post`: nearest line below.
            for dir in [Direction::Up, Direction::Down] {
                if self.direction_result(c, arg_line, dir, indent) {
                    return true;
                }
            }
        }
        false
    }

    /// `aligned_with_line?`: the FIRST line in the direction that is not a
    /// line-starting comment, not blank (and matches `indent` when given)
    /// decides; absent lines (past `line_limit`) are transparent like blanks.
    fn direction_result(
        &mut self,
        c: &Candidate,
        arg_line: usize,
        dir: Direction,
        indent: Option<usize>,
    ) -> bool {
        let mut line = arg_line;
        loop {
            match dir {
                Direction::Up => {
                    if line == 1 {
                        return false;
                    }
                    line -= 1;
                }
                Direction::Down => {
                    line += 1;
                    if line > self.line_index.line_starts().len() {
                        return false;
                    }
                }
            }
            if self.comment_lines.contains(&line) {
                continue;
            }
            if line > self.line_limit {
                continue;
            }
            let ls = self.line_index.line_starts()[line - 1];
            let le = self.line_end(line);
            let Some(first_non_ws) = self.source[ls..le]
                .iter()
                .position(|&b| !is_ruby_space_no_nl(b))
            else {
                continue; // blank line
            };
            if let Some(ind) = indent
                && ind != first_non_ws
            {
                continue;
            }
            return self.aligned_token(c, line, ls, le);
        }
    }

    /// `aligned_token?` = `aligned_words?` || `aligned_equals_operator?`.
    fn aligned_token(&mut self, c: &Candidate, line: usize, ls: usize, le: usize) -> bool {
        let left_edge = self.line_index.column(self.source, c.arg_start);
        let line_bytes = &self.source[ls..le];
        // `/\s\S/.match?(line[left_edge - 1, 2])`
        if left_edge >= 1
            && let Some(bp) = char_index_to_byte(line_bytes, left_edge - 1)
            && bp < line_bytes.len()
            && is_ruby_space_no_nl(line_bytes[bp])
        {
            let next = bp + 1; // the space is one byte
            if next < line_bytes.len() && !is_ruby_space_no_nl(line_bytes[next]) {
                return true;
            }
        }
        // `token == line[left_edge, token.length]`
        let argb = &self.source[c.arg_start..c.arg_end];
        if let Some(bp) = char_index_to_byte(line_bytes, left_edge)
            && bp + argb.len() <= line_bytes.len()
            && &line_bytes[bp..bp + argb.len()] == argb
        {
            return true;
        }
        self.aligned_equals_operator(c, line)
    }

    /// `aligned_equals_operator?`: gated on the argument's own text, then
    /// resolved against the first assignment-or-comparison token on `line`.
    fn aligned_equals_operator(&mut self, c: &Candidate, line: usize) -> bool {
        let argb = &self.source[c.arg_start..c.arg_end];
        let ends_eq = argb.last() == Some(&b'=');
        let is_lshift = argb == b"<<";
        if !ends_eq && !is_lshift {
            return false;
        }
        let ls = self.line_index.line_starts()[line - 1];
        let le = self.line_end(line);
        if self.masks.is_none() {
            self.masks = Some(build_masks(self.source, &self.comments, self.data_start));
        }
        let masks = self.masks.as_ref().unwrap();
        let Some((kind, _tok_start, tok_end)) = first_asgn_token(self.source, ls, le, masks)
        else {
            return false;
        };
        if self.line_index.column(self.source, tok_end)
            != self.line_index.column(self.source, c.arg_end)
        {
            return false;
        }
        // `aligned_with_preceding_equals?` accepts any listed token when the
        // argument ends with `=`; `aligned_with_append_operator?` adds the
        // `<<`-argument-vs-equal-sign pairing (its `tLSHFT` clause is
        // subsumed by the first test).
        ends_eq || (is_lshift && kind == AsgnKind::EqualSign)
    }

    /// End of 1-based `line`'s content (before its `\n`, or EOF).
    fn line_end(&self, line: usize) -> usize {
        let starts = self.line_index.line_starts();
        if line < starts.len() {
            starts[line] - 1
        } else {
            self.source.len()
        }
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Up,
    Down,
}

/// Ruby `/\s/` minus the newline (line content never contains one).
fn is_ruby_space_no_nl(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c)
}

/// Byte offset of the `idx`-th character of `line` (UTF-8; invalid bytes fall
/// back to byte counting like `LineIndex::column`). `None` when the line has
/// fewer characters.
fn char_index_to_byte(line: &[u8], idx: usize) -> Option<usize> {
    match std::str::from_utf8(line) {
        Ok(s) => {
            let mut count = 0;
            for (bp, _) in s.char_indices() {
                if count == idx {
                    return Some(bp);
                }
                count += 1;
            }
            if count == idx { Some(line.len()) } else { None }
        }
        Err(_) => {
            if idx <= line.len() {
                Some(idx)
            } else {
                None
            }
        }
    }
}

/// Build the opaque mask with a dedicated mini-walk (this path is gated on a
/// `:sym=` / `<<` first argument, so it almost never runs).
fn build_masks(
    source: &[u8],
    comments: &[(usize, usize)],
    data_start: Option<usize>,
) -> Vec<(usize, usize)> {
    struct MaskWalker {
        masks: Vec<(usize, usize)>,
    }
    impl<'pr> Visit<'pr> for MaskWalker {
        fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
            crate::rules::opaque_mask::collect_enter(&node, &mut self.masks);
        }
        fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
            crate::rules::opaque_mask::collect_leaf(&node, &mut self.masks);
        }
    }
    let mut walker = MaskWalker { masks: Vec::new() };
    super::parse_cache::with_parsed(source, |_src, root| walker.visit(root));
    super::opaque_mask::merge(walker.masks, comments, data_start, source.len())
}

/// The first `ASSIGNMENT_OR_COMPARISON_TOKENS` token inside
/// `[line_start, line_end)`, found with a longest-match scan over unmasked
/// bytes (`processed_source.tokens_within(line_range).detect`).
fn first_asgn_token(
    source: &[u8],
    line_start: usize,
    line_end: usize,
    masks: &[(usize, usize)],
) -> Option<(AsgnKind, usize, usize)> {
    let mut i = line_start;
    while i < line_end {
        let b = source[i];
        if !matches!(
            b,
            b'=' | b'<' | b'>' | b'!' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^'
        ) {
            i += 1;
            continue;
        }
        if super::opaque_mask::contains(masks, i) {
            i += 1;
            continue;
        }
        let rest = &source[i..line_end.min(i + 3)];
        // Longest match first, mirroring the lexer.
        let (len, kind) = match rest {
            [b'=', b'=', b'='] => (3, Some(AsgnKind::Comparison)), // ===
            [b'<', b'=', b'>'] => (3, None),                       // <=> (tCMP)
            [b'<', b'<', b'='] | [b'>', b'>', b'='] | [b'*', b'*', b'='] => {
                (3, Some(AsgnKind::EqualSign)) // <<= / >>= / **=
            }
            [b'&', b'&', b'='] | [b'|', b'|', b'='] => (3, Some(AsgnKind::EqualSign)),
            _ => match &rest[..rest.len().min(2)] {
                [b'=', b'='] => (2, Some(AsgnKind::Comparison)),
                [b'!', b'='] => (2, Some(AsgnKind::Comparison)),
                [b'<', b'='] | [b'>', b'='] => (2, Some(AsgnKind::Comparison)),
                [b'<', b'<'] => (2, Some(AsgnKind::Lshft)),
                [b'=', b'~'] | [b'!', b'~'] | [b'=', b'>'] | [b'-', b'>'] => (2, None),
                [b'+', b'='] | [b'-', b'='] | [b'*', b'='] | [b'/', b'='] | [b'%', b'=']
                | [b'&', b'='] | [b'|', b'='] | [b'^', b'='] => (2, Some(AsgnKind::EqualSign)),
                [b'>', b'>'] | [b'*', b'*'] | [b'&', b'&'] | [b'|', b'|'] => (2, None),
                [b'&', b'.'] => (2, None),
                _ => match b {
                    b'=' => (1, Some(AsgnKind::EqualSign)),
                    _ => (1, None),
                },
            },
        };
        if let Some(kind) = kind {
            return Some((kind, i, i + len));
        }
        i += len;
    }
    None
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Node::CallNode { .. } = node {
            let call = node.as_call_node().unwrap();
            self.check_call(&call, node.location().start_offset());
        }
    }

    fn leave(&mut self) {}

    fn interest(&self) -> super::dispatch::Interest {
        // `on_send` / `on_csend` only: CallNode covers both in prism.
        super::dispatch::Interest(super::dispatch::Interest::ENTER_CALL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, allow: bool) -> Vec<(usize, usize)> {
        check_space_before_first_arg(
            source.as_bytes(),
            Config {
                allow_for_alignment: allow,
            },
        )
    }

    #[test]
    fn basic_offenses() {
        assert_eq!(run("something  x\n", true), vec![(9, 11)]);
        assert!(run("something x\n", true).is_empty());
        // Any single character passes, tab included.
        assert!(run("something\tx\n", true).is_empty());
        // Glued argument: empty range, alignment skipped.
        assert_eq!(run("something'hello'\n", true), vec![(9, 9)]);
    }

    #[test]
    fn skipped_shapes() {
        assert!(run("x.foo =  1\n", true).is_empty());
        assert!(run("x +  1\n", true).is_empty());
        assert!(run("not  x\n", true).is_empty());
        assert!(run("x.\n  foo  1\n", true).is_empty());
        assert!(run("foo \\\n  bar\n", true).is_empty());
        assert!(run("super  1\n", true).is_empty());
        assert!(run("yield  1\n", true).is_empty());
        assert!(run("f(  x)\n", true).is_empty());
    }

    #[test]
    fn block_pass_and_splats_are_arguments() {
        assert_eq!(run("foo  &b\n", true), vec![(3, 5)]);
        assert_eq!(run("foo  &:sym\n", true), vec![(3, 5)]);
        assert_eq!(run("foo  *a\n", true), vec![(3, 5)]);
        assert_eq!(run("foo  **h\n", true), vec![(3, 5)]);
        assert_eq!(run("x&.foo  1\n", true), vec![(6, 8)]);
    }

    #[test]
    fn alignment_with_words() {
        assert!(run("foo  1\nbar  2\n", true).is_empty());
        assert_eq!(
            run("foo  1\nbar  2\n", false),
            vec![(3, 5), (10, 12)]
        );
        // `\s\S` boundary through the arg column.
        assert!(run("foo    1\nbarbar 2\n", true).is_empty());
        // Nearest line decides: a non-aligned word above.
        assert_eq!(run("baz\nfoo  1\n", true), vec![(7, 9)]);
        assert_eq!(run("wwwww 0\nfoo  1\n", true), vec![(11, 13)]);
    }

    #[test]
    fn alignment_skips_comment_and_blank_lines() {
        assert!(run("# c\nfoo  1\nbar  2\n", true).is_empty());
        assert!(run("foo  1\n# c\nbar  2\n", true).is_empty());
        assert!(run("foo  1\n\nbar  2\n", true).is_empty());
    }

    #[test]
    fn alignment_second_pass_uses_indentation() {
        assert!(run("foo  1\n  x.bar\nbaz  2\n", true).is_empty());
        assert_eq!(run("foo  1\n  indented  2\n", true), vec![(3, 5), (17, 19)]);
    }

    #[test]
    fn alignment_with_equals_operator() {
        // `:foo=` last column 13 == the `=` token's last column (the
        // `aligned_words?` checks fail on the `xx` boundary first).
        assert!(run("define  :foo=\nxxxxxxxxxxx = 1\n", true).is_empty());
        // The `=` ends at a different column: offense.
        assert_eq!(run("define  :foo=\nxxxxxxxxx = 111\n", true), vec![(6, 8)]);
        // A shorter word line aligns through the `\\s\\S` boundary instead.
        assert!(run("define  :foo=\nxxxxxxx = 1\n", true).is_empty());
        // `=`s inside a string are not tokens.
        assert_eq!(
            run("define  :foo=\nx = \"===========\"\n", true),
            vec![(6, 8)]
        );
        // `<=>` is a tCMP, not a comparison from the list: longest-match must
        // skip it (a naive scan would take `<=` ending at the arg column).
        assert_eq!(
            run("define  :foo=\nzzzzzzzzzzz<=>b = 9\n", true),
            vec![(6, 8)]
        );
    }

    #[test]
    fn multibyte_columns() {
        assert!(run("foo  \u{5b9f}\u{5f15}\u{6570}\nbar  x\n", true).is_empty());
    }

    #[test]
    fn data_segment_lines_are_absent() {
        // The line below the call is past `__END__`: not a candidate line.
        assert_eq!(run("foo  1\n__END__\nbar  2\n", true), vec![(3, 5)]);
    }
}

