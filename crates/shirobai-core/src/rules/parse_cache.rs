//! A thread-local single-entry parse cache.
//!
//! Every shirobai cop parses the file it is given with Prism. When several cops
//! run over the same `ProcessedSource` in one RuboCop investigation, they would
//! otherwise each re-parse the identical source. This cache keeps the most
//! recent parse so the second and later cops on a file reuse it — collapsing N
//! re-parses per file down to one.
//!
//! The default path parses with `ruby_prism::parse` and keeps no tokens, so it
//! stays byte-for-byte the parse cops already relied on. A cop that also wants
//! the lexer token stream calls [`with_parsed_and_tokens`], which builds the
//! shared entry with [`parse_with_lex_into`] instead — one prism parse that
//! yields the AST and the tokens together (see `crate::pm_lex`).

use std::cell::RefCell;

use ruby_prism::{Node, ParseResult, parse};
use self_cell::self_cell;

use crate::pm_lex::{RawToken, parse_with_lex_into};

self_cell!(
    struct OwnedParse {
        owner: Vec<u8>,
        #[covariant]
        dependent: ParseResult,
    }
);

/// The cached parse plus the tokens from that same parse, when a token consumer
/// asked for them. `tokens` is `None` for entries built by the default
/// (token-free) path.
struct CacheEntry {
    parsed: OwnedParse,
    tokens: Option<Vec<RawToken>>,
}

thread_local! {
    static CACHE: RefCell<Option<CacheEntry>> = const { RefCell::new(None) };
    // A single retired token buffer, kept so the next token collection can reuse
    // its heap capacity instead of allocating. Filled when a tokens-bearing
    // entry is evicted; drained when the next entry collects tokens.
    static SPARE: RefCell<Option<Vec<RawToken>>> = const { RefCell::new(None) };
}

fn collect_comments(result: &ParseResult<'_>) -> Vec<(usize, usize)> {
    result
        .comments()
        .map(|c| {
            let l = c.location();
            (l.start_offset(), l.end_offset())
        })
        .collect()
}

/// Move the token buffer of an evicted entry into the spare slot so its capacity
/// can be reused. Entries without tokens carry nothing to recover.
fn recover_spare(spare: &mut Option<Vec<RawToken>>, evicted: Option<CacheEntry>) {
    if let Some(CacheEntry {
        tokens: Some(buf), ..
    }) = evicted
    {
        *spare = Some(buf);
    }
}

/// Ensure the cache holds a (token-free) parse of `source`, rebuilding on a
/// miss. A hit is kept as-is even if it already carries tokens — a tokens
/// entry serves plain queries just as well.
fn ensure_parsed(slot: &mut Option<CacheEntry>, spare: &mut Option<Vec<RawToken>>, source: &[u8]) {
    let hit = slot
        .as_ref()
        .is_some_and(|e| e.parsed.borrow_owner().as_slice() == source);
    if hit {
        return;
    }
    recover_spare(spare, slot.take());
    let parsed = OwnedParse::new(source.to_vec(), |owner| parse(owner));
    *slot = Some(CacheEntry {
        parsed,
        tokens: None,
    });
}

/// Ensure the cache holds a parse of `source` that also has its tokens,
/// (re)building with `parse_with_lex_into` when the current entry is a miss or
/// is a token-free hit. Reuses the spare token buffer's capacity if present.
fn ensure_parsed_with_tokens(
    slot: &mut Option<CacheEntry>,
    spare: &mut Option<Vec<RawToken>>,
    source: &[u8],
) {
    let hit = slot
        .as_ref()
        .is_some_and(|e| e.tokens.is_some() && e.parsed.borrow_owner().as_slice() == source);
    if hit {
        return;
    }
    recover_spare(spare, slot.take());
    // `parse_with_lex_into` clears the buffer first, so any reused spare starts
    // empty; the closure only borrows `buf` for the duration of the build, after
    // which `buf` owns the collected tokens (offsets into `owner`, not borrows).
    let mut buf = spare.take().unwrap_or_default();
    let parsed = OwnedParse::new(source.to_vec(), |owner| parse_with_lex_into(owner, &mut buf));
    *slot = Some(CacheEntry {
        parsed,
        tokens: Some(buf),
    });
}

