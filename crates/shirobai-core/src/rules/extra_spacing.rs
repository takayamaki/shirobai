//! `Layout/ExtraSpacing`.
//!
//! Flags unnecessary whitespace between two tokens on the same line, with three
//! configurable exemptions:
//!
//! - `AllowForAlignment` (default `true`): extra space used to vertically align
//!   a token with something on an adjacent line is permitted (the shared
//!   [`Aligner`], `aligned_with_something?` / `@aligned_comments`).
//! - `AllowBeforeTrailingComments` (default `false`): extra space before a
//!   trailing `#` comment is permitted.
//! - `ForceEqualSignAlignment` (default `false`): instead of removing extra
//!   space, `=`-assignment tokens are *aligned* across consecutive assignment
//!   lines (a different check + autocorrect entirely).
//!
//! ## Structure
//!
//! Stock walks `processed_source.tokens.each_cons(2)` (`check_tokens`). shirobai
//! drives the same adjacent-pair scan off the parser-gem token stream, which is
//! the pm_lex stream translated into the parser-gem shape by [`super::tokens`]
//! (the same token input `Layout/SpaceAroundOperators` reads). The per-pair core
//! ([`ExtraSpacingRule::check_pair`]) only collects cheap candidates; the
//! alignment / `ignored_ranges` decisions (which need the whole token list + the
//! AST) are applied in [`resolve`], the token-cop analogue of
//! `Layout/SpaceAroundOperators`' walk→resolve split. This is the only cop that
//! walks the whole token stream as adjacent pairs, so it iterates
//! `tokens.windows(2)` directly rather than through a shared dispatcher.
//!
//! `ignored_ranges` (the key↔value spans of the pairs of a multi-line hash, left
//! to `Layout/HashAlignment`) comes from an AST walk. Like `LineLength`'s heredoc
//! collection it is gathered in the bundle's walk-outer phase and handed to
//! [`resolve`]; it cannot be collected from inside the shared AST walk (the
//! token cache shares a `RefCell` with the AST parse).
//!
//! Offsets are **byte** offsets; the Ruby wrapper maps them through
//! `Shirobai::SourceOffsets`.

use std::collections::BTreeSet;

use ruby_prism::Visit;

use super::aligner::Aligner;
use super::line_index::LineIndex;
use super::tokens::Token;

/// `Layout/ExtraSpacing` configuration.
#[derive(Clone, Copy)]
pub struct Config {
    /// `AllowForAlignment` (default true).
    pub allow_for_alignment: bool,
    /// `AllowBeforeTrailingComments` (default false).
    pub allow_before_trailing_comments: bool,
    /// `ForceEqualSignAlignment` (default false).
    pub force_equal_sign_alignment: bool,
}

/// A single autocorrect edit: replace `[start, end)` with `text` (a zero-width
/// range is an insertion; an empty `text` is a deletion).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: Vec<u8>,
}

/// The autocorrect an offense carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Correction {
    /// `corrector.remove(range)`: delete the offense's own `[start, end)`.
    Remove,
    /// The `ForceEqualSignAlignment` edits: a (possibly empty) list of
    /// `insert_before` / `remove_preceding` operations on the assignment tokens
    /// of the offense's block that no earlier offense has already corrected.
    Align(Vec<Edit>),
}

/// One reported offense: the highlight range `[start, end)`, its `message`, and
/// the autocorrect to apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtraSpacingOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: Vec<u8>,
    pub correction: Correction,
}

pub const MSG_UNNECESSARY: &[u8] = b"Unnecessary spacing detected.";
pub const MSG_UNALIGNED_ASGN: &[u8] =
    b"`=` is not aligned with the preceding assignment.";

/// A same-line extra-space candidate from the scan (`check_other`'s
/// `extra_space_range`). The alignment / ignored-range filters in [`resolve`]
/// decide whether it becomes an offense.
struct GapCandidate {
    /// The whitespace range to remove `[range_start, range_end)`.
    range_start: usize,
    range_end: usize,
    /// `token2.begin_pos` (the alignment test's anchor).
    tok2_begin: usize,
    tok2_end: usize,
    /// `token2.comment?`.
    tok2_is_comment: bool,
}

