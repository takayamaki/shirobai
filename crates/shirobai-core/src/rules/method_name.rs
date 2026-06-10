//! `Naming/MethodName`.

/// A method-name site whose name may violate the configured style.
pub struct MethodNameCandidate {
    /// Offense range used for the style-violation message.
    pub start_offset: usize,
    pub end_offset: usize,
    /// The bare method name (no sigil, no colon/quotes).
    pub name: String,
    /// Whether the name matches the configured style (or is a class emitter).
    pub valid: bool,
    /// Alternative style index the name matches (0/1), or 255 (unrecognized).
    pub alternative: u8,
    /// Offense range / name used for the `ForbiddenIdentifiers` message.
    pub forbidden_start: usize,
    pub forbidden_end: usize,
    pub forbidden_name: String,
}

pub fn check_method_name(_source: &[u8], _style: u8) -> Vec<MethodNameCandidate> {
    Vec::new()
}
