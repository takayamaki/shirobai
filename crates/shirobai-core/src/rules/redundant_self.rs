//! `Style/RedundantSelf`.

/// A redundant `self` receiver. `[self_start, self_end)` is the `self` token
/// range (the offense range), `[dot_start, dot_end)` is the `.` operator range.
pub struct RedundantSelfOffense {
    pub self_start: usize,
    pub self_end: usize,
    pub dot_start: usize,
    pub dot_end: usize,
}

pub fn check_redundant_self(
    _source: &[u8],
    _kernel_methods: &[String],
) -> Vec<RedundantSelfOffense> {
    Vec::new()
}
