//! `Layout/SpaceInsideBlockBraces`.
//!
//! Checks that block braces (`{ ... }`) have, or don't have, surrounding space
//! inside them, per `EnforcedStyle` (`space` / `no_space`). Empty braces are
//! governed by `EnforcedStyleForEmptyBraces` (`space` / `no_space`); the space
//! between `{` and a block-parameter `|` is governed by
//! `SpaceBeforeBlockParameters` (which overrides `EnforcedStyle`).
//!
//! Stock fires `on_block` (and the `numblock` / `itblock` aliases) and reads
//! the block's `begin` (`{`) and `end` (`}`) locations:
//!
//! - keyword blocks (`do ... end`) are skipped (`node.keywords?`);
//! - a multi-line empty block (`{` and `}` with no body, on different lines) is
//!   skipped to avoid fighting a single-line empty-brace correction;
//! - adjacent braces (`{}`): governed by `EnforcedStyleForEmptyBraces`;
//! - whitespace-only inner: empty-brace handling (only `no_space` flags);
//! - non-whitespace inner: `check_left_brace` + `check_right_brace`.
//!
//! Reconstructed over Prism in one walk. Every `BlockNode`'s `opening_loc` /
//! `closing_loc` are the braces; the parameter `|` (only for a regular block,
//! not a `numblock` / `itblock`) is the `BlockParametersNode`'s `opening_loc`.
//! All the character classes (`\S`, `[ \t]`, `range_with_surrounding_space`)
//! are ASCII, so the scan is byte-level; the only char-sensitive arithmetic
//! (the multi-line / `]`-ending right-brace cases) is done in characters via a
//! line index so it matches parser's char columns exactly.
//!
//! Each offense carries a `kind` tag mirroring stock's message and the
//! autocorrect action stock derives from `range.source`. The Ruby wrapper
//! reproduces stock's `offense` corrector verbatim (`remove` / `replace '{ }'`
//! / `replace '{ |'` / `insert_before ' '`) by inspecting the live range
//! source, so this cop keeps no cross-pass state.

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// `EnforcedStyle` / `EnforcedStyleForEmptyBraces` value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Space,
    NoSpace,
}

/// Config for `Layout/SpaceInsideBlockBraces`.
#[derive(Clone, Copy)]
pub struct Config {
    pub style: Style,
    pub empty_braces_style: Style,
    pub space_before_block_parameters: bool,
}

/// One offense. `(start_offset, end_offset)` is the byte range stock reports
/// (the offense highlight, which is also the autocorrect target). `message` is
/// the fixed message id; the Ruby wrapper formats the string and applies the
/// `range.source`-based corrector exactly like stock.
pub struct SpaceInsideBlockBracesOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: MessageId,
}

/// The seven fixed messages stock emits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MessageId {
    /// `'Space missing inside {.'`
    SpaceMissingLeft,
    /// `'Space inside { detected.'`
    SpaceInsideLeft,
    /// `'Space missing inside }.'`
    SpaceMissingRight,
    /// `'Space inside } detected.'`
    SpaceInsideRight,
    /// `'Space inside empty braces detected.'`
    EmptySpaceDetected,
    /// `'Space missing inside empty braces.'`
    EmptySpaceMissing,
    /// `'Space between { and | missing.'`
    PipeSpaceMissing,
    /// `'Space between { and | detected.'`
    PipeSpaceDetected,
}

impl MessageId {
    /// The numeric tag carried over the wire to the Ruby wrapper.
    pub fn code(self) -> u8 {
        match self {
            MessageId::SpaceMissingLeft => 0,
            MessageId::SpaceInsideLeft => 1,
            MessageId::SpaceMissingRight => 2,
            MessageId::SpaceInsideRight => 3,
            MessageId::EmptySpaceDetected => 4,
            MessageId::EmptySpaceMissing => 5,
            MessageId::PipeSpaceMissing => 6,
            MessageId::PipeSpaceDetected => 7,
        }
    }
}

