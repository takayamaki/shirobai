//! `Naming/AsciiIdentifiers`.
//!
//! Stock's `on_new_investigation`
//! (`vendor/rubocop/lib/rubocop/cop/naming/ascii_identifiers.rb`):
//!
//! ```ruby
//! def on_new_investigation
//!   processed_source.tokens.each do |token|
//!     next if !should_check?(token) || token.text.ascii_only?
//!     message = token.type == :tIDENTIFIER ? IDENTIFIER_MSG : CONSTANT_MSG
//!     add_offense(first_offense_range(token), message: message)
//!   end
//! end
//!
//! def should_check?(token)
//!   token.type == :tIDENTIFIER || (token.type == :tCONSTANT && cop_config['AsciiConstants'])
//! end
//! ```
//!
//! The cop iterates the parser-gem token stream and flags every `tIDENTIFIER`
//! (always) and `tCONSTANT` (when `AsciiConstants`) whose text has a non-ASCII
//! byte, at the FIRST maximal run of non-ASCII chars inside the token
//! (`/[^[:ascii:]]+/`). No autocorrect.
//!
//! # Why this is cheap
//!
//! Only a file that HAS a non-ASCII byte can produce an offense
//! (`token.text.ascii_only?` skips every token otherwise). So the common case
//! — an all-ASCII file — is a single `is_ascii` byte scan that returns nothing,
//! with no token stream built at all. Only the rare non-ASCII file falls
//! through to the lex.
//!
//! # Token vocabulary (parser-gem `tIDENTIFIER` / `tCONSTANT` from prism lex)
//!
//! shirobai has no parser-gem token stream; it reads prism's own lex tokens
//! (the same `Prism.lex` stream `crate::pm_lex` collects) and maps them to the
//! parser-gem distinction stock tests. Probed against stock over every
//! non-ASCII file in the five corpora (0 divergences), the mapping is:
//!
//! - prism `IDENTIFIER` -> `tIDENTIFIER` (flag), UNLESS it is a symbol body
//!   (`:foo`): prism lexes the body as `IDENTIFIER` but parser-gem makes the
//!   whole thing `tSYMBOL`. A symbol body is exactly an `IDENTIFIER` /
//!   `CONSTANT` whose immediately preceding token is `SYMBOL_BEGIN`.
//! - prism `CONSTANT` -> `tCONSTANT` (flag when `AsciiConstants`), same
//!   `SYMBOL_BEGIN` exclusion — BUT only when the name starts with an ASCII
//!   `A`-`Z`. parser-gem treats a Unicode-uppercase (non-ASCII) start —
//!   Cyrillic `Ф`, Greek `Ω`, full-width `Ａ`, ... — as a `tIDENTIFIER`, while
//!   prism lexes it as `CONSTANT`. So a prism `CONSTANT` with a non-ASCII first
//!   byte is re-mapped to an identifier: flagged with the identifier message
//!   regardless of `AsciiConstants` (identifiers are always checked).
//! - prism `METHOD_NAME` -> normally `tFID` (a `foo?` / `foo!` call, a
//!   `def self.foo!` singleton name, a `:foo!` symbol, `alias`/`undef`
//!   operands) which stock does NOT flag — EXCEPT an INSTANCE method
//!   definition name `def foo!` / `def foo?`, which parser-gem tokenizes as
//!   `tIDENTIFIER` (flag). That case is exactly a `METHOD_NAME` whose
//!   immediately preceding token is `KEYWORD_DEF` (`def self.foo!` puts a
//!   `DOT` right before the name, so it is excluded). `def foo=` and `def foo`
//!   are already prism `IDENTIFIER`, so only the `!` / `?` def-name needs this.
//! - every other prism token type (`LABEL`, `INSTANCE_VARIABLE`,
//!   `CLASS_VARIABLE`, `GLOBAL_VARIABLE`, keywords, string content, ...) is a
//!   parser-gem type stock does not flag, so it is skipped by not matching the
//!   three cases above.

