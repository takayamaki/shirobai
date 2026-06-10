//! Per-source line index for O(log n) line/column lookups.
//!
//! Several cops compute, for many byte offsets, the 1-based line number, the
//! byte offset of the start of the line, and the (display) column. The naive
//! helpers scan the source from the front for every offset, which is
//! O(n) per call and O(n × nodes) over a file. [`LineIndex`] precomputes the
//! sorted byte offsets of every line start once per source, so each lookup is a
//! binary search.
//!
//! The lookups are defined to be byte-for-byte identical to the previous
//! front-scanning helpers:
//!
//! - `line_of(off)` = `source[..off].iter().filter(|&&b| b == b'\n').count() + 1`
//! - `line_start(off)` = offset after the last `\n` before `off`, or 0
//! - `column(off)` = char count of `source[line_start(off)..off]`
//!   (falling back to byte length on invalid UTF-8)
//! - `display_column(off)` = unicode display width of the same slice
//!
//! A thread-local cache ([`with_line_index`]) keeps the most recently built
//! index so cops that run over the same source in one investigation share a
//! single construction.

use std::cell::RefCell;
use std::rc::Rc;

/// Sorted byte offsets of the start of each line (`line_starts[0] == 0`).
/// `line_starts[k]` is the byte offset just past the `k`-th `\n`.
pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    /// Build the index from `source`.
    pub fn new(source: &[u8]) -> Self {
        let mut line_starts = Vec::with_capacity(source.len() / 32 + 1);
        line_starts.push(0);
        for (i, &b) in source.iter().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        LineIndex { line_starts }
    }

    /// 1-based line number of `off`.
    ///
    /// Equal to the number of `\n` bytes in `source[..off]` plus one. We count
    /// the line starts that are `<= off`: `line_starts[0] = 0 <= off` always
    /// contributes the `+ 1`, and each newline strictly before `off` adds a
    /// further start `<= off`.
    pub fn line_of(&self, off: usize) -> usize {
        // partition_point returns the count of elements for which the predicate
        // holds (they form a prefix, since line_starts is sorted ascending).
        self.line_starts.partition_point(|&s| s <= off)
    }

    /// Byte offset of the start of the line containing `off` (offset just past
    /// the last `\n` before `off`, or 0).
    pub fn line_start(&self, off: usize) -> usize {
        // The greatest line start that is `<= off`. partition_point gives the
        // count of starts `<= off`; that count is at least 1 (the 0 start), so
        // the index of the last such start is count - 1.
        let count = self.line_starts.partition_point(|&s| s <= off);
        self.line_starts[count - 1]
    }

    /// Character column of `off` within its line (codepoint count from the line
    /// start). Falls back to byte length on invalid UTF-8, matching the scan
    /// helpers.
    pub fn column(&self, source: &[u8], off: usize) -> usize {
        let ls = self.line_start(off);
        let slice = &source[ls..off];
        match std::str::from_utf8(slice) {
            Ok(s) => s.chars().count(),
            Err(_) => slice.len(),
        }
    }

    /// Display column of `off` (East-Asian wide characters count as two). Falls
    /// back to byte length on invalid UTF-8, matching the scan helpers.
    pub fn display_column(&self, source: &[u8], off: usize) -> usize {
        let ls = self.line_start(off);
        let slice = &source[ls..off];
        match std::str::from_utf8(slice) {
            Ok(s) => unicode_width::UnicodeWidthStr::width(s),
            Err(_) => slice.len(),
        }
    }
}

thread_local! {
    static CACHE: RefCell<Option<(Vec<u8>, Rc<LineIndex>)>> = const { RefCell::new(None) };
}

