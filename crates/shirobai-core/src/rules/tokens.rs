//! parser-gem-compatible token stream, reconstructed from the prism lex tokens.
//!
//! RuboCop's token-based cops (`Layout/SpaceInsideParens`, the comma-spacing
//! cops, `Layout/ExtraSpacing`, …) consume `processed_source.tokens` — the
//! whitequark **parser-gem** token stream (type, begin_pos, end_pos), *not*
//! prism's native tokens. shirobai runs on prism, so to drive those cops we take
//! the prism lex token stream and translate it into the parser-gem types the cop
//! predicates test.
//!
//! The input is the [`RawToken`] stream that `crate::pm_lex` collects during the
//! single parse+lex pass (see [`translate_tokens`]). Those tokens are the exact
//! `Prism.lex` 4-tuple (`type`, `start_offset`, `length`, `lex_state`); this
//! module never lexes on its own.
//!
//! The translation is a partial port of prism's own
//! `lib/prism/translation/parser/lexer.rb`, restricted to the token kinds the
//! cop cluster actually inspects, plus the two whitequark-parity fixes that
//! prism's translator gets wrong (verified against the whitequark oracle over
//! the 2994-file effective Mastodon corpus in the Stage 0 spike):
//!
//! - **(A) `PARENTHESIS_LEFT_PARENTHESES`** — prism's TYPES table always maps it
//!   to `tLPAREN_ARG`, but whitequark emits `tLPAREN` (which the cops treat as a
//!   "left paren") when the preceding token is a `LABEL` / `LABEL_END`
//!   (`foo: (...)` grouping paren) and keeps `tLPAREN_ARG` (which is *not* a left
//!   paren for `Token#left_parens?`) when it follows an identifier
//!   (`foo (...)` command-call arg paren). The distinction is load-bearing for
//!   `Layout/SpaceInsideParens`: `foo (1 )` must not flag the inner edge.
//! - **(B) `tNL` emission** — whitequark suppresses / relocates the synthetic
//!   newline token in inline-comment, heredoc and forward-args contexts. This is
//!   only needed by cops that test `new_line?` (`Layout/ExtraSpacing`). The port
//!   here now reproduces every whitequark `tNL` rule exactly ([`TnlPlan`]):
//!     - an **inline comment**'s trailing `tNL` is suppressed when the comment's
//!       lex_state requests `EXPR_BEG` (`) # comment` etc.), with `is_inline`
//!       decided by the *physical* (begin-order) previous token's line (the
//!       lex-order previous token is unreliable after a heredoc reorder);
//!     - a **heredoc opener** line whose opener carries a trailing method call
//!       (`<<~SQL.squish\n`, physical-prev `IDENTIFIER`) relocates its `tNL` to
//!       the heredoc body's terminating `\n`; bare / parenthesised openers keep
//!       the opener-line `tNL`;
//!     - a **forward-all-args call** (`f(x, ...)\n`) emits its `IGNORED_NEWLINE`
//!       as a `tNL` (a `def f(...)` / bare `f(...)` does not).
//!
//!   Verified `new_line?` 2994/2994 against the whitequark oracle.
//!
//! Offsets are **byte** offsets (prism native); the Ruby wrapper converts them
//! to character offsets through `Shirobai::SourceOffsets` like every other cop.

use std::collections::HashSet;

use crate::pm_lex::{PM_TOKEN_NAMES, RawToken};

/// The parser-gem token kinds the cop-cluster predicates distinguish. Every
/// other token is `Other` (its position still matters for adjacency, but its
/// type is never tested).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParserTokenType {
    /// `tLPAREN` / `tLPAREN2` — `Token#left_parens?`.
    LParen,
    /// `tRPAREN` — `Token#right_parens?`.
    RParen,
    /// `tLCURLY` / `tLAMBEG` — `Token#left_curly_brace?`.
    LCurly,
    /// `tLBRACE` — `Token#left_brace?` (a `{` opening a hash literal / block arg,
    /// emitted in the `EXPR_BEG|EXPR_LABEL` lex state). Distinct from `LCurly`:
    /// `left_curly_brace?` does *not* include it.
    LBrace,
    /// `tRCURLY` — `Token#right_curly_brace?`.
    RCurly,
    /// `tRBRACK` — `Token#right_bracket?`.
    RBracket,
    /// `tPIPE` — a `|` token (block-parameter / bitwise-or pipe). No dedicated
    /// `Token#` predicate; tested by type in `SpaceAfterPunctuation#allowed_type?`.
    Pipe,
    /// `tSTRING_DEND` — the `}` closing a string interpolation (`#{...}`). No
    /// dedicated `Token#` predicate; tested by type in
    /// `SpaceAfterPunctuation#allowed_type?`.
    StringDEnd,
    /// `tCOMMA` — `Token#comma?`.
    Comma,
    /// `tSEMI` — `Token#semicolon?`.
    Semicolon,
    /// `tCOMMENT` — `Token#comment?`.
    Comment,
    /// `tNL` — `Token#new_line?`.
    NewLine,
    /// `tEQL` / `tOP_ASGN` — `Token#equal_sign?`.
    EqualSign,
    /// One of `ASSIGNMENT_OR_COMPARISON_TOKENS` minus the `equal_sign?` pair:
    /// `tEQ tEQQ tNEQ tLEQ tGEQ tLSHFT` (used by `Layout/SpaceAroundOperators`).
    Comparison,
    /// Any other token — type irrelevant, position preserved.
    Other,
}

