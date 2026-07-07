//! `Rails/Pluck` (rubocop-rails 2.35.5). Detects `map { |x| x[:key] }`
//! replaceable with `pluck(:key)`.
//!
//! Mirrors `vendor/rubocop-rails/lib/rubocop/cop/rails/pluck.rb`
//! (`on_block` / `on_numblock` / `on_itblock`). Every quirk was probed
//! against stock:
//!
//! - Pattern: `(any_block (call _ {:map :collect}) $argument (send lvar :[] $key))`
//!   — a block/numblock/itblock whose call is `map` or `collect`, with a body
//!   that is a single `lvar[:key]` expression.
//! - Block parameter validation:
//!   - Regular block: exactly one block parameter, and the body's receiver
//!     lvar matches it. The key must not contain the block argument as a
//!     descendant lvar (prevents `x.map { |x| x[x] }` false positives).
//!   - Numblock: the argument is `1` (i.e. `_1`). The key must not contain
//!     `_1` as a descendant lvar.
//!   - Itblock: the argument is `:it`. The key must not contain `it` as a
//!     descendant lvar.
//! - Ancestor guard: `node.each_ancestor(:any_block).first&.receiver` — if
//!   the map call is inside another block that has a receiver, skip (N+1
//!   query risk).
//! - Regexp key: `key.regexp_type?` skips regexp keys.
//! - Autocorrect: replace `map { |x| x[:key] }` (from selector start to
//!   block end) with `pluck(key.source)`.

use ruby_prism::{CallNode, Node, Visit};

/// One pluck offense. `[sel_start, block_end)` is the offense highlight and
/// replace range. `key_src_start` / `key_src_end` are the key expression
/// source byte range (the replacement is `pluck(<key source>)`).
#[derive(Debug, Clone)]
pub struct PluckOffense {
    pub sel_start: usize,
    pub block_end: usize,
    pub key_src_start: usize,
    pub key_src_end: usize,
}

/// Standalone entry point for the per-cop fallback.
pub fn check_rails_pluck(source: &[u8]) -> Vec<PluckOffense> {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.finish()
}

pub(crate) fn build_rule() -> PluckVisitor {
    PluckVisitor {
        offenses: Vec::new(),
        frames: Vec::new(),
    }
}

pub(crate) struct PluckVisitor {
    offenses: Vec<PluckOffense>,
    /// One entry per entered branch node: `true` when this frame is a block
    /// whose call has a receiver (for the N+1 ancestor guard). Every branch
    /// node pushes a frame so the stack stays 1:1 with `leave`.
    frames: Vec<bool>,
}

impl PluckVisitor {
    pub(crate) fn finish(self) -> Vec<PluckOffense> {
        self.offenses
    }

    /// Whether any ancestor block has a receiver (the N+1 query guard).
    fn inside_block_with_receiver(&self) -> bool {
        self.frames.iter().any(|&f| f)
    }

