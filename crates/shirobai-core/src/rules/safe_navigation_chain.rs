//! `Lint/SafeNavigationChain`.

/// An ordinary method call chained after a safe-navigation call.
pub struct SafeNavChainOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    /// Replacement text for `[start_offset, end_offset)`.
    pub replacement: String,
    /// Optional range to wrap in parentheses (`wrap_end > wrap_start`).
    pub wrap_start: usize,
    pub wrap_end: usize,
}

pub fn check_safe_navigation_chain(
    _source: &[u8],
    _nil_methods: &[String],
) -> Vec<SafeNavChainOffense> {
    // not implemented yet
    Vec::new()
}
