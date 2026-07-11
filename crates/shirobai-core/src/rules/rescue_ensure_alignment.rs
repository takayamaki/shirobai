//! `Layout/RescueEnsureAlignment` — the toucher part only.
//!
//! Stock's `on_new_investigation`
//! (`vendor/rubocop/lib/rubocop/cop/layout/rescue_ensure_alignment.rb`):
//!
//! ```ruby
//! def on_new_investigation
//!   @modifier_locations =
//!     processed_source.tokens.each_with_object([]) do |token, locations|
//!       next unless token.rescue_modifier?
//!       locations << token.pos
//!     end
//! end
//! ```
//!
//! This materializes the parser-gem token stream on EVERY file just to find the
//! keyword position of each modifier `rescue` (`x rescue y`), which the cop then
//! uses to SKIP those `resbody` nodes in `on_resbody` (a modifier rescue is not
//! an alignment target). Everything else the cop does — `on_resbody` /
//! `on_ensure`, `alignment_node` resolution, the offense, and the autocorrect —
//! is cheap AST work stock keeps.
//!
//! shirobai has no parser-gem token stream. prism separates a modifier rescue
//! into its own node type, `RescueModifierNode` (distinct from the `RescueNode`
//! of a `begin`/`def` rescue clause), so the modifier keyword positions are
//! exactly the `keyword_loc` of every `RescueModifierNode`. This rule collects
//! them in one walk (shared with every other bundled cop); the Ruby wrapper
//! turns the byte ranges into the `@modifier_locations` set and runs stock's
//! `on_resbody` / `on_ensure` verbatim.

use ruby_prism::{Node, Visit};

use super::dispatch::{Interest, Rule};

/// The `(begin_byte, end_byte)` range of one modifier-`rescue` keyword.
pub type ModifierRescuePos = (usize, usize);

/// Standalone entry point used by the per-cop fallback (CRLF/BOM files, where
/// `buffer.source != raw_source`, take this path against `buffer.source`).
pub fn check_rescue_ensure_alignment(source: &[u8]) -> Vec<ModifierRescuePos> {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.positions
}

/// Build the rule for use standalone or in a shared-walk bundle. The cop needs
/// no per-cop config here (the modifier-keyword set is config-independent; the
/// enabled gate lives in the bundle), so the builder takes no arguments.
pub(crate) fn build_rule() -> RescueEnsureAlignmentRule {
    RescueEnsureAlignmentRule {
        positions: Vec::new(),
    }
}

pub(crate) struct RescueEnsureAlignmentRule {
    pub(crate) positions: Vec<ModifierRescuePos>,
}

impl RescueEnsureAlignmentRule {
    fn record(&mut self, node: &ruby_prism::RescueModifierNode<'_>) {
        let kw = node.keyword_loc();
        self.positions.push((kw.start_offset(), kw.end_offset()));
    }
}

impl<'pr> Visit<'pr> for RescueEnsureAlignmentRule {
    fn visit_rescue_modifier_node(&mut self, node: &ruby_prism::RescueModifierNode<'pr>) {
        self.record(node);
        // Recurse so a modifier rescue nested inside another expression
        // (`x = (y rescue z)`, a modifier in a method argument, ...) is found.
        ruby_prism::visit_rescue_modifier_node(self, node);
    }
}

impl Rule for RescueEnsureAlignmentRule {
    fn interest(&self) -> Interest {
        // `RescueModifierNode` is a plain branch node (no dedicated `ENTER_*`
        // class), so it reaches `enter` through the `ENTER_OTHER` bucket.
        Interest(Interest::ENTER_OTHER)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if let Some(rescue_mod) = node.as_rescue_modifier_node() {
            self.record(&rescue_mod);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(src: &str) -> Vec<ModifierRescuePos> {
        check_rescue_ensure_alignment(src.as_bytes())
    }

    // A modifier rescue: the `rescue` keyword range is collected.
    #[test]
    fn flags_modifier_rescue_keyword() {
        // "z = y rescue 0": `rescue` at bytes 6..12.
        assert_eq!(detect("z = y rescue 0\n"), vec![(6, 12)]);
    }

    // A begin/rescue clause is a `RescueNode`, NOT a `RescueModifierNode`, so it
    // is not collected (its `resbody` IS an alignment target for the cop).
    #[test]
    fn ignores_begin_rescue() {
        assert_eq!(detect("begin\n  x\nrescue\n  y\nend\n"), Vec::new());
    }

    // A def-body rescue (implicit begin) is likewise a `RescueNode`.
    #[test]
    fn ignores_def_rescue() {
        assert_eq!(detect("def m\n  x\nrescue\n  y\nend\n"), Vec::new());
    }

    // A modifier rescue nested inside an assignment value is still found.
    #[test]
    fn finds_nested_modifier_rescue() {
        // "x = foo(y rescue 1)": `rescue` at bytes 10..16.
        assert_eq!(detect("x = foo(y rescue 1)\n"), vec![(10, 16)]);
    }

    // Two modifier rescues on separate lines.
    #[test]
    fn finds_multiple() {
        let out = detect("a rescue 1\nb rescue 2\n");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], (2, 8));
    }

    // A file with no rescue of any kind.
    #[test]
    fn empty_when_no_rescue() {
        assert_eq!(detect("x = 1\n"), Vec::new());
    }
}