    /// Check a call node whose block is a pluck candidate.
    fn check_call(&mut self, call: &CallNode<'_>) {
        let Some(block) = call.block() else { return };
        let Some(block_node) = block.as_block_node() else {
            return;
        };

        // Method must be `map` or `collect`.
        if !matches!(call.name().as_slice(), b"map" | b"collect") {
            return;
        }

        // Ancestor guard: if the map is inside another block that has a
        // receiver, skip (N+1 query risk).
        if self.inside_block_with_receiver() {
            return;
        }

        // Determine block kind from parameters.
        let params = block_node.parameters();
        let block_kind = match &params {
            Some(p) if p.as_block_parameters_node().is_some() => BlockKind::Block,
            Some(p) if p.as_numbered_parameters_node().is_some() => BlockKind::Numblock,
            Some(p) if p.as_it_parameters_node().is_some() => BlockKind::Itblock,
            None => return, // No parameters at all, not a pluck candidate
            _ => return,
        };

        // For regular blocks, must have exactly one parameter.
        let block_arg_name: &[u8] = match block_kind {
            BlockKind::Block => {
                let bp = params
                    .as_ref()
                    .unwrap()
                    .as_block_parameters_node()
                    .unwrap();
                let param_node = match bp.parameters() {
                    Some(pp) => pp,
                    None => return, // `map { || ... }` — no params
                };
                let plist: Vec<Node<'_>> = param_node.requireds().iter().collect();
                if plist.len() != 1
                    || param_node.optionals().iter().count() != 0
                    || param_node.rest().is_some()
                    || param_node.keywords().iter().count() != 0
                    || param_node.keyword_rest().is_some()
                    || param_node.block().is_some()
                {
                    return;
                }
                let Some(req) = plist[0].as_required_parameter_node() else {
                    return;
                };
                req.name().as_slice()
            }
            BlockKind::Numblock => b"_1",
            BlockKind::Itblock => b"it",
        };

        // Body must be a single `lvar[:key]` expression.
        let body = match block_node.body() {
            Some(b) => b,
            None => return,
        };

        // The body might be wrapped in a StatementsNode. Extract the
        // single index call from it.
        let index_call = if let Some(stmts) = body.as_statements_node() {
            let mut iter = stmts.body().iter();
            let first = match iter.next() {
                Some(n) => n,
                None => return,
            };
            if iter.next().is_some() {
                return; // Multiple statements
            }
            match first.as_call_node() {
                Some(c) => c,
                None => return,
            }
        } else {
            match body.as_call_node() {
                Some(c) => c,
                None => return,
            }
        };
        if index_call.name().as_slice() != b"[]" {
            return;
        }

        // Receiver must be an lvar matching the block argument.
        let Some(receiver) = index_call.receiver() else {
            return;
        };
        match block_kind {
            BlockKind::Block | BlockKind::Numblock => {
                let Some(lvar) = receiver.as_local_variable_read_node() else {
                    return;
                };
                if lvar.name().as_slice() != block_arg_name {
                    return;
                }
            }
            BlockKind::Itblock => {
                // In itblock, `it` is an `ItLocalVariableReadNode`.
                if receiver.as_it_local_variable_read_node().is_none() {
                    return;
                }
            }
        }

        // Must have exactly one argument (the key).
        let args = match index_call.arguments() {
            Some(a) => a,
            None => return,
        };
        let arg_list: Vec<Node<'_>> = args.arguments().iter().collect();
        if arg_list.len() != 1 {
            return;
        }
        let key = &arg_list[0];

        // Skip regexp keys.
        if key.as_regular_expression_node().is_some()
            || key.as_interpolated_regular_expression_node().is_some()
        {
            return;
        }

        // `use_block_argument_in_key?`: the key must not be the block
        // argument itself, and must not contain the block argument as a
        // descendant lvar.
        if !use_block_argument_in_key(block_arg_name, key, block_kind) {
            return;
        }

        // Build the offense.
        let Some(sel_loc) = call.message_loc() else {
            return;
        };
        let block_loc = block_node.location();
        let key_loc = key.location();

        self.offenses.push(PluckOffense {
            sel_start: sel_loc.start_offset(),
            block_end: block_loc.end_offset(),
            key_src_start: key_loc.start_offset(),
            key_src_end: key_loc.end_offset(),
        });
    }

    /// Whether entering `node` opens a block-with-receiver frame.
    fn is_block_with_receiver(node: &Node<'_>) -> bool {
        let Some(call) = node.as_call_node() else {
            return false;
        };
        let has_block = call
            .block()
            .is_some_and(|b| b.as_block_node().is_some());
        has_block && call.receiver().is_some()
    }
}

#[derive(Clone, Copy)]
enum BlockKind {
    Block,
    Numblock,
    Itblock,
}

/// Stock's `use_block_argument_in_key?`: returns `true` (allow the offense)
/// when the key is NOT the block argument itself and does NOT contain it
/// as a descendant lvar.
fn use_block_argument_in_key(block_arg: &[u8], key: &Node<'_>, kind: BlockKind) -> bool {
    // If the key source equals the block argument name, it's a false
    // positive like `x.map { |x| x[x] }`.
    let key_loc = key.location();
    if key_loc.as_slice() == block_arg {
        return false;
    }
    // Check descendants for lvar matching block argument.
    let mut finder = LvarFinder {
        block_arg,
        kind,
        found: false,
    };
    finder.visit(key);
    !finder.found
}

struct LvarFinder<'a> {
    block_arg: &'a [u8],
    kind: BlockKind,
    found: bool,
}

impl<'pr> Visit<'pr> for LvarFinder<'_> {
    fn visit_local_variable_read_node(
        &mut self,
        node: &ruby_prism::LocalVariableReadNode<'pr>,
    ) {
        if !self.found
            && !matches!(self.kind, BlockKind::Itblock)
            && node.name().as_slice() == self.block_arg
        {
            self.found = true;
        }
    }

    fn visit_it_local_variable_read_node(
        &mut self,
        _node: &ruby_prism::ItLocalVariableReadNode<'pr>,
    ) {
        if matches!(self.kind, BlockKind::Itblock) {
            self.found = true;
        }
    }
}

