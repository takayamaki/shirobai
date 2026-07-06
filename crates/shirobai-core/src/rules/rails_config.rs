//! Packed configuration for the shirobai-rails plugin origin.
//!
//! The Application* cluster carried no behavioral config, so the segment used
//! to be just the wake-up flag. The send/block-table cluster adds two cops
//! that DO read config, so the segment grows (append only, never reorder):
//!
//! `nums`:
//!
//! | idx | field |
//! |-----|-------|
//! |  0  | rails_enabled (wake-up flag; `0` = core-only install) |
//! |  1  | unknown_env_supports_local (`Rails/UnknownEnv` `target_rails_version >= 7.1`) |
//!
//! `lists`:
//!
//! | idx | field |
//! |-----|-------|
//! |  0  | unknown_env_environments (`Rails/UnknownEnv` `Environments`; `[]` for nil) |
//! |  1  | dynamic_find_by_allowed_methods (`Rails/DynamicFindBy` `AllowedMethods`) |
//! |  2  | dynamic_find_by_allowed_receivers (`Rails/DynamicFindBy` `AllowedReceivers`) |
//! |  3  | dynamic_find_by_whitelist (`Rails/DynamicFindBy` deprecated `Whitelist`) |
//!
//! Every list is `[]` when the underlying config is nil — stock guards each
//! read with `return false unless cop_config[...]`, so an empty list is
//! behaviorally identical to nil.
//!
//! The origin has no per-file gate (rails cops run on every Ruby file), so once
//! the plugin gem registers its packer the flag is always `1`.

use super::rails_dynamic_find_by;

/// Length of the rails segment's `nums`.
pub const SEGMENT_NUMS_LEN: usize = 2;

/// Number of wire lists in the rails segment.
pub const N_LISTS: usize = 4;

/// Packed configuration for the shirobai-rails plugin. `Some` means the origin
/// is awake (the plugin gem is loaded).
#[derive(Debug, Clone)]
pub struct RailsConfig {
    /// `Rails/UnknownEnv` `Environments` (empty for nil).
    pub unknown_env_environments: Vec<String>,
    /// `Rails/UnknownEnv` `target_rails_version >= 7.1` (adds `local` to the
    /// predicate-form known set).
    pub unknown_env_supports_local: bool,
    /// `Rails/DynamicFindBy` suppression lists.
    pub dynamic_find_by: rails_dynamic_find_by::Config,
}

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
        Ok(Some(RailsConfig {
            unknown_env_environments: lists[0].clone(),
            unknown_env_supports_local: nums[1] != 0,
            dynamic_find_by: rails_dynamic_find_by::Config {
                allowed_methods: lists[1].clone(),
                allowed_receivers: lists[2].clone(),
                whitelist: lists[3].clone(),
            },
        }))
    }
}