/// An `=`-assignment token deferred to the `ForceEqualSignAlignment` check.
struct AsgnCandidate {
    begin: usize,
    end: usize,
}

/// The accumulator that rides the shared token scan.
pub struct ExtraSpacingRule {
    cfg: Config,
    /// `assignment_tokens` begin positions (only populated when
    /// `force_equal_sign_alignment`): every `equal_sign?` token minus the
    /// def/optarg `=` positions, first per line.
    assignment_set: BTreeSet<usize>,
    gaps: Vec<GapCandidate>,
    asgns: Vec<AsgnCandidate>,
}

impl ExtraSpacingRule {
    pub fn new(cfg: Config, assignment_set: BTreeSet<usize>) -> Self {
        Self {
            cfg,
            assignment_set,
            gaps: Vec::new(),
            asgns: Vec::new(),
        }
    }

    /// `check_tokens` per pair. Collects the cheap candidates; [`resolve`]
    /// applies the alignment / ignored-range filters.
    fn check_pair(&mut self, source: &[u8], line_index: &LineIndex, t1: &Token, t2: &Token) {
        // `return if token2.type == :tNL`.
        if t2.new_line() {
            return;
        }
        if self.cfg.force_equal_sign_alignment && self.assignment_set.contains(&t2.begin_pos) {
            self.asgns.push(AsgnCandidate {
                begin: t2.begin_pos,
                end: t2.end_pos,
            });
            return;
        }
        self.check_other(source, line_index, t1, t2);
    }

    /// `check_other`: collect a same-line extra-space gap candidate.
    fn check_other(&mut self, source: &[u8], line_index: &LineIndex, t1: &Token, t2: &Token) {
        // `return false if allow_for_trailing_comments? && token2.text.start_with?('#')`.
        if self.cfg.allow_before_trailing_comments
            && source.get(t2.begin_pos) == Some(&b'#')
        {
            return;
        }
        // `extra_space_range`: same line, and a non-empty gap between the tokens.
        // Same-line test via the binary-search line index: the free `line_of`
        // scan is O(offset), which over every adjacent token pair is O(tokens x
        // file size) per file (the dominant cost of this token cop).
        if line_index.line_of(t1.begin_pos) != line_index.line_of(t2.begin_pos) {
            return;
        }
        let start_pos = t1.end_pos;
        // `end_pos = token2.begin_pos - 1`.
        let Some(end_pos) = t2.begin_pos.checked_sub(1) else {
            return;
        };
        // `return if end_pos <= start_pos`.
        if end_pos <= start_pos {
            return;
        }
        self.gaps.push(GapCandidate {
            range_start: start_pos,
            range_end: end_pos,
            tok2_begin: t2.begin_pos,
            tok2_end: t2.end_pos,
            tok2_is_comment: t2.comment(),
        });
    }
}

/// `assignment_tokens`: every `equal_sign?` token minus the def/optarg `=`
/// positions (`remove_equals_in_def`), deduplicated to the first per line.
pub fn assignment_token_set(
    tokens: &[Token],
    line_index: &LineIndex,
    def_equals: &[usize],
) -> BTreeSet<usize> {
    let mut seen_lines = BTreeSet::new();
    let mut out = BTreeSet::new();
    for t in tokens {
        if !t.equal_sign() {
            continue;
        }
        if def_equals.contains(&t.begin_pos) {
            continue;
        }
        let line = line_index.line_of(t.begin_pos);
        if seen_lines.insert(line) {
            out.insert(t.begin_pos);
        }
    }
    out
}