pub fn check_space_inside_block_braces(
    source: &[u8],
    config: Config,
) -> Vec<SpaceInsideBlockBracesOffense> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    Visitor {
        source,
        config,
        line_index: LineIndex::new(source),
        offenses: Vec::new(),
        ancestor_starts: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    config: Config,
    line_index: LineIndex,
    pub(crate) offenses: Vec<SpaceInsideBlockBracesOffense>,
    /// Start byte offset of every open ancestor; top = parent of the entering
    /// node. The parent of a brace `BlockNode` is the `CallNode` / `SuperNode`
    /// it belongs to, whose start offset gives stock's `node.source_range`
    /// column (parser's block node spans the whole call).
    ancestor_starts: Vec<usize>,
}

impl<'a> Visitor<'a> {
    fn push(&mut self, start: usize, end: usize, message: MessageId) {
        // Stock's `offense` returns early when `begin_pos > end_pos`.
        if start > end {
            return;
        }
        self.offenses.push(SpaceInsideBlockBracesOffense {
            start_offset: start,
            end_offset: end,
            message,
        });
    }

    /// 0-based char column of a byte offset within its line (matches parser's
    /// `range.column`).
    fn column(&self, off: usize) -> usize {
        self.line_index.column(self.source, off)
    }

    /// Whether `a` and `b` are on different lines.
    fn different_line(&self, a: usize, b: usize) -> bool {
        self.line_index.line_of(a) != self.line_index.line_of(b)
    }

    /// `range_with_surrounding_space`, `side: :right`, default flags: skip
    /// `[ \t]`, then a `\\\n` continuation is *not* skipped (continuations
    /// off), then `\n` (newlines on). Returns the end byte position.
    fn surrounding_space_right(&self, mut pos: usize) -> usize {
        while matches!(self.source.get(pos), Some(b' ' | b'\t')) {
            pos += 1;
        }
        while self.source.get(pos) == Some(&b'\n') {
            pos += 1;
        }
        pos
    }

    /// `range_with_surrounding_space`, `side: :left`: from `pos` move left over
    /// `[ \t]` then `\n`. Returns the begin byte position.
    fn surrounding_space_left(&self, mut pos: usize) -> usize {
        while pos > 0 && matches!(self.source.get(pos - 1), Some(b' ' | b'\t')) {
            pos -= 1;
        }
        while pos > 0 && self.source.get(pos - 1) == Some(&b'\n') {
            pos -= 1;
        }
        pos
    }

    /// Compute `end_pos - n` characters as a byte position, where `n` may be
    /// negative (parser's `end_pos - (col_a - col_b)` arithmetic). A positive
    /// `n` moves the byte position left by `n` chars; a negative `n` moves it
    /// right by `-n` chars. The result is UTF-8 char-boundary aligned and may
    /// exceed `end_pos` (mirroring stock, where `begin_pos > end_pos` makes the
    /// `offense` return early without flagging).
    fn move_chars(&self, end_pos: usize, n: i64) -> usize {
        let mut pos = end_pos;
        if n >= 0 {
            for _ in 0..n {
                if pos == 0 {
                    break;
                }
                pos -= 1;
                // Skip UTF-8 continuation bytes to land on a char boundary.
                while pos > 0 && (self.source[pos] & 0xC0) == 0x80 {
                    pos -= 1;
                }
            }
        } else {
            for _ in 0..(-n) {
                if pos >= self.source.len() {
                    pos += 1;
                    continue;
                }
                pos += 1;
                while pos < self.source.len() && (self.source[pos] & 0xC0) == 0x80 {
                    pos += 1;
                }
            }
        }
        pos
    }

