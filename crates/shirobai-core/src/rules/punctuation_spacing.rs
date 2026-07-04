//! The punctuation-spacing cop family:
//!
//! - `Layout/SpaceBeforeComma` / `Layout/SpaceBeforeSemicolon`
//!   (stock: `SpaceBeforePunctuation` over `sorted_tokens`)
//! - `Layout/SpaceAfterComma` / `Layout/SpaceAfterSemicolon`
//!   (stock: `SpaceAfterPunctuation` over `tokens`)
//! - `Layout/SpaceAfterColon` (stock: `on_pair` / `on_kwoptarg`)
//! - `Layout/SpaceBeforeComment` (stock: comment tokens in `sorted_tokens`)
//!
//! Stock reads the parser-gem token stream for four of these; prism has no
//! token stream, so the token facts are reconstructed byte-side:
//!
//! - A comma / semicolon **token** is a `,` / `;` byte outside every "opaque"
//!   region — string / symbol / regexp / xstring opening+content+closing
//!   (heredoc bodies and quoted heredoc terminators included), character
//!   literals (`?,`), comments, global variable names (`$,` / `$;`), and the
//!   `__END__` data segment. Interpolation code inside strings is NOT opaque
//!   (probed: `"#{f(a,b)}"` and heredoc interpolations are flagged by stock).
//! - "Next token is adjacent on the same line" (`token2.column == column + 1`)
//!   is "the next byte is not whitespace and not a `\`-newline continuation".
//!   Tokens are separated by whitespace / continuations only; heredoc bodies
//!   sit past the line break so they can never fake a same-line neighbor. A
//!   `tNL` never directly follows a comma or semicolon (the lexer suppresses
//!   the newline token there — probed on `x = 1;\n`).
//! - "Previous token ends on the same line at `q`" is a left scan skipping
//!   `[ \t\f\v\r]`; a `\n` (or a continuation's `\n`) means a different line.
//!   `q == 0` means the punctuation is the first token of the file — stock's
//!   `each_cons(2)` never yields it as `token2` (probed on `" ;x"`).
//! - Token-type tests on the neighbors reduce to byte tests plus two
//!   position sets collected during the walk:
//!   `tSTRING_DEND` = closing `}` of an `EmbeddedStatementsNode`;
//!   `left_curly_brace?` (`tLCURLY` / `tLAMBEG`) = opening `{` of a
//!   block / lambda / `BEGIN` / `END` (probed: `BEGIN { ; 1 }` behaves like
//!   a block; `"#{ ;1}"`'s `tSTRING_DBEG` does not).
//!
//! `Layout/SpaceAfterColon` needs no token facts at all: `on_pair` colons are
//! the last byte of the key's `closing_loc` (prism keeps the label colon in
//! the symbol), `on_kwoptarg` colons are the last byte of the parameter's
//! `name_loc`. Value omissions (`{a:}`, `in {a:}`) carry an `ImplicitNode`
//! value and are skipped, mirroring stock's `value_omission?` guard.
//!
//! `Layout/SpaceBeforeComment` is a prism-comment check: a comment whose
//! preceding byte exists and is not whitespace is exactly stock's
//! `token1.pos.end == token2.pos.begin` on the same line (a comment can never
//! share its line with a later token, and `=begin` docs sit at column 0).

use std::collections::HashSet;

use ruby_prism::Node;

use super::space_scan::is_ruby_space;

/// Config bits, one per consumer cop (each mirrors that cop's own read of the
/// referenced cop's `EnforcedStyle`, `nil` falling back like stock).
#[derive(Clone, Copy, Default)]
pub struct Config {
    /// `SpaceBeforeComma`'s `space_required_after_lcurly?`
    /// (`Layout/SpaceInsideBlockBraces` `EnforcedStyle` == `space`).
    pub before_comma_lcurly_space: bool,
    /// `SpaceBeforeSemicolon`'s `space_required_after_lcurly?`.
    pub before_semi_lcurly_space: bool,
    /// `SpaceAfterComma`'s `space_forbidden_before_rcurly?`
    /// (`Layout/SpaceInsideHashLiteralBraces` `EnforcedStyle` == `no_space`).
    pub after_comma_rcurly_no_space: bool,
    /// `SpaceAfterSemicolon`'s `space_forbidden_before_rcurly?`
    /// (`Layout/SpaceInsideBlockBraces` `EnforcedStyle` == `no_space`).
    pub after_semi_rcurly_no_space: bool,
}