/// `aligned_locations(processed_source.comments.map(&:loc))`: the set of lines
/// of any two *consecutive* comments that share a column (`@aligned_comments`).
fn aligned_comment_lines(tokens: &[Token], line_index: &LineIndex, source: &[u8]) -> BTreeSet<usize> {
    let comments: Vec<&Token> = tokens.iter().filter(|t| t.comment()).collect();
    let mut out = BTreeSet::new();
    for w in comments.windows(2) {
        let (c1, c2) = (w[0], w[1]);
        if line_index.column(source, c1.begin_pos) == line_index.column(source, c2.begin_pos) {
            out.insert(line_index.line_of(c1.begin_pos));
            out.insert(line_index.line_of(c2.begin_pos));
        }
    }
    out
}

/// Resolve the scan's candidates against the token list, producing the final
/// offense list in source order.
///
/// The AST `ignored_range?` filter (the `[key.end, value.begin)` spans of
/// multi-line hash pairs, `Layout/HashAlignment`'s territory) is applied by the
/// caller *after* this, against `[ignored_ranges]`: stock memoizes
/// `@ignored_ranges` on the cop instance, so across an autocorrect re-pass on a
/// *reused* instance the ranges go stale (their byte offsets are from the first
/// source). The Ruby wrapper reproduces that instance memoization, so it owns the
/// `ignored_range?` step; here every gap survivor that passes the alignment
/// filter is emitted as a `Remove` offense and tagged with its `range_start` (==
/// `start_offset`) for the wrapper to filter. `def_equals` are the `=` byte
/// positions `remove_equals_in_def` excludes.
pub fn resolve(
    source: &[u8],
    cfg: Config,
    rule: ExtraSpacingRule,
    tokens: &[Token],
    def_equals: &[usize],
) -> Vec<ExtraSpacingOffense> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    let aligner = Aligner::new(source, &line_index, tokens, def_equals);
    let aligned_comments = aligned_comment_lines(tokens, &line_index, source);

    let mut out: Vec<ExtraSpacingOffense> = Vec::new();

    // check_other survivors (the `ignored_range?` filter is the caller's).
    for g in &rule.gaps {
        // `return if allow_for_alignment? && aligned_tok?(token2)`.
        if cfg.allow_for_alignment {
            let aligned = if g.tok2_is_comment {
                aligned_comments.contains(&line_index.line_of(g.tok2_begin))
            } else {
                aligner.aligned_with_something(g.tok2_begin, g.tok2_end)
            };
            if aligned {
                continue;
            }
        }
        out.push(ExtraSpacingOffense {
            start_offset: g.range_start,
            end_offset: g.range_end,
            message: MSG_UNNECESSARY.to_vec(),
            correction: Correction::Remove,
        });
    }

    // ForceEqualSignAlignment: check_assignment.
    if cfg.force_equal_sign_alignment && !rule.asgns.is_empty() {
        let mut corrected: BTreeSet<usize> = BTreeSet::new();
        for a in &rule.asgns {
            // `return unless aligned_with_preceding_equals_operator(token) == :no`.
            if aligner.aligned_with_preceding_equals_operator(a.begin, a.end)
                != super::aligner::Tri::No
            {
                continue;
            }
            let edits =
                align_equal_signs(source, &line_index, &rule.assignment_set, a.begin, &mut corrected);
            out.push(ExtraSpacingOffense {
                start_offset: a.begin,
                end_offset: a.end,
                message: MSG_UNALIGNED_ASGN.to_vec(),
                correction: Correction::Align(edits),
            });
        }
    }

    out.sort_by_key(|o| o.start_offset);
    out
}

