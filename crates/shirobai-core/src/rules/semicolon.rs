//! `Style/Semicolon`, detection path (a): the per-line token-index checks.
//!
//! Stock's `on_new_investigation` groups `processed_source.tokens` by line and
//! flags a semicolon in six positional patterns only (the `semicolon_position`
//! chain). shirobai builds no parser token stream, so these facts are
//! reconstructed byte-side plus a few AST-collected positions:
//!
//! - A semicolon token is a `;` byte outside every opaque region (strings /
//!   symbols / regexps / xstrings, heredoc bodies, comments, `$;` global
//!   names and the `__END__` data segment — see [`opaque_mask`]). Interpolation
//!   code inside a string is NOT opaque, so a `;` in `"#{a;b}"` is a real token.
//! - "last token of the line is a `;`" (`tokens.last.semicolon?`, pattern -1):
//!   the rightmost non-space byte of the physical line is an unmasked `;`. A
//!   trailing comment makes the last token a `tCOMMENT`, so the rightmost byte
//!   is inside the (masked) comment and the pattern does not fire — matching
//!   the stock quirk that `foo; # x` is not flagged. The lexer suppresses the
//!   `tNL` after a trailing `;`, so `;` really is the last token.
//! - "first token of the line is a `;`" (pattern 0): the leftmost non-space
//!   byte of the line is an unmasked `;`.
//! - "`; }` at the end" (pattern -3): the line's last real token is a `tRCURLY`
//!   `}` (unmasked, not an interpolation `}`) and the token right before it is
//!   a `;`.
//! - "`{ ;` / `#{ ;` / `-> { ;`" (patterns 2 / 3): a `;` immediately after an
//!   opening brace / interpolation-begin that sits at the required leading
//!   token index. Those index facts are reconstructed from the AST: the brace
//!   must open a block / lambda / `BEGIN` / `END` / interpolation whose head is
//!   exactly the first token(s) of the line (see [`Visitor::enter`]).
//! - "`; }` before an interpolation end" (pattern -4): a `;` immediately before
//!   an interpolation `}` that is directly followed by the string's closing
//!   delimiter and the end of the line.
//!
//! Only ONE semicolon per line is reported (stock's `semicolon_position`
//! returns a single index); the checks run in stock's priority order.
//!
//! Detection path (b) (`on_begin`, the raw-line scan for expression
//! separators) stays in the Ruby wrapper: it is pure parser-AST + line text,
//! costs no token stream, and matches stock verbatim.

use std::collections::{HashMap, HashSet};

use ruby_prism::Node;

/// One reported path-(a) semicolon.
pub struct PathAOffense {
    /// Byte offset of the `;`.
    pub offset: usize,
    /// True for the "last token of the line" pattern (-1), where stock passes
    /// the real preceding token to the corrector (endless-range / value-omission
    /// wrapping). False for every other pattern (the corrector only removes
    /// the `;`).
    pub last_token: bool,
}

/// Standalone entry point (per-cop fallback).
pub fn check_semicolon(source: &[u8]) -> Vec<PathAOffense> {
    let mut rule = build_rule(source);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.into_offenses()
}

/// Build the rule for standalone or shared-walk use.
pub(crate) fn build_rule(source: &[u8]) -> Visitor<'_> {
    let comments = super::parse_cache::comment_ranges(source);
    let data_start = super::parse_cache::data_start(source);
    Visitor {
        source,
        comments,
        data_start,
        masks: Vec::new(),
        dends: HashSet::new(),
        opener_ends: HashSet::new(),
        p6: HashMap::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    comments: Vec<(usize, usize)>,
    data_start: Option<usize>,
    masks: Vec<(usize, usize)>,
    /// Interpolation close `}` positions (`tSTRING_DEND`).
    dends: HashSet<usize>,
    /// Byte position right after an opening brace / `#{` that sits at the
    /// leading token index required by patterns 2 / 3 (`{ ;` / `#{ ;` /
    /// `-> { ;`). A `;` whose only left neighbours are spaces down to one of
    /// these positions is flagged.
    opener_ends: HashSet<usize>,
    /// Interpolation `}` position -> byte offset just past the enclosing
    /// string's closing delimiter, for pattern -4 (`; }` before an
    /// interpolation whose `}` is the string's last content).
    p6: HashMap<usize, usize>,
}

/// A within-line space byte (`[ \t\f\v\r]`); `\n` bounds a line.
fn is_line_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | 0x0c | 0x0b | b'\r')
}

impl<'a> Visitor<'a> {
    /// First non-space byte position on `pos`'s physical line (leading
    /// indentation is not a token, so this is the start of token 0).
    fn line_first_nonspace(&self, pos: usize) -> usize {
        let src = self.source;
        let mut s = pos;
        while s > 0 && src[s - 1] != b'\n' {
            s -= 1;
        }
        while s < src.len() && is_line_space(src[s]) {
            s += 1;
        }
        s
    }