/// A parser-gem token: its classified `kind` and byte range `[begin_pos, end_pos)`.
#[derive(Clone, Copy, Debug)]
pub struct Token {
    pub kind: ParserTokenType,
    pub begin_pos: usize,
    pub end_pos: usize,
}

impl Token {
    /// `Token#left_parens?` (`tLPAREN` / `tLPAREN2`).
    pub fn left_parens(&self) -> bool {
        self.kind == ParserTokenType::LParen
    }
    /// `Token#right_parens?` (`tRPAREN`).
    pub fn right_parens(&self) -> bool {
        self.kind == ParserTokenType::RParen
    }
    /// `Token#left_curly_brace?` (`tLCURLY` / `tLAMBEG`).
    pub fn left_curly_brace(&self) -> bool {
        self.kind == ParserTokenType::LCurly
    }
    /// `Token#left_brace?` (`tLBRACE`).
    pub fn left_brace(&self) -> bool {
        self.kind == ParserTokenType::LBrace
    }
    /// `Token#right_curly_brace?` (`tRCURLY`).
    pub fn right_curly_brace(&self) -> bool {
        self.kind == ParserTokenType::RCurly
    }
    /// `Token#right_bracket?` (`tRBRACK`).
    pub fn right_bracket(&self) -> bool {
        self.kind == ParserTokenType::RBracket
    }
    /// A `tPIPE` token (`|`). Used by `SpaceAfterPunctuation#allowed_type?`.
    pub fn pipe(&self) -> bool {
        self.kind == ParserTokenType::Pipe
    }
    /// A `tSTRING_DEND` token (`}` closing `#{...}`). Used by
    /// `SpaceAfterPunctuation#allowed_type?`.
    pub fn string_dend(&self) -> bool {
        self.kind == ParserTokenType::StringDEnd
    }
    /// `Token#comma?` (`tCOMMA`).
    pub fn comma(&self) -> bool {
        self.kind == ParserTokenType::Comma
    }
    /// `Token#semicolon?` (`tSEMI`).
    pub fn semicolon(&self) -> bool {
        self.kind == ParserTokenType::Semicolon
    }
    /// `Token#comment?` (`tCOMMENT`).
    pub fn comment(&self) -> bool {
        self.kind == ParserTokenType::Comment
    }
    /// `Token#new_line?` (`tNL`).
    pub fn new_line(&self) -> bool {
        self.kind == ParserTokenType::NewLine
    }
    /// `Token#equal_sign?` (`tEQL` / `tOP_ASGN`).
    pub fn equal_sign(&self) -> bool {
        self.kind == ParserTokenType::EqualSign
    }
    /// Membership in `PrecedingFollowingAlignment::ASSIGNMENT_OR_COMPARISON_TOKENS`
    /// (`tEQL tEQ tEQQ tNEQ tLEQ tGEQ tOP_ASGN tLSHFT`).
    pub fn assignment_or_comparison(&self) -> bool {
        matches!(
            self.kind,
            ParserTokenType::EqualSign | ParserTokenType::Comparison
        )
    }
}

// --- prism lex token adapter ---------------------------------------------

/// A `Copy` snapshot of one prism lex token. Mirrors [`RawToken`] but with the
/// fields the translation loop reads by value; the translation was written
/// against this shape (many `let tk = raws[i];` copies), so we keep it and map
/// the borrowed [`RawToken`] stream into it once at entry.
#[derive(Clone, Copy)]
struct RawTok {
    type_id: u32,
    start: usize,
    len: usize,
    lex_state: u32,
}

/// Prism token symbol name for a `pm_token_type` integer. Shares the single
/// `PM_TOKEN_NAMES` table with `crate::pm_lex`; out-of-range ids (never emitted
/// by the lexer) map to `""`.
fn prism_token_name(id: u32) -> &'static str {
    PM_TOKEN_NAMES.get(id as usize).copied().unwrap_or("")
}

const EXPR_BEG: u32 = 0x1;
const EXPR_LABEL: u32 = 0x400;