/// `align_equal_signs(range, corrector)`: align every relevant assignment `=` in
/// the contiguous block around `asgn_begin`'s line to a common column, returning
/// the `insert_before` / `remove_preceding` edits for the block's tokens not yet
/// corrected by an earlier offense (`@corrected` dedup; `corrected` is shared
/// across the file's offenses, so the first offense in a block carries its
/// edits and later ones carry none).
fn align_equal_signs(
    source: &[u8],
    line_index: &LineIndex,
    assignment_set: &BTreeSet<usize>,
    asgn_begin: usize,
    corrected: &mut BTreeSet<usize>,
) -> Vec<Edit> {
    let line = line_index.line_of(asgn_begin);
    let lines = all_relevant_assignment_lines(source, line_index, assignment_set, line);
    // tokens = assignment_tokens on those lines, in source order.
    let block_tokens: Vec<usize> = assignment_set
        .iter()
        .copied()
        .filter(|&p| lines.contains(&line_index.line_of(p)))
        .collect();
    let align_to = block_tokens
        .iter()
        .map(|&p| align_column(source, line_index, p))
        .max()
        .unwrap_or(0);

    let mut edits = Vec::new();
    for &p in &block_tokens {
        // `return unless @corrected.add?(token)`.
        if !corrected.insert(p) {
            continue;
        }
        // `diff = align_to - token.pos.last_column`. The token's end is its
        // last `=` char; for an op-assign (`+=`) the token spans several chars,
        // so use the token's actual end. We recover the `=` token end from the
        // assignment set position: an assignment token ends at the `=`; its
        // last_column is `column(token_end)`. We approximate end by scanning the
        // operator run `[p, ..)` up to and including the final `=`.
        let tok_end = assignment_token_end(source, p);
        let last_column = line_index.column(source, tok_end);
        let diff = align_to as isize - last_column as isize;
        if diff > 0 {
            // `insert_before(token.pos, ' ' * diff)`: insert at `[p, p)`.
            edits.push(Edit {
                start: p,
                end: p,
                text: vec![b' '; diff as usize],
            });
        } else if diff < 0 {
            // `remove_preceding(token.pos, -diff)`: delete the `-diff` bytes
            // immediately before `p`.
            let n = (-diff) as usize;
            edits.push(Edit {
                start: p - n,
                end: p,
                text: Vec::new(),
            });
        }
    }
    edits
}

/// The byte just past an assignment token that begins at `p`: the position right
/// after its final `=`. Plain `=` is one byte; an op-assign (`+=`, `||=`, …) runs
/// over operator bytes up to and including the trailing `=`.
fn assignment_token_end(source: &[u8], p: usize) -> usize {
    let mut e = p;
    while e < source.len() && source[e] != b'=' {
        e += 1;
    }
    // include the `=`
    if e < source.len() {
        e + 1
    } else {
        e
    }
}

/// `align_column(asgn_token)`: the column its `=` would *end* at if the leading
/// spaces directly before it were removed (`last_column - spaces + 1`).
fn align_column(source: &[u8], line_index: &LineIndex, token_begin: usize) -> usize {
    let line = line_index.line_of(token_begin);
    let line_start = line_index.line_starts()[line - 1];
    // `leading = line[0...col]`; `spaces = leading.size - (leading =~ / *\Z/)`:
    // the count of trailing spaces in the text before the token.
    let mut spaces = 0usize;
    let mut q = token_begin;
    while q > line_start && source.get(q - 1) == Some(&b' ') {
        spaces += 1;
        q -= 1;
    }
    let tok_end = assignment_token_end(source, token_begin);
    let last_column = line_index.column(source, tok_end);
    last_column - spaces + 1
}

/// `all_relevant_assignment_lines(line_number)`: the sorted union of the
/// downward and upward `relevant_assignment_lines` blocks around `line` (1-based).
fn all_relevant_assignment_lines(
    source: &[u8],
    line_index: &LineIndex,
    assignment_set: &BTreeSet<usize>,
    line: usize,
) -> BTreeSet<usize> {
    let asgn_lines: BTreeSet<usize> =
        assignment_set.iter().map(|&p| line_index.line_of(p)).collect();
    let last_line = line_count(source);
    let down: Vec<usize> = (1..=line).rev().collect();
    let up: Vec<usize> = (line..=last_line).collect();
    let mut out = BTreeSet::new();
    for ln in relevant_assignment_lines(source, line_index, &asgn_lines, &down) {
        out.insert(ln);
    }
    for ln in relevant_assignment_lines(source, line_index, &asgn_lines, &up) {
        out.insert(ln);
    }
    out
}