    /// `on_block` for a `BlockNode` with brace delimiters. `do ... end` blocks
    /// (`keywords?`) and multi-line empty braces are filtered by the caller.
    fn on_block(
        &mut self,
        left_begin: usize,
        left_end: usize,
        right_begin: usize,
        right_end: usize,
        block_start: usize,
        pipe: Option<Location<'_>>,
    ) {
        if left_end == right_begin {
            self.adjacent_braces(left_begin, right_end);
            return;
        }
        let inner = &self.source[left_end..right_begin];
        if inner.iter().any(|b| !b.is_ascii_whitespace_strict()) {
            self.braces_with_contents_inside(
                left_begin, left_end, right_begin, right_end, block_start, pipe,
            );
        } else if self.config.empty_braces_style == Style::NoSpace {
            // Whitespace-only inner, `no_space` empty style: the inner range.
            self.push(left_end, right_begin, MessageId::EmptySpaceDetected);
        }
    }

    /// `adjacent_braces` (`{}`): only flagged when the empty style is `space`.
    fn adjacent_braces(&mut self, left_begin: usize, right_end: usize) {
        if self.config.empty_braces_style != Style::Space {
            return;
        }
        self.push(left_begin, right_end, MessageId::EmptySpaceMissing);
    }

    #[allow(clippy::too_many_arguments)]
    fn braces_with_contents_inside(
        &mut self,
        left_begin: usize,
        left_end: usize,
        right_begin: usize,
        right_end: usize,
        block_start: usize,
        pipe: Option<Location<'_>>,
    ) {
        self.check_left_brace(left_begin, left_end, pipe.as_ref());
        let single_line = !self.different_line(left_begin, right_begin);
        self.check_right_brace(
            left_begin, left_end, right_begin, right_end, block_start, single_line,
        );
    }

    fn check_left_brace(&mut self, left_begin: usize, left_end: usize, pipe: Option<&Location<'_>>) {
        // `inner` starts at `left_end`; first inner char.
        let first = self.source[left_end];
        if !first.is_ascii_whitespace_strict() {
            self.no_space_inside_left_brace(left_begin, left_end, pipe);
        } else {
            self.space_inside_left_brace(left_begin, left_end, pipe);
        }
    }

    fn no_space_inside_left_brace(
        &mut self,
        left_begin: usize,
        left_end: usize,
        pipe: Option<&Location<'_>>,
    ) {
        if let Some(pipe) = pipe {
            if left_end == pipe.start_offset() && self.config.space_before_block_parameters {
                // `{|` : highlight `{` through the `|`.
                self.push(left_begin, pipe.end_offset(), MessageId::PipeSpaceMissing);
            }
            // else: `correct_style_detected` (no offense).
        } else {
            // No pipe: position after the left brace, length 1.
            self.no_space(left_end, left_end + 1, MessageId::SpaceMissingLeft);
        }
    }

