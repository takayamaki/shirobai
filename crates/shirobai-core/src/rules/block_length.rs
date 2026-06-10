use ruby_prism::{Node, Visit, parse, visit_call_node, visit_lambda_node};

/// A block whose body length exceeds `Max`. The cheap, config-driven filtering
/// (`AllowedMethods` / `AllowedPatterns` / receiver exclusion) is left to the
/// Ruby side, which has the exact regexp semantics; Rust only does the costly
/// parsing and line counting and reports the candidates.
pub struct BlockLengthCandidate {
    pub start_offset: usize,
    pub end_offset: usize,
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
    let result = parse(source);
    let node = result.node();
    let mut visitor = Visitor {
        source,
        max,
        count_comments,
        fold: Fold::from_types(count_as_one),
        out: Vec::new(),
    };
    visitor.visit(&node);
    visitor.out
}

/// Which constructs `CountAsOne` folds. Unknown types are ignored here; the
/// Ruby side raises `RuboCop::Warning` for them.
#[derive(Clone, Copy, Default)]
struct Fold {
    array: bool,
    hash: bool,
    heredoc: bool,
    method_call: bool,
}

impl Fold {
    fn from_types(types: &[String]) -> Self {
        let mut f = Fold::default();
        for t in types {
            match t.as_str() {
                "array" => f.array = true,
                "hash" => f.hash = true,
                "heredoc" => f.heredoc = true,
                "method_call" => f.method_call = true,
                _ => {}
            }
        }
        f
    }

    fn any(&self) -> bool {
        self.array || self.hash || self.heredoc || self.method_call
    }
}

struct Visitor<'a> {
    source: &'a [u8],
    max: usize,
    count_comments: bool,
    fold: Fold,
    out: Vec<BlockLengthCandidate>,
}

impl Visitor<'_> {
    fn source_text(&self, start: usize, end: usize) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.source[start..end])
    }

    /// Body length of a block, matching RuboCop's `CodeLengthCalculator`:
    /// non-blank/non-comment line count of the body, with `CountAsOne`
    /// foldable constructs collapsed to a single line each.
    fn body_length(&self, body: Option<Node<'_>>) -> usize {
        let Some(body) = body else { return 0 };
        let base = self.count_source_lines(&body) as isize;
        if !self.fold.any() {
            return base.max(0) as usize;
        }
        let mut fv = FoldVisitor {
            outer: self,
            suppress: 0,
            stack: Vec::new(),
            delta: 0,
        };
        fv.visit(&body);
        (base + fv.delta).max(0) as usize
    }

    /// Non-blank, non-comment line count of a node's own source span
    /// (`node.source.lines.count { !irrelevant_line?(line) }`), with heredocs
    /// measured by their body like RuboCop's `heredoc_length`.
    fn code_length(&self, node: &Node<'_>) -> usize {
        if let Some(loc) = self.heredoc_body_loc(node) {
            let text = self.source_text(loc.start_offset(), loc.end_offset());
            return text.lines().filter(|line| !self.irrelevant(line)).count() + 2;
        }
        self.count_source_lines(node)
    }

    fn count_source_lines(&self, node: &Node<'_>) -> usize {
        let loc = node.location();
        let text = self.source_text(loc.start_offset(), loc.end_offset());
        text.lines().filter(|line| !self.irrelevant(line)).count()
    }

    fn is_foldable(&self, node: &Node<'_>) -> bool {
        (self.fold.array && node.as_array_node().is_some())
            || (self.fold.hash && node.as_hash_node().is_some())
            || (self.fold.method_call && node.as_call_node().is_some())
            || (self.fold.heredoc && self.is_heredoc(node))
    }

    fn opening_is_heredoc(&self, opening: Option<ruby_prism::Location<'_>>) -> bool {
        match opening {
            Some(loc) => self
                .source
                .get(loc.start_offset()..loc.end_offset())
                .is_some_and(|s| s.starts_with(b"<<")),
            None => false,
        }
    }

    fn is_heredoc(&self, node: &Node<'_>) -> bool {
        if let Some(s) = node.as_string_node() {
            self.opening_is_heredoc(s.opening_loc())
        } else if let Some(s) = node.as_interpolated_string_node() {
            self.opening_is_heredoc(s.opening_loc())
        } else {
            false
        }
    }

    /// Body location of a heredoc string (between the marker and the closing
    /// delimiter), or `None` when the node is not a plain heredoc string.
    fn heredoc_body_loc<'a>(&self, node: &Node<'a>) -> Option<ruby_prism::Location<'a>> {
        let s = node.as_string_node()?;
        if self.opening_is_heredoc(s.opening_loc()) {
            Some(s.content_loc())
        } else {
            None
        }
    }

    fn irrelevant(&self, line: &str) -> bool {
        line.trim().is_empty() || (!self.count_comments && line.trim_start().starts_with('#'))
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

    fn push_candidate(
        &mut self,
        start: usize,
        end: usize,
        length: usize,
        method_name: String,
        receiver: String,
    ) {
        self.out.push(BlockLengthCandidate {
            start_offset: start,
            end_offset: end,
            length,
            method_name,
            receiver,
        });
    }
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

fn is_classlike(node: &Node<'_>) -> bool {
    matches!(node, Node::ClassNode { .. } | Node::ModuleNode { .. })
}

/// Applies `CountAsOne` folding over a block body, mirroring RuboCop's
/// `each_top_level_descendant`: each outermost foldable construct collapses to
/// one line, and class/module bodies are not descended into.
struct FoldVisitor<'a, 'b> {
    outer: &'b Visitor<'a>,
    /// Depth of foldable/classlike boundaries we are nested inside.
    suppress: usize,
    /// Per-node record of whether it raised `suppress`, popped on leave.
    stack: Vec<bool>,
    delta: isize,
}

impl FoldVisitor<'_, '_> {
    fn count_top_level(&mut self, node: &Node<'_>) {
        self.delta += 1 - self.outer.code_length(node) as isize;
    }
}

impl<'pr> Visit<'pr> for FoldVisitor<'_, '_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        let boundary = if self.suppress == 0 {
            if self.outer.is_foldable(&node) {
                self.count_top_level(&node);
                true
            } else {
                is_classlike(&node)
            }
        } else {
            false
        };
        if boundary {
            self.suppress += 1;
        }
        self.stack.push(boundary);
    }

    fn visit_branch_node_leave(&mut self) {
        if self.stack.pop().unwrap_or(false) {
            self.suppress -= 1;
        }
    }

    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        if self.suppress == 0 && self.outer.is_foldable(&node) {
            self.count_top_level(&node);
        }
    }
}

impl<'pr> Visit<'pr> for Visitor<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if let Some(block) = node.block()
            && let Some(block_node) = block.as_block_node()
            && !self.is_class_constructor(node)
        {
            let length = self.body_length(block_node.body());
            if length > self.max {
                let loc = node.location();
                let method_name = String::from_utf8_lossy(node.name().as_slice()).into_owned();
                let receiver = self.receiver_source(node);
                self.push_candidate(
                    loc.start_offset(),
                    loc.end_offset(),
                    length,
                    method_name,
                    receiver,
                );
            }
        }
        visit_call_node(self, node);
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        let length = self.body_length(node.body());
        if length > self.max {
            let loc = node.location();
            self.push_candidate(
                loc.start_offset(),
                loc.end_offset(),
                length,
                "lambda".to_string(),
                String::new(),
            );
        }
        visit_lambda_node(self, node);
    }
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
}