/// Run `f` with a [`LineIndex`] for `source`, reusing a cached index when the
/// source is byte-identical to the previous call. The returned `Rc` lets the
/// caller hold the index for the lifetime of a single `check_*` run without
/// rebuilding it.
pub fn with_line_index<R>(source: &[u8], f: impl FnOnce(&Rc<LineIndex>) -> R) -> R {
    CACHE.with(|cell| {
        {
            let mut slot = cell.borrow_mut();
            let hit = slot
                .as_ref()
                .is_some_and(|(src, _)| src.as_slice() == source);
            if !hit {
                *slot = Some((source.to_vec(), Rc::new(LineIndex::new(source))));
            }
        }
        let idx = cell.borrow().as_ref().unwrap().1.clone();
        f(&idx)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference (scan) implementations, copied from the cops verbatim.
    fn ref_line_of(source: &[u8], off: usize) -> usize {
        source[..off].iter().filter(|&&b| b == b'\n').count() + 1
    }

    fn ref_line_start(source: &[u8], off: usize) -> usize {
        match source[..off].iter().rposition(|&b| b == b'\n') {
            Some(i) => i + 1,
            None => 0,
        }
    }

    fn ref_column(source: &[u8], off: usize) -> usize {
        let ls = ref_line_start(source, off);
        std::str::from_utf8(&source[ls..off])
            .map(|s| s.chars().count())
            .unwrap_or(off - ls)
    }

    fn ref_display_column(source: &[u8], off: usize) -> usize {
        let ls = ref_line_start(source, off);
        std::str::from_utf8(&source[ls..off])
            .map(unicode_width::UnicodeWidthStr::width)
            .unwrap_or(off - ls)
    }

    fn check_all_offsets(source: &[u8]) {
        let idx = LineIndex::new(source);
        for off in 0..=source.len() {
            assert_eq!(
                idx.line_of(off),
                ref_line_of(source, off),
                "line_of mismatch at off={off} in {source:?}"
            );
            assert_eq!(
                idx.line_start(off),
                ref_line_start(source, off),
                "line_start mismatch at off={off} in {source:?}"
            );
            assert_eq!(
                idx.column(source, off),
                ref_column(source, off),
                "column mismatch at off={off} in {source:?}"
            );
            assert_eq!(
                idx.display_column(source, off),
                ref_display_column(source, off),
                "display_column mismatch at off={off} in {source:?}"
            );
        }
    }

    #[test]
    fn matches_scan_on_simple_multiline_source() {
        check_all_offsets(b"foo = 1\nbar = 2\nbaz = 3\n");
    }

    #[test]
    fn matches_scan_on_source_without_trailing_newline() {
        check_all_offsets(b"def m\n  x\nend");
    }

    #[test]
    fn matches_scan_on_empty_source() {
        check_all_offsets(b"");
    }

    #[test]
    fn matches_scan_on_single_line_no_newline() {
        check_all_offsets(b"x = 42");
    }

    #[test]
    fn matches_scan_with_consecutive_and_leading_newlines() {
        check_all_offsets(b"\n\na\n\nb\n\n");
    }

    #[test]
    fn matches_scan_with_multibyte_characters() {
        // Mixed ASCII, multibyte UTF-8 and East-Asian wide characters across
        // lines exercise the char-count vs display-width vs byte distinction.
        check_all_offsets("a = 'あいう'\nb = '日本語テスト'\nc = 'mañana'\n".as_bytes());
    }

    #[test]
    fn matches_scan_when_source_is_only_newlines() {
        check_all_offsets(b"\n");
        check_all_offsets(b"\n\n\n");
    }

    #[test]
    fn with_line_index_reuses_cached_index_for_identical_source() {
        let src = b"a\nb\nc\n";
        let first = with_line_index(src, |idx| Rc::as_ptr(idx));
        let second = with_line_index(src, |idx| Rc::as_ptr(idx));
        assert_eq!(first, second, "identical source should reuse the index");

        let other = b"d\ne\n";
        let third = with_line_index(other, |idx| Rc::as_ptr(idx));
        assert_ne!(first, third, "different source should rebuild the index");
    }
}
