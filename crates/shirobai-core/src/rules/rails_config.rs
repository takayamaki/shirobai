//! Packed configuration for the shirobai-rails plugin origin.
//!
//! Unlike rubocop-rspec (sixteen `RSpec/Language` role lists) the four
//! Application* cops in this first cluster carry NO behavioral config — they
//! are fixed class-inheritance checks. So the rails wire segment is just the
//! wake-up flag: `nums = [rails_enabled]`, `lists = []`. The origin has no
//! per-file gate (rails cops run on every Ruby file), so once the plugin gem
//! registers its packer the flag is always `1`.
//!
//! Future rails clusters (send-table cops, Architecture-B cops) extend the
//! segment by appending nums / lists here, never reordering — same growth
//! discipline as every other origin.

/// Length of the rails segment's `nums`: `[rails_enabled]`.
pub const SEGMENT_NUMS_LEN: usize = 1;

/// Number of wire lists in the rails segment (none for the Application*
/// cluster).
pub const N_LISTS: usize = 0;

/// Packed configuration for the shirobai-rails plugin. `Some` means the
/// origin is awake (the plugin gem is loaded); the four Application* cops
/// need no fields yet.
#[derive(Debug, Clone)]
pub struct RailsConfig;

impl RailsConfig {
    /// Parse the rails wire segment. `Ok(None)` when the wake-up flag is off
    /// (core-only install: the plugin gem was never required).
    pub fn from_segment(nums: &[i64], lists: &[Vec<String>]) -> Result<Option<Self>, String> {
        if nums.len() != SEGMENT_NUMS_LEN {
            return Err(format!(
                "rails segment expects {SEGMENT_NUMS_LEN} nums, got {}",
                nums.len()
            ));
        }
        if lists.len() != N_LISTS {
            return Err(format!(
                "rails segment expects {N_LISTS} lists, got {}",
                lists.len()
            ));
        }
        if nums[0] == 0 {
            return Ok(None);
        }
        Ok(Some(RailsConfig))
    }
}