/// Classify a Prism token name (post-(A)-fix) into a [`ParserTokenType`].
fn classify(prism_name: &str) -> ParserTokenType {
    match prism_name {
        "PARENTHESIS_LEFT" => ParserTokenType::LParen, // tLPAREN2
        // PARENTHESIS_LEFT_PARENTHESES is resolved by the (A) fix before this.
        "PARENTHESIS_RIGHT" => ParserTokenType::RParen,
        "BRACE_LEFT" => ParserTokenType::LCurly, // tLCURLY (EXPR_BEG|LABEL -> tLBRACE, see below)
        "LAMBDA_BEGIN" => ParserTokenType::LCurly, // tLAMBEG
        "BRACE_RIGHT" => ParserTokenType::RCurly,
        "BRACKET_RIGHT" => ParserTokenType::RBracket, // tRBRACK
        "PIPE" => ParserTokenType::Pipe,              // tPIPE
        "EMBEXPR_END" => ParserTokenType::StringDEnd, // tSTRING_DEND
        "COMMA" => ParserTokenType::Comma,
        "SEMICOLON" => ParserTokenType::Semicolon,
        "COMMENT" | "EMBDOC_BEGIN" => ParserTokenType::Comment,
        "NEWLINE" => ParserTokenType::NewLine,
        "EQUAL" => ParserTokenType::EqualSign, // tEQL
        // tOP_ASGN family
        "AMPERSAND_AMPERSAND_EQUAL" | "AMPERSAND_EQUAL" | "CARET_EQUAL" | "GREATER_GREATER_EQUAL"
        | "LESS_LESS_EQUAL" | "MINUS_EQUAL" | "PERCENT_EQUAL" | "PIPE_EQUAL" | "PIPE_PIPE_EQUAL"
        | "PLUS_EQUAL" | "SLASH_EQUAL" | "STAR_EQUAL" | "STAR_STAR_EQUAL" => {
            ParserTokenType::EqualSign
        }
        // ASSIGNMENT_OR_COMPARISON_TOKENS minus equal_sign?: tEQ tEQQ tNEQ tLEQ tGEQ tLSHFT
        "EQUAL_EQUAL" | "EQUAL_EQUAL_EQUAL" | "BANG_EQUAL" | "LESS_EQUAL" | "GREATER_EQUAL"
        | "LESS_LESS" => ParserTokenType::Comparison,
        _ => ParserTokenType::Other,
    }
}

/// Precomputed `tNL` decisions that need physical (begin-position) ordering or a
/// global view of the token stream — the heredoc-opener relocation, the
/// forward-args `IGNORED_NEWLINE`, and the inline-comment classification.
/// Resolved once over the begin-sorted raw stream, then consulted by the
/// lex-order emission loop in [`convert`]. All keys are byte offsets.
struct TnlPlan {
    /// `NEWLINE` begin positions to *suppress* (heredoc-opener lines whose opener
    /// carries a trailing method call, e.g. `<<~SQL.squish\n`).
    heredoc_suppress: HashSet<usize>,
    /// `HEREDOC_END` end positions whose trailing `\n` should emit the relocated
    /// `tNL` (one per suppressed opener).
    heredoc_end_nl: HashSet<usize>,
    /// `IGNORED_NEWLINE` begin positions to emit as `tNL` (forward-all-args
    /// *calls*, `f(x, ...)\n` — not `def`s / bare `f(...)`).
    forward_args: HashSet<usize>,
    /// `COMMENT` begin positions that are inline (their physically preceding
    /// token shares their line).
    comment_inline: HashSet<usize>,
}