    /// Whether `[a, b)` is all within-line spaces.
    fn all_spaces(&self, a: usize, b: usize) -> bool {
        a <= b && self.source[a..b].iter().all(|&c| is_line_space(c))
    }

    /// `foo {` / `-> {` blocks and lambdas whose brace sits at leading index 1,
    /// and `foo -> {` lambdas whose brace sits at index 2. Collected from the
    /// call side so the "head is exactly the first token(s) of the line" test
    /// is an AST-structure check (no token counting).
    fn collect_call(&mut self, call: &ruby_prism::CallNode<'_>) {
        // Only a bare, parenless call can put its brace at index 1 / 2: a
        // receiver (`a.b {`), explicit arguments-parens (`foo() {`) or a
        // preceding argument all shift the brace right.
        if call.receiver().is_some() || call.opening_loc().is_some() {
            return;
        }
        let Some(msg) = call.message_loc() else { return };
        let msg_start = msg.start_offset();
        if msg_start != self.line_first_nonspace(msg_start) {
            return;
        }
        // Pattern 2, brace block: `foo {` — no arguments, block brace `{`.
        if call.arguments().is_none()
            && let Some(block) = call.block()
            && let Node::BlockNode { .. } = block
        {
            let block = block.as_block_node().unwrap();
            let open = block.opening_loc();
            if self.source.get(open.start_offset()) == Some(&b'{')
                && self.all_spaces(msg.end_offset(), open.start_offset())
            {
                self.opener_ends.insert(open.end_offset());
            }
        }
        // Pattern 3, lambda argument: `foo -> {` — first argument is a lambda
        // whose `->` follows the method name and whose brace follows `->`.
        if let Some(args) = call.arguments()
            && let Some(arg0) = args.arguments().iter().next()
            && let Node::LambdaNode { .. } = arg0
        {
            let lam = arg0.as_lambda_node().unwrap();
            let op = lam.operator_loc();
            let open = lam.opening_loc();
            if self.source.get(open.start_offset()) == Some(&b'{')
                && self.all_spaces(msg.end_offset(), op.start_offset())
                && self.all_spaces(op.end_offset(), open.start_offset())
            {
                self.opener_ends.insert(open.end_offset());
            }
        }
    }

    /// Bare `-> {` at the start of the line (pattern 2 with `tLAMBEG` at index 1).
    fn collect_lambda(&mut self, lam: &ruby_prism::LambdaNode<'_>) {
        let op = lam.operator_loc();
        let open = lam.opening_loc();
        if self.source.get(open.start_offset()) != Some(&b'{') {
            return;
        }
        if op.start_offset() == self.line_first_nonspace(op.start_offset())
            && self.all_spaces(op.end_offset(), open.start_offset())
        {
            self.opener_ends.insert(open.end_offset());
        }
    }

    /// `BEGIN {` / `END {` at the start of the line (pattern 2, `{` at index 1).
    fn collect_exec(&mut self, keyword: ruby_prism::Location<'_>, opening: ruby_prism::Location<'_>) {
        if self.source.get(opening.start_offset()) != Some(&b'{') {
            return;
        }
        if keyword.start_offset() == self.line_first_nonspace(keyword.start_offset())
            && self.all_spaces(keyword.end_offset(), opening.start_offset())
        {
            self.opener_ends.insert(opening.end_offset());
        }
    }

    /// Interpolated string leading `#{` (pattern 2) and trailing `#{ ... }`
    /// (pattern -4) facts.
    fn collect_istring(&mut self, istr: &ruby_prism::InterpolatedStringNode<'_>) {
        let parts = istr.parts();
        // Pattern 2: `"#{ ;` — the string opens the line and its first part is
        // an interpolation directly after the opening delimiter.
        if let Some(open) = istr.opening_loc()
            && open.start_offset() == self.line_first_nonspace(open.start_offset())
            && let Some(first) = parts.iter().next()
            && let Node::EmbeddedStatementsNode { .. } = first
        {
            let emb = first.as_embedded_statements_node().unwrap();
            if emb.opening_loc().start_offset() == open.end_offset() {
                self.opener_ends.insert(emb.opening_loc().end_offset());
            }
        }
        // Pattern -4: `; }"` — the string's last part is an interpolation whose
        // `}` is directly followed by the string's closing delimiter.
        if let Some(close) = istr.closing_loc()
            && let Some(last) = parts.iter().last()
            && let Node::EmbeddedStatementsNode { .. } = last
        {
            let emb = last.as_embedded_statements_node().unwrap();
            let dend = emb.closing_loc().start_offset();
            if emb.closing_loc().end_offset() == close.start_offset() {
                self.p6.insert(dend, close.end_offset());
            }
        }
    }

