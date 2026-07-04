//! `Performance/TimesMap` (rubocop-performance): flags `x.times.map { ... }`
//! / `x.times.map(&:blk)` (and `collect`) and rewrites the call to
//! `Array.new(x) { ... }`.
//!
//! Mirrors `vendor/rubocop-performance/lib/rubocop/cop/performance/times_map.rb`
//! (v1.26.1). The stock node pattern has two branches; in prism terms (a
//! block is a field of `CallNode`, not a wrapper node, and prism keeps a
//! block-pass in that same field, not in the argument list):
//!
//! - outer node is a `map` / `collect` call carrying EITHER a literal block
//!   (prism `BlockNode` — including numbered `_1` and `it` blocks, which
//!   stock matches through `any_block`) OR a sole block-pass (prism
//!   `BlockArgumentNode`). No block at all is not an offense. A positional
//!   argument disqualifies both shapes (`map(1) { }`, `map(1, &:b)`).
//! - the receiver is a bare `times` call with a receiver and no args / block
//!   (`(call $!nil? :times)`). `$!nil?` only means "times has a receiver"
//!   (a receiverless `times.map` is skipped) — the `nil` LITERAL receiver
//!   `nil.times.map` still matches and is flagged.
//! - `handleable_receiver?`: accept when the `times` receiver (the count) is
//!   an int / float literal, OR when `times` is reached through a `.` dot.
//!   Safe-navigation `&.times` and `::times` are NOT dots, so a non-literal
//!   count there stays unflagged (`foo&.times.map`, `foo::times.map`), while
//!   an int / float count is still handled (`5&.times.map`, `5::times.map`).
//!
//! Offense range = the whole outer node (prism `CallNode.location`, which
//! includes the block). Autocorrect replaces stock's `map_or_collect`:
//!
//! - block form: the send WITHOUT its literal block — up to the argument
//!   list's closing paren (`map() { }` keeps its `()` in the replaced range)
//!   or the method-name token when there is none — with `Array.new(count)`.
//! - block-pass form: the whole send (the block-pass is part of it) with
//!   `Array.new(count, &blk)`, the block-pass source appended verbatim.
//!
//! The message gains ` only if \`count\` is always 0 or more.` unless the
//! count is a `literal?` node. Stock `Node#literal?` is a FLAT type check
//! (`LITERALS.include?(type)`, not recursive): `[a]` counts as an array
//! literal even though `a` is not literal, but a PARENTHESIZED receiver is a
//! `begin` node — `(1..5)` / `(2 + 3)` are not literals and do get the
//! `only if` clause.

use ruby_prism::{CallNode, Node, Visit};

#[derive(Debug, Clone)]
pub struct PerfTimesMapOffense {
    /// Whole-node byte range (offense highlight).
    pub start: usize,
    pub end: usize,
    /// Parser-send byte range of the `x.times.map(...)` call WITHOUT its
    /// literal block: the autocorrect replace target.
    pub replace_start: usize,
    pub replace_end: usize,
    pub message: String,
    /// `Array.new(count[, extra args])`.
    pub replacement: String,
}

