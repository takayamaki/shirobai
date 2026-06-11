//! `Style/LineEndConcatenation`.
//!
//! RuboCop's cop is token-based: it scans adjacent token triples
//! `(predecessor, operator, successor)` where the operator is `+`/`<<`, the
//! predecessor and successor are standard (`'`/`"`) string literals, and the
//! operator and successor sit on different lines. We reconstruct the same
//! decision from the Prism AST.
//!
//! A `+`/`<<` concatenation is a `CallNode` with one argument and no `.`
//! operator. The token-level predecessor is the *last token of the receiver*;
//! for a left-associative chain (`a + b + c`) that is the trailing string
//! operand of the receiver subtree, so we follow the receiver's argument down.
//! The token-level successor is the *first token of the argument*, which equals
//! the argument node when it is directly a string literal. RuboCop's
//! "high-precedence op after the successor" rejection falls out naturally: if
//! the argument is `'a'.reverse` or `'a' * 3`, the argument node is a call, not
//! a string, so it is rejected.

use ruby_prism::{Node, Visit};

/// A multiline string concatenation (`"a" +` / `"a" <<` at line end followed by
/// a string literal on the next line) that should use `\` continuation.
pub struct LineEndConcatOffense {
    /// Offense range: the operator token (`+` or `<<`).
    pub start_offset: usize,
    pub end_offset: usize,
    /// Operator text (`+` or `<<`), used by Ruby to format the message.
    pub operator: String,
    /// Autocorrect replacement range (operator plus trailing same-line
    /// whitespace, extended by one byte when followed by a backslash).
    pub replace_start: usize,
    pub replace_end: usize,
}

pub fn check_line_end_concatenation(source: &[u8]) -> Vec<LineEndConcatOffense> {
    let mut visitor = build_rule(source);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    visitor.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule(source: &[u8]) -> Visitor<'_> {
    Visitor {
        source,
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    pub(crate) offenses: Vec<LineEndConcatOffense>,
}

fn is_quote(source: &[u8], offset: usize) -> bool {
    matches!(source.get(offset), Some(b'\'') | Some(b'"'))
}

/// Whether the *leading* token of `node` is a standard (`'`/`"`-delimited)
/// string literal token. Mirrors RuboCop's `eligible_successor?`: the successor
/// is the first token of the argument. Heredocs, `%()` literals and `__FILE__`
/// have a non-quote opening and are excluded; an implicit string concatenation
/// (`"a" "b"`) has no opening, so its first part decides.
fn leading_token_is_standard_string(node: &Node<'_>, source: &[u8]) -> bool {
    if let Some(s) = node.as_string_node() {
        return s
            .opening_loc()
            .is_some_and(|l| is_quote(source, l.start_offset()));
    }
    if let Some(s) = node.as_interpolated_string_node() {
        if let Some(l) = s.opening_loc() {
            return is_quote(source, l.start_offset());
        }
        return s
            .parts()
            .iter()
            .next()
            .is_some_and(|p| leading_token_is_standard_string(&p, source));
    }
    // A `+`/`<<` concatenation binds looser than the high-precedence ops
    // (`* % . []`) that RuboCop's `next_successor` rejects, so its leftmost
    // operand (the receiver) is the next-line successor token. Anything else
    // (a `*`/`.`/`[]` call, `__FILE__`, etc.) is not a string token.
    if let Some(call) = node.as_call_node()
        && is_concat_operator(call.name().as_slice())
        && call.call_operator_loc().is_none()
        && let Some(receiver) = call.receiver()
    {
        return leading_token_is_standard_string(&receiver, source);
    }
    false
}

/// Whether the *trailing* token of `node` is a standard string literal token.
/// Mirrors RuboCop's `eligible_predecessor?`: the predecessor is the last token
/// before the operator. For a `+`/`<<` chain it is the trailing string operand
/// of the right-hand argument; for an implicit concatenation it is the last
/// part's trailing token.
fn trailing_token_is_standard_string(node: &Node<'_>, source: &[u8]) -> bool {
    if let Some(s) = node.as_string_node() {
        return s
            .opening_loc()
            .is_some_and(|l| is_quote(source, l.start_offset()));
    }
    if let Some(s) = node.as_interpolated_string_node() {
        if let Some(l) = s.opening_loc() {
            return is_quote(source, l.start_offset());
        }
        return s
            .parts()
            .iter()
            .last()
            .is_some_and(|p| trailing_token_is_standard_string(&p, source));
    }
    if let Some(call) = node.as_call_node()
        && is_concat_operator(call.name().as_slice())
        && call.call_operator_loc().is_none()
        && let Some(args) = call.arguments()
        && args.arguments().iter().count() == 1
        && let Some(arg) = args.arguments().iter().next()
    {
        return trailing_token_is_standard_string(&arg, source);
    }
    false
}

fn is_concat_operator(name: &[u8]) -> bool {
    name == b"+" || name == b"<<"
}

impl<'a> Visitor<'a> {
    fn line_of(&self, offset: usize) -> usize {
        self.source[..offset]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
    }

    fn process_call(&mut self, node: &Node<'_>) {
        let Some(call) = node.as_call_node() else {
            return;
        };
        if !is_concat_operator(call.name().as_slice()) || call.call_operator_loc().is_some() {
            return;
        }
        let Some(receiver) = call.receiver() else {
            return;
        };
        let Some(args) = call.arguments() else {
            return;
        };
        if args.arguments().iter().count() != 1 {
            return;
        }
        let arg = args.arguments().iter().next().unwrap();

        // eligible_predecessor? / eligible_successor?
        if !trailing_token_is_standard_string(&receiver, self.source)
            || !leading_token_is_standard_string(&arg, self.source)
        {
            return;
        }

        let Some(op_loc) = call.message_loc() else {
            return;
        };
        let op_start = op_loc.start_offset();
        let op_end = op_loc.end_offset();
        let arg_start = arg.location().start_offset();

        // same_line?(operator, successor): the operator and the successor string
        // must be on different lines.
        if self.line_of(op_start) == self.line_of(arg_start) {
            return;
        }

        // The token-level successor must be adjacent to the operator (RuboCop
        // reads tokens[index + 2]); a comment between them makes the successor a
        // comment token and disqualifies the triple.
        if self.has_comment_between(op_end, arg_start) {
            return;
        }

        let operator = std::str::from_utf8(&self.source[op_start..op_end])
            .unwrap_or("")
            .to_string();
        let (replace_start, replace_end) = self.autocorrect_range(op_start, op_end);

        self.offenses.push(LineEndConcatOffense {
            start_offset: op_start,
            end_offset: op_end,
            operator,
            replace_start,
            replace_end,
        });
    }

    /// Whether a `#` comment starts between `from` and `to` (exclusive of any
    /// string content, which cannot occur in operator→argument gaps here).
    fn has_comment_between(&self, from: usize, to: usize) -> bool {
        self.source[from..to].contains(&b'#')
    }

    /// Mirrors RuboCop's `autocorrect`: extend the operator range over trailing
    /// same-line whitespace, then absorb a single following backslash so the
    /// replacement does not produce a double backslash.
    fn autocorrect_range(&self, op_start: usize, op_end: usize) -> (usize, usize) {
        let mut end = op_end;
        while matches!(self.source.get(end), Some(b' ') | Some(b'\t')) {
            end += 1;
        }
        if self.source.get(end) == Some(&b'\\') {
            end += 1;
        }
        (op_start, end)
    }
}

impl<'pr> Visit<'pr> for Visitor<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.process_call(&node.as_node());
        ruby_prism::visit_call_node(self, node);
    }
}

