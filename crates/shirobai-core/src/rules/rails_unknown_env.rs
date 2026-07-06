//! `Rails/UnknownEnv` (rubocop-rails 2.35.5). Flags `Rails.env` predicate and
//! comparison forms that name an environment not in the configured
//! `Environments` list.
//!
//! Mirrors `vendor/rubocop-rails/lib/rubocop/cop/rails/unknown_env.rb`. The
//! stock cop matches three shapes, all keyed off `rails_env?`
//! (`(send {(const nil? :Rails) (const (cbase) :Rails)} :env)`):
//!
//! - **predicate** `(send #rails_env? $sym)` where `sym` ends with `?` and its
//!   name minus the `?` is not in `environments(with_local: true)`. Offense on
//!   the outer send's selector.
//! - **comparison** `(send #rails_env? {:== :=== :!=} $(str))` or the mirrored
//!   `(send $(str) {:== :=== :!=} #rails_env?)`, where the string value is not
//!   in `environments` (NO local). Offense on the string node.
//! - **case/when** `case #rails_env?` — each `when` string condition whose
//!   value is not in `environments` (NO local). Offense on the condition.
//!
//! `with_local` adds `local` to the known set only for the predicate form and
//! only when `supports_local` (`target_rails_version >= 7.1`); the comparison
//! and case forms never allow `local`. That asymmetry is stock's, pinned as an
//! edge spec.
//!
//! The message ("Unknown environment `X`" / "... Did you mean `Y`?") is built
//! Ruby-side: the DidYouMean spell suggestion cannot be reproduced in Rust
//! (and must not be), so the rule only emits `(start, end, name)` and the
//! wrapper synthesizes the message with `DidYouMean::SpellChecker` over its own
//! `Environments` cop config. No autocorrect (stock has none).

use ruby_prism::{CallNode, Node, Visit};

/// One flagged unknown-environment usage. `[start, end)` is the offense
/// highlight (selector for predicates, string node for comparison / case);
/// `name` is stock's pre-chomp `name` argument to `message` (the wrapper does
/// `name.chomp('?')` and the DidYouMean lookup).
#[derive(Debug, Clone)]
pub struct UnknownEnvOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub name: String,
}

/// Standalone entry point for the per-cop fallback.
pub fn check_rails_unknown_env(
    source: &[u8],
    environments: Vec<String>,
    supports_local: bool,
) -> Vec<UnknownEnvOffense> {
    let mut visitor = build_rule(environments, supports_local);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.finish()
}

pub(crate) fn build_rule(environments: Vec<String>, supports_local: bool) -> UnknownEnvVisitor {
    UnknownEnvVisitor {
        environments,
        supports_local,
        offenses: Vec::new(),
    }
}

pub(crate) struct UnknownEnvVisitor {
    environments: Vec<String>,
    supports_local: bool,
    offenses: Vec<UnknownEnvOffense>,
}

impl UnknownEnvVisitor {
    pub(crate) fn finish(self) -> Vec<UnknownEnvOffense> {
        self.offenses
    }

    /// `environments` view without local (comparison / case forms).
    fn known_no_local(&self, value: &str) -> bool {
        self.environments.iter().any(|e| e == value)
    }

    /// `environments(with_local: true)` view (predicate form).
    fn known_with_local(&self, value: &str) -> bool {
        if self.supports_local && value == "local" {
            return true;
        }
        self.known_no_local(value)
    }

    fn push(&mut self, start: usize, end: usize, name: String) {
        self.offenses.push(UnknownEnvOffense {
            start_offset: start,
            end_offset: end,
            name,
        });
    }

