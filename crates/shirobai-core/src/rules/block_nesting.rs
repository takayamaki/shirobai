//! `Metrics/BlockNesting` (stub — implemented in the GREEN commit).

/// One reportable offense: the byte range of the offending node.
pub struct BlockNestingOffense {
    pub start_offset: usize,
    pub end_offset: usize,
}

/// Returns the reportable offenses plus the deepest nesting level observed at
/// any node that exceeded `max`.
pub fn check_block_nesting(
    _source: &[u8],
    _max: usize,
    _count_blocks: bool,
    _count_modifier_forms: bool,
) -> (Vec<BlockNestingOffense>, usize) {
    (Vec::new(), 0)
}