use super::parse_cache;

/// One offense: `(is_constant, start_byte, end_byte)`. `is_constant` picks the
/// message (`CONSTANT_MSG` vs `IDENTIFIER_MSG`); the byte range is the first
/// non-ASCII run inside the token, converted to char offsets by the wrapper.
pub type AsciiIdentOffense = (bool, usize, usize);

/// prism lex token type NAMES this cop distinguishes (see [`crate::pm_lex`]).
const IDENTIFIER: &str = "IDENTIFIER";
const CONSTANT: &str = "CONSTANT";
const METHOD_NAME: &str = "METHOD_NAME";
const SYMBOL_BEGIN: &str = "SYMBOL_BEGIN";
const KEYWORD_DEF: &str = "KEYWORD_DEF";

pub fn check_ascii_identifiers(source: &[u8], ascii_constants: bool) -> Vec<AsciiIdentOffense> {
    // Fast path: no non-ASCII byte anywhere -> no token can offend, and no lex
    // is built. This is the overwhelming majority of files.
    if source.is_ascii() {
        return Vec::new();
    }

    parse_cache::with_parsed_and_tokens(source, |owner, _root, raw| {
        let mut out = Vec::new();
        let mut prev: &str = "";
        for tok in raw {
            let ty = tok.type_name();
            // Which parser-gem kind this prism token maps to (see module docs).
            // `Some(is_constant)` flags; `None` skips.
            let decision: Option<bool> = match ty {
                IDENTIFIER if prev != SYMBOL_BEGIN => Some(false),
                CONSTANT if prev != SYMBOL_BEGIN => {
                    // parser-gem calls a name `tCONSTANT` only when it starts
                    // with an ASCII `A`-`Z`; a Unicode-uppercase (non-ASCII)
                    // start — Cyrillic `Ф`, Greek `Ω`, full-width `Ａ`, ... — is
                    // a `tIDENTIFIER`. prism calls both `CONSTANT`, so re-split
                    // on the first byte: an ASCII-uppercase start is a real
                    // constant (gated by `AsciiConstants`); anything else is an
                    // identifier, always flagged.
                    if owner
                        .get(tok.start_offset)
                        .is_some_and(u8::is_ascii_uppercase)
                    {
                        ascii_constants.then_some(true)
                    } else {
                        Some(false)
                    }
                }
                // An instance method-def name `def foo!` / `def foo?` is
                // parser-gem `tIDENTIFIER`; every other `METHOD_NAME` is `tFID`.
                METHOD_NAME if prev == KEYWORD_DEF => Some(false),
                _ => None,
            };
            prev = ty;

            if let Some(is_constant) = decision {
                let start = tok.start_offset;
                let end = (start + tok.length).min(owner.len());
                if let Some((rs, re)) = first_non_ascii_run(&owner[start..end]) {
                    out.push((is_constant, start + rs, start + re));
                }
            }
        }
        out
    })
}