    fn space_inside_left_brace(
        &mut self,
        left_begin: usize,
        left_end: usize,
        pipe: Option<&Location<'_>>,
    ) {
        if let Some(pipe) = pipe {
            if self.config.space_before_block_parameters {
                // `correct_style_detected` (no offense).
            } else {
                self.push(left_end, pipe.start_offset(), MessageId::PipeSpaceDetected);
            }
        } else {
            // stock: range_with_surrounding_space(left_brace, side: :right)
            // keeps `begin_pos == left_begin` and extends the end from
            // `left_end`. space(bws.begin_pos + 1, bws.end_pos).
            let bws_end = self.surrounding_space_right(left_end);
            self.space(left_begin + 1, bws_end, MessageId::SpaceInsideLeft);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_right_brace(
        &mut self,
        left_begin: usize,
        left_end: usize,
        right_begin: usize,
        right_end: usize,
        block_start: usize,
        single_line: bool,
    ) {
        let inner = &self.source[left_end..right_begin];
        let inner_ends_nonspace = inner
            .last()
            .is_some_and(|b| !b.is_ascii_whitespace_strict());
        if single_line && inner_ends_nonspace {
            self.no_space(right_begin, right_end, MessageId::SpaceMissingRight);
        } else {
            let column = self.column(block_start);
            if self.different_line(left_begin, right_begin)
                && self.aligned_braces(inner, right_begin, column)
            {
                return;
            }
            self.space_inside_right_brace(inner, right_begin, right_end, column);
        }
    }

    /// `aligned_braces?`: the closing brace column equals the block column, or
    /// the block column equals the trailing-space count of the inner's last
    /// line.
    fn aligned_braces(&self, inner: &[u8], right_begin: usize, column: usize) -> bool {
        column == self.column(right_begin) || column == inner_last_space_count(inner)
    }

    fn space_inside_right_brace(
        &mut self,
        inner: &[u8],
        right_begin: usize,
        right_end: usize,
        column: usize,
    ) {
        // stock: range_with_surrounding_space(right_brace, side: :left) keeps
        // `end_pos == right_end` and extends the begin left from `right_begin`.
        let bws_begin = self.surrounding_space_left(right_begin);
        let mut begin_pos = bws_begin;
        // end_pos = bws.end_pos - 1.
        let mut end_pos = right_end - 1;

        // `if brace_with_space.source.match?(/\R/)` — the surrounding-space
        // range spans a line break.
        if self.different_line(bws_begin, right_begin) {
            // begin_pos = end_pos - (right_brace.column - column)
            let right_col = self.column(right_begin) as i64;
            begin_pos = self.move_chars(end_pos, right_col - column as i64);
        }

        if inner.last() == Some(&b']') {
            end_pos -= 1;
            // begin_pos = end_pos - (inner_last_space_count(inner) - column)
            let delta = inner_last_space_count(inner) as i64 - column as i64;
            begin_pos = self.move_chars(end_pos, delta);
        }

        self.space(begin_pos, end_pos, MessageId::SpaceInsideRight);
    }

    /// `no_space`: emit only when `style == space`.
    fn no_space(&mut self, start: usize, end: usize, message: MessageId) {
        if self.config.style == Style::Space {
            self.push(start, end, message);
        }
    }

    /// `space`: emit only when `style == no_space`.
    fn space(&mut self, start: usize, end: usize, message: MessageId) {
        if self.config.style == Style::NoSpace {
            self.push(start, end, message);
        }
    }
}

/// `inner.split("\n").last.count(' ')`: number of ASCII spaces on the inner's
/// last physical line. Counts `' '` only (matches Ruby `String#count(' ')`).
fn inner_last_space_count(inner: &[u8]) -> usize {
    let last_line = match inner.iter().rposition(|&b| b == b'\n') {
        Some(i) => &inner[i + 1..],
        None => inner,
    };
    last_line.iter().filter(|&&b| b == b' ').count()
}

/// Ruby `/\s/` and `/\S/` treat ASCII space, tab, newline, carriage return,
/// form feed and vertical tab as whitespace. `u8::is_ascii_whitespace` omits
/// the vertical tab (`\x0B`); add it so `\S` matches Ruby exactly.
trait AsciiWhitespaceStrict {
    fn is_ascii_whitespace_strict(&self) -> bool;
}

impl AsciiWhitespaceStrict for u8 {
    fn is_ascii_whitespace_strict(&self) -> bool {
        self.is_ascii_whitespace() || *self == 0x0B
    }
}

/// Shared-walk driver. `BlockNode`s are reached through the generic `enter`
/// hook; brace vs `do`/`end` is decided by the opening-loc text.
impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        self.visit(node);
        self.ancestor_starts.push(node.location().start_offset());
    }

    fn leave(&mut self) {
        self.ancestor_starts.pop();
    }

    fn enter_rescue(&mut self, node: &Node<'_>) {
        self.ancestor_starts.push(node.location().start_offset());
    }

    fn leave_rescue(&mut self) {
        self.ancestor_starts.pop();
    }
}