    fn check_call(&mut self, node: &CallNode<'_>) {
        // Predicate: `Rails.env.foo?`.
        if let Some(recv) = node.receiver()
            && is_rails_env(&recv)
            && !is_csend(node)
            && node.arguments().is_none()
            && node.block().is_none()
        {
            let name = String::from_utf8_lossy(node.name().as_slice()).into_owned();
            if let Some(base) = name.strip_suffix('?')
                && !self.known_with_local(base)
            {
                if let Some(sel) = node.message_loc() {
                    self.push(sel.start_offset(), sel.end_offset(), name);
                }
                return;
            }
        }

        // Comparison: `Rails.env {== === !=} 'str'` or the mirror.
        let sel = node.name().as_slice();
        if sel != b"==" && sel != b"===" && sel != b"!=" {
            return;
        }
        let Some(recv) = node.receiver() else { return };
        let Some(args) = node.arguments() else { return };
        let mut it = args.arguments().iter();
        let Some(arg) = it.next() else { return };
        if it.next().is_some() {
            return; // exactly one argument
        }
        // Form (a): receiver is `Rails.env`, arg is the string.
        if is_rails_env(&recv) {
            if let Some(s) = as_plain_string(&arg) {
                let value = String::from_utf8_lossy(s.unescaped()).into_owned();
                if !self.known_no_local(&value) {
                    let loc = arg.location();
                    self.push(loc.start_offset(), loc.end_offset(), value);
                }
            }
            return;
        }
        // Form (b): receiver is the string, arg is `Rails.env`.
        if is_rails_env(&arg)
            && let Some(s) = as_plain_string(&recv)
        {
            let value = String::from_utf8_lossy(s.unescaped()).into_owned();
            if !self.known_no_local(&value) {
                let loc = recv.location();
                self.push(loc.start_offset(), loc.end_offset(), value);
            }
        }
    }

    fn check_case(&mut self, node: &ruby_prism::CaseNode<'_>) {
        let Some(pred) = node.predicate() else { return };
        if !is_rails_env(&pred) {
            return;
        }
        for when in node.conditions().iter() {
            let Some(when) = when.as_when_node() else {
                continue;
            };
            for cond in when.conditions().iter() {
                if let Some(s) = as_plain_string(&cond) {
                    let value = String::from_utf8_lossy(s.unescaped()).into_owned();
                    if !self.known_no_local(&value) {
                        let loc = cond.location();
                        self.push(loc.start_offset(), loc.end_offset(), value);
                    }
                }
            }
        }
    }
}

/// `&.` call head (`on_csend` is not aliased for this cop's send pattern —
/// `(send ...)` is a plain send only).
fn is_csend(node: &CallNode<'_>) -> bool {
    node.call_operator_loc()
        .is_some_and(|l| l.as_slice() == b"&.")
}

/// `(const {nil? cbase} :Rails)`: a bare `Rails` or `::Rails`.
fn top_level_const_name<'a>(node: &Node<'a>) -> Option<&'a [u8]> {
    if let Some(read) = node.as_constant_read_node() {
        return Some(read.name().as_slice());
    }
    if let Some(path) = node.as_constant_path_node()
        && path.parent().is_none()
    {
        return path.name().map(|n| n.as_slice());
    }
    None
}

/// `rails_env?`: `(send {(const nil? :Rails) (const (cbase) :Rails)} :env)` —
/// a plain send named `env`, no args, no block, receiver a top-level `Rails`
/// const.
fn is_rails_env(node: &Node<'_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };
    if is_csend(&call) || call.name().as_slice() != b"env" {
        return false;
    }
    if call.arguments().is_some() || call.block().is_some() {
        return false;
    }
    match call.receiver() {
        Some(recv) => top_level_const_name(&recv) == Some(b"Rails"),
        None => false,
    }
}

/// `(str ...)` — a plain, non-interpolated string literal.
fn as_plain_string<'a>(node: &Node<'a>) -> Option<ruby_prism::StringNode<'a>> {
    node.as_string_node()
}

impl<'pr> Visit<'pr> for UnknownEnvVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_case_node(&mut self, node: &ruby_prism::CaseNode<'pr>) {
        self.check_case(node);
        ruby_prism::visit_case_node(self, node);
    }
}

