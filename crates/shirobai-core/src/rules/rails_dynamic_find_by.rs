//! `Rails/DynamicFindBy` (rubocop-rails 2.35.5). Flags dynamic `find_by_*`
//! finders and rewrites them to `find_by(col: ...)`.
//!
//! Mirrors `vendor/rubocop-rails/lib/rubocop/cop/rails/dynamic_find_by.rb`
//! (`on_send` / `on_csend`). Every quirk was probed against stock:
//!
//! - Method name matches `/^find_by_(.+?)(!)?$/`. Group 1 is the columns
//!   (lazy, so a lone `find_by_!` captures `!` as the column and no bang);
//!   `static_name` is `find_by!` when the trailing `!` is captured, else
//!   `find_by`. `column_keywords` = group 1 `split('_and_')` (Ruby drops
//!   trailing empties), each `+ ": "`.
//! - Fires only when the argument count equals the keyword count AND no
//!   argument is a `splat` or `hash` (a braceless trailing `k: v` is one
//!   `hash` arg in both parser and prism). A block-pass `&blk` is a parser
//!   argument (trap-table rule), so it is counted as a virtual trailing arg.
//! - A receiverless `find_by_x` fires only inside a class that inherits
//!   `ApplicationRecord` / `::ApplicationRecord` / `ActiveRecord::Base` /
//!   `::ActiveRecord::Base` (`inherit_active_record_base?`, an
//!   `each_ancestor(:class)` check — hence the class-inheritance stack).
//! - `AllowedMethods` (method name), `AllowedReceivers` (the receiver's raw
//!   source), and the deprecated `Whitelist` (method name) each suppress.
//!   These need the wire lists.
//!
//! Autocorrect: replace the selector with `static_name`, then insert each
//! `col: ` keyword before the matching argument — all byte-computable, so this
//! is a full-Rust cop. The wrapper owns only the fixed `MSG` string.

use ruby_prism::{CallNode, Node, Visit};

/// One dynamic-finder offense. `[start, end)` is the whole send (offense
/// highlight); `[sel_start, sel_end)` is the selector to replace with
/// `static_name`; `inserts` is `(arg_start, keyword)` for each column keyword
/// inserted before its argument.
#[derive(Debug, Clone)]
pub struct DynamicFindByOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub static_name: String,
    pub sel_start: usize,
    pub sel_end: usize,
    pub inserts: Vec<(usize, String)>,
}

/// Wire config: the three suppression lists. Empty behaves like stock's `nil`
/// for all three (`include?` on `[]` is always false, and stock guards each
/// with `return false unless cop_config[...]`).
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub allowed_methods: Vec<String>,
    pub allowed_receivers: Vec<String>,
    pub whitelist: Vec<String>,
}

/// Standalone entry point for the per-cop fallback.
pub fn check_rails_dynamic_find_by(source: &[u8], config: Config) -> Vec<DynamicFindByOffense> {
    let mut visitor = build_rule(config);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.finish()
}

pub(crate) fn build_rule(config: Config) -> DynamicFindByVisitor {
    DynamicFindByVisitor {
        config,
        offenses: Vec::new(),
        frames: Vec::new(),
        ar_class_depth: 0,
    }
}

pub(crate) struct DynamicFindByVisitor {
    config: Config,
    offenses: Vec<DynamicFindByOffense>,
    /// One entry per entered branch node: whether it is a class that inherits
    /// an ActiveRecord base (so `leave` knows to decrement the counter). Keeps
    /// the branch stack in 1:1 sync with `leave` (ENTER_ALL + LEAVE).
    frames: Vec<bool>,
    /// Number of ActiveRecord-inheriting class ancestors currently open.
    ar_class_depth: usize,
}

impl DynamicFindByVisitor {
    pub(crate) fn finish(self) -> Vec<DynamicFindByOffense> {
        self.offenses
    }

