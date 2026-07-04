//! A thread-local single-entry parse cache.
//!
//! Every shirobai cop parses the file it is given with Prism. When several cops
//! run over the same `ProcessedSource` in one RuboCop investigation, they would
//! otherwise each re-parse the identical source. This cache keeps the most
//! recent parse so the second and later cops on a file reuse it — collapsing N
//! re-parses per file down to one.

use std::cell::RefCell;

use ruby_prism::{Node, ParseResult, parse};
use self_cell::self_cell;

self_cell!(
    struct OwnedParse {
        owner: Vec<u8>,
        #[covariant]
        dependent: ParseResult,
    }
);

thread_local! {
    static CACHE: RefCell<Option<OwnedParse>> = const { RefCell::new(None) };
}

/// Parse `source` (or reuse the cached parse when the source is identical to the
/// previous call) and run `f` on the AST root. The `&[u8]` passed to `f` is the
/// cached copy of the source, byte-identical to `source`.
pub fn with_parsed<R>(source: &[u8], f: impl FnOnce(&[u8], &Node<'_>) -> R) -> R {
    CACHE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let hit = slot
            .as_ref()
            .is_some_and(|parsed| parsed.borrow_owner().as_slice() == source);
        if !hit {
            *slot = Some(OwnedParse::new(source.to_vec(), |owner| parse(owner)));
        }
        slot.as_ref()
            .unwrap()
            .with_dependent(|owner, result| f(owner, &result.node()))
    })
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
    CACHE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let hit = slot
            .as_ref()
            .is_some_and(|parsed| parsed.borrow_owner().as_slice() == source);
        if !hit {
            *slot = Some(OwnedParse::new(source.to_vec(), |owner| parse(owner)));
        }
        slot.as_ref()
            .unwrap()
            .with_dependent(|owner, result| {
                let comments: Vec<(usize, usize)> = result
                    .comments()
                    .map(|c| {
                        let l = c.location();
                        (l.start_offset(), l.end_offset())
                    })
                    .collect();
                f(owner, &result.node(), comments)
            })
    })
}

/// The byte offset where the `__END__` data segment starts (the `__END__`
/// line itself), if the source has one. Reuses the shared parse.
pub fn data_start(source: &[u8]) -> Option<usize> {
    CACHE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let hit = slot
            .as_ref()
            .is_some_and(|parsed| parsed.borrow_owner().as_slice() == source);
        if !hit {
            *slot = Some(OwnedParse::new(source.to_vec(), |owner| parse(owner)));
        }
        slot.as_ref().unwrap().with_dependent(|_owner, result| {
            result.data_loc().map(|l| l.start_offset())
        })
    })
}

/// Collect the `(start_offset, end_offset)` byte ranges of every comment in the
/// (cached) parse of `source`. Reuses the shared parse instead of re-parsing,
/// so cops that need comment positions do not pay for a second full parse.
pub fn comment_ranges(source: &[u8]) -> Vec<(usize, usize)> {
    CACHE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let hit = slot
            .as_ref()
            .is_some_and(|parsed| parsed.borrow_owner().as_slice() == source);
        if !hit {
            *slot = Some(OwnedParse::new(source.to_vec(), |owner| parse(owner)));
        }
        slot.as_ref().unwrap().with_dependent(|_owner, result| {
            result
                .comments()
                .map(|c| {
                    let l = c.location();
                    (l.start_offset(), l.end_offset())
                })
                .collect()
        })
    })
}