impl<'pr> Visit<'pr> for PluckVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        let is_bwr = Self::is_block_with_receiver(&node.as_node());
        self.frames.push(is_bwr);
        ruby_prism::visit_call_node(self, node);
        self.frames.pop();
    }

    // Non-call branch nodes still need frame tracking so the frame stack
    // stays in sync with the Visit walk. However, only CallNode can be a
    // block-with-receiver, so every other branch gets `false`.
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        self.frames.push(false);
        ruby_prism::visit_class_node(self, node);
        self.frames.pop();
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        self.frames.push(false);
        ruby_prism::visit_module_node(self, node);
        self.frames.pop();
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        self.frames.push(false);
        ruby_prism::visit_def_node(self, node);
        self.frames.pop();
    }
}

impl super::dispatch::Rule for PluckVisitor {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        // Every branch node must push a frame so the branch stack stays 1:1
        // with `leave` (the ancestor-block-with-receiver counter rides that
        // stack).
        Interest(Interest::ENTER_ALL | Interest::LEAVE)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.check_call(&call);
        }
        let is_bwr = Self::is_block_with_receiver(node);
        self.frames.push(is_bwr);
    }

    fn leave(&mut self) {
        self.frames.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<PluckOffense> {
        check_rails_pluck(src.as_bytes())
    }

    #[test]
    fn flags_simple_map() {
        let off = run("x.map { |a| a[:foo] }\n");
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].sel_start, 2); // 'map' starts at byte 2
    }

    #[test]
    fn flags_collect() {
        let off = run("x.collect { |a| a[:foo] }\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_string_key() {
        let off = run("x.map { |a| a['foo'] }\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_method_call_key() {
        let off = run("x.map { |a| a[obj.do_something] }\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_safe_navigation() {
        let off = run("x&.map { |a| a[:foo] }\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn accepts_different_receiver() {
        assert!(run("x.map { |a| b[:foo] }\n").is_empty());
    }

    #[test]
    fn accepts_multiple_args() {
        assert!(run("x.map { |_, obj| obj['id'] }\n").is_empty());
    }

    #[test]
    fn accepts_regexp_key() {
        assert!(run("x.map { |a| a[/regexp/] }\n").is_empty());
    }

    #[test]
    fn accepts_block_arg_in_key() {
        // `x.map { |x| x[x] }` — key IS the block argument
        assert!(run("x.map { |x| x[x] }\n").is_empty());
    }

    #[test]
    fn accepts_block_arg_descendant_in_key() {
        // key contains block arg as descendant
        assert!(run("x.map { |a| a[foo...a.to_something] }\n").is_empty());
    }

    #[test]
    fn flags_numblock() {
        let off = run("x.map { _1[:foo] }\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn flags_itblock() {
        let off = run("x.map { it[:foo] }\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn accepts_numblock_arg_in_key() {
        assert!(run("x.map { _1[foo..._1.to_something] }\n").is_empty());
    }

    #[test]
    fn accepts_numblock_not_1() {
        assert!(run("x.map { _2['id'] }\n").is_empty());
    }

    #[test]
    fn ancestor_block_with_receiver_suppresses() {
        // map inside a repeatable block with receiver — N+1 risk
        assert!(run("n.each do |x|\n  x.map { |a| a[:foo] }\nend\n").is_empty());
    }

    #[test]
    fn ancestor_numblock_with_receiver_suppresses() {
        assert!(run("n.each do\n  _1.map { |a| a[:foo] }\nend\n").is_empty());
    }

    #[test]
    fn ancestor_block_without_receiver_allows() {
        // map inside a block without a receiver (e.g. `foo do ... end`)
        let off = run("foo do\n  x.map { |a| a[:foo] }\nend\n");
        assert_eq!(off.len(), 1);
    }

    #[test]
    fn bundle_rule_matches_standalone() {
        let src = "x.map { |a| a[:foo] }\ny.collect { |b| b[:bar] }\n";
        let alone = check_rails_pluck(src.as_bytes());
        let mut rule = build_rule();
        super::super::dispatch::run(src.as_bytes(), &mut [&mut rule]);
        let bundled = rule.finish();
        assert_eq!(bundled.len(), alone.len());
        assert_eq!(bundled.len(), 2);
    }

    #[test]
    fn bundle_with_ancestor_guard() {
        let src = "n.each do |x|\n  x.map { |a| a[:foo] }\nend\n";
        let alone = check_rails_pluck(src.as_bytes());
        let mut rule = build_rule();
        super::super::dispatch::run(src.as_bytes(), &mut [&mut rule]);
        let bundled = rule.finish();
        assert_eq!(bundled.len(), alone.len());
        assert_eq!(bundled.len(), 0);
    }
}