    pub(crate) fn into_offenses(self) -> Vec<PathAOffense> {
        let Visitor {
            source: src,
            comments,
            data_start,
            masks,
            dends,
            opener_ends,
            p6,
        } = self;
        if !src.contains(&b';') {
            return Vec::new();
        }
        let masks = super::opaque_mask::merge(masks, &comments, data_start, src.len());
        let mut out = Vec::new();
        let n = src.len();
        let mut ls = 0;
        loop {
            let line_end = match src[ls..n].iter().position(|&b| b == b'\n') {
                Some(off) => ls + off,
                None => n,
            };
            check_line(src, &masks, &dends, &opener_ends, &p6, ls, line_end, &mut out);
            if line_end >= n {
                break;
            }
            ls = line_end + 1;
        }
        out
    }
}

fn unmasked(masks: &[(usize, usize)], pos: usize) -> bool {
    !super::opaque_mask::contains(masks, pos)
}

/// Whether `[a, b)` is all within-line spaces.
fn all_spaces(src: &[u8], a: usize, b: usize) -> bool {
    a <= b && src[a..b].iter().all(|&c| is_line_space(c))
}

#[allow(clippy::too_many_arguments)]
fn check_line(
    src: &[u8],
    masks: &[(usize, usize)],
    dends: &HashSet<usize>,
    opener_ends: &HashSet<usize>,
    p6: &HashMap<usize, usize>,
    ls: usize,
    le: usize,
    out: &mut Vec<PathAOffense>,
) {
    // Quick bail: no real `;` on this line.
    let has_semi = (ls..le).any(|i| src[i] == b';' && unmasked(masks, i));
    if !has_semi {
        return;
    }
    // Rightmost / leftmost non-space byte of the line.
    let rightmost = (ls..le).rev().find(|&i| !is_line_space(src[i]));
    let leftmost = (ls..le).find(|&i| !is_line_space(src[i]));

    // Pattern -1: last token of the line is a `;`.
    if let Some(r) = rightmost
        && src[r] == b';'
        && unmasked(masks, r)
    {
        out.push(PathAOffense { offset: r, last_token: true });
        return;
    }
    // Pattern 0: first token of the line is a `;`.
    if let Some(l) = leftmost
        && src[l] == b';'
        && unmasked(masks, l)
    {
        out.push(PathAOffense { offset: l, last_token: false });
        return;
    }
    // Pattern -3: `; }` at the end (`}` is a `tRCURLY`, not an interp end).
    if let Some(r) = rightmost
        && src[r] == b'}'
        && unmasked(masks, r)
        && !dends.contains(&r)
        && let Some(q) = (ls..r).rev().find(|&i| !is_line_space(src[i]))
        && src[q] == b';'
        && unmasked(masks, q)
    {
        out.push(PathAOffense { offset: q, last_token: false });
        return;
    }
    // Patterns 2 / 3: `;` right after a leading opener.
    for i in ls..le {
        if src[i] != b';' || !unmasked(masks, i) {
            continue;
        }
        // Skip within-line spaces to the left; the landing position is the
        // opener's end offset when it is one of the collected openers.
        let mut q = i;
        while q > ls && is_line_space(src[q - 1]) {
            q -= 1;
        }
        if opener_ends.contains(&q) {
            out.push(PathAOffense { offset: i, last_token: false });
            return;
        }
    }
    // Pattern -4: `; }` before an interpolation end that closes the string.
    for i in ls..le {
        if src[i] != b';' || !unmasked(masks, i) {
            continue;
        }
        let mut b = i + 1;
        while b < le && is_line_space(src[b]) {
            b += 1;
        }
        if b < le
            && src[b] == b'}'
            && dends.contains(&b)
            && let Some(&close_end) = p6.get(&b)
            && all_spaces(src, close_end, le)
        {
            out.push(PathAOffense { offset: i, last_token: false });
            return;
        }
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        match node {
            Node::CallNode { .. } => {
                let call = node.as_call_node().unwrap();
                self.collect_call(&call);
            }
            Node::LambdaNode { .. } => {
                let lam = node.as_lambda_node().unwrap();
                self.collect_lambda(&lam);
            }
            Node::PreExecutionNode { .. } => {
                let pre = node.as_pre_execution_node().unwrap();
                self.collect_exec(pre.keyword_loc(), pre.opening_loc());
            }
            Node::PostExecutionNode { .. } => {
                let post = node.as_post_execution_node().unwrap();
                self.collect_exec(post.keyword_loc(), post.opening_loc());
            }
            Node::EmbeddedStatementsNode { .. } => {
                let emb = node.as_embedded_statements_node().unwrap();
                self.dends.insert(emb.closing_loc().start_offset());
                super::opaque_mask::collect_enter(node, &mut self.masks);
            }
            Node::InterpolatedStringNode { .. } => {
                let istr = node.as_interpolated_string_node().unwrap();
                self.collect_istring(&istr);
                super::opaque_mask::collect_enter(node, &mut self.masks);
            }
            _ => super::opaque_mask::collect_enter(node, &mut self.masks),
        }
    }

    fn leave(&mut self) {}

    fn enter_leaf(&mut self, node: &Node<'_>) {
        super::opaque_mask::collect_leaf(node, &mut self.masks);
    }

    fn interest(&self) -> super::dispatch::Interest {
        super::dispatch::Interest(
            super::dispatch::Interest::LEAF
                | super::dispatch::Interest::ENTER_CALL
                | super::dispatch::Interest::ENTER_LAMBDA
                | super::dispatch::Interest::ENTER_ISTRING
                | super::dispatch::Interest::ENTER_LITERAL
                | super::dispatch::Interest::ENTER_WRITE
                | super::dispatch::Interest::ENTER_OTHER,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<(usize, bool)> {
        check_semicolon(source.as_bytes())
            .into_iter()
            .map(|o| (o.offset, o.last_token))
            .collect()
    }

    #[test]
    fn last_token_semicolon() {
        assert_eq!(run("foo;\n"), vec![(3, true)]);
        // A trailing comment makes the last token a comment: not flagged.
        assert_eq!(run("foo; # c\n"), vec![]);
        assert_eq!(run("x = 1;\n"), vec![(5, true)]);
    }

    #[test]
    fn first_token_semicolon() {
        assert_eq!(run("; puts 1\n"), vec![(0, false)]);
        // Lone `;` is caught by the higher-priority last-token pattern.
        assert_eq!(run(";\n"), vec![(0, true)]);
    }

    #[test]
    fn semicolon_before_right_curly() {
        assert_eq!(run("foo { bar; }\n"), vec![(9, false)]);
        // `.baz` after the `}` means the `}` is not the last token.
        assert_eq!(run("foo { bar; }.baz\n"), vec![]);
    }

    #[test]
    fn semicolon_after_left_curly() {
        assert_eq!(run("foo {; bar }\n"), vec![(5, false)]);
        assert_eq!(run("-> {; x }\n"), vec![(4, false)]);
        // Shifted openers are not at the leading index.
        assert_eq!(run("a.b {; x }\n"), vec![]);
        assert_eq!(run("foo(1) {; x }\n"), vec![]);
        assert_eq!(run("z = foo {; x }\n"), vec![]);
    }

    #[test]
    fn semicolon_after_lambda_curly() {
        assert_eq!(run("foo -> {; bar }\n"), vec![(8, false)]);
        assert_eq!(run("baz ->() {; qux }\n"), vec![]);
        assert_eq!(run("z = -> {; x }\n"), vec![]);
    }

    #[test]
    fn string_interpolation_patterns() {
        assert_eq!(run("\"#{;foo}\"\n"), vec![(3, false)]);
        assert_eq!(run("\"#{foo;}\"\n"), vec![(6, false)]);
        // Content after the interpolation `}` blocks pattern -4.
        assert_eq!(run("\"#{foo;}bar\"\n"), vec![]);
        assert_eq!(run("\"#{foo;} \"\n"), vec![]);
        // Not at line start: neither pattern applies.
        assert_eq!(run("x = \"#{;foo}\"\n"), vec![]);
    }

    #[test]
    fn masked_semicolons_are_not_tokens() {
        assert_eq!(run("x = \"a;b\"\n"), vec![]);
        assert_eq!(run("x = 'a;b'\n"), vec![]);
        assert_eq!(run("x = 1 # a;b\n"), vec![]);
        assert_eq!(run("puts $;\n"), vec![]);
    }

    #[test]
    fn heredoc_opener_line() {
        // The `;` is the last token of the opener line; the body stays on its
        // own lines.
        assert_eq!(run("x = <<~T;\n  body\nT\n"), vec![(8, true)]);
        // `;` in the middle of the opener line is not a path-(a) pattern.
        assert_eq!(run("x = <<~T; y = 2\n  body\nT\n"), vec![]);
    }

    #[test]
    fn one_offense_per_line() {
        // Two `;` on the line: the last-token pattern (higher priority) wins
        // over the leading `;`, and only that one is a path-(a) offense.
        assert_eq!(run("; foo;\n"), vec![(5, true)]);
        // `{ ;` is the single path-(a) offense; the trailing `;` here is a
        // path-(b) separator handled by the wrapper, not this rule.
        assert_eq!(run("foo {; a }; b\n"), vec![(5, false)]);
    }

    #[test]
    fn endless_range_is_last_token() {
        assert_eq!(run("42..;\n"), vec![(4, true)]);
    }
}