/// Standalone entry point used by the per-cop fallback.
pub fn check_perf_times_map(source: &[u8]) -> Vec<PerfTimesMapOffense> {
    let mut visitor = build_rule();
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in the shared-walk bundle.
/// Config-less: the cop has no options beyond Enabled/SafeAutoCorrect.
pub(crate) fn build_rule() -> PerfTimesMapVisitor {
    PerfTimesMapVisitor {
        offenses: Vec::new(),
    }
}

pub(crate) struct PerfTimesMapVisitor {
    pub(crate) offenses: Vec<PerfTimesMapOffense>,
}

/// A parser `send` with at least one positional argument. A literal block is
/// not a positional argument; a block-pass lives in prism's block slot, not
/// here, so it is not counted either.
fn has_positional_args(call: &CallNode<'_>) -> bool {
    call.arguments().is_some_and(|a| !a.arguments().is_empty())
}

/// Stock `Node#literal?`: a FLAT membership test of the node's parser type in
/// `LITERALS` (truthy + falsey), NOT the recursive `all_literal?`. Mapped to
/// the prism node classes that carry those parser types.
fn count_is_literal(node: &Node<'_>) -> bool {
    node.as_integer_node().is_some()                       // int
        || node.as_float_node().is_some()                  // float
        || node.as_imaginary_node().is_some()              // complex
        || node.as_rational_node().is_some()               // rational
        || node.as_string_node().is_some()                 // str
        || node.as_interpolated_string_node().is_some()    // dstr
        || node.as_x_string_node().is_some()               // xstr
        || node.as_interpolated_x_string_node().is_some()  // xstr (interp)
        || node.as_symbol_node().is_some()                 // sym
        || node.as_interpolated_symbol_node().is_some()    // dsym
        || node.as_array_node().is_some()                  // array
        || node.as_hash_node().is_some()                   // hash
        || node.as_regular_expression_node().is_some()     // regexp
        || node.as_interpolated_regular_expression_node().is_some() // regexp (interp)
        || node.as_true_node().is_some()                   // true
        || node.as_false_node().is_some()                  // false
        || node.as_nil_node().is_some()                    // nil
        || node.as_range_node().is_some()                  // irange / erange
        // Magic constants keep their parser LITERAL types: `__LINE__` is an
        // `int` and `__FILE__` a `str` on the parser side (stock probed:
        // both get the no-`only if` message), while `__ENCODING__` is its
        // own non-literal type (stock probed: `only if` clause).
        || node.as_source_line_node().is_some()            // int (__LINE__)
        || node.as_source_file_node().is_some() // str (__FILE__)
}

impl PerfTimesMapVisitor {
    fn check_call(&mut self, node: &CallNode<'_>) {
        // Outer call must be `map` or `collect`.
        let method = match node.name().as_slice() {
            b"map" => "map",
            b"collect" => "collect",
            _ => return,
        };

        // Classify the block slot: a literal block (`BlockNode`, incl. numbered
        // and `it` blocks) is the block form; a sole block-pass
        // (`BlockArgumentNode`) is the block-pass form; no block is not an
        // offense.
        let block = node.block();
        let block_arg = block.as_ref().and_then(|b| b.as_block_argument_node());
        let is_block = block.as_ref().is_some_and(|b| b.as_block_node().is_some());
        if !is_block && block_arg.is_none() {
            return;
        }
        // Neither stock branch allows a positional argument on `map`/`collect`.
        if has_positional_args(node) {
            return;
        }

        // Receiver must be a bare `times` call with a receiver (`$!nil?`) and
        // no args or block of its own.
        let Some(receiver) = node.receiver() else {
            return;
        };
        let Some(times) = receiver.as_call_node() else {
            return;
        };
        if times.name().as_slice() != b"times" {
            return;
        }
        let Some(count) = times.receiver() else {
            return;
        };
        if has_positional_args(&times) || times.block().is_some() {
            return;
        }

        // handleable_receiver?: int/float literal count, or a `.`-dotted
        // `times` (`&.` and `::` are not dots). `__LINE__` is an `int` on
        // the parser side, so it satisfies the literal arm (stock probed:
        // `__LINE__::times.map { }` is flagged).
        let count_int_or_float = count.as_integer_node().is_some()
            || count.as_float_node().is_some()
            || count.as_source_line_node().is_some();
        let times_dot = times
            .call_operator_loc()
            .is_some_and(|l| l.as_slice() == b".");
        if !count_int_or_float && !times_dot {
            return;
        }

        let count_src = String::from_utf8_lossy(count.location().as_slice()).into_owned();
        let node_loc = node.location();
        let (start, end) = (node_loc.start_offset(), node_loc.end_offset());

        let (replace_end, args_suffix) = if let Some(ba) = block_arg {
            // Block-pass form: stock replaces the whole send (block-pass
            // included) and appends `, <block-pass source>`.
            let ba_src = String::from_utf8_lossy(ba.location().as_slice()).into_owned();
            (end, format!(", {ba_src}"))
        } else {
            // Block form: stock replaces the send WITHOUT its literal block —
            // up to the argument list's closing paren, or the method name when
            // there is none.
            let replace_end = node
                .closing_loc()
                .or_else(|| node.message_loc())
                .map(|l| l.end_offset())
                .unwrap_or(end);
            (replace_end, String::new())
        };

        let replacement = format!("Array.new({count_src}{args_suffix})");
        let base =
            format!("Use `Array.new({count_src})` with a block instead of `.times.{method}`");
        let message = if count_is_literal(&count) {
            format!("{base}.")
        } else {
            format!("{base} only if `{count_src}` is always 0 or more.")
        };

        self.offenses.push(PerfTimesMapOffense {
            start,
            end,
            replace_start: start,
            replace_end,
            message,
            replacement,
        });
    }
}

impl<'pr> Visit<'pr> for PerfTimesMapVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        ruby_prism::visit_call_node(self, node);
    }
}