/// `relevant_assignment_lines(line_range)` (see the mixin). `line_range` is a
/// sequence of 1-based line numbers in iteration order.
fn relevant_assignment_lines(
    source: &[u8],
    line_index: &LineIndex,
    asgn_lines: &BTreeSet<usize>,
    line_range: &[usize],
) -> Vec<usize> {
    let mut result = Vec::new();
    let Some(&first) = line_range.first() else {
        return result;
    };
    let original_indent = line_indentation(source, line_index, first);
    let mut relevant_at_level = true;
    for &ln in line_range {
        let cur_indent = line_indentation(source, line_index, ln);
        let blank = line_is_blank(source, line_index, ln);
        if (cur_indent < original_indent && !blank) || (relevant_at_level && blank) {
            break;
        }
        if asgn_lines.contains(&ln) && cur_indent == original_indent {
            result.push(ln);
        }
        if !blank {
            relevant_at_level = cur_indent == original_indent;
        }
    }
    result
}

/// `processed_source.line_indentation(line)`: leading spaces/tabs of 1-based
/// `line`.
fn line_indentation(source: &[u8], line_index: &LineIndex, line: usize) -> usize {
    match line_text(source, line_index, line) {
        Some(t) => t.iter().take_while(|&&b| b == b' ' || b == b'\t').count(),
        None => 0,
    }
}

/// `processed_source.lines[line - 1].blank?`.
fn line_is_blank(source: &[u8], line_index: &LineIndex, line: usize) -> bool {
    match line_text(source, line_index, line) {
        Some(t) => t
            .iter()
            .all(|&b| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c)),
        None => true,
    }
}

/// Text of 1-based `line` without its trailing `\n`.
fn line_text<'a>(source: &'a [u8], line_index: &LineIndex, line: usize) -> Option<&'a [u8]> {
    if line == 0 || line > line_count(source) {
        return None;
    }
    let starts = line_index.line_starts();
    let start = *starts.get(line - 1)?;
    let end = starts
        .get(line)
        .map(|&s| {
            if s > start && source.get(s - 1) == Some(&b'\n') {
                s - 1
            } else {
                s
            }
        })
        .unwrap_or(source.len());
    Some(&source[start..end])
}

/// `processed_source.lines.size` (Ruby `String#lines`, drops the trailing empty
/// field after a final `\n`).
fn line_count(source: &[u8]) -> usize {
    let mut count = 0usize;
    let mut had = false;
    for &b in source {
        had = true;
        if b == b'\n' {
            count += 1;
            had = false;
        }
    }
    if had {
        count += 1;
    }
    count
}

/// 0-based line of byte offset `off` (for `same_line` checks; the cop's own
/// `token.line` is 1-based but only equality matters here).
fn line_of(source: &[u8], off: usize) -> usize {
    let end = off.min(source.len());
    source[..end].iter().filter(|&&c| c == b'\n').count()
}