    fn source_slice<'a>(&self, node: &Node<'a>) -> &'a [u8] {
        let loc = node.location();
        loc.as_slice()
    }

    fn allowed_invocation(&self, node: &CallNode<'_>, method_name: &[u8]) -> bool {
        let name = method_name;
        // allowed_method?
        if self
            .config
            .allowed_methods
            .iter()
            .any(|m| m.as_bytes() == name)
        {
            return true;
        }
        // allowed_receiver? (receiver's raw source)
        if !self.config.allowed_receivers.is_empty()
            && let Some(recv) = node.receiver()
        {
            let src = self.source_slice(&recv);
            if self
                .config
                .allowed_receivers
                .iter()
                .any(|r| r.as_bytes() == src)
            {
                return true;
            }
        }
        // whitelisted? (deprecated)
        if self.config.whitelist.iter().any(|m| m.as_bytes() == name) {
            return true;
        }
        false
    }

    fn check_call(&mut self, node: &CallNode<'_>) {
        let has_receiver = node.receiver().is_some();
        // `return if (receiver.nil? && !inherit_active_record_base?) ||
        // allowed_invocation?`
        if !has_receiver && self.ar_class_depth == 0 {
            return;
        }
        let method_name = node.name().as_slice();
        if self.allowed_invocation(node, method_name) {
            return;
        }
        let Some((static_name, keywords)) = parse_dynamic_finder(method_name) else {
            return;
        };
        // Argument list: positional / keyword args plus a virtual block-pass.
        let mut args: Vec<Node<'_>> = Vec::new();
        if let Some(a) = node.arguments() {
            for arg in a.arguments().iter() {
                args.push(arg);
            }
        }
        if let Some(block) = node.block()
            && let Some(ba) = block.as_block_argument_node()
        {
            args.push(ba.as_node());
        }
        // dynamic_find_by_arguments?: count matches AND no splat / hash arg.
        if keywords.len() != args.len() {
            return;
        }
        if args.iter().any(is_ignored_argument_type) {
            return;
        }
        let node_loc = node.location();
        let Some(sel) = node.message_loc() else {
            return;
        };
        let inserts = keywords
            .iter()
            .zip(args.iter())
            .map(|(kw, arg)| (arg.location().start_offset(), kw.clone()))
            .collect();
        self.offenses.push(DynamicFindByOffense {
            start_offset: node_loc.start_offset(),
            end_offset: node_loc.end_offset(),
            static_name: static_name.to_string(),
            sel_start: sel.start_offset(),
            sel_end: sel.end_offset(),
            inserts,
        });
    }

    /// Whether entering `node` opens an ActiveRecord-inheriting class frame.
    fn is_ar_class(node: &Node<'_>) -> bool {
        let Some(class) = node.as_class_node() else {
            return false;
        };
        match class.superclass() {
            Some(sc) => is_active_record_superclass(&sc),
            None => false,
        }
    }
}

/// `active_record?`: `(const {nil? cbase} :ApplicationRecord)` or
/// `(const (const {nil? cbase} :ActiveRecord) :Base)`.
fn is_active_record_superclass(node: &Node<'_>) -> bool {
    // Bare / cbase `ApplicationRecord`.
    if top_level_const_name(node) == Some(b"ApplicationRecord") {
        return true;
    }
    // `ActiveRecord::Base` (ActiveRecord at nil/cbase scope).
    if let Some(path) = node.as_constant_path_node()
        && path.name().map(|n| n.as_slice()) == Some(b"Base")
        && let Some(parent) = path.parent()
    {
        return top_level_const_name(&parent) == Some(b"ActiveRecord");
    }
    false
}

/// `(const {nil? cbase} :NAME)`: a bare `NAME` or `::NAME`.
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

/// A `splat` or `hash` argument (both suppress the offense).
fn is_ignored_argument_type(node: &Node<'_>) -> bool {
    node.as_splat_node().is_some()
        || node.as_hash_node().is_some()
        || node.as_keyword_hash_node().is_some()
}

/// Parse a method name against `/^find_by_(.+?)(!)?$/`. Returns
/// `(static_name, column_keywords)` where each keyword already carries its
/// `": "` suffix, or `None` when the name is not a dynamic finder.
fn parse_dynamic_finder(name: &[u8]) -> Option<(&'static str, Vec<String>)> {
    let rest = name.strip_prefix(b"find_by_")?;
    if rest.is_empty() {
        return None;
    }
    // Lazy group 1 + optional trailing `!`: the `!` is the bang only when at
    // least one column char precedes it.
    let (columns, bang) = if rest.len() >= 2 && rest.last() == Some(&b'!') {
        (&rest[..rest.len() - 1], true)
    } else {
        (rest, false)
    };
    let static_name = if bang { "find_by!" } else { "find_by" };
    let keywords = split_and(columns)
        .into_iter()
        .map(|c| format!("{}: ", String::from_utf8_lossy(c)))
        .collect();
    Some((static_name, keywords))
}

/// Ruby `String#split('_and_')` semantics: split on the literal separator and
/// drop trailing empty fields (Ruby drops trailing empties when no limit).
fn split_and(s: &[u8]) -> Vec<&[u8]> {
    let sep = b"_and_";
    let mut parts: Vec<&[u8]> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i + sep.len() <= s.len() {
        if &s[i..i + sep.len()] == sep {
            parts.push(&s[start..i]);
            i += sep.len();
            start = i;
        } else {
            i += 1;
        }
    }
    parts.push(&s[start..]);
    while matches!(parts.last(), Some(p) if p.is_empty()) {
        parts.pop();
    }
    parts
}

impl<'pr> Visit<'pr> for DynamicFindByVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        let is_ar = Self::is_ar_class(&node.as_node());
        self.frames.push(is_ar);
        if is_ar {
            self.ar_class_depth += 1;
        }
        ruby_prism::visit_call_node(self, node);
        if self.frames.pop() == Some(true) {
            self.ar_class_depth -= 1;
        }
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        let is_ar = Self::is_ar_class(&node.as_node());
        self.frames.push(is_ar);
        if is_ar {
            self.ar_class_depth += 1;
        }
        ruby_prism::visit_class_node(self, node);
        if self.frames.pop() == Some(true) {
            self.ar_class_depth -= 1;
        }
    }
}

