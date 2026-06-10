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