impl<'a> Visitor<'a> {
    fn visit(&mut self, node: &Node<'_>) {
        // Parser's `on_block` fires for both `block` nodes (method-call blocks,
        // numbered / `it` blocks) and lambda literals (`-> { }` / `lambda { }`
        // are `block` nodes too). Prism splits these into `BlockNode` and
        // `LambdaNode`; both expose `opening_loc` / `closing_loc` / `body`.
        // stock's `node.source_range.column`: parser's block node spans the
        // whole expression. For a method-call block (`foo { }`) that is the
        // enclosing call, i.e. the parent on the walk stack; for a lambda
        // (`->(x) { }`) it is the lambda's own `->`.
        let (opening, closing, body_none, pipe, block_start) =
            if let Some(block) = node.as_block_node() {
                // The parameter `|` delimiter, only for a regular block (a
                // `numblock` / `itblock` is `block_type? == false`, so stock
                // leaves the delimiter `nil`).
                let pipe = block
                    .parameters()
                    .and_then(|p| p.as_block_parameters_node().and_then(|bp| bp.opening_loc()));
                let start = self
                    .ancestor_starts
                    .last()
                    .copied()
                    .unwrap_or_else(|| block.opening_loc().start_offset());
                (
                    block.opening_loc(),
                    block.closing_loc(),
                    block.body().is_none(),
                    pipe,
                    start,
                )
            } else if let Some(lambda) = node.as_lambda_node() {
                // A lambda's args are `->(x)`, outside the braces: parser's
                // `args_delimiter` is the `(`, so `pipe?` is always false. The
                // parser block spans the lambda, so the column is the `->`.
                (
                    lambda.opening_loc(),
                    lambda.closing_loc(),
                    lambda.body().is_none(),
                    None,
                    node.location().start_offset(),
                )
            } else if let Some(zsuper) = node.as_forwarding_super_node() {
                // A bare `super { }` block hides behind the concretely-typed
                // `block` field: prism's generated walker calls
                // `visit_block_node` directly, so the block never reaches the
                // shared-walk enter hook (the RescueNode-family trap). Reach it
                // here. `super(...) { }` is a `SuperNode` whose `block` is a
                // normal `Option<Node>` child, so it is already covered by the
                // block-node arm above; only the bare form needs this.
                // Parser wraps `super { }` in one block node spanning the whole
                // `super … }`, so the column is the `super` keyword, i.e. this
                // node's own start offset.
                let Some(block) = zsuper.block() else {
                    return;
                };
                let pipe = block
                    .parameters()
                    .and_then(|p| p.as_block_parameters_node().and_then(|bp| bp.opening_loc()));
                (
                    block.opening_loc(),
                    block.closing_loc(),
                    block.body().is_none(),
                    pipe,
                    node.location().start_offset(),
                )
            } else {
                return;
            };

        // `node.keywords?`: a `do ... end` block. Brace blocks open with `{`.
        if self.source.get(opening.start_offset()) != Some(&b'{') {
            return;
        }
        let left_begin = opening.start_offset();
        let left_end = opening.end_offset();
        let right_begin = closing.start_offset();
        let right_end = closing.end_offset();

        // `return if node.body.nil? && node.multiline?` — skip a multi-line
        // empty brace block.
        if body_none && self.different_line(left_begin, right_begin) {
            return;
        }

        self.on_block(left_begin, left_end, right_begin, right_end, block_start, pipe);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(style: Style, empty: Style, sbbp: bool) -> Config {
        Config {
            style,
            empty_braces_style: empty,
            space_before_block_parameters: sbbp,
        }
    }

    fn run(source: &str, config: Config) -> Vec<(usize, usize, u8)> {
        check_space_inside_block_braces(source.as_bytes(), config)
            .into_iter()
            .map(|o| (o.start_offset, o.end_offset, o.message.code()))
            .collect()
    }

    const SPACE: Config = Config {
        style: Style::Space,
        empty_braces_style: Style::NoSpace,
        space_before_block_parameters: true,
    };

    #[test]
    fn space_missing_both_sides() {
        // "some.each {puts e}" -> { offense at 11..12 (after {), } at 17..18.
        assert_eq!(run("some.each {puts e}\n", SPACE), vec![(11, 12, 0), (17, 18, 2)]);
    }

    #[test]
    fn clean_spaced_block() {
        assert!(run("some.each { puts e }\n", SPACE).is_empty());
    }

    #[test]
    fn empty_no_space_clean() {
        assert!(run("x.each {}\n", SPACE).is_empty());
    }

    #[test]
    fn empty_no_space_one_space() {
        assert_eq!(run("x.each { }\n", SPACE), vec![(8, 9, 4)]);
    }

    #[test]
    fn empty_no_space_two_spaces() {
        assert_eq!(run("x.each {  }\n", SPACE), vec![(8, 10, 4)]);
    }

    #[test]
    fn empty_space_missing() {
        let c = cfg(Style::Space, Style::Space, true);
        assert_eq!(run("x.each {}\n", c), vec![(7, 9, 5)]);
    }

    #[test]
    fn empty_space_clean() {
        let c = cfg(Style::Space, Style::Space, true);
        assert!(run("x.each { }\n", c).is_empty());
    }

    #[test]
    fn empty_multiline_skipped() {
        assert!(run("x.each {\n}\n", SPACE).is_empty());
        let c = cfg(Style::Space, Style::Space, true);
        assert!(run("x.each {\n}\n", c).is_empty());
    }

    #[test]
    fn pipe_space_missing() {
        assert_eq!(run("[1].each {|n| n }\n", SPACE), vec![(9, 11, 6)]);
    }

    #[test]
    fn pipe_clean() {
        assert!(run("[1].each { |n| n }\n", SPACE).is_empty());
    }

    #[test]
    fn pipe_space_detected_when_sbbp_false() {
        let c = cfg(Style::Space, Style::NoSpace, false);
        assert_eq!(run("[1].each { |n| n }\n", c), vec![(10, 11, 7)]);
    }

    #[test]
    fn no_space_style_detects_inner_spaces() {
        let c = cfg(Style::NoSpace, Style::NoSpace, false);
        assert_eq!(run("x.each { puts e }\n", c), vec![(8, 9, 1), (15, 16, 3)]);
    }

    #[test]
    fn left_missing_only() {
        assert_eq!(run("x.each {puts e }\n", SPACE), vec![(8, 9, 0)]);
    }

    #[test]
    fn right_missing_only() {
        assert_eq!(run("x.each { puts e}\n", SPACE), vec![(15, 16, 2)]);
    }

    #[test]
    fn multiline_aligned_clean() {
        assert!(run("x.each { |a|\n  b\n}\n", SPACE).is_empty());
    }

    #[test]
    fn hash_literal_ignored() {
        assert!(run("h = {a: 1}\n", SPACE).is_empty());
    }

    #[test]
    fn do_end_block_ignored() {
        assert!(run("x.each do |n| n end\n", SPACE).is_empty());
    }

    #[test]
    fn nested_blocks() {
        // "x.each {y.map {z}}"
        assert_eq!(
            run("x.each {y.map {z}}\n", SPACE),
            vec![(8, 9, 0), (17, 18, 2), (15, 16, 0), (16, 17, 2)]
        );
    }

    #[test]
    fn bare_super_block_reached() {
        // A bare `super { }` block hides behind ForwardingSuperNode's concrete
        // `block` field, which the generated walker visits directly (bypassing
        // the shared-walk enter hook). Stock still checks its braces; so must we.
        // Offsets confirmed against stock RuboCop 1.88.0.
        assert_eq!(run("super {x}\n", SPACE), vec![(7, 8, 0), (8, 9, 2)]);
        // A `{|` pipe: `{`..`|` span for the missing space, plus right brace.
        assert_eq!(run("super {|n| n}\n", SPACE), vec![(6, 8, 6), (12, 13, 2)]);
        // Empty braces with the no_space empty style: the inner space is flagged.
        assert_eq!(run("super { }\n", SPACE), vec![(7, 8, 4)]);
        // A `super(...) { }` block is a SuperNode with a normal child block and
        // is already reached by the generic walk; the bare arm must not fire
        // twice for it. One offense pair, no duplicates.
        assert_eq!(run("super() {x}\n", SPACE), vec![(9, 10, 0), (10, 11, 2)]);
    }
}
