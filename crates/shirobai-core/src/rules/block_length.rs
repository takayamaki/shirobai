use ruby_prism::{
    Node, Visit, visit_call_node, visit_forwarding_super_node, visit_lambda_node, visit_super_node,
};

use super::code_length::{CodeLength, Fold};

/// A block whose body length exceeds `Max`. On the default fast path the
/// `AllowedMethods` exclusion (incl. receiver-qualified entries) is applied
/// here too; the regex-based `AllowedPatterns` filtering always stays on the
/// Ruby side, which has the exact regexp semantics.
pub struct BlockLengthCandidate {
    pub start_offset: usize,
    pub end_offset: usize,
    /// End of the block opening (`do` / `{`), used for the LSP location mode.
    pub head_end: usize,
    pub length: usize,
    pub method_name: String,
    /// Raw receiver source, or empty when the call has no receiver.
    pub receiver: String,
}

pub fn check_block_length(
    source: &[u8],
    max: usize,
    count_comments: bool,
    count_as_one: &[String],
) -> Vec<BlockLengthCandidate> {
    check_block_length_filtered(source, max, count_comments, count_as_one, &[], false)
}

/// Like [`check_block_length`], but when `filtered` is set the `AllowedMethods`
/// exclusion (exact-name entries and receiver-qualified `Foo.bar` entries) is
/// applied here, byte-for-byte like the Ruby wrapper's `allowed_method?` +
/// `method_receiver_excluded?`, so allowlisted blocks never cross back into
/// Ruby. The regex-based `AllowedPatterns` cannot move here; when patterns are
/// configured the Ruby side passes `filtered: false` and keeps the full path.
pub fn check_block_length_filtered(
    source: &[u8],
    max: usize,
    count_comments: bool,
    count_as_one: &[String],
    allowed_methods: &[String],
    filtered: bool,
) -> Vec<BlockLengthCandidate> {
    let mut visitor = build_rule(
        source,
        max,
        count_comments,
        count_as_one,
        allowed_methods,
        filtered,
    );
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.out
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule<'a>(
    source: &'a [u8],
    max: usize,
    count_comments: bool,
    count_as_one: &[String],
    allowed_methods: &'a [String],
    filtered: bool,
) -> Visitor<'a> {
    Visitor {
        source,
        max,
        calc: CodeLength::new(source, count_comments, Fold::from_types(count_as_one)),
        allowed_methods,
        filtered,
        out: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    max: usize,
    calc: CodeLength<'a>,
    allowed_methods: &'a [String],
    filtered: bool,
    pub(crate) out: Vec<BlockLengthCandidate>,
}