impl super::dispatch::Rule for UnknownEnvVisitor {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        // CallNode (predicate / comparison) and CaseNode (ENTER_OTHER). No
        // stack, so no LEAVE.
        Interest(Interest::ENTER_CALL | Interest::ENTER_OTHER)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.check_call(&call);
        } else if let Some(case) = node.as_case_node() {
            self.check_case(&case);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<UnknownEnvOffense> {
        check_rails_unknown_env(
            src.as_bytes(),
            vec!["development".into(), "production".into(), "test".into()],
            false,
        )
    }

    fn names(src: &str) -> Vec<String> {
        run(src).into_iter().map(|o| o.name).collect()
    }

    #[test]
    fn flags_unknown_predicate() {
        assert_eq!(names("Rails.env.proudction?\n"), vec!["proudction?"]);
    }

    #[test]
    fn accepts_known_predicate() {
        assert!(run("Rails.env.production?\n").is_empty());
    }

    #[test]
    fn flags_unknown_equal_both_orders() {
        assert_eq!(names("Rails.env == 'proudction'\n"), vec!["proudction"]);
        assert_eq!(names("'proudction' == Rails.env\n"), vec!["proudction"]);
    }

    #[test]
    fn flags_triple_equal_and_bang_equal() {
        assert_eq!(names("Rails.env === 'x'\n"), vec!["x"]);
        assert_eq!(names("Rails.env != 'x'\n"), vec!["x"]);
    }

    #[test]
    fn accepts_known_equal() {
        assert!(run("Rails.env == 'production'\n").is_empty());
    }

    #[test]
    fn flags_case_when() {
        assert_eq!(
            names("case Rails.env\nwhen 'proudction'\n  x\nend\n"),
            vec!["proudction"]
        );
    }

    #[test]
    fn flags_case_multiple_conditions() {
        assert_eq!(
            names("case Rails.env\nwhen 'development', 'proudction'\n  x\nend\n"),
            vec!["proudction"]
        );
    }

    #[test]
    fn case_ignores_non_string_condition() {
        assert!(run("case Rails.env\nwhen proudction\n  x\nend\n").is_empty());
    }

    #[test]
    fn cbase_rails_env() {
        assert_eq!(names("::Rails.env.proudction?\n"), vec!["proudction?"]);
    }

    #[test]
    fn local_predicate_unknown_without_support() {
        assert_eq!(names("Rails.env.local?\n"), vec!["local?"]);
    }

    #[test]
    fn local_predicate_known_with_support() {
        let off = check_rails_unknown_env(
            b"Rails.env.local?\n",
            vec!["development".into(), "production".into(), "test".into()],
            true,
        );
        assert!(off.is_empty());
    }

    #[test]
    fn local_equal_unknown_even_with_support() {
        // Comparison form never allows local, even under supports_local.
        let off = check_rails_unknown_env(
            b"Rails.env == 'local'\n",
            vec!["development".into(), "production".into(), "test".into()],
            true,
        );
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn ignores_non_rails_env() {
        assert!(run("Foo.env.proudction?\n").is_empty());
        assert!(run("Rails.environment.proudction?\n").is_empty());
    }

    #[test]
    fn bundle_rule_matches_standalone() {
        let src = "Rails.env.proudction?\nRails.env == 'x'\ncase Rails.env\nwhen 'y'\nend\n";
        let alone = check_rails_unknown_env(
            src.as_bytes(),
            vec!["development".into(), "production".into(), "test".into()],
            false,
        );
        let mut rule = build_rule(
            vec!["development".into(), "production".into(), "test".into()],
            false,
        );
        super::super::dispatch::run(src.as_bytes(), &mut [&mut rule]);
        let bundled = rule.finish();
        assert_eq!(bundled.len(), alone.len());
        assert_eq!(bundled.len(), 3);
    }
}