/// One `[start, end)` byte range per offense:
///
/// - `space_before_*`: the whitespace run before the punctuation
///   (`pos_before_punctuation`; the corrector removes it);
/// - `space_after_*`: the punctuation byte itself (the corrector replaces it
///   with `", "` / `"; "`);
/// - `space_after_colon`: the colon byte (the corrector appends a space);
/// - `space_before_comment`: the comment (the corrector prepends a space).
#[derive(Default)]
pub struct PunctuationSpacingOffenses {
    pub space_before_comma: Vec<(usize, usize)>,
    pub space_after_comma: Vec<(usize, usize)>,
    pub space_before_semicolon: Vec<(usize, usize)>,
    pub space_after_semicolon: Vec<(usize, usize)>,
    pub space_after_colon: Vec<(usize, usize)>,
    pub space_before_comment: Vec<(usize, usize)>,
}

/// Standalone entry point (per-cop fallback): compute the whole family and
/// let the wrapper pick its slice.
pub fn check_punctuation_spacing(source: &[u8], config: Config) -> PunctuationSpacingOffenses {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.into_offenses()
}

/// Build the family rule for standalone or shared-walk (bundle) use.
pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    // Comments and the data segment come from the shared parse; both are
    // fetched here, before `dispatch::run` re-borrows the parse cache.
    let comments = super::parse_cache::comment_ranges(source);
    let data_start = super::parse_cache::data_start(source);
    Visitor {
        source,
        config,
        comments,
        data_start,
        masks: Vec::new(),
        lcurly_opens: HashSet::new(),
        dends: HashSet::new(),
        space_after_colon: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    config: Config,
    /// `(start, end)` of every comment, ascending (used both as opaque
    /// regions and as the `SpaceBeforeComment` subjects).
    comments: Vec<(usize, usize)>,
    /// Start of the `__END__` data segment, if any.
    data_start: Option<usize>,
    /// Opaque regions collected from the walk (unsorted; heredoc bodies are
    /// pushed at their opener's visit, out of document order).
    masks: Vec<(usize, usize)>,
    /// Opening `{` positions lexing as `tLCURLY` / `tLAMBEG`.
    lcurly_opens: HashSet<usize>,
    /// Closing `}` positions lexing as `tSTRING_DEND`.
    dends: HashSet<usize>,
    /// `Layout/SpaceAfterColon` offenses, resolved during the walk.
    space_after_colon: Vec<(usize, usize)>,
}

impl<'a> Visitor<'a> {
    pub(crate) fn into_offenses(self) -> PunctuationSpacingOffenses {
        let Visitor {
            source,
            config,
            comments,
            data_start,
            masks,
            lcurly_opens,
            dends,
            space_after_colon,
        } = self;

        let mut out = PunctuationSpacingOffenses {
            space_after_colon,
            ..Default::default()
        };

        // `SpaceBeforeComment`: previous byte exists and is not whitespace.
        for &(cs, ce) in &comments {
            if cs > 0 && !is_ruby_space(source[cs - 1]) {
                out.space_before_comment.push((cs, ce));
            }
        }

        // Merge every opaque region: walk-collected masks + comments + data.
        let masks = super::opaque_mask::merge(masks, &comments, data_start, source.len());

        let scan_end = data_start.unwrap_or(source.len()).min(source.len());
        let mut mask_i = 0;
        for p in 0..scan_end {
            let b = source[p];
            if b != b',' && b != b';' {
                continue;
            }
            // Skip punctuation bytes inside an opaque region.
            while mask_i < masks.len() && masks[mask_i].1 <= p {
                mask_i += 1;
            }
            if mask_i < masks.len() && masks[mask_i].0 <= p {
                continue;
            }
            let comma = b == b',';
            if let Some(range) = before_offense(
                source,
                p,
                &lcurly_opens,
                if comma {
                    config.before_comma_lcurly_space
                } else {
                    config.before_semi_lcurly_space
                },
            ) {
                if comma {
                    out.space_before_comma.push(range);
                } else {
                    out.space_before_semicolon.push(range);
                }
            }
            if after_offense(
                source,
                p,
                &dends,
                if comma {
                    config.after_comma_rcurly_no_space
                } else {
                    config.after_semi_rcurly_no_space
                },
            ) {
                if comma {
                    out.space_after_comma.push((p, p + 1));
                } else {
                    out.space_after_semicolon.push((p, p + 1));
                }
            }
        }
        out
    }