impl Visitor<'_> {
    fn source_text(&self, start: usize, end: usize) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.source[start..end])
    }

    /// Body length of a block via the shared `CodeLength` calculator (matching
    /// RuboCop's `CodeLengthCalculator`).
    fn body_length(&self, body: Option<Node<'_>>) -> usize {
        self.calc.body_length(body)
    }

    /// Port of `Node#class_constructor?` restricted to the block case: the
    /// block's call is `{Class,Module,Struct}.new` or `Data.define` on a
    /// top-level constant.
    fn is_class_constructor(&self, call: &ruby_prism::CallNode<'_>) -> bool {
        let allowed: &[&[u8]] = match call.name().as_slice() {
            b"new" => &[b"Class", b"Module", b"Struct"],
            b"define" => &[b"Data"],
            _ => return false,
        };
        let Some(receiver) = call.receiver() else {
            return false;
        };
        match top_level_const_name(&receiver) {
            Some(name) => allowed.contains(&name),
            None => false,
        }
    }

    fn receiver_source(&self, call: &ruby_prism::CallNode<'_>) -> String {
        match call.receiver() {
            Some(recv) => {
                let loc = recv.location();
                self.source_text(loc.start_offset(), loc.end_offset())
                    .into_owned()
            }
            None => String::new(),
        }
    }

    /// Fast-path port of the Ruby wrapper's `allowed_method?` +
    /// `method_receiver_excluded?` (themselves ports of stock `BlockLength`).
    /// `allowed_method?` matches the whole entry against the method name;
    /// `method_receiver_excluded?` splits each entry Ruby-style on `"."` — a
    /// dotless entry is method-only and matches any receiver, otherwise the
    /// receiver part must equal the whitespace-stripped receiver source.
    fn excluded(&self, method_name: &str, receiver: &str) -> bool {
        if !self.filtered || self.allowed_methods.is_empty() {
            return false;
        }
        if self.allowed_methods.iter().any(|e| e == method_name) {
            return true;
        }
        // `node_receiver.empty? ? nil : node_receiver.gsub(/\s+/, "")`.
        let node_receiver: Option<String> = if receiver.is_empty() {
            None
        } else {
            Some(receiver.chars().filter(|c| !is_ruby_space(*c)).collect())
        };
        self.allowed_methods.iter().any(|entry| {
            let (first, second) = split_dot_first_two(entry);
            let (cfg_receiver, cfg_method) = match second {
                Some(method) => (first, Some(method)),
                // Dotless entry: method-only, matches any receiver.
                None => (node_receiver.as_deref(), first),
            };
            cfg_method == Some(method_name) && cfg_receiver == node_receiver.as_deref()
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn push_candidate(
        &mut self,
        start: usize,
        end: usize,
        head_end: usize,
        length: usize,
        method_name: String,
        receiver: String,
    ) {
        self.out.push(BlockLengthCandidate {
            start_offset: start,
            end_offset: end,
            head_end,
            length,
            method_name,
            receiver,
        });
    }
}

/// Ruby `\s` (the ASCII whitespace class incl. vertical tab), as matched by
/// the `gsub(/\s+/, "")` receiver normalization.
fn is_ruby_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\u{b}' | '\u{c}')
}

/// Ruby-compatible `entry.split(".")` limited to the first two parts: trailing
/// empty strings are dropped (`"a." → ["a"]`, `"" → []`), embedded empties are
/// kept (`".b" → ["", "b"]`) and parts beyond the second are ignored
/// (`"a.b.c"` → `("a", "b")`), exactly like the Ruby destructuring assignment.
fn split_dot_first_two(entry: &str) -> (Option<&str>, Option<&str>) {
    let mut parts: Vec<&str> = entry.split('.').collect();
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    (parts.first().copied(), parts.get(1).copied())
}

/// Returns the constant name when `node` is a top-level constant (`Foo` or
/// `::Foo`), matching RuboCop's `global_const?` (`(const {nil? cbase} _)`).
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

impl Visitor<'_> {
    /// `on_block` (a block attached to a call): measure the body and push a
    /// candidate unless excluded.
    fn process_call(&mut self, node: &ruby_prism::CallNode<'_>) {
        if let Some(block) = node.block()
            && let Some(block_node) = block.as_block_node()
            && !self.is_class_constructor(node)
        {
            let length = self.body_length(block_node.body());
            if length > self.max {
                let loc = node.location();
                let method_name = String::from_utf8_lossy(node.name().as_slice()).into_owned();
                let receiver = self.receiver_source(node);
                if !self.excluded(&method_name, &receiver) {
                    self.push_candidate(
                        loc.start_offset(),
                        loc.end_offset(),
                        block_node.opening_loc().end_offset(),
                        length,
                        method_name,
                        receiver,
                    );
                }
            }
        }
    }

    /// `on_block` for `-> { ... }` lambda literals.
    fn process_lambda(&mut self, node: &ruby_prism::LambdaNode<'_>) {
        let length = self.body_length(node.body());
        if length > self.max && !self.excluded("lambda", "") {
            let loc = node.location();
            self.push_candidate(
                loc.start_offset(),
                loc.end_offset(),
                node.opening_loc().end_offset(),
                length,
                "lambda".to_string(),
                String::new(),
            );
        }
    }

    /// `on_block` for `super do..end` / `super(arg) do..end`. parser-gem
    /// represents this as a `block` whose send is a `super`/`zsuper`, with
    /// `method_name == :super` and no receiver, so stock's `on_block` runs
    /// `check_code_length` on it. Prism splits it off as `SuperNode` /
    /// `ForwardingSuperNode` (carrying a `BlockNode`), which the generic call
    /// visit doesn't see — without this hook a long `super do..end` is silently
    /// missed (the BlockLength taking-too-many-lines offense on Discourse's
    /// `plugins/chat/lib/discourse_dev/category_channel.rb` was this case).
    fn process_super(&mut self, loc: ruby_prism::Location<'_>, block: Option<Node<'_>>) {
        let Some(block) = block else { return };
        let Some(block_node) = block.as_block_node() else {
            return;
        };
        let length = self.body_length(block_node.body());
        if length > self.max && !self.excluded("super", "") {
            self.push_candidate(
                loc.start_offset(),
                loc.end_offset(),
                block_node.opening_loc().end_offset(),
                length,
                "super".to_string(),
                String::new(),
            );
        }
    }

    fn process_super_call(&mut self, node: &ruby_prism::SuperNode<'_>) {
        self.process_super(node.location(), node.block());
    }

    fn process_forwarding_super(&mut self, node: &ruby_prism::ForwardingSuperNode<'_>) {
        self.process_super(node.location(), node.block().map(|b| b.as_node()));
    }
}

