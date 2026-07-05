//! RSpec/Language role classification shared by every `RSpec/*` rule.
//!
//! rubocop-rspec resolves the RSpec DSL through `RuboCop::RSpec::Language`:
//! sixteen configurable name lists (`RSpec/Language` in the resolved config)
//! plus receiver / block-shape constraints per matcher. Stock re-answers the
//! membership question per cop per node (`Array#include?` + `Symbol#to_s`);
//! shirobai folds the sixteen lists into one `name -> RoleMask` table at
//! config-registration time so classification during the walk is a single
//! hash probe.
//!
//! The sixteen sub-role lists arrive over the wire in the fixed order
//! documented on `BundleConfig` (the `LIST_*` indexes below). The two
//! non-configurable stock role sets (`Runners` = `to/to_not/not_to`,
//! `HookScopes` = `each/example/context/all/suite`) are compile-time
//! constants here, not wire data — stock hard-codes them the same way.

use std::collections::HashMap;

/// One bit per configurable sub-role, in wire-list order.
pub mod roles {
    pub const EG_REGULAR: u32 = 1 << 0;
    pub const EG_FOCUSED: u32 = 1 << 1;
    pub const EG_SKIPPED: u32 = 1 << 2;
    pub const EX_REGULAR: u32 = 1 << 3;
    pub const EX_FOCUSED: u32 = 1 << 4;
    pub const EX_SKIPPED: u32 = 1 << 5;
    pub const EX_PENDING: u32 = 1 << 6;
    pub const EXPECTATIONS: u32 = 1 << 7;
    pub const HELPERS: u32 = 1 << 8;
    pub const HOOKS: u32 = 1 << 9;
    pub const ERROR_MATCHERS: u32 = 1 << 10;
    pub const INC_EXAMPLES: u32 = 1 << 11;
    pub const INC_CONTEXT: u32 = 1 << 12;
    pub const SG_EXAMPLES: u32 = 1 << 13;
    pub const SG_CONTEXT: u32 = 1 << 14;
    pub const SUBJECTS: u32 = 1 << 15;

    /// `ExampleGroups.all`
    pub const EG_ALL: u32 = EG_REGULAR | EG_FOCUSED | EG_SKIPPED;
    /// `Examples.all`
    pub const EX_ALL: u32 = EX_REGULAR | EX_FOCUSED | EX_SKIPPED | EX_PENDING;
    /// `Includes.all`
    pub const INC_ALL: u32 = INC_EXAMPLES | INC_CONTEXT;
    /// `SharedGroups.all`
    pub const SG_ALL: u32 = SG_EXAMPLES | SG_CONTEXT;
}

/// Wire order of the sixteen role lists inside the rspec segment's `lists`
/// (mirrored by the Ruby-side packer in `gems/shirobai-rspec`).
pub const N_ROLE_LISTS: usize = 16;

/// Stock `Language::Runners::ALL` (not configurable).
pub const RUNNERS: [&[u8]; 3] = [b"to", b"to_not", b"not_to"];

/// Stock `Language::HookScopes::ALL` (not configurable).
pub const HOOK_SCOPES: [&[u8]; 5] = [b"each", b"example", b"context", b"all", b"suite"];

/// Packed configuration for the shirobai-rspec plugin: the role table plus
/// (as cops land) per-cop settings.
#[derive(Debug, Clone)]
pub struct RSpecConfig {
    /// Method-name bytes -> OR of every sub-role the name belongs to.
    /// A name registered in several roles (user aliases can overlap)
    /// carries all of its bits.
    role_of: HashMap<Box<[u8]>, u32>,
}

impl RSpecConfig {
    /// Build the role table from the sixteen wire lists (wire order is the
    /// bit order of [`roles`]).
    pub fn from_role_lists(lists: &[Vec<String>]) -> Result<Self, String> {
        if lists.len() != N_ROLE_LISTS {
            return Err(format!(
                "rspec segment expects {N_ROLE_LISTS} role lists, got {}",
                lists.len()
            ));
        }
        let mut role_of: HashMap<Box<[u8]>, u32> = HashMap::new();
        for (i, list) in lists.iter().enumerate() {
            let bit = 1u32 << i;
            for name in list {
                *role_of
                    .entry(name.as_bytes().to_vec().into_boxed_slice())
                    .or_insert(0) |= bit;
            }
        }
        Ok(RSpecConfig { role_of })
    }

    /// Role mask for a method name — `0` when the name is not part of the
    /// configured RSpec language.
    pub fn roles_of(&self, name: &[u8]) -> u32 {
        self.role_of.get(name).copied().unwrap_or(0)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// The rubocop-rspec 3.10.2 `config/default.yml` Language lists in wire
    /// order, shared with bundle tests.
    pub fn default_role_lists() -> Vec<Vec<String>> {
        let v = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        vec![
            v(&["describe", "context", "feature", "example_group"]),
            v(&["fdescribe", "fcontext", "ffeature"]),
            v(&["xdescribe", "xcontext", "xfeature"]),
            v(&["it", "specify", "example", "scenario", "its"]),
            v(&["fit", "fspecify", "fexample", "fscenario", "focus"]),
            v(&["xit", "xspecify", "xexample", "xscenario", "skip"]),
            v(&["pending"]),
            v(&[
                "are_expected",
                "expect",
                "expect_any_instance_of",
                "is_expected",
                "should",
                "should_not",
                "should_not_receive",
                "should_receive",
            ]),
            v(&["let", "let!"]),
            v(&[
                "prepend_before",
                "before",
                "append_before",
                "around",
                "prepend_after",
                "after",
                "append_after",
            ]),
            v(&["raise_error", "raise_exception"]),
            v(&["it_behaves_like", "it_should_behave_like", "include_examples"]),
            v(&["include_context"]),
            v(&["shared_examples", "shared_examples_for"]),
            v(&["shared_context"]),
            v(&["subject", "subject!"]),
        ]
    }

    #[test]
    fn classifies_default_names() {
        let cfg = RSpecConfig::from_role_lists(&default_role_lists()).unwrap();
        assert_eq!(cfg.roles_of(b"describe"), roles::EG_REGULAR);
        assert_eq!(cfg.roles_of(b"fdescribe"), roles::EG_FOCUSED);
        assert_eq!(cfg.roles_of(b"xit"), roles::EX_SKIPPED);
        assert_eq!(cfg.roles_of(b"its"), roles::EX_REGULAR);
        assert_eq!(cfg.roles_of(b"let!"), roles::HELPERS);
        assert_eq!(cfg.roles_of(b"subject!"), roles::SUBJECTS);
        assert_eq!(cfg.roles_of(b"include_context"), roles::INC_CONTEXT);
        assert_eq!(cfg.roles_of(b"shared_context"), roles::SG_CONTEXT);
        assert_eq!(cfg.roles_of(b"nonsense"), 0);
        assert_ne!(cfg.roles_of(b"describe") & roles::EG_ALL, 0);
        assert_eq!(cfg.roles_of(b"describe") & roles::EX_ALL, 0);
    }

    #[test]
    fn a_name_can_carry_several_roles() {
        let mut lists = default_role_lists();
        // A user alias registered both as an example and as a helper.
        lists[3].push("given".to_string());
        lists[8].push("given".to_string());
        let cfg = RSpecConfig::from_role_lists(&lists).unwrap();
        assert_eq!(cfg.roles_of(b"given"), roles::EX_REGULAR | roles::HELPERS);
    }

    #[test]
    fn wrong_list_count_errors() {
        assert!(RSpecConfig::from_role_lists(&vec![vec![]; 15]).is_err());
    }
}
