//! `Metrics/CyclomaticComplexity` + `Metrics/PerceivedComplexity`.
//!
//! Both metrics are computed in a single pass over each method body so the two
//! cops share one re-parse per file.

/// Per-method complexity result. Both scores are reported; each cop selects the
/// one it needs.
pub struct MethodComplexity {
    pub start_offset: usize,
    pub end_offset: usize,
    /// End of the offense head (method name for `def`, block opening for
    /// `define_method`), used by the LSP location mode.
    pub head_end: usize,
    pub method_name: String,
    pub cyclomatic: usize,
    pub perceived: usize,
}

pub fn check_complexity(_source: &[u8]) -> Vec<MethodComplexity> {
    // not implemented yet
    Vec::new()
}