impl<'pr> Visit<'pr> for Visitor<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.process_call(node);
        visit_call_node(self, node);
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        self.process_lambda(node);
        visit_lambda_node(self, node);
    }

    fn visit_super_node(&mut self, node: &ruby_prism::SuperNode<'pr>) {
        self.process_super_call(node);
        visit_super_node(self, node);
    }

    fn visit_forwarding_super_node(&mut self, node: &ruby_prism::ForwardingSuperNode<'pr>) {
        self.process_forwarding_super(node);
        visit_forwarding_super_node(self, node);
    }
}

/// Shared-walk driver. The generic branch hook fires for every `CallNode` /
/// `LambdaNode` / `SuperNode` / `ForwardingSuperNode` the typed visits see
/// except the `CallNode` reached through `MatchWriteNode`'s concretely-typed
/// `call` field — an `=~` operator call, which never carries a block literal,
/// so `process_call` skips it anyway.
impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.process_call(&call);
        } else if let Some(lambda) = node.as_lambda_node() {
            self.process_lambda(&lambda);
        } else if let Some(s) = node.as_super_node() {
            self.process_super_call(&s);
        } else if let Some(s) = node.as_forwarding_super_node() {
            self.process_forwarding_super(&s);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Got {
        ranges: Vec<(usize, usize)>,
        lengths: Vec<usize>,
        methods: Vec<String>,
        receivers: Vec<String>,
    }

    fn run(source: &str, max: usize, count_comments: bool) -> Got {
        run_fold(source, max, count_comments, &[])
    }

    fn run_fold(source: &str, max: usize, count_comments: bool, fold: &[&str]) -> Got {
        let fold: Vec<String> = fold.iter().map(|s| s.to_string()).collect();
        let c = check_block_length(source.as_bytes(), max, count_comments, &fold);
        Got {
            ranges: c.iter().map(|o| (o.start_offset, o.end_offset)).collect(),
            lengths: c.iter().map(|o| o.length).collect(),
            methods: c.iter().map(|o| o.method_name.clone()).collect(),
            receivers: c.iter().map(|o| o.receiver.clone()).collect(),
        }
    }

    // Typical: a do..end block over the limit.
    #[test]
    fn long_do_block() {
        let src = "something do\n  a = 1\n  a = 2\n  a = 3\nend";
        let got = run(src, 2, false);
        assert_eq!(got.lengths, vec![3]);
        assert_eq!(got.ranges, vec![(0, src.len())]);
        assert_eq!(got.methods, vec!["something"]);
    }

    // Typical: a block under the limit produces nothing.
    #[test]
    fn short_block() {
        let got = run("something do\n  a = 1\n  a = 2\nend", 2, false);
        assert!(got.lengths.is_empty());
    }

    // Blank lines are not counted.
    #[test]
    fn blank_lines_excluded() {
        let got = run("something do\n  a = 1\n\n\n  a = 4\nend", 2, false);
        assert!(got.lengths.is_empty());
    }

    // Comment lines are not counted by default.
    #[test]
    fn comment_lines_excluded_by_default() {
        let got = run(
            "something do\n  a = 1\n  #a = 2\n  #a = 3\n  a = 4\nend",
            2,
            false,
        );
        assert!(got.lengths.is_empty());
    }

    // With CountComments, comment lines are counted.
    #[test]
    fn comment_lines_counted_when_enabled() {
        let got = run("something do\n  a = 1\n  #a = 2\n  a = 3\nend", 2, true);
        assert_eq!(got.lengths, vec![3]);
    }

    // Brace blocks count too.
    #[test]
    fn brace_block() {
        let src = "something {\n  a = 1\n  a = 2\n  a = 3\n}";
        let got = run(src, 2, false);
        assert_eq!(got.lengths, vec![3]);
        assert_eq!(got.ranges, vec![(0, src.len())]);
    }

    // Empty blocks produce nothing.
    #[test]
    fn empty_block() {
        assert!(run("something do\nend", 2, false).lengths.is_empty());
    }

    // Numbered-parameter blocks are counted.
    #[test]
    fn numbered_block() {
        let got = run("something do\n  a = _1\n  a = _2\n  a = _3\nend", 2, false);
        assert_eq!(got.lengths, vec![3]);
    }

    // Nested blocks are counted independently; the outer counts inner source lines.
    #[test]
    fn nested_blocks() {
        let src =
            "something do\n  something do\n    a = 2\n    a = 3\n    a = 4\n    a = 5\n  end\nend";
        let got = run(src, 2, false);
        assert_eq!(got.lengths.len(), 2);
        // outer length 6, inner length 4 (order: outer entered first)
        assert_eq!(got.lengths, vec![6, 4]);
    }

    // Multiline receiver with a short body is not an offense.
    #[test]
    fn multiline_receiver_short_body() {
        let src = "[\n  :a,\n  :b,\n  :c,\n].each do\n  a = 1\n  a = 2\nend";
        assert!(run(src, 2, false).lengths.is_empty());
    }

    // Class/Module/Struct constructors and Data.define are skipped.
    #[test]
    fn class_constructors_skipped() {
        let body = "\n  a = 1\n  a = 2\n  a = 3\n  a = 4\n  a = 5\n  a = 6\n";
        for ctor in [
            "Class.new",
            "Module.new",
            "Struct.new(:a)",
            "Data.define(:a)",
            "::Class.new",
        ] {
            let src = format!("{ctor} do{body}end");
            assert!(
                run(&src, 2, false).lengths.is_empty(),
                "{ctor} should be skipped"
            );
        }
    }

    // A non-top-level constructor-like call is still checked.
    #[test]
    fn namespaced_constructor_checked() {
        let src = "Foo::Class.new do\n  a = 1\n  a = 2\n  a = 3\nend";
        let got = run(src, 2, false);
        assert_eq!(got.lengths, vec![3]);
        assert_eq!(got.methods, vec!["new"]);
        assert_eq!(got.receivers, vec!["Foo::Class"]);
    }

    // CountAsOne 'array' folds a multi-line array literal to one line.
    #[test]
    fn count_as_one_folds_array() {
        let src = "something do\n  a = 1\n  a = [\n    2,\n    3\n  ]\nend";
        assert!(run_fold(src, 2, false, &["array"]).lengths.is_empty());
        // Without folding it is over the limit (5 body lines).
        assert_eq!(run(src, 2, false).lengths, vec![5]);
    }

    // CountAsOne 'hash' folds a multi-line hash literal.
    #[test]
    fn count_as_one_folds_hash() {
        let src = "something do\n  a = 1\n  a = {\n    b: 2,\n    c: 3\n  }\nend";
        assert!(run_fold(src, 2, false, &["hash"]).lengths.is_empty());
    }

    // Foldables nested inside another foldable are not double-counted.
    #[test]
    fn count_as_one_nested_foldable() {
        let src = "something do\n  a = 1\n  a = [\n    [\n      2\n    ],\n    3\n  ]\nend";
        // outer array folds to 1; nested array is inside it, not counted separately.
        assert!(run_fold(src, 2, false, &["array"]).lengths.is_empty());
    }

    // Receiver source is captured for the Ruby-side receiver exclusion.
    #[test]
    fn receiver_captured() {
        let src = "Foo::Bar.baz do\n  a = 1\n  a = 2\n  a = 3\nend";
        let got = run(src, 2, false);
        assert_eq!(got.methods, vec!["baz"]);
        assert_eq!(got.receivers, vec!["Foo::Bar"]);
    }

    fn run_allowed(source: &str, allowed: &[&str]) -> Vec<String> {
        let allowed: Vec<String> = allowed.iter().map(|s| s.to_string()).collect();
        check_block_length_filtered(source.as_bytes(), 2, false, &[], &allowed, true)
            .into_iter()
            .map(|c| c.method_name)
            .collect()
    }

    const LONG_BLOCK: &str = "foo.refine do\n  a = 1\n  a = 2\n  a = 3\nend";

    // allowed_method?: a whole entry equal to the method name excludes it.
    #[test]
    fn allowed_method_exact_name() {
        assert!(run_allowed(LONG_BLOCK, &["refine"]).is_empty());
        assert_eq!(run_allowed(LONG_BLOCK, &["refine2"]), vec!["refine"]);
    }

    // Receiver-qualified entry: receiver must equal the whitespace-stripped
    // receiver source, method must equal the method name.
    #[test]
    fn receiver_qualified_entry() {
        assert!(run_allowed(LONG_BLOCK, &["foo.refine"]).is_empty());
        assert_eq!(run_allowed(LONG_BLOCK, &["bar.refine"]), vec!["refine"]);
        assert_eq!(run_allowed(LONG_BLOCK, &["foo.other"]), vec!["refine"]);
    }

    // The node receiver is whitespace-stripped before comparing; the entry is
    // not (it is cut at every dot instead).
    #[test]
    fn whitespace_stripped_receiver() {
        let src = "[1, 2].each do\n  a = 1\n  a = 2\n  a = 3\nend";
        assert!(run_allowed(src, &["[1,2].each"]).is_empty());
        assert_eq!(run_allowed(src, &["[1, 2].each"]), vec!["each"]);
    }

    // A dotted receiver can only be excluded by a dotless (method-only) entry,
    // because Ruby's split(".") cuts the entry at every dot ("Foo.bar.baz"
    // yields receiver "Foo", method "bar" — never receiver "Foo.bar").
    #[test]
    fn dotted_receiver_split_at_every_dot() {
        let src = "Foo\n  .bar\n  .baz do\n  a = 1\n  a = 2\n  a = 3\nend";
        assert_eq!(run_allowed(src, &["Foo.bar.baz"]), vec!["baz"]);
        assert_eq!(run_allowed(src, &["Foo.baz"]), vec!["baz"]);
        assert!(run_allowed(src, &["baz"]).is_empty());
    }

    // A receiver-qualified entry never matches a receiverless block, and a
    // dotless entry matches regardless of receiver (stock semantics).
    #[test]
    fn receiverless_block_vs_qualified_entry() {
        let src = "something do\n  a = 1\n  a = 2\n  a = 3\nend";
        assert_eq!(run_allowed(src, &["Foo.something"]), vec!["something"]);
        assert!(run_allowed(src, &["something"]).is_empty());
        assert!(run_allowed(LONG_BLOCK, &["refine"]).is_empty());
    }

    // Ruby split(".") edge cases: trailing dots are dropped ("refine." is
    // method-only), parts beyond the second are ignored ("a.b.c" => ("a","b")),
    // and an empty entry matches nothing.
    #[test]
    fn split_edge_cases() {
        assert!(run_allowed(LONG_BLOCK, &["refine."]).is_empty());
        let src = "a.b do\n  x = 1\n  x = 2\n  x = 3\nend";
        assert!(run_allowed(src, &["a.b.c"]).is_empty());
        assert_eq!(run_allowed(src, &[""]), vec!["b"]);
        assert_eq!(run_allowed(src, &["."]), vec!["b"]);
    }

    // Lambdas are matched as method "lambda" with no receiver.
    #[test]
    fn lambda_allowed_by_name() {
        let src = "-> {\n  a = 1\n  a = 2\n  a = 3\n}";
        assert_eq!(run_allowed(src, &[]), vec!["lambda"]);
        assert!(run_allowed(src, &["lambda"]).is_empty());
        assert_eq!(run_allowed(src, &["Foo.lambda"]), vec!["lambda"]);
    }

    // filtered: false ignores the allowlist entirely (slow path).
    #[test]
    fn unfiltered_ignores_allowlist() {
        let allowed = vec!["refine".to_string()];
        let got =
            check_block_length_filtered(LONG_BLOCK.as_bytes(), 2, false, &[], &allowed, false);
        assert_eq!(got.len(), 1);
    }
}