impl super::dispatch::Rule for PerfTimesMapVisitor {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(Interest::ENTER_CALL)
    }

    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.check_call(&call);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(src: &str) -> Vec<PerfTimesMapOffense> {
        check_perf_times_map(src.as_bytes())
    }

    fn spans(src: &str) -> Vec<(usize, usize)> {
        detect(src).into_iter().map(|o| (o.start, o.end)).collect()
    }

    // Expectations below are stock-derived: every offense range, message and
    // correction was probed against rubocop-performance 1.26.1 with rubocop
    // 1.88.0 (.tmp/2026-07-05/probe-perf).

    #[test]
    fn magic_constants_keep_parser_literal_types() {
        // Stock probed: `__LINE__` is a parser `int` and `__FILE__` a `str`
        // (no `only if` clause); `__ENCODING__` is its own non-literal type.
        let off = detect("__LINE__.times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        assert_eq!(
            off[0].message,
            "Use `Array.new(__LINE__)` with a block instead of `.times.map`."
        );
        let off = detect("__FILE__.times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        assert_eq!(
            off[0].message,
            "Use `Array.new(__FILE__)` with a block instead of `.times.map`."
        );
        let off = detect("__ENCODING__.times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        assert_eq!(
            off[0].message,
            "Use `Array.new(__ENCODING__)` with a block instead of `.times.map` \
             only if `__ENCODING__` is always 0 or more."
        );
    }

    #[test]
    fn magic_int_satisfies_the_literal_arm() {
        // `__LINE__::times.map` has no `.` dot, but the parser-int count
        // satisfies `handleable_receiver?`'s literal arm (stock probed).
        let off = detect("__LINE__::times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].replacement, "Array.new(__LINE__)");
    }

    #[test]
    fn flags_block_form() {
        let off = detect("5.times.map { |i| i.to_s }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 26));
        assert_eq!((o.replace_start, o.replace_end), (0, 11)); // `5.times.map`
        assert_eq!(
            o.message,
            "Use `Array.new(5)` with a block instead of `.times.map`."
        );
        assert_eq!(o.replacement, "Array.new(5)");
    }

    #[test]
    fn flags_collect() {
        let off = detect("5.times.collect { |i| i.to_s }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 30));
        assert_eq!((o.replace_start, o.replace_end), (0, 15)); // `5.times.collect`
        assert_eq!(
            o.message,
            "Use `Array.new(5)` with a block instead of `.times.collect`."
        );
        assert_eq!(o.replacement, "Array.new(5)");
    }

    #[test]
    fn flags_block_pass() {
        let off = detect("5.times.map(&:to_s)\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 19));
        // Whole send replaced (block-pass included).
        assert_eq!((o.replace_start, o.replace_end), (0, 19));
        assert_eq!(o.replacement, "Array.new(5, &:to_s)");
        assert_eq!(
            o.message,
            "Use `Array.new(5)` with a block instead of `.times.map`."
        );
    }

    #[test]
    fn flags_block_pass_no_parens() {
        let off = detect("5.times.map &:to_s\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 18));
        assert_eq!((o.replace_start, o.replace_end), (0, 18));
        assert_eq!(o.replacement, "Array.new(5, &:to_s)");
    }

    #[test]
    fn flags_block_pass_method_ref() {
        let off = detect("5.times.map(&method(:foo))\n");
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].replacement, "Array.new(5, &method(:foo))");
        assert_eq!((off[0].start, off[0].end), (0, 26));
    }

    #[test]
    fn flags_nonliteral_receiver_only_if() {
        let off = detect("n.times.map { |i| i.to_s }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 26));
        assert_eq!((o.replace_start, o.replace_end), (0, 11)); // `n.times.map`
        assert_eq!(
            o.message,
            "Use `Array.new(n)` with a block instead of `.times.map` only if `n` is always 0 or more."
        );
        assert_eq!(o.replacement, "Array.new(n)");
    }

    #[test]
    fn flags_method_chain_receiver() {
        let off = detect("foo.bar.times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 27));
        assert_eq!((o.replace_start, o.replace_end), (0, 17)); // `foo.bar.times.map`
        assert_eq!(
            o.message,
            "Use `Array.new(foo.bar)` with a block instead of `.times.map` \
             only if `foo.bar` is always 0 or more."
        );
    }

    #[test]
    fn flags_float_receiver_no_only_if() {
        let off = detect("5.0.times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 23));
        assert_eq!((o.replace_start, o.replace_end), (0, 13)); // `5.0.times.map`
        assert_eq!(o.replacement, "Array.new(5.0)");
        assert_eq!(
            o.message,
            "Use `Array.new(5.0)` with a block instead of `.times.map`."
        );
    }

    #[test]
    fn flags_numbered_and_it_blocks() {
        // `any_block` matches numbered `_1` and `it` blocks — unlike Detect,
        // TimesMap does NOT exclude them.
        assert_eq!(spans("5.times.map { _1.to_s }\n"), vec![(0, 23)]);
        assert_eq!(spans("5.times.map { it.to_s }\n"), vec![(0, 23)]);
    }

    #[test]
    fn flags_csend_map() {
        let off = detect("5.times&.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 22));
        assert_eq!((o.replace_start, o.replace_end), (0, 12)); // `5.times&.map`
        assert_eq!(o.replacement, "Array.new(5)");
    }

    #[test]
    fn flags_empty_parens_block() {
        // `map()` keeps its empty argument parens inside the replaced range.
        let off = detect("5.times.map() { |i| i }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 23));
        assert_eq!((o.replace_start, o.replace_end), (0, 13)); // `5.times.map()`
        assert_eq!(o.replacement, "Array.new(5)");
    }

    #[test]
    fn flags_do_end_block() {
        let off = detect("5.times.map do |i|\n  i.to_s\nend\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 31));
        assert_eq!((o.replace_start, o.replace_end), (0, 11)); // `5.times.map`
        assert_eq!(o.replacement, "Array.new(5)");
    }

    #[test]
    fn flags_chained_after_block() {
        // The offense is the `any_block` node only; the trailing `.join` is a
        // separate outer call and stays out of the range.
        assert_eq!(spans("5.times.map { |i| i }.join\n"), vec![(0, 21)]);
    }

    #[test]
    fn flags_paren_receiver_only_if() {
        // A parenthesized receiver is a `begin` node — NOT a literal — so the
        // `only if` clause is added even though the inside is an int sum.
        let off = detect("(2 + 3).times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 27));
        assert_eq!((o.replace_start, o.replace_end), (0, 17)); // `(2 + 3).times.map`
        assert_eq!(o.replacement, "Array.new((2 + 3))");
        assert_eq!(
            o.message,
            "Use `Array.new((2 + 3))` with a block instead of `.times.map` \
             only if `(2 + 3)` is always 0 or more."
        );
    }

    #[test]
    fn flags_paren_range_receiver_only_if() {
        // Same trap: a range LITERAL would suppress `only if`, but the parens
        // make it a `begin` node.
        let off = detect("(1..5).times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.replace_start, o.replace_end), (0, 16)); // `(1..5).times.map`
        assert_eq!(o.replacement, "Array.new((1..5))");
        assert!(o.message.ends_with("only if `(1..5)` is always 0 or more."));
    }

    #[test]
    fn flags_string_receiver_no_only_if() {
        // A str literal receiver: no `only if`, and the source is copied
        // verbatim into `Array.new(...)`.
        let off = detect("\"5\".times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.replace_start, o.replace_end), (0, 13)); // `"5".times.map`
        assert_eq!(o.replacement, "Array.new(\"5\")");
        assert_eq!(
            o.message,
            "Use `Array.new(\"5\")` with a block instead of `.times.map`."
        );
    }

    #[test]
    fn flags_array_receiver_flat_literal() {
        // `literal?` is flat: `[a]` is an array literal (no `only if`) even
        // though `a` is not a literal.
        let off = detect("[a].times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!(o.replacement, "Array.new([a])");
        assert_eq!(
            o.message,
            "Use `Array.new([a])` with a block instead of `.times.map`."
        );
    }

    #[test]
    fn flags_nil_literal_receiver() {
        // `$!nil?` only rejects a receiverless `times`; the `nil` LITERAL
        // receiver still matches (and is a literal — no `only if`).
        let off = detect("nil.times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 23));
        assert_eq!(o.replacement, "Array.new(nil)");
        assert_eq!(
            o.message,
            "Use `Array.new(nil)` with a block instead of `.times.map`."
        );
    }

    #[test]
    fn flags_negative_int_receiver() {
        // prism folds `-5` into a single IntegerNode literal.
        let off = detect("-5.times.map { |i| i }\n");
        assert_eq!(off.len(), 1);
        let o = &off[0];
        assert_eq!((o.start, o.end), (0, 22));
        assert_eq!((o.replace_start, o.replace_end), (0, 12)); // `-5.times.map`
        assert_eq!(o.replacement, "Array.new(-5)");
        assert!(o.message.ends_with("instead of `.times.map`."));
    }

    #[test]
    fn flags_csend_times_with_int() {
        // `5&.times.map`: the int literal count makes it handleable even
        // though `&.times` is not a dot.
        assert_eq!(spans("5&.times.map { |i| i }\n"), vec![(0, 22)]);
    }

    #[test]
    fn flags_double_colon_times_with_int() {
        assert_eq!(spans("5::times.map { |i| i }\n"), vec![(0, 22)]);
    }

    #[test]
    fn accepts_csend_times_non_literal() {
        // `foo&.times.map`: non-literal count and `&.times` (not a dot).
        assert!(spans("foo&.times.map { |i| i }\n").is_empty());
    }

    #[test]
    fn accepts_double_colon_times_non_literal() {
        assert!(spans("foo::times.map { |i| i }\n").is_empty());
    }

    #[test]
    fn accepts_bare_map_without_block() {
        assert!(spans("5.times.map\n").is_empty());
    }

    #[test]
    fn accepts_times_with_arg() {
        assert!(spans("5.times(2).map { |i| i }\n").is_empty());
    }

    #[test]
    fn accepts_map_with_extra_arg_and_block_pass() {
        assert!(spans("5.times.map(1, &:to_s)\n").is_empty());
    }

    #[test]
    fn accepts_receiverless_times() {
        assert!(spans("times.map { |i| i }\n").is_empty());
    }

    #[test]
    fn accepts_times_with_block() {
        // `5.times { }.map`: the `times` call carries a block, so its parser
        // node is no longer a bare `(call ... :times)`.
        assert!(spans("5.times { }.map { |i| i }\n").is_empty());
    }

    #[test]
    fn accepts_times_with_block_pass() {
        assert!(spans("5.times(&:x).map { |i| i }\n").is_empty());
    }
}