impl TnlPlan {
    fn build(raws: &[RawTok], source: &[u8]) -> TnlPlan {
        let mut heredoc_suppress = HashSet::new();
        let mut heredoc_end_nl = HashSet::new();
        let mut forward_args = HashSet::new();
        let mut comment_inline = HashSet::new();

        // Begin-sorted view of the raws (the lex stream reorders heredoc bodies).
        let mut order: Vec<usize> = (0..raws.len()).collect();
        order.sort_by_key(|&j| raws[j].start);
        let name = |j: usize| prism_token_name(raws[j].type_id);
        let is_nl = |j: usize| matches!(name(j), "NEWLINE" | "IGNORED_NEWLINE");

        // Forward-args: `… (something) UDOT_DOT_DOT PARENTHESIS_RIGHT
        // IGNORED_NEWLINE`, where the token before `...` is *not* `(` (that would
        // be a `def f(...)` / bare `f(...)`, whose newline stays ignored).
        for (k, tk) in raws.iter().enumerate() {
            if name(k) != "IGNORED_NEWLINE" {
                continue;
            }
            if k >= 3
                && name(k - 1) == "PARENTHESIS_RIGHT"
                && name(k - 2) == "UDOT_DOT_DOT"
                && name(k - 3) != "PARENTHESIS_LEFT"
            {
                forward_args.insert(tk.start);
            }
        }

        // Heredoc-opener relocation: for each `HEREDOC_START`, find the terminator
        // (`NEWLINE`/`IGNORED_NEWLINE`) on its line; if it is a `NEWLINE` whose
        // physically previous non-newline token is an `IDENTIFIER` (a method call
        // chained onto the heredoc), suppress that `NEWLINE` and mark the heredoc
        // body's terminating `HEREDOC_END` to emit the `tNL` instead.
        for (k, hs) in raws.iter().enumerate() {
            if name(k) != "HEREDOC_START" {
                continue;
            }
            let hs_line = line_of(source, hs.start);
            // The terminator newline on this line, in begin order.
            let term = order
                .iter()
                .copied()
                .find(|&j| is_nl(j) && line_of(source, raws[j].start) == hs_line);
            let Some(term) = term else { continue };
            if name(term) != "NEWLINE" {
                continue;
            }
            // Physical previous non-newline token before the terminator.
            let prev = order
                .iter()
                .rev()
                .copied()
                .find(|&j| raws[j].start + raws[j].len <= raws[term].start && !is_nl(j));
            if prev.map(name) != Some("IDENTIFIER") {
                continue;
            }
            // The heredoc body end: the first `HEREDOC_END` beginning after the
            // opener (in begin order), matching this heredoc.
            let hend = order
                .iter()
                .copied()
                .find(|&j| name(j) == "HEREDOC_END" && raws[j].start > hs.start);
            if let Some(hend) = hend {
                let hend_end = raws[hend].start + raws[hend].len;
                // The relocated `tNL` is `HEREDOC_END`'s trailing `\n`; only
                // record it when the end really sits on a newline.
                if hend_end > 0 && source.get(hend_end - 1) == Some(&b'\n') {
                    heredoc_suppress.insert(raws[term].start);
                    heredoc_end_nl.insert(hend_end);
                }
            }
        }

        // Inline comments: a comment whose physically previous non-newline token
        // shares its line. (Heredoc reordering makes the lex-previous token
        // unreliable here, so we use begin order.)
        for (k, c) in raws.iter().enumerate() {
            if name(k) != "COMMENT" {
                continue;
            }
            let c_line = line_of(source, c.start);
            let prev = order
                .iter()
                .rev()
                .copied()
                .find(|&j| raws[j].start + raws[j].len <= c.start && !is_nl(j));
            if prev.is_some_and(|prev| line_of(source, raws[prev].start) == c_line) {
                comment_inline.insert(c.start);
            }
        }

        TnlPlan {
            heredoc_suppress,
            heredoc_end_nl,
            forward_args,
            comment_inline,
        }
    }
}

