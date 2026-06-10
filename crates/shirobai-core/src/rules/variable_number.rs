//! `Naming/VariableNumber`.

/// An identifier whose numbering does not match the configured style.
pub struct VariableNumberOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    /// 0 = variable, 1 = method name, 2 = symbol.
    pub identifier_type: u8,
    pub name: String,
    /// First alternative style (0=snake_case,1=normalcase,2=non_integer) that
    /// the name matches, or 255 when none (unrecognized).
    pub alternative: u8,
}

/// Returns the offending identifiers plus whether at least one (non-allowed)
/// identifier used the configured style correctly (`had_correct`, needed for
/// `config_to_allow_offenses`).
pub fn check_variable_number(
    _source: &[u8],
    _style: u8,
    _flags: u8,
    _allowed_identifiers: &[String],
) -> (Vec<VariableNumberOffense>, bool) {
    // not implemented yet
    (Vec::new(), false)
}