    /// `on_pair`: a colon pair whose colon (the key's closing byte) is not
    /// followed by whitespace. Rockets (`=>`) and value omissions are skipped.
    fn check_assoc(&mut self, assoc: &ruby_prism::AssocNode<'_>) {
        if assoc.operator_loc().is_some() {
            return;
        }
        if matches!(assoc.value(), Node::ImplicitNode { .. }) {
            return;
        }
        let key = assoc.key();
        let closing = match &key {
            Node::SymbolNode { .. } => key.as_symbol_node().unwrap().closing_loc(),
            Node::InterpolatedSymbolNode { .. } => {
                key.as_interpolated_symbol_node().unwrap().closing_loc()
            }
            _ => None,
        };
        let Some(closing) = closing else { return };
        let end = closing.end_offset();
        if end == 0 || self.source.get(end - 1) != Some(&b':') {
            return;
        }
        self.check_colon(end);
    }

    /// `followed_by_space?(colon)`: `/\s/` on the byte after the colon
    /// (`nil` at EOF does not match, so EOF is an offense).
    fn check_colon(&mut self, colon_end: usize) {
        if !self.source.get(colon_end).is_some_and(|&b| is_ruby_space(b)) {
            self.space_after_colon.push((colon_end - 1, colon_end));
        }
    }

    /// A block-ish `{` opening (lexes as `tLCURLY` / `tLAMBEG`, both
    /// `left_curly_brace?` to stock).
    fn collect_lcurly(&mut self, opening: Option<ruby_prism::Location<'_>>) {
        if let Some(o) = opening
            && self.source.get(o.start_offset()) == Some(&b'{')
        {
            self.lcurly_opens.insert(o.start_offset());
        }
    }
}

/// `SpaceBeforePunctuation#each_missing_space` for the punctuation at `p`:
/// the previous token ends at `q` on the same line with a non-empty gap, and
/// (`space_required_after?`) that token is not a space-style block `{`.
fn before_offense(
    source: &[u8],
    p: usize,
    lcurly_opens: &HashSet<usize>,
    lcurly_space: bool,
) -> Option<(usize, usize)> {
    let q = super::space_scan::prev_token_end_same_line(source, p)?;
    // `q == 0`: only whitespace before the punctuation on the first line —
    // there is no previous token, so `each_cons` never pairs it.
    if q == 0 || q == p {
        return None;
    }
    if lcurly_space && source[q - 1] == b'{' && lcurly_opens.contains(&(q - 1)) {
        return None;
    }
    Some((q, p))
}