impl super::dispatch::Rule for DynamicFindByVisitor {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        // Every branch node must push a frame so the branch stack stays 1:1
        // with `leave` (the class-ancestor counter rides that stack).
        Interest(Interest::ENTER_ALL | Interest::LEAVE)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.check_call(&call);
        }
        let is_ar = Self::is_ar_class(node);
        self.frames.push(is_ar);
        if is_ar {
            self.ar_class_depth += 1;
        }
    }

    fn leave(&mut self) {
        if self.frames.pop() == Some(true) {
            self.ar_class_depth -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<DynamicFindByOffense> {
        check_rails_dynamic_find_by(src.as_bytes(), Config::default())
    }

    #[test]
    fn flags_simple() {
        let off = run("User.find_by_name(name)\n");
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].static_name, "find_by");
        assert_eq!(off[0].start_offset, 0);
        assert_eq!(off[0].end_offset, 23);
        assert_eq!(off[0].inserts.len(), 1);
        assert_eq!(off[0].inserts[0].1, "name: ");
    }

    #[test]
    fn flags_and_bang() {
        let off = run("User.find_by_name!(name)\n");
        assert_eq!(off[0].static_name, "find_by!");
    }

    #[test]
    fn flags_multi_columns() {
        let off = run("User.find_by_name_and_email(name, email)\n");
        assert_eq!(off.len(), 1);
        let kws: Vec<_> = off[0].inserts.iter().map(|(_, k)| k.clone()).collect();
        assert_eq!(kws, vec!["name: ", "email: "]);
    }

    #[test]
    fn accepts_wrong_arg_count() {
        assert!(run("User.find_by_name_and_email(name)\n").is_empty());
        assert!(run("User.find_by_name_and_email(name, email, token)\n").is_empty());
    }

    #[test]
    fn accepts_splat_and_hash() {
        assert!(run("User.find_by_scan(*args)\n").is_empty());
        assert!(run("User.find_by_foo_and_bar(arg, *args)\n").is_empty());
        assert!(run("Post.find_by_id(limit: 1)\n").is_empty());
        assert!(run("Post.find_by_title_and_id(\"foo\", limit: 1)\n").is_empty());
    }

    #[test]
    fn accepts_plain_find_by() {
        assert!(run("User.find_by(name: name)\n").is_empty());
    }

    #[test]
    fn csend_fires() {
        let off = run("user&.find_by_name(name)\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn no_receiver_needs_ar_ancestor() {
        assert!(run("class C\n  def m\n    find_by_name(name)\n  end\nend\n").is_empty());
        assert!(run("class C < Foo\n  def m\n    find_by_name(name)\n  end\nend\n").is_empty());
        assert_eq!(
            run("class C < ApplicationRecord\n  def m\n    find_by_name(name)\n  end\nend\n").len(),
            1
        );
        assert_eq!(
            run("class C < ActiveRecord::Base\n  def m\n    find_by_name(name)\n  end\nend\n").len(),
            1
        );
        assert_eq!(
            run("class C < ::ActiveRecord::Base\n  def m\n    find_by_name(name)\n  end\nend\n")
                .len(),
            1
        );
    }

    #[test]
    fn allowed_method_and_receiver_and_whitelist() {
        let am = check_rails_dynamic_find_by(
            b"User.find_by_sql([q])\n",
            Config {
                allowed_methods: vec!["find_by_sql".into()],
                ..Default::default()
            },
        );
        assert!(am.is_empty());
        let ar = check_rails_dynamic_find_by(
            b"Gem::Specification.find_by_name(\"x\").gem_dir\n",
            Config {
                allowed_receivers: vec!["Gem::Specification".into()],
                ..Default::default()
            },
        );
        assert!(ar.is_empty());
        // Namespaced-different receiver still fires.
        let ar2 = check_rails_dynamic_find_by(
            b"Specification.find_by_name(\"x\").gem_dir\n",
            Config {
                allowed_receivers: vec!["Gem::Specification".into()],
                ..Default::default()
            },
        );
        assert_eq!(ar2.len(), 1);
        let wl = check_rails_dynamic_find_by(
            b"User.find_by_name(\"x\")\n",
            Config {
                whitelist: vec!["find_by_name".into()],
                ..Default::default()
            },
        );
        assert!(wl.is_empty());
    }

    #[test]
    fn bundle_rule_matches_standalone() {
        let src = "User.find_by_name(name)\nclass C < ApplicationRecord\n  def m\n    find_by_id(x)\n  end\nend\n";
        let alone = check_rails_dynamic_find_by(src.as_bytes(), Config::default());
        let mut rule = build_rule(Config::default());
        super::super::dispatch::run(src.as_bytes(), &mut [&mut rule]);
        let bundled = rule.finish();
        assert_eq!(bundled.len(), alone.len());
        assert_eq!(bundled.len(), 2);
    }
}