/// Run `f` on the shared (cached or freshly built, token-free) parse of
/// `source`. Centralizes the hit/miss handling for the token-free API.
fn with_shared_parse<R>(source: &[u8], f: impl FnOnce(&[u8], &ParseResult<'_>) -> R) -> R {
    CACHE.with(|cell| {
        SPARE.with(|spare_cell| {
            let mut slot = cell.borrow_mut();
            let mut spare = spare_cell.borrow_mut();
            ensure_parsed(&mut slot, &mut spare, source);
            slot.as_ref()
                .unwrap()
                .parsed
                .with_dependent(|owner, result| f(owner, result))
        })
    })
}

/// Parse `source` (or reuse the cached parse when the source is identical to the
/// previous call) and run `f` on the AST root. The `&[u8]` passed to `f` is the
/// cached copy of the source, byte-identical to `source`.
pub fn with_parsed<R>(source: &[u8], f: impl FnOnce(&[u8], &Node<'_>) -> R) -> R {
    with_shared_parse(source, |owner, result| f(owner, &result.node()))
}

/// Run `f` with both the parsed AST root and the parse's comment byte
/// ranges. Reuses the shared parse instead of re-parsing, so cops that need
/// both comment positions AND AST node positions do not pay for a second
/// full parse. `f` receives `(owner, root, comment_ranges)` where the
/// comment list yields `(start_offset, end_offset)` in document order — the
/// 1-based line is computed by the caller via the shared `LineIndex`.
pub fn with_parsed_and_comments<R>(
    source: &[u8],
    f: impl FnOnce(&[u8], &Node<'_>, Vec<(usize, usize)>) -> R,
) -> R {
    with_shared_parse(source, |owner, result| {
        let comments = collect_comments(result);
        f(owner, &result.node(), comments)
    })
}

/// Run `f` with the parsed AST root and the token stream from the *same* parse.
///
/// The tokens come from one `parse_with_lex_into` call, so a cop that needs both
/// the AST and the lexer tokens pays for a single parse, not two.
///
/// The intended call order is that the first cop to touch a file is the token
/// consumer, so the shared entry is built with tokens up front. If instead a
/// token-free cop reaches the file first, this falls back to rebuilding the
/// entry with tokens — correct, but it pays a second parse for that file. Keep
/// token consumers early to avoid the tax.
pub fn with_parsed_and_tokens<R>(
    source: &[u8],
    f: impl FnOnce(&[u8], &Node<'_>, &[RawToken]) -> R,
) -> R {
    CACHE.with(|cell| {
        SPARE.with(|spare_cell| {
            let mut slot = cell.borrow_mut();
            let mut spare = spare_cell.borrow_mut();
            ensure_parsed_with_tokens(&mut slot, &mut spare, source);
            let entry = slot.as_ref().unwrap();
            // `ensure_parsed_with_tokens` guarantees tokens are present.
            let tokens = entry.tokens.as_deref().unwrap();
            entry
                .parsed
                .with_dependent(|owner, result| f(owner, &result.node(), tokens))
        })
    })
}

/// The byte offset where the `__END__` data segment starts (the `__END__`
/// line itself), if the source has one. Reuses the shared parse.
pub fn data_start(source: &[u8]) -> Option<usize> {
    with_shared_parse(source, |_owner, result| {
        result.data_loc().map(|l| l.start_offset())
    })
}

/// Collect the `(start_offset, end_offset)` byte ranges of every comment in the
/// (cached) parse of `source`. Reuses the shared parse instead of re-parsing,
/// so cops that need comment positions do not pay for a second full parse.
pub fn comment_ranges(source: &[u8]) -> Vec<(usize, usize)> {
    with_shared_parse(source, |_owner, result| collect_comments(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pm_lex::parse_with_lex;

    // NOTE: the cache is a single RefCell. Do not call any parse_cache function
    // from inside a parse_cache closure in these tests — it panics (BorrowMut).

    #[test]
    fn tokens_match_a_direct_parse_with_lex() {
        let src = b"x = 1 + 2\n";
        let (_r, expected) = parse_with_lex(src);
        let got = with_parsed_and_tokens(src, |_o, _n, toks| toks.to_vec());
        assert_eq!(got, expected);
    }

    #[test]
    fn same_source_twice_yields_the_same_tokens() {
        let src = b"def twice(a); a; end\n";
        let first = with_parsed_and_tokens(src, |_o, _n, t| t.to_vec());
        let second = with_parsed_and_tokens(src, |_o, _n, t| t.to_vec());
        assert!(!first.is_empty());
        assert_eq!(first, second);
    }

    #[test]
    fn tokens_are_built_after_a_token_free_parse_of_the_same_source() {
        let src = b"[10, 20].map { |i| i * 3 }\n";
        // Token-free cop touches the file first (entry has tokens = None).
        let _ = with_parsed(src, |_o, n| n.location().end_offset());
        // Token consumer arrives second: fallback rebuild must still be correct.
        let (_r, expected) = parse_with_lex(src);
        let got = with_parsed_and_tokens(src, |_o, _n, t| t.to_vec());
        assert_eq!(got, expected);
    }

    #[test]
    fn switching_source_rebuilds_tokens_without_residue() {
        let a = b"aaa = 111\n";
        let b = b"def bbbbb(x, y, z); x + y + z; end\n";
        // Collect tokens for A, then for B: B must not carry any of A's tokens
        // even though B's buffer reuses A's retired capacity.
        let _ = with_parsed_and_tokens(a, |_o, _n, t| t.len());
        let (_r, expected_b) = parse_with_lex(b);
        let got_b = with_parsed_and_tokens(b, |_o, _n, t| t.to_vec());
        assert_eq!(got_b, expected_b);
    }

    #[test]
    fn plain_apis_work_on_a_tokens_bearing_entry() {
        let src = b"# lead\nvalue = 1 # trailing\n";
        // Build a tokens-bearing entry first.
        let n_tokens = with_parsed_and_tokens(src, |_o, _n, t| t.len());
        assert!(n_tokens > 0);
        // The token-free APIs must reuse that same entry (no re-parse needed).
        let end = with_parsed(src, |_o, n| n.location().end_offset());
        assert!(end > 0);
        let ranges = comment_ranges(src);
        let direct = ruby_prism::parse(src);
        let expected: Vec<(usize, usize)> = direct
            .comments()
            .map(|c| {
                let l = c.location();
                (l.start_offset(), l.end_offset())
            })
            .collect();
        assert_eq!(ranges, expected);
        assert_eq!(ranges.len(), 2);
    }
}