/// The byte range of the first maximal run of non-ASCII bytes in `text`
/// (`String#match(/[^[:ascii:]]+/)`), or `None` when `text` is all ASCII. In
/// UTF-8 every byte of a non-ASCII char is `>= 0x80` and every ASCII char is a
/// single byte `< 0x80`, so a run of non-ASCII CHARS is exactly a run of bytes
/// `>= 0x80`.
fn first_non_ascii_run(text: &[u8]) -> Option<(usize, usize)> {
    let start = text.iter().position(|&b| b >= 0x80)?;
    let end = text[start..]
        .iter()
        .position(|&b| b < 0x80)
        .map_or(text.len(), |off| start + off);
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str, ac: bool) -> Vec<AsciiIdentOffense> {
        check_ascii_identifiers(src.as_bytes(), ac)
    }

    // All-ASCII source -> nothing (fast path, no lex).
    #[test]
    fn ascii_only() {
        assert_eq!(run("def foo; Bar.baz(:qux); end\n", true), Vec::new());
    }

    // A non-ASCII local variable: offense at the first non-ASCII run.
    #[test]
    fn non_ascii_lvar() {
        // "älg = 1": the run is the 2-byte "ä" at bytes 0..2.
        assert_eq!(run("älg = 1\n", true), vec![(false, 0, 2)]);
    }

    // A non-ASCII run in the MIDDLE of an identifier (ascii prefix/suffix).
    #[test]
    fn mixed_identifier() {
        // "foo∂∂bar": each ∂ is 3 bytes; the run is bytes 3..9.
        assert_eq!(run("foo∂∂bar = baz\n", true), vec![(false, 3, 9)]);
    }

    // A constant (ASCII-uppercase start): flagged as a CONSTANT only when
    // AsciiConstants is on.
    #[test]
    fn constant_gated() {
        // "Foö = 1": "ö" is bytes 2..4 ("Fo" is 2 ascii bytes).
        assert_eq!(run("Foö = 1\n", true), vec![(true, 2, 4)]);
        assert_eq!(run("Foö = 1\n", false), Vec::new());
    }

    // A Unicode-uppercase (non-ASCII) START is a parser-gem tIDENTIFIER, not a
    // constant: flagged with the IDENTIFIER kind, and REGARDLESS of
    // AsciiConstants (identifiers are always checked). prism lexes it as
    // CONSTANT, so this is the re-split on the first byte.
    #[test]
    fn unicode_uppercase_start_is_identifier() {
        // "ФУ = 1": whole "ФУ" is the run (bytes 0..4), kind = identifier.
        assert_eq!(run("ФУ = 1\n", true), vec![(false, 0, 4)]);
        assert_eq!(run("ФУ = 1\n", false), vec![(false, 0, 4)]);
        // Greek and full-width uppercase starts likewise.
        assert_eq!(run("Ω = 1\n", false), vec![(false, 0, 2)]);
        // An ASCII-uppercase start with a non-ASCII TAIL stays a constant.
        assert_eq!(run("Añ = 1\n", true), vec![(true, 1, 3)]);
        assert_eq!(run("Añ = 1\n", false), Vec::new());
    }

    // A plain symbol body is `tSYMBOL`, never flagged (SYMBOL_BEGIN exclusion).
    #[test]
    fn symbol_body_skipped() {
        assert_eq!(run(":サンプル\n", true), Vec::new());
        // ... but the same word as a method call IS an identifier.
        assert_eq!(run("x.каллε\n", true), vec![(false, 2, 12)]);
    }

    // `?` / `!` method CALLS are `tFID` (skip); an INSTANCE def name is
    // `tIDENTIFIER` (flag). `def self.` names are `tFID` (skip).
    #[test]
    fn method_name_only_flagged_for_instance_def() {
        assert_eq!(run("x.каллε?\n", true), Vec::new());
        assert_eq!(run("def кир!; end\n", true), vec![(false, 4, 10)]);
        assert_eq!(run("def кир?; end\n", true), vec![(false, 4, 10)]);
        assert_eq!(run("def self.кир!; end\n", true), Vec::new());
    }

    // ivar / gvar / cvar / label are their own prism types -> never flagged.
    #[test]
    fn other_name_kinds_skipped() {
        assert_eq!(run("@名前 = 1\n", true), Vec::new());
        assert_eq!(run("$グ = 1\n", true), Vec::new());
        assert_eq!(run("@@клас = 1\n", true), Vec::new());
        assert_eq!(run("{ 名前: 1 }\n", true), Vec::new());
    }

    // A leading non-ASCII comment does not perturb prev-token tracking: the
    // identifier after it is still flagged (prev is a COMMENT, not SYMBOL_BEGIN).
    #[test]
    fn comment_before_identifier() {
        // "# кир\n" is 9 bytes (`# ` + 3x2-byte cyrillic + `\n`); the line-2
        // `кир` identifier is bytes 9..15.
        assert_eq!(run("# кир\nкир = 1\n", true), vec![(false, 9, 15)]);
    }
}