/// Collect the AST `ignored_ranges`: for every `AssocNode` (a `key => value` /
/// `key: value` pair) whose enclosing hash is *not* single-line, the span
/// `[key.end, value.begin)`. Mirrors stock's `on_node(:pair)` with the
/// `pair.parent.single_line?` guard.
pub fn ignored_ranges(source: &[u8]) -> Vec<(usize, usize)> {
    // The `pair.parent.single_line?` guard needs the enclosing hash, so we walk
    // hashes and inspect their own `AssocNode` pairs (a braced hash uses its
    // `{`..`}`; a braceless keyword hash uses its first..last element span).
    struct HashWalk<'a> {
        source: &'a [u8],
        out: Vec<(usize, usize)>,
    }
    impl<'a> HashWalk<'a> {
        fn handle_pairs<'pr>(
            &mut self,
            open: usize,
            close: usize,
            elements: ruby_prism::NodeListIter<'pr>,
        ) {
            // `pair.parent.single_line?`.
            if line_of(self.source, open) == line_of(self.source, close) {
                return;
            }
            for el in elements {
                if let Some(assoc) = el.as_assoc_node() {
                    let kend = assoc.key().location().end_offset();
                    let vbegin = assoc.value().location().start_offset();
                    self.out.push((kend, vbegin));
                }
            }
        }
    }
    impl<'pr> Visit<'pr> for HashWalk<'_> {
        fn visit_hash_node(&mut self, node: &ruby_prism::HashNode<'pr>) {
            let open = node.opening_loc().start_offset();
            let close = node.closing_loc().start_offset();
            self.handle_pairs(open, close, node.elements().iter());
            ruby_prism::visit_hash_node(self, node);
        }
        fn visit_keyword_hash_node(&mut self, node: &ruby_prism::KeywordHashNode<'pr>) {
            // A braceless keyword hash (`foo(a: 1,\n  b: 2)`): `single_line?` is
            // the span of the hash itself (first element begin .. last element end).
            let elems: Vec<ruby_prism::Node<'pr>> = node.elements().iter().collect();
            if let (Some(first), Some(last)) = (elems.first(), elems.last()) {
                let open = first.location().start_offset();
                let close = last.location().end_offset();
                self.handle_pairs(open, close, node.elements().iter());
            }
            ruby_prism::visit_keyword_hash_node(self, node);
        }
    }
    let mut w = HashWalk {
        source,
        out: Vec::new(),
    };
    super::parse_cache::with_parsed(source, |_s, node| w.visit(node));
    w.out
}

/// Collect the def/optarg `=` byte positions excluded by `remove_equals_in_def`.
pub fn collect_def_equals(source: &[u8]) -> Vec<usize> {
    struct DefEq {
        out: Vec<usize>,
    }
    impl<'pr> Visit<'pr> for DefEq {
        fn visit_optional_parameter_node(
            &mut self,
            node: &ruby_prism::OptionalParameterNode<'pr>,
        ) {
            self.out.push(node.operator_loc().start_offset());
            ruby_prism::visit_optional_parameter_node(self, node);
        }
        fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
            if let Some(eq) = node.equal_loc() {
                self.out.push(eq.start_offset());
            }
            ruby_prism::visit_def_node(self, node);
        }
    }
    let mut v = DefEq { out: Vec::new() };
    super::parse_cache::with_parsed(source, |_s, node| v.visit(node));
    v.out
}

/// Standalone entry point (per-cop ext fallback). Collects the parser-gem token
/// stream from the shared parse, then runs the scan + resolve in one shot. The
/// `ignored_range?` filter is the caller's (the Ruby wrapper, which memoizes
/// `@ignored_ranges` like stock); this returns the gap survivors tagged for that
/// filter.
pub fn check_extra_spacing(source: &[u8], cfg: Config) -> Vec<ExtraSpacingOffense> {
    // Collect the tokens first so this token consumer is the first toucher of the
    // shared parse cache (the entry is built with tokens; the later `with_parsed`
    // in `check_with_tokens` reuses it with no re-parse). The tokens are the
    // pm_lex stream translated into the parser-gem shape the scan reads.
    let tokens = super::parse_cache::with_parsed_and_tokens(source, |owner, _root, raw| {
        super::tokens::translate_tokens(owner, raw)
    });
    check_with_tokens(source, cfg, &tokens)
}