/// Shared-walk driver. The generic branch hook fires for every `CallNode` the
/// typed `visit_call_node` sees except the one reached through
/// `MatchWriteNode`'s concretely-typed `call` field — an `=~` operator call,
/// whose name is never `+`/`<<`, so `process_call` rejects it anyway.
impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if node.as_call_node().is_some() {
            self.process_call(node);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<(usize, usize, String, usize, usize)> {
        check_line_end_concatenation(source.as_bytes())
            .into_iter()
            .map(|o| {
                (
                    o.start_offset,
                    o.end_offset,
                    o.operator,
                    o.replace_start,
                    o.replace_end,
                )
            })
            .collect()
    }

    #[test]
    fn basic_plus() {
        let src = "top = \"test\" +\n\"top\"\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(&src[got[0].0..got[0].1], "+");
        assert_eq!(got[0].2, "+");
    }

    #[test]
    fn basic_shift() {
        let src = "top = \"test\" <<\n\"top\"\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(&src[got[0].0..got[0].1], "<<");
        assert_eq!(got[0].2, "<<");
    }

    #[test]
    fn accepts_same_line() {
        assert!(run("top = \"test\" + \"top\"\n").is_empty());
    }

    #[test]
    fn accepts_method_on_string() {
        assert!(run("a = 'a ' +\n'gniht'.reverse\n").is_empty());
    }

    #[test]
    fn accepts_percent_literal() {
        assert!(run("top = %(test) +\n\"top\"\n").is_empty());
    }

    #[test]
    fn accepts_file_const() {
        assert!(run("top = __FILE__ +\n\"top\"\n").is_empty());
    }

    #[test]
    fn accepts_comment_after_operator() {
        assert!(run("top = \"test\" + # something\n\"top\"\n").is_empty());
    }

    #[test]
    fn chained_only_last_offense() {
        // "foo" + %(bar) + "baz" + "qux": only the "baz" + "qux" triple offends.
        let src = "top = \"foo\" +\n%(bar) +\n\"baz\" +\n\"qux\"\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn chained_two_offenses() {
        let src = "top = \"a\" +\n\"b\" +\n\"c\"\n";
        assert_eq!(run(src).len(), 2);
    }

    #[test]
    fn autocorrect_absorbs_backslash() {
        let src = "top = \"test\" + \\\n\"top\"\n";
        let got = run(src);
        assert_eq!(got.len(), 1);
        // replacement range covers "+ \" so it collapses to a single backslash.
        assert_eq!(&src[got[0].3..got[0].4], "+ \\");
    }
}