/// `SpaceAfterPunctuation#each_missing_space` for the punctuation at `p`:
/// the next token starts at exactly `p + 1` on the same line
/// (`space_missing?`), the pair is not `,;` / `;;` (`kind` /
/// `semicolon_sequence?`), and the next token's type is not allowed
/// (`space_required_before?`).
fn after_offense(source: &[u8], p: usize, dends: &HashSet<usize>, rcurly_no_space: bool) -> bool {
    let q = p + 1;
    let Some(&nb) = source.get(q) else {
        // EOF: no next token, no `each_cons` pair.
        return false;
    };
    if is_ruby_space(nb) {
        return false;
    }
    if nb == b'\\' {
        // A `\`-newline continuation: the next token is on the next line.
        // (A stray `\` cannot lex, so a non-continuation `\` is unreachable
        // in valid code.)
        match source.get(q + 1) {
            Some(&b'\n') => return false,
            Some(&b'\r') if source.get(q + 2) == Some(&b'\n') => return false,
            _ => {}
        }
    }
    match nb {
        // Comma before a semicolon has no `kind`; a semicolon sequence is
        // skipped. Both reduce to "next byte is `;`".
        b';' => false,
        // `allowed_type?`: tRPAREN / tRBRACK / tPIPE (`tOROP` cannot follow
        // a comma or semicolon in code that parses).
        b')' | b']' | b'|' => false,
        b'}' => {
            if dends.contains(&q) {
                // tSTRING_DEND is an `allowed_type?`.
                false
            } else {
                // Any other `}` after a real `,` / `;` closes a hash, block
                // or lambda — a `tRCURLY` (`right_curly_brace?`), allowed
                // only under the referenced cop's `no_space` style.
                !rcurly_no_space
            }
        }
        _ => true,
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        match node {
            Node::AssocNode { .. } => {
                let assoc = node.as_assoc_node().unwrap();
                self.check_assoc(&assoc);
            }
            Node::OptionalKeywordParameterNode { .. } => {
                // `on_kwoptarg`: prism's `name_loc` includes the colon.
                let param = node.as_optional_keyword_parameter_node().unwrap();
                let end = param.name_loc().end_offset();
                if end > 0 && self.source.get(end - 1) == Some(&b':') {
                    self.check_colon(end);
                }
            }
            Node::BlockNode { .. } => {
                let block = node.as_block_node().unwrap();
                self.collect_lcurly(Some(block.opening_loc()));
            }
            Node::LambdaNode { .. } => {
                let lambda = node.as_lambda_node().unwrap();
                self.collect_lcurly(Some(lambda.opening_loc()));
            }
            Node::ForwardingSuperNode { .. } => {
                // A bare `super { }` block hides behind the concretely-typed
                // `block` field (the RescueNode trap): collect its `{` here.
                let zsuper = node.as_forwarding_super_node().unwrap();
                if let Some(block) = zsuper.block() {
                    self.collect_lcurly(Some(block.opening_loc()));
                }
            }
            Node::PreExecutionNode { .. } => {
                let pre = node.as_pre_execution_node().unwrap();
                self.collect_lcurly(Some(pre.opening_loc()));
            }
            Node::PostExecutionNode { .. } => {
                let post = node.as_post_execution_node().unwrap();
                self.collect_lcurly(Some(post.opening_loc()));
            }
            Node::EmbeddedStatementsNode { .. } => {
                let emb = node.as_embedded_statements_node().unwrap();
                let closing = emb.closing_loc();
                self.dends.insert(closing.start_offset());
            }
            // Interpolated literal delimiters and gvar-write names are
            // opaque; see `opaque_mask`.
            _ => super::opaque_mask::collect_enter(node, &mut self.masks),
        }
    }

    fn leave(&mut self) {}

    fn enter_leaf(&mut self, node: &Node<'_>) {
        super::opaque_mask::collect_leaf(node, &mut self.masks);
    }

    fn interest(&self) -> super::dispatch::Interest {
        // `enter` is a pure kind match over: AssocNode / kwoptarg /
        // PreExecution / PostExecution / EmbeddedStatements / the block-ish
        // openers, plus the opaque-mask branch nodes (interpolated literals,
        // percent arrays, gvar writes); `enter_leaf` masks the leaf
        // literals. `leave` and the rescue hooks are unused.
        super::dispatch::Interest(
            super::dispatch::Interest::LEAF
                | super::dispatch::Interest::ENTER_BLOCK
                | super::dispatch::Interest::ENTER_LAMBDA
                | super::dispatch::Interest::ENTER_SUPER
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

    fn run(source: &str) -> PunctuationSpacingOffenses {
        check_punctuation_spacing(source.as_bytes(), Config::default())
    }

    fn run_cfg(source: &str, config: Config) -> PunctuationSpacingOffenses {
        check_punctuation_spacing(source.as_bytes(), config)
    }

    /// The stock-default config: SpaceInsideBlockBraces = space (so
    /// `lcurly_space` on, `rcurly_no_space` for semicolons off) and
    /// SpaceInsideHashLiteralBraces = space.
    fn default_cfg() -> Config {
        Config {
            before_comma_lcurly_space: true,
            before_semi_lcurly_space: true,
            after_comma_rcurly_no_space: false,
            after_semi_rcurly_no_space: false,
        }
    }

    #[test]
    fn space_after_comma_flags_adjacent_args() {
        let r = run("f(a,b)\n");
        assert_eq!(r.space_after_comma, vec![(3, 4)]);
        assert!(r.space_before_comma.is_empty());
    }

    #[test]
    fn space_after_comma_accepts_spaced_and_tab() {
        assert!(run("f(a, b)\n").space_after_comma.is_empty());
        assert!(run("f(a,\tb)\n").space_after_comma.is_empty());
        assert!(run("f(a,\n  b)\n").space_after_comma.is_empty());
    }

    #[test]
    fn space_after_comma_allowed_next_tokens() {
        assert!(run("f(a,)\n").space_after_comma.is_empty());
        assert!(run("[1,]\n").space_after_comma.is_empty());
        assert!(run("foo { |a,| }\n").space_after_comma.is_empty());
        // `,;` in block-locals has no kind.
        let r = run_cfg("foo { |a,;b| b }\n", default_cfg());
        assert!(r.space_after_comma.is_empty());
        assert_eq!(r.space_after_semicolon, vec![(9, 10)]);
    }

    #[test]
    fn space_after_comma_rcurly_depends_on_hash_brace_style() {
        let space = run_cfg("h = {foo: 1,}\n", default_cfg());
        assert_eq!(space.space_after_comma, vec![(11, 12)]);
        let no_space = run_cfg(
            "h = {foo: 1,}\n",
            Config {
                after_comma_rcurly_no_space: true,
                ..default_cfg()
            },
        );
        assert!(no_space.space_after_comma.is_empty());
    }

    #[test]
    fn space_after_comma_before_comment_is_flagged() {
        let r = run("f(a,# c\n  b)\n");
        assert_eq!(r.space_after_comma, vec![(3, 4)]);
        assert_eq!(r.space_before_comment, vec![(4, 7)]);
    }

    #[test]
    fn percent_array_comma_delimiters_are_not_tokens() {
        // `%w,a b,`: the opener `%w,` and closer `,` are string delimiters.
        let r = run("x = %w,a b,\n");
        assert!(r.space_after_comma.is_empty());
        assert!(r.space_before_comma.is_empty());
        let r = run("x = %i;a b;\n");
        assert!(r.space_after_semicolon.is_empty());
        assert!(r.space_before_semicolon.is_empty());
    }

    #[test]
    fn masked_regions_are_not_tokens() {
        assert!(run("x = \"a,b\"\n").space_after_comma.is_empty());
        assert!(run("x = 'a;b'\n").space_after_semicolon.is_empty());
        assert!(run("x = :\",\"\n").space_after_comma.is_empty());
        assert!(run("x = %w{a,b}\n").space_after_comma.is_empty());
        assert!(run("x = %s{a;b}\n").space_after_semicolon.is_empty());
        assert!(run("x = /,;/\n").space_after_comma.is_empty());
        assert!(run("x = ?,\n").space_after_comma.is_empty());
        assert!(run("puts $,\n").space_after_comma.is_empty());
        assert!(run("$, = 'x'\n").space_after_comma.is_empty());
        assert!(run("puts $;\n").space_after_semicolon.is_empty());
        assert!(run("x = 1 # a,b\n").space_after_comma.is_empty());
        assert!(run("x = `ls a,b`\n").space_after_comma.is_empty());
        assert!(run("if /a{1,2}/ then end\n").space_after_comma.is_empty());
    }

    #[test]
    fn heredoc_bodies_are_masked_but_interpolations_are_not() {
        assert!(
            run("f(<<~EOS, x)\n  a,body\nEOS\n")
                .space_after_comma
                .is_empty()
        );
        // Quoted terminator with a comma.
        assert!(
            run("f(<<'E,S', x)\n  a,b\nE,S\n")
                .space_after_comma
                .is_empty()
        );
        // Interpolated code inside a heredoc is real.
        let r = run("f(<<~EOS)\n  pre #{g(1,2)} post\nEOS\n");
        assert_eq!(r.space_after_comma, vec![(21, 22)]);
    }

    #[test]
    fn data_segment_is_masked() {
        let r = run("x = 1\n__END__\na,b;c\nx ;y\n");
        assert!(r.space_after_comma.is_empty());
        assert!(r.space_before_semicolon.is_empty());
        assert!(r.space_after_semicolon.is_empty());
    }

    #[test]
    fn space_before_comma_flags_gap() {
        // "f(a , b)": gap [3,4).
        assert_eq!(run("f(a , b)\n").space_before_comma, vec![(3, 4)]);
        // Tab and multi-space gaps.
        assert_eq!(run("f(a\t,b)\n").space_before_comma, vec![(3, 4)]);
        assert_eq!(run("f(a  ,b)\n").space_before_comma, vec![(3, 5)]);
    }

    #[test]
    fn space_before_comma_needs_same_line_prev_token() {
        assert!(run("f(a \\\n, b)\n").space_before_comma.is_empty());
        // Heredoc opener then space then comma: same line, flagged.
        assert_eq!(
            run("foo(<<~EOS , x)\n  body\nEOS\n").space_before_comma,
            vec![(10, 11)]
        );
    }

    #[test]
    fn space_before_semicolon_flags_gap() {
        assert_eq!(run("x = 1 ;\n").space_before_semicolon, vec![(5, 6)]);
        // `; ;` flags both gaps.
        assert_eq!(
            run("x = 1 ; ;\n").space_before_semicolon,
            vec![(5, 6), (7, 8)]
        );
    }

    #[test]
    fn space_before_semicolon_first_token_is_skipped() {
        assert!(run(" ;x\n").space_before_semicolon.is_empty());
    }

    #[test]
    fn space_before_semicolon_lcurly_style() {
        // Block / lambda / BEGIN `{` with the `space` style: skipped.
        assert!(
            run_cfg("loop { ; 1 }\n", default_cfg())
                .space_before_semicolon
                .is_empty()
        );
        assert!(
            run_cfg("-> { ; 1 }\n", default_cfg())
                .space_before_semicolon
                .is_empty()
        );
        assert!(
            run_cfg("BEGIN { ; 1 }\n", default_cfg())
                .space_before_semicolon
                .is_empty()
        );
        assert!(
            run_cfg("z = super { ; 1 }\n", default_cfg())
                .space_before_semicolon
                .is_empty()
        );
        // Under `no_space` the same gap is an offense.
        let cfg = Config {
            before_semi_lcurly_space: false,
            ..default_cfg()
        };
        assert_eq!(
            run_cfg("loop { ; 1 }\n", cfg).space_before_semicolon,
            vec![(6, 7)]
        );
        // `#{` is a tSTRING_DBEG, not a left curly: flagged either way.
        assert_eq!(
            run_cfg("\"#{ ;1}\"\n", default_cfg()).space_before_semicolon,
            vec![(3, 4)]
        );
    }

    #[test]
    fn space_after_semicolon_basics() {
        assert_eq!(run("x = 1;y = 2\n").space_after_semicolon, vec![(5, 6)]);
        assert!(run("x = 1; y = 2\n").space_after_semicolon.is_empty());
        assert!(run("x = 1;\ny = 2\n").space_after_semicolon.is_empty());
        assert!(run("x = 1;").space_after_semicolon.is_empty());
        // `;;`: only the second (followed by a token) is flagged.
        assert_eq!(run("x = 1;;y = 2\n").space_after_semicolon, vec![(6, 7)]);
        assert!(run("x = 1;;\n").space_after_semicolon.is_empty());
    }

    #[test]
    fn space_after_semicolon_allowed_next_tokens() {
        assert!(run("(a;)\n").space_after_semicolon.is_empty());
        // tSTRING_DEND.
        assert!(run("\"#{a;}\"\n").space_after_semicolon.is_empty());
        // tRCURLY under the space style is an offense…
        assert_eq!(
            run_cfg("foo {a;}\n", default_cfg()).space_after_semicolon,
            vec![(6, 7)]
        );
        // …and allowed under no_space.
        let cfg = Config {
            after_semi_rcurly_no_space: true,
            ..default_cfg()
        };
        assert!(run_cfg("foo {a;}\n", cfg).space_after_semicolon.is_empty());
        // Block-locals pipe.
        let r = run("foo { |a;b| b }\n");
        assert_eq!(r.space_after_semicolon, vec![(8, 9)]);
    }

    #[test]
    fn space_after_colon_pairs_and_kwoptargs() {
        assert_eq!(run("h = {a:1}\n").space_after_colon, vec![(6, 7)]);
        assert!(run("h = {a: 1}\n").space_after_colon.is_empty());
        assert_eq!(run("f(a:1)\n").space_after_colon, vec![(3, 4)]);
        assert_eq!(run("def f(a:1); end\n").space_after_colon, vec![(7, 8)]);
        // Required kwargs are not kwoptargs.
        assert_eq!(
            run("def f(a:, b:2); end\n").space_after_colon,
            vec![(11, 12)]
        );
        // Value omission.
        assert!(run("h = {a:}\n").space_after_colon.is_empty());
        assert!(run("f(a:)\n").space_after_colon.is_empty());
        // Rockets.
        assert!(run("h = {:a=>1}\n").space_after_colon.is_empty());
        // Quoted / interpolated labels: the colon is the closing's last byte.
        assert_eq!(run("h = {\"a\":1}\n").space_after_colon, vec![(8, 9)]);
        assert_eq!(run("h = {\"#{x}\":1}\n").space_after_colon, vec![(11, 12)]);
        // Block and lambda kwoptargs.
        assert_eq!(run("foo { |a:1| }\n").space_after_colon, vec![(8, 9)]);
        assert_eq!(run("->(a:1) {}\n").space_after_colon, vec![(4, 5)]);
        // A newline counts as space.
        assert!(run("h = {a:\n1}\n").space_after_colon.is_empty());
        // EOF right after the colon would be an offense (nil never matches),
        // but that cannot parse; the nearest parseable neighbour:
        assert_eq!(run("h = {a:1}").space_after_colon, vec![(6, 7)]);
    }

    #[test]
    fn space_after_colon_patterns() {
        assert_eq!(
            run("case x\nin {a:1}\n  y\nend\n").space_after_colon,
            vec![(12, 13)]
        );
        assert_eq!(
            run("case x\nin a:1\n  y\nend\n").space_after_colon,
            vec![(11, 12)]
        );
        // Pattern value omission carries an ImplicitNode too.
        assert!(
            run("case x\nin {a:, b: 1}\n  y\nend\n")
                .space_after_colon
                .is_empty()
        );
        assert_eq!(
            run("case x\nin {b:Integer}\nend\n").space_after_colon,
            vec![(12, 13)]
        );
    }

    #[test]
    fn space_before_comment_adjacency() {
        assert_eq!(run("1 + 1# c\n").space_before_comment, vec![(5, 8)]);
        assert!(run("1 + 1 # c\n").space_before_comment.is_empty());
        assert!(run("# c\n").space_before_comment.is_empty());
        assert!(run("x = 1\t# c\n").space_before_comment.is_empty());
        assert_eq!(run("x = \"s\"# c\n").space_before_comment, vec![(7, 10)]);
        assert_eq!(run("x = 1;# c\n").space_before_comment, vec![(6, 9)]);
        // Heredoc opener directly followed by a comment.
        assert_eq!(
            run("foo <<~EOS# c\n  b\nEOS\n").space_before_comment,
            vec![(10, 13)]
        );
        // `=begin` docs start at column 0.
        assert!(
            run("=begin\ndoc\n=end\nx = 1\n")
                .space_before_comment
                .is_empty()
        );
        // Comment at EOF without a newline.
        assert_eq!(run("x = 1# c").space_before_comment, vec![(5, 8)]);
    }

    #[test]
    fn multibyte_bytes_are_not_ruby_space() {
        // "x = \"あ\"# c" — the byte before `#` is the closing quote.
        let r = run("x = \"あ\"# c\n");
        assert_eq!(r.space_before_comment, vec![(9, 12)]);
    }

    #[test]
    fn interpolation_code_is_scanned() {
        assert_eq!(run("\"#{f(a,b)}\"\n").space_after_comma, vec![(6, 7)]);
        let r = run_cfg("\"#{a ;b}\"\n", default_cfg());
        assert_eq!(r.space_before_semicolon, vec![(4, 5)]);
        assert_eq!(r.space_after_semicolon, vec![(5, 6)]);
    }

    #[test]
    fn pattern_and_index_commas_are_real() {
        assert_eq!(
            run("case x\nin [1,2] then y\nend\n").space_after_comma,
            vec![(12, 13)]
        );
        assert_eq!(run("a[1,2]\n").space_after_comma, vec![(3, 4)]);
        assert_eq!(run("undef :a,:b\n").space_after_comma, vec![(8, 9)]);
    }
}