/// The shared core: run the adjacent-pair scan over a pre-collected token list and
/// resolve. Both the standalone entry point and the bundle's walk-outer phase call
/// this with the same `translate_tokens` output, so the two paths cannot diverge
/// (the `cargo test` equivalence case in `bundle.rs` pins them equal).
pub fn check_with_tokens(source: &[u8], cfg: Config, tokens: &[Token]) -> Vec<ExtraSpacingOffense> {
    let line_index = super::line_index::with_line_index(source, |li| li.clone());
    let def_equals = collect_def_equals(source);
    let assignment_set = if cfg.force_equal_sign_alignment {
        assignment_token_set(tokens, &line_index, &def_equals)
    } else {
        BTreeSet::new()
    };
    let mut rule = ExtraSpacingRule::new(cfg, assignment_set);
    // `sorted_tokens.each_cons(2)`: drive the per-pair core directly.
    for w in tokens.windows(2) {
        rule.check_pair(source, &line_index, &w[0], &w[1]);
    }
    resolve(source, cfg, rule, tokens, &def_equals)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(allow_align: bool, allow_tc: bool, force: bool) -> Config {
        Config {
            allow_for_alignment: allow_align,
            allow_before_trailing_comments: allow_tc,
            force_equal_sign_alignment: force,
        }
    }

    fn run(src: &str, c: Config) -> Vec<(usize, usize)> {
        // The Ruby wrapper applies the `ignored_range?` filter (memoized); the
        // test reproduces a fresh (non-stale) application of it here.
        let ranges = ignored_ranges(src.as_bytes());
        check_extra_spacing(src.as_bytes(), c)
            .into_iter()
            .filter(|o| {
                o.correction != Correction::Remove
                    || !ranges.iter().any(|&(s, e)| (s..e).contains(&o.start_offset))
            })
            .map(|o| (o.start_offset, o.end_offset))
            .collect()
    }

    // Two spaces between two tokens on the same line is flagged over the extra
    // whitespace (minus the one kept space).
    #[test]
    fn extra_space_between_tokens() {
        // "x =  1\n": `=` ends at 3, `1` begins at 5, range [3,4) (one extra space).
        assert_eq!(run("x =  1\n", cfg(true, false, false)), vec![(3, 4)]);
    }

    // A single space is clean.
    #[test]
    fn single_space_clean() {
        assert!(run("x = 1\n", cfg(true, false, false)).is_empty());
    }

    // Vertically aligned assignments are accepted under AllowForAlignment, flagged
    // when off.
    #[test]
    fn allow_for_alignment() {
        let src = "a   = 1\nbbb = 2\n";
        assert!(run(src, cfg(true, false, false)).is_empty());
        // With alignment off, the extra spaces after `a` are flagged.
        let off = run(src, cfg(false, false, false));
        assert_eq!(off, vec![(1, 3)]);
    }

    // Extra space before a trailing comment: flagged by default, allowed under
    // AllowBeforeTrailingComments.
    #[test]
    fn before_trailing_comment() {
        let src = "x = 1  # c\n";
        assert_eq!(run(src, cfg(true, false, false)), vec![(5, 6)]);
        assert!(run(src, cfg(true, true, false)).is_empty());
    }

    // Aligned trailing comments are accepted under AllowForAlignment.
    #[test]
    fn aligned_comments() {
        let src = "a   = 1 # one\nbbb = 2 # two\n";
        // The `=` columns align (alignment), and the comments align, so nothing
        // is flagged.
        assert!(run(src, cfg(true, false, false)).is_empty());
    }

    // The extra space between a key and value in a multi-line hash is ignored
    // (Layout/HashAlignment's job).
    #[test]
    fn ignored_multiline_hash_pair() {
        let src = "{\n  a =>  1,\n  bb => 2,\n}\n";
        // The double space after `=>` on the first line is in an ignored range.
        assert!(run(src, cfg(true, false, false)).is_empty());
    }

    // A single-line hash's pair spacing is NOT ignored.
    #[test]
    fn single_line_hash_not_ignored() {
        // "{ a =>  1 }\n": `=>` ends at 6, `1` at 8 -> extra space flagged at [6,7).
        let off = run("{ a =>  1 }\n", cfg(true, false, false));
        assert_eq!(off, vec![(6, 7)]);
    }

    // ForceEqualSignAlignment flags an `=` not aligned with the preceding one.
    #[test]
    fn force_equal_sign_alignment_flags() {
        let src = "a = 1\nbbb = 2\n";
        let off = run(src, cfg(true, false, true));
        // The first `=` (col 2) is not aligned with the block's max -> flagged.
        assert_eq!(off.len(), 1);
    }
}
