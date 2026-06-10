//! `Layout/DotPosition`.
//!
//! Checks the `.`/`&.` position in multi-line method calls (`leading` vs
//! `trailing` style).

/// One misplaced dot. `(start, end)` is the dot range (the offense highlight).
/// `(remove_start, remove_end)` is the range autocorrect deletes (the dot, or
/// its whole line when the dot stands alone). `insert_pos` is where the dot text
/// is re-inserted (before the selector for `leading`, after the receiver for
/// `trailing`).
pub struct DotPositionOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub remove_start: usize,
    pub remove_end: usize,
    pub insert_pos: usize,
}

pub fn check_dot_position(source: &[u8], _style: u8) -> Vec<DotPositionOffense> {
    super::parse_cache::with_parsed(source, |_source, _node| {
        // TODO: implement.
        Vec::new()
    })
}