/// Translate the decoded Prism token stream into the parser-gem token stream the
/// cop predicates consume. See the module docs for the (A)/(B) fixes.
fn convert(raws: &[RawTok], source: &[u8]) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(raws.len());
    let mut pending_comment_nl: Option<(usize, usize)> = None;
    let mut i = 0usize;
    let n = raws.len();

    // tNL precomputation for the heredoc / forward-args / inline-comment rules
    // (whitequark parity, verified against the corpus oracle). These need
    // physical (begin-position) ordering and lex_state, so they are resolved
    // here before the lex-order emission loop below consults them.
    let nl = TnlPlan::build(raws, source);

    while i < n {
        let tk = raws[i];
        let prism_name = prism_token_name(tk.type_id);
        let b = tk.start;
        let e = tk.start + tk.len;
        i += 1;

        match prism_name {
            // `IGNORED_NEWLINE` is normally dropped, but whitequark emits a `tNL`
            // for the one terminating a forward-all-args *call* (`f(x, ...)\n`).
            "IGNORED_NEWLINE" => {
                if nl.forward_args.contains(&b) {
                    out.push(Token {
                        kind: ParserTokenType::NewLine,
                        begin_pos: b,
                        end_pos: e,
                    });
                }
                continue;
            }
            // Never emitted by the parser-gem stream.
            "__END__" | "EOF" | "MISSING" | "NOT_PROVIDED" | "EMBDOC_END" | "EMBDOC_LINE" => {
                continue
            }
            _ => {}
        }

        // (A) PARENTHESIS_LEFT_PARENTHESES: tLPAREN after a label (a left paren
        // for the cops), tLPAREN_ARG otherwise (NOT a left paren).
        if prism_name == "PARENTHESIS_LEFT_PARENTHESES" {
            let prev_name = if i >= 2 {
                prism_token_name(raws[i - 2].type_id)
            } else {
                ""
            };
            let kind = if prev_name == "LABEL" || prev_name == "LABEL_END" {
                ParserTokenType::LParen
            } else {
                ParserTokenType::Other // tLPAREN_ARG: not left_parens?
            };
            out.push(Token {
                kind,
                begin_pos: b,
                end_pos: e,
            });
            continue;
        }

        // `SYMBOL_BEGIN` (`:`) introduces an operator/identifier symbol like
        // `:|`, `:[]`, `:==`. prism lexes the operator as its own token
        // (e.g. `:|` -> `SYMBOL_BEGIN` + `PIPE`), but whitequark's translator
        // merges the two into a single `tSYMBOL` (consuming the next token)
        // unless the symbol body is a string (`:"#{x}"` -> `STRING_CONTENT` /
        // `EMBEXPR_BEGIN` / `EMBVAR` / `STRING_END`). Reproduce the merge so the
        // absorbed operator does not surface as a `tPIPE` / `tRBRACK` / ... token
        // (it is symbol content, not punctuation). `&:|` in the corpus is the
        // case that requires this.
        if prism_name == "SYMBOL_BEGIN" {
            let next_is_string_part = matches!(
                raws.get(i).map(|t| prism_token_name(t.type_id)),
                Some("STRING_CONTENT") | Some("EMBEXPR_BEGIN") | Some("EMBVAR")
                    | Some("STRING_END")
            );
            if let Some(next) = raws.get(i).filter(|_| !next_is_string_part) {
                // Merge `:` + operator into one Other token spanning both.
                let merged_end = next.start + next.len;
                out.push(Token {
                    kind: ParserTokenType::Other,
                    begin_pos: b,
                    end_pos: merged_end,
                });
                i += 1;
                continue;
            }
        }

        // A non-interpolated string literal is one `tSTRING` token in the
        // parser-gem stream, but prism splits every string into `STRING_BEGIN` /
        // `STRING_CONTENT`* / `STRING_END`. whitequark's translator merges the
        // parts back into a single `tSTRING` when there is no interpolation, and
        // keeps them split (`tSTRING_BEG` / content / `tSTRING_END`) when there
        // is. Reproduce the merge: scan forward from `STRING_BEGIN` over
        // `STRING_CONTENT` runs; if the next token is `STRING_END` the string is
        // plain, so emit one token spanning both delimiters. An interpolation
        // part (`EMBEXPR_BEGIN` / `EMBVAR` / ...) stops the scan and the
        // `STRING_BEGIN` falls through to a lone `tSTRING_BEG`-shaped token, as
        // before. This matters for `Layout/ExtraSpacing`, which walks the whole
        // token stream as adjacent pairs: without the merge a bare opening `"`
        // would spuriously align (via `aligned_words?`) with a `"` one line away.
        if prism_name == "STRING_BEGIN" {
            let mut j = i; // `i` already points past this STRING_BEGIN.
            while j < n && prism_token_name(raws[j].type_id) == "STRING_CONTENT" {
                j += 1;
            }
            if j < n && prism_token_name(raws[j].type_id) == "STRING_END" {
                let merged_end = raws[j].start + raws[j].len;
                out.push(Token {
                    kind: ParserTokenType::Other,
                    begin_pos: b,
                    end_pos: merged_end,
                });
                i = j + 1;
                continue;
            }
        }

        let mut kind = classify(prism_name);

        // A `{` in the EXPR_BEG|EXPR_LABEL lex state opens a hash literal / block
        // argument: parser emits tLBRACE (left_brace?), not tLCURLY
        // (left_curly_brace?). prism's translator reproduces this from lex_state.
        if prism_name == "BRACE_LEFT" && tk.lex_state == (EXPR_BEG | EXPR_LABEL) {
            kind = ParserTokenType::LBrace;
        }

        match prism_name {
            // Line comments: parser trims the trailing newline off the comment
            // text and emits a following tNL out of order. Reproduce the
            // position adjustment + tNL relocation (needed for positional
            // correctness of `comment?` ranges; the tNL suppression rules are
            // only consumed by ExtraSpacing).
            "COMMENT" => {
                let ends_nl = e > b && source.get(e - 1) == Some(&b'\n');
                let cend = if ends_nl { e - 1 } else { e };
                // `is_inline_comment` (whitequark): the physically preceding
                // token is on the comment's line. Unlike prism's translator,
                // which uses the *lex-order* previous token, we use the
                // physical (begin-order) previous one — heredocs reorder the lex
                // stream, so the lex-previous token of a comment trailing a
                // heredoc opener sits on a different line. A comment whose own
                // lex_state requests `EXPR_BEG` does not get a trailing `tNL`
                // (`) # comment` etc.), matching the oracle.
                let inline = nl.comment_inline.contains(&b);
                let suppressed = (tk.lex_state & EXPR_BEG) != 0;
                let prev_same_line = inline && !suppressed;
                let next = raws.get(i);
                let next_is_comment =
                    next.map(|t| prism_token_name(t.type_id)) == Some("COMMENT");
                let next_is_cont = matches!(
                    next.map(|t| prism_token_name(t.type_id)),
                    Some("COMMENT") | Some("AMPERSAND_DOT") | Some("DOT")
                );
                if prev_same_line && ends_nl && !next_is_cont {
                    out.push(Token {
                        kind: ParserTokenType::Comment,
                        begin_pos: b,
                        end_pos: cend,
                    });
                    out.push(Token {
                        kind: ParserTokenType::NewLine,
                        begin_pos: e - 1,
                        end_pos: e,
                    });
                    continue;
                } else if prev_same_line && next_is_comment {
                    pending_comment_nl = Some((e - 1, e));
                    out.push(Token {
                        kind: ParserTokenType::Comment,
                        begin_pos: b,
                        end_pos: cend,
                    });
                    continue;
                } else if pending_comment_nl.is_some() && !next_is_cont {
                    let (nb, ne) = pending_comment_nl.take().unwrap();
                    out.push(Token {
                        kind: ParserTokenType::Comment,
                        begin_pos: b,
                        end_pos: cend,
                    });
                    out.push(Token {
                        kind: ParserTokenType::NewLine,
                        begin_pos: nb,
                        end_pos: ne,
                    });
                    continue;
                }
                out.push(Token {
                    kind: ParserTokenType::Comment,
                    begin_pos: b,
                    end_pos: cend,
                });
                continue;
            }
            // Newlines preceding a comment are emitted out of order.
            "NEWLINE" => {
                // whitequark relocates the `tNL` terminating a heredoc-opener
                // line whose opener carries a trailing method call
                // (`<<~SQL.squish\n`): the newline moves to the heredoc body's
                // end. Suppress it here; the matching `HEREDOC_END` branch emits
                // the relocated `tNL`.
                if nl.heredoc_suppress.contains(&b) {
                    continue;
                }
                let next = raws.get(i);
                if next.map(|t| prism_token_name(t.type_id)) == Some("COMMENT") {
                    pending_comment_nl = Some((b, e));
                    continue;
                }
            }
            // A heredoc-opener line whose opener carries a trailing method call
            // gets its `tNL` relocated to the heredoc body's terminating newline
            // (the last byte of `HEREDOC_END`; the plan only records ends that
            // really sit on a `\n`).
            "HEREDOC_END" if nl.heredoc_end_nl.contains(&e) => {
                out.push(Token {
                    kind: ParserTokenType::NewLine,
                    begin_pos: e - 1,
                    end_pos: e,
                });
            }
            _ => {}
        }

        out.push(Token {
            kind,
            begin_pos: b,
            end_pos: e,
        });
    }
    out
}

