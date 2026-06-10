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
) -> Vec<BlockLengthCandidate> {
    let result = parse(source);
    let node = result.node();
    let mut visitor = Visitor {
        source,
        max,
        count_comments,
        out: Vec::new(),
    };
    visitor.visit(&node);
    visitor.out
}

struct Visitor<'a> {
    source: &'a [u8],
    max: usize,
    count_comments: bool,
    out: Vec<BlockLengthCandidate>,
}

impl Visitor<'_> {
    fn source_text(&self, start: usize, end: usize) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.source[start..end])
    }

    /// Count non-blank, non-comment lines of a block body, matching RuboCop's
    /// `body.source.lines.count { !irrelevant_line?(line) }`.
    fn body_length(&self, body: Option<Node<'_>>) -> usize {
        let Some(body) = body else { return 0 };
        let loc = body.location();
        let text = self.source_text(loc.start_offset(), loc.end_offset());
        text.lines().filter(|line| !self.irrelevant(line)).count()
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
        let c = check_block_length(source.as_bytes(), max, count_comments);
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

    // Receiver source is captured for the Ruby-side receiver exclusion.
    #[test]
    fn receiver_captured() {
        let src = "Foo::Bar.baz do\n  a = 1\n  a = 2\n  a = 3\nend";
        let got = run(src, 2, false);
        assert_eq!(got.methods, vec!["baz"]);
        assert_eq!(got.receivers, vec!["Foo::Bar"]);
    }
}