fn line_of(source: &[u8], off: usize) -> usize {
    let end = off.min(source.len());
    source[..end].iter().filter(|&&c| c == b'\n').count()
}

/// Translate the prism lex token stream (`raw`, collected by `crate::pm_lex`
/// during the shared parse+lex pass) into the parser-gem token stream in
/// `processed_source.sorted_tokens` order: tokens sorted by `begin_pos`, ties
/// broken by lex order (a stable sort). Most tokens already come out of prism in
/// source order, but heredocs are the exception — prism emits the heredoc
/// body/end tokens contiguously after the opening `<<~ID`, so a token that
/// *starts* before them (e.g. the `)` closing the call that owns the heredoc)
/// appears later in lex order. whitequark's `sorted_tokens` re-sorts by
/// `begin_pos`, and the cops in this cluster consume `sorted_tokens`, so we
/// reproduce that ordering here.
///
/// `source` is the exact bytes `raw` was lexed from; the two must come from the
/// same parse (as `crate::rules::parse_cache::with_parsed_and_tokens` guarantees).
#[must_use]
pub fn translate_tokens(source: &[u8], raw: &[RawToken]) -> Vec<Token> {
    // Adapt the borrowed RawToken stream into the `Copy` shape the translation
    // was written against. The two carry the same 4-tuple; this only renames.
    let raws: Vec<RawTok> = raw
        .iter()
        .map(|t| RawTok {
            type_id: t.token_type,
            start: t.start_offset,
            len: t.length,
            lex_state: t.lex_state,
        })
        .collect();
    let mut toks = convert(&raws, source);
    // Stable sort by begin_pos (matches `tokens.sort_by.with_index { [bp, i] }`).
    toks.sort_by_key(|t| t.begin_pos);
    toks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pm_lex::parse_with_lex;

    /// Lex `src` through the shared parse+lex path and translate it, so the unit
    /// tests exercise the real production input (the `RawToken` stream) instead
    /// of a bespoke lexer.
    fn ranges(src: &[u8]) -> Vec<Token> {
        let (_result, raw) = parse_with_lex(src);
        translate_tokens(src, &raw)
    }

    // A method-call paren is tLPAREN2 (a left paren); its matching close is tRPAREN.
    #[test]
    fn method_call_parens_are_left_and_right() {
        let toks = ranges(b"f(3)\n");
        let lp = toks.iter().find(|t| t.left_parens());
        let rp = toks.iter().find(|t| t.right_parens());
        assert_eq!(lp.map(|t| (t.begin_pos, t.end_pos)), Some((1, 2)));
        assert_eq!(rp.map(|t| (t.begin_pos, t.end_pos)), Some((3, 4)));
    }

    // (A): a command-call arg paren `foo (...)` is tLPAREN_ARG, NOT a left paren.
    #[test]
    fn command_call_arg_paren_is_not_left_paren() {
        let toks = ranges(b"foo (1)\n");
        // The `(` at byte 4 must not be classified as a left paren.
        let at4 = toks.iter().find(|t| t.begin_pos == 4).unwrap();
        assert!(!at4.left_parens());
        assert_eq!(at4.kind, ParserTokenType::Other);
        // Its `)` is still a right paren.
        assert!(toks.iter().any(|t| t.right_parens() && t.begin_pos == 6));
    }

    // (A): the grouping paren after a label `foo(bar: (...))` is tLPAREN (a left paren).
    #[test]
    fn label_grouping_paren_is_left_paren() {
        let toks = ranges(b"foo(bar: (1 + 2))\n");
        // `foo(` at 3 is tLPAREN2 (left), inner `(` at 9 (after label) is tLPAREN (left).
        assert!(toks.iter().any(|t| t.left_parens() && t.begin_pos == 3));
        assert!(toks.iter().any(|t| t.left_parens() && t.begin_pos == 9));
        // Both closing parens are right parens.
        assert_eq!(toks.iter().filter(|t| t.right_parens()).count(), 2);
    }

    // A line comment's range excludes its trailing newline; a tNL follows it.
    #[test]
    fn line_comment_excludes_newline() {
        let toks = ranges(b"x = 1 # hi\n");
        let c = toks.iter().find(|t| t.comment()).unwrap();
        // "# hi" spans bytes 6..10 (the `\n` at 10 is excluded).
        assert_eq!((c.begin_pos, c.end_pos), (6, 10));
    }

    // comma / semicolon / equal-sign classification.
    #[test]
    fn punctuation_kinds() {
        let toks = ranges(b"a = 1; f(b, c)\n");
        assert!(toks.iter().any(|t| t.equal_sign()));
        assert!(toks.iter().any(|t| t.semicolon()));
        assert!(toks.iter().any(|t| t.comma()));
    }

    // Curly braces: a block brace is tLCURLY; the close is tRCURLY.
    #[test]
    fn block_braces_kinds() {
        let toks = ranges(b"foo { |x| x }\n");
        assert!(toks.iter().any(|t| t.left_curly_brace()));
        assert!(toks.iter().any(|t| t.right_curly_brace()));
    }

    // Heredoc tokens come out of the prism lexer out of begin-position order
    // (body/end after the opener); translate_tokens must re-sort so the `)` that
    // begins before the heredoc body lands right after the opener, as
    // `sorted_tokens` does.
    #[test]
    fn heredoc_tokens_sorted_by_begin_pos() {
        let toks = ranges(b"f(<<~HEREDOC)\n  text\nHEREDOC\n");
        let positions: Vec<usize> = toks.iter().map(|t| t.begin_pos).collect();
        let mut sorted = positions.clone();
        sorted.sort_unstable();
        assert_eq!(positions, sorted, "tokens must be in begin_pos order");
        // The `)` (a right paren at byte 12) must appear before the heredoc body.
        let rp = toks.iter().position(|t| t.right_parens()).unwrap();
        let beg = toks
            .iter()
            .position(|t| t.begin_pos == 2)
            .unwrap(); // tSTRING_BEG `<<~HEREDOC`
        assert!(rp == beg + 1, "the close paren follows the heredoc opener");
    }

    // Corpus parity regression for the promoted token translation. Compares the
    // module's predicate true-sets (begin_pos, end_pos) against the whitequark
    // parser-gem oracle (the stream RuboCop actually consumes), reproducing the
    // Stage 0 spike numbers. Ignored by default — it needs the corpus oracle
    // dump; run with the path in `SHIROBAI_TOKEN_ORACLE`:
    //
    //   SHIROBAI_TOKEN_ORACLE=.tmp/2026-06-14/pm-lex-spike/oracle_all.jsonl \
    //     SHIROBAI_CORPUS_BASE=<repo root that .tmp/mastodon lives under> \
    //     cargo test -p shirobai-core --release token_oracle_corpus_parity -- --ignored --nocapture
    //
    // The oracle JSONL is one record per file: {"path","tokens":[{"t","b","e"}],
    // "valid_syntax":bool|"error":...}. Offsets are byte offsets.
    #[test]
    #[ignore = "needs corpus oracle dump (SHIROBAI_TOKEN_ORACLE)"]
    fn token_oracle_corpus_parity() {
        use std::collections::BTreeSet;

        let oracle_path = match std::env::var("SHIROBAI_TOKEN_ORACLE") {
            Ok(p) => p,
            Err(_) => return,
        };
        let data = std::fs::read_to_string(&oracle_path).expect("read oracle dump");

        // Predicate -> the parser-gem type names that satisfy it (oracle side).
        // Every predicate the cops on this token base consume is asserted to
        // match the whitequark oracle on every effective file, `new_line?`
        // included (the `Layout/ExtraSpacing` `tNL` rules; see `TnlPlan`).
        #[allow(clippy::type_complexity)]
        let predicates: &[(&str, &[&str], fn(&Token) -> bool)] = &[
            ("left_parens?", &["tLPAREN", "tLPAREN2"], Token::left_parens),
            ("right_parens?", &["tRPAREN"], Token::right_parens),
            ("comment?", &["tCOMMENT"], Token::comment),
            (
                "left_curly_brace?",
                &["tLCURLY", "tLAMBEG"],
                Token::left_curly_brace,
            ),
            ("comma?", &["tCOMMA"], Token::comma),
            ("semicolon?", &["tSEMI"], Token::semicolon),
            ("equal_sign?", &["tEQL", "tOP_ASGN"], Token::equal_sign),
            ("right_curly_brace?", &["tRCURLY"], Token::right_curly_brace),
            ("right_bracket?", &["tRBRACK"], Token::right_bracket),
            ("pipe", &["tPIPE"], Token::pipe),
            ("string_dend", &["tSTRING_DEND"], Token::string_dend),
            ("new_line?", &["tNL"], Token::new_line),
        ];

        let mut effective = 0usize;
        let mut match_counts = vec![0usize; predicates.len()];
        let mut mismatches: Vec<(String, &str)> = Vec::new();

        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let rec = parse_oracle_record(line);
            let Some(rec) = rec else { continue };
            if rec.error || !rec.valid_syntax {
                continue;
            }
            // Oracle paths are relative to the repo root; resolve against
            // SHIROBAI_CORPUS_BASE when set (cargo test's cwd is the crate dir).
            let full = match std::env::var("SHIROBAI_CORPUS_BASE") {
                Ok(base) => std::path::Path::new(&base).join(&rec.path),
                Err(_) => std::path::PathBuf::from(&rec.path),
            };
            let source = match std::fs::read(&full) {
                Ok(s) => s,
                Err(_) => continue,
            };
            effective += 1;
            let (_result, raw) = parse_with_lex(&source);
            let toks = translate_tokens(&source, &raw);

            for (idx, (name, oracle_types, pred)) in predicates.iter().enumerate() {
                let oracle_set: BTreeSet<(usize, usize)> = rec
                    .tokens
                    .iter()
                    .filter(|(t, _, _)| oracle_types.contains(&t.as_str()))
                    .map(|&(_, b, e)| (b, e))
                    .collect();
                let mine: BTreeSet<(usize, usize)> = toks
                    .iter()
                    .filter(|t| pred(t))
                    .map(|t| (t.begin_pos, t.end_pos))
                    .collect();
                if oracle_set == mine {
                    match_counts[idx] += 1;
                } else if mismatches.len() < 20 {
                    mismatches.push((rec.path.clone(), name));
                }
            }
        }

        eprintln!("effective files: {effective}");
        for (idx, (name, _, _)) in predicates.iter().enumerate() {
            eprintln!("  {name}: {}/{}", match_counts[idx], effective);
        }
        if !mismatches.is_empty() {
            eprintln!("first mismatches: {mismatches:?}");
        }
        assert!(effective > 2000, "expected a real corpus, got {effective}");
        for (idx, (name, _, _)) in predicates.iter().enumerate() {
            assert_eq!(
                match_counts[idx], effective,
                "predicate {name} diverged on {} files",
                effective - match_counts[idx]
            );
        }
    }

    struct OracleRecord {
        path: String,
        tokens: Vec<(String, usize, usize)>,
        valid_syntax: bool,
        error: bool,
    }

    /// Minimal hand parser for the oracle JSONL records (avoids a serde dep).
    fn parse_oracle_record(line: &str) -> Option<OracleRecord> {
        let path = json_string_field(line, "\"path\":")?;
        let error = line.contains("\"error\":");
        let valid_syntax = !line.contains("\"valid_syntax\":false");
        let mut tokens = Vec::new();
        // Each token object: {"t":"tX","b":N,"e":M}
        let mut rest = line;
        while let Some(tpos) = rest.find("{\"t\":") {
            rest = &rest[tpos..];
            let t = json_string_field(rest, "\"t\":")?;
            let b = json_uint_field(rest, "\"b\":")?;
            let e = json_uint_field(rest, "\"e\":")?;
            tokens.push((t, b, e));
            rest = &rest[5..];
        }
        Some(OracleRecord {
            path,
            tokens,
            valid_syntax,
            error,
        })
    }

    fn json_string_field(s: &str, key: &str) -> Option<String> {
        let pos = s.find(key)? + key.len();
        let bytes = s.as_bytes();
        let mut i = pos;
        while i < bytes.len() && bytes[i] != b'"' {
            i += 1;
        }
        i += 1; // opening quote
        let start = i;
        let mut val = String::new();
        while i < bytes.len() && bytes[i] != b'"' {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                i += 1;
                val.push(match bytes[i] {
                    b'n' => '\n',
                    b't' => '\t',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'"' => '"',
                    other => other as char,
                });
            } else {
                val.push(bytes[i] as char);
            }
            i += 1;
        }
        let _ = start;
        Some(val)
    }

    fn json_uint_field(s: &str, key: &str) -> Option<usize> {
        let pos = s.find(key)? + key.len();
        let bytes = s.as_bytes();
        let mut i = pos;
        while i < bytes.len() && !bytes[i].is_ascii_digit() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        s[start..i].parse().ok()
    }
}

