//! `Style/StringLiterals`.
//!
//! Enforces the configured quote style (`EnforcedStyle`: `single_quotes` /
//! `double_quotes`) for string literals, optionally extended to multi-line
//! continued strings via `ConsistentQuotesInMultiline`.
//!
//! Reconstructed over Prism, mirroring stock's two callbacks:
//!
//! - `StringHelp#on_str` (fires for every `:str` node that has a begin loc, is
//!   not part of an ignored node, and is not a regexp child). The offense test
//!   is `wrong_quotes?(source) && !inside_interpolation?`; a non-offending such
//!   node emits a `correct_style_detected` marker (so `config_to_allow_offenses`
//!   matches). Heredocs are excluded: in parser-gem a heredoc `:str` has no
//!   `loc.begin`, whereas Prism's `StringNode` *does* carry an opening loc
//!   (`<<…`), so the heredoc case is filtered explicitly.
//! - `StringLiterals#on_dstr` (only when `ConsistentQuotesInMultiline`): a
//!   `:dstr` whose children are all string literals is the multi-line continued
//!   form; if the children mix quote styles it is an "Inconsistent quote style."
//!   offense, otherwise `check_multiline_quote_style` may register a regular
//!   offense. Either way the `:dstr` is then `ignore_node`d, suppressing the
//!   children's `on_str`. The `:dstr` autocorrect is a no-op
//!   (`StringLiteralCorrector` returns for a `dstr` node), so these offenses
//!   carry no fix.
//!
//! Division of labour with the Ruby wrapper: Rust decides which nodes offend,
//! with which message and detection marker, and for an `on_str` offense which
//! fix to apply (`single` -> `to_string_literal`, `double` -> `inspect`) plus
//! the decoded string content. The wrapper computes the replacement text with
//! stock's genuine `RuboCop::Cop::Util` helpers (Ruby's `String#inspect` and the
//! `to_string_literal` escaping are Ruby string semantics) and applies it as a
//! single `replace` op; the detection markers are replayed through the genuine
//! `ConfigurableEnforcedStyle` methods. The two source-text regexes
//! (`double_quotes_required?` and the double-style `wrong_quotes?` pattern) are
//! ported to byte scanners below — they touch only ASCII delimiters, so the
//! port is exact.

use ruby_prism::{Location, Node};

/// Configured style.
pub const STYLE_SINGLE: u8 = 0;
pub const STYLE_DOUBLE: u8 = 1;

/// Which autocorrect to apply (computed by the Ruby wrapper from `content`).
pub const FIX_SINGLE: u8 = 0;
pub const FIX_DOUBLE: u8 = 1;
pub const FIX_NONE: u8 = 2;

/// Detection side effect replayed through the genuine stock methods.
pub const DETECT_OPPOSITE: u8 = 0;
pub const DETECT_CORRECT: u8 = 1;
pub const DETECT_NONE: u8 = 3;

/// Message selector.
pub const MSG_PREFER_SINGLE: u8 = 0;
pub const MSG_PREFER_DOUBLE: u8 = 1;
pub const MSG_INCONSISTENT: u8 = 2;

/// `Style/StringLiterals` configuration.
#[derive(Clone, Copy)]
pub struct Config {
    /// 0 single_quotes, 1 double_quotes.
    pub style: u8,
    /// `ConsistentQuotesInMultiline`.
    pub consistent_multiline: bool,
}

/// One record from the walk, in walk order. When `is_offense` is false it is a
/// pure `correct_style_detected` marker (no caret, no fix).
pub struct StringLiteralsOffense {
    pub is_offense: bool,
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: u8,
    pub detect: u8,
    /// For an `on_str` offense, the autocorrect kind; `FIX_NONE` for the
    /// `dstr` (multiline) offenses and for non-offense markers.
    pub fix: u8,
    /// Decoded (`unescaped`) string content, for `FIX_SINGLE` / `FIX_DOUBLE`.
    pub content: String,
}

pub fn check_string_literals(source: &[u8], cfg: &Config) -> Vec<StringLiteralsOffense> {
    let mut rule = build_rule(source, cfg);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

pub(crate) fn build_rule<'a>(source: &'a [u8], cfg: &Config) -> Visitor<'a> {
    Visitor {
        source,
        cfg: *cfg,
        stack: Vec::new(),
        offenses: Vec::new(),
    }
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

/// A frame for an enclosing branch node: tracks whether the current position is
/// inside an interpolation (`#{...}` / `#@x`), whether it is inside a `dstr`
/// that `on_dstr` ignored (consistent mode), and whether the frame itself is a
/// `dstr` / `dsym` / `regexp` (the only parser-gem node types stock's
/// `inside_interpolation?` recognises as carrying interpolation semantics).
struct Frame {
    /// This frame is an `EmbeddedStatementsNode` / `EmbeddedVariableNode`
    /// (interpolation marker): the `:begin` ancestor in stock's
    /// `inside_interpolation?` traversal.
    is_interp: bool,
    /// This frame is a `dstr` ignored by `on_dstr`: descendant `on_str` is
    /// suppressed (`part_of_ignored_node?`).
    is_ignored_dstr: bool,
    /// This frame corresponds to a parser-gem `:dstr` / `:dsym` / `:regexp`
    /// (Prism `InterpolatedStringNode` / `InterpolatedSymbolNode` /
    /// `InterpolatedRegularExpressionNode`, plus the heredoc forms). The
    /// physical-string `StringNode` / `SymbolNode` / `RegularExpressionNode`
    /// (no interpolation) are *not* dstr-equivalent. Crucially `xstr` /
    /// `InterpolatedXStringNode` are *not* on this list, so a `#{...}` inside
    /// backticks is **not** "inside interpolation" by stock's definition.
    is_dstr_dsym_or_regexp: bool,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    cfg: Config,
    stack: Vec<Frame>,
    pub offenses: Vec<StringLiteralsOffense>,
}

impl<'a> Visitor<'a> {
    /// Port of stock `StringHelp#inside_interpolation?`:
    ///
    /// ```ruby
    /// node.ancestors
    ///     .drop_while { |a| !a.begin_type? }
    ///     .any? { |a| a.type?(:dstr, :dsym, :regexp) }
    /// ```
    ///
    /// Ancestors are ordered inner→outer. We scan the frame stack from leaf
    /// (innermost) to root: find the closest interpolation marker
    /// (`Embedded*` = parser-gem `:begin`), then look strictly *outside* it for
    /// a `dstr` / `dsym` / `regexp` frame. If no `Embedded*` is on the stack at
    /// all, the result is `false` regardless of what else is enclosing — this
    /// is exactly the `drop_while` behaviour (empty list yields `any? = false`).
    ///
    /// The discourse parity bug: previously this just checked
    /// `any(|f| f.is_interp)`, which fired for *any* `Embedded*` ancestor
    /// including ones inside an `xstr` (backticks). Stock excludes that case
    /// because `xstr` is not in the recognised type list, so `xstr` interior
    /// strings are *not* "inside interpolation" and `Style/StringLiterals`
    /// claims them. We now require an outer dstr/dsym/regexp frame, matching
    /// stock.
    fn inside_interpolation(&self) -> bool {
        let mut iter = self.stack.iter().rev();
        // drop_while !begin_type?
        for f in iter.by_ref() {
            if f.is_interp {
                break;
            }
        }
        // any?(:dstr, :dsym, :regexp) over the strictly-outer remaining
        // ancestors (and over any further Embedded*'s, which are not in the
        // type list anyway — irrelevant either way).
        iter.any(|f| f.is_dstr_dsym_or_regexp)
    }

    fn inside_ignored_dstr(&self) -> bool {
        self.stack.iter().any(|f| f.is_ignored_dstr)
    }

    fn src(&self, a: usize, b: usize) -> &[u8] {
        &self.source[a..b]
    }

    fn decoded(&self, n: &ruby_prism::StringNode<'_>) -> String {
        String::from_utf8_lossy(n.unescaped()).into_owned()
    }

    /// Replicate `StringHelp#on_str` for a single `:str` node.
    fn on_str(&mut self, n: &ruby_prism::StringNode<'_>) {
        // `return unless node.loc?(:begin)`: parser-gem `:str` nodes without an
        // opening (interpolation / `%w` parts) are skipped. In Prism those have
        // no `opening_loc`.
        let Some(opening) = n.opening_loc() else {
            return;
        };
        // Heredoc: parser-gem heredoc `:str` has no `loc.begin`, but Prism's
        // opening is the `<<…` marker. Exclude it.
        if self.src(opening.start_offset(), opening.end_offset()).starts_with(b"<<") {
            return;
        }
        // `return if part_of_ignored_node?`: a child of a `dstr` ignored by
        // `on_dstr` (consistent mode).
        if self.inside_ignored_dstr() {
            return;
        }

        let (start, end) = loc(&n.location());
        let node_src = self.src(start, end);

        // Parser-gem parses a string literal that spans physical lines (a raw
        // newline inside the quotes, not a `\n` escape) as a `:dstr` with
        // `begin`-less `:str` children — never as a `:str`. Prism keeps it a
        // single `StringNode`. Route it through the `:dstr` path so it matches:
        // when `ConsistentQuotesInMultiline` is off it produces no offense (the
        // children have no begin loc, `on_dstr` returns early); when on it is
        // judged as the "quotes only on the outer node" multiline case.
        if node_src.contains(&b'\n') {
            if self.cfg.consistent_multiline {
                self.on_dstr_multiline_literal(n, start, end);
            }
            return;
        }

        // `offense?(node) = wrong_quotes?(node) && !inside_interpolation?(node)`.
        let offending = wrong_quotes(node_src, self.cfg.style) && !self.inside_interpolation();

        if offending {
            let (message, fix) = if self.cfg.style == STYLE_SINGLE {
                (MSG_PREFER_SINGLE, FIX_SINGLE)
            } else {
                (MSG_PREFER_DOUBLE, FIX_DOUBLE)
            };
            self.offenses.push(StringLiteralsOffense {
                is_offense: true,
                start_offset: start,
                end_offset: end,
                message,
                detect: DETECT_OPPOSITE,
                fix,
                content: self.decoded(n),
            });
        } else {
            // `correct_style_detected`.
            self.push_correct_marker();
        }
    }

    fn push_correct_marker(&mut self) {
        self.offenses.push(StringLiteralsOffense {
            is_offense: false,
            start_offset: 0,
            end_offset: 0,
            message: 0,
            detect: DETECT_CORRECT,
            fix: FIX_NONE,
            content: String::new(),
        });
    }

    /// The `on_dstr` path for a multi-line string literal that Prism keeps as a
    /// single `StringNode` (parser-gem makes it a `:dstr` whose children have no
    /// begin loc). This is exactly stock's "quote marks only on the parent"
    /// branch: `detect_quote_styles` returns `[node.loc.begin.source]`, and
    /// `check_multiline_quote_style` judges it against that single quote. The
    /// children are the per-line content fragments (begin-less), so
    /// `accept_child_double_quotes?` reduces to `double_quotes_required?` over
    /// the content split on newlines.
    fn on_dstr_multiline_literal(
        &mut self,
        n: &ruby_prism::StringNode<'_>,
        start: usize,
        end: usize,
    ) {
        let Some(opening) = n.opening_loc() else {
            return;
        };
        let quote = first_quote_byte(self.src(opening.start_offset(), opening.end_offset()));
        let content = {
            let cl = n.content_loc();
            self.src(cl.start_offset(), cl.end_offset()).to_vec()
        };
        let style = self.cfg.style;
        if quote == b'\'' && style == STYLE_DOUBLE {
            // `unexpected_single_quotes?`: every child has wrong quotes. For
            // begin-less fragments `wrong_quotes?` is `!double_quotes_required?`
            // (single fragment can't start with `%`/`?`). Register if every line
            // fragment lacks a double-quote requirement.
            let all_wrong = content
                .split(|&b| b == b'\n')
                .all(|frag| wrong_quotes(frag, style));
            if all_wrong {
                self.register_multiline(start, end);
            }
        } else if quote == b'"' && style == STYLE_SINGLE {
            // `accept_child_double_quotes?`: any line fragment
            // `double_quotes_required?`.
            let accept = content.split(|&b| b == b'\n').any(double_quotes_required);
            if !accept {
                self.register_multiline(start, end);
            }
        }
    }

    /// Replicate `StringLiterals#on_dstr` for a `:dstr` (Prism
    /// `InterpolatedStringNode`). Returns `true` if the node was ignored (so the
    /// caller marks the frame).
    ///
    /// Only called in consistent mode (the early `return unless
    /// consistent_multiline?` is handled by the caller).
    fn on_dstr(&mut self, node: &ruby_prism::InterpolatedStringNode<'_>) -> bool {
        // `return if node.heredoc?`
        if let Some(opening) = node.opening_loc()
            && self
                .src(opening.start_offset(), opening.end_offset())
                .starts_with(b"<<")
        {
            return false;
        }

        let parts: Vec<Node> = node.parts().iter().collect();
        // `return unless all_string_literals?(children)`: every part is a
        // `:str` (StringNode) or `:dstr` (InterpolatedStringNode).
        if !parts.iter().all(|p| {
            matches!(p, Node::StringNode { .. } | Node::InterpolatedStringNode { .. })
        }) {
            return false;
        }

        // `detect_quote_styles`: the opening quote of each child; if all are
        // nil, fall back to the parent's opening.
        let mut quotes: Vec<u8> = Vec::new();
        let mut all_nil = true;
        for p in &parts {
            let opening = match p {
                Node::StringNode { .. } => p.as_string_node().unwrap().opening_loc(),
                Node::InterpolatedStringNode { .. } => {
                    p.as_interpolated_string_node().unwrap().opening_loc()
                }
                _ => None,
            };
            if let Some(o) = opening {
                all_nil = false;
                let s = self.src(o.start_offset(), o.end_offset());
                quotes.push(first_quote_byte(s));
            }
        }
        if all_nil {
            // Only the parent carries the quote marks.
            if let Some(o) = node.opening_loc() {
                let s = self.src(o.start_offset(), o.end_offset());
                quotes = vec![first_quote_byte(s)];
            } else {
                quotes = vec![];
            }
        } else {
            // `.uniq`
            quotes.sort_unstable();
            quotes.dedup();
        }

        let (start, end) = loc(&node.location());

        if quotes.len() > 1 {
            // Inconsistent.
            self.offenses.push(StringLiteralsOffense {
                is_offense: true,
                start_offset: start,
                end_offset: end,
                message: MSG_INCONSISTENT,
                detect: DETECT_NONE,
                fix: FIX_NONE,
                content: String::new(),
            });
        } else if let Some(&quote) = quotes.first() {
            self.check_multiline_quote_style(node, &parts, quote, start, end);
        }
        // `ignore_node(node)`
        true
    }

    /// `check_multiline_quote_style`.
    fn check_multiline_quote_style(
        &mut self,
        _node: &ruby_prism::InterpolatedStringNode<'_>,
        parts: &[Node],
        quote: u8,
        start: usize,
        end: usize,
    ) {
        let style = self.cfg.style;
        if quote == b'\'' && style == STYLE_DOUBLE {
            // `unexpected_single_quotes?`: register if every child has wrong
            // quotes.
            let all_wrong = parts.iter().all(|c| self.child_wrong_quotes(c));
            if all_wrong {
                self.register_multiline(start, end);
            }
        } else if quote == b'"' && style == STYLE_SINGLE && !self.accept_child_double_quotes(parts) {
            // `unexpected_double_quotes?` and not accepted.
            self.register_multiline(start, end);
        }
    }

    fn register_multiline(&mut self, start: usize, end: usize) {
        let message = if self.cfg.style == STYLE_SINGLE {
            MSG_PREFER_SINGLE
        } else {
            MSG_PREFER_DOUBLE
        };
        self.offenses.push(StringLiteralsOffense {
            is_offense: true,
            start_offset: start,
            end_offset: end,
            message,
            detect: DETECT_NONE,
            fix: FIX_NONE,
            content: String::new(),
        });
    }

    /// `wrong_quotes?(child)` over a child node's source.
    fn child_wrong_quotes(&self, c: &Node) -> bool {
        let (s, e) = loc(&c.location());
        wrong_quotes(self.src(s, e), self.cfg.style)
    }

    /// `accept_child_double_quotes?`: any child is a `:dstr` (interpolation) or
    /// its source `double_quotes_required?`.
    fn accept_child_double_quotes(&self, parts: &[Node]) -> bool {
        parts.iter().any(|c| {
            if matches!(c, Node::InterpolatedStringNode { .. }) {
                return true;
            }
            let (s, e) = loc(&c.location());
            double_quotes_required(self.src(s, e))
        })
    }
}

/// The first quote-like byte (`'`, `"`, or the byte after `%` for percent
/// literals) of an opening location's source. Multiline children only ever use
/// `'` or `"`, which is all stock's `detect_quote_styles` compares.
fn first_quote_byte(opening: &[u8]) -> u8 {
    opening.first().copied().unwrap_or(b'"')
}

/// Port of `RuboCop::Cop::Util#double_quotes_required?`:
/// `/'|(?<! \\) \\{2}* \\ (?![\\"])/x`.
///
/// True if the string source contains a `'`, or a maximal run of backslashes of
/// odd length whose following byte is not `"` (the run end is, by definition,
/// not a backslash, so the `(?![\\"])` reduces to "not followed by a quote").
pub fn double_quotes_required(src: &[u8]) -> bool {
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'\'' => return true,
            b'\\' => {
                let run_start = i;
                while i < src.len() && src[i] == b'\\' {
                    i += 1;
                }
                let run_len = i - run_start;
                let next = src.get(i).copied();
                if run_len % 2 == 1 && next != Some(b'"') {
                    return true;
                }
            }
            _ => i += 1,
        }
    }
    false
}

/// `wrong_quotes?(src)` for the configured style. `src` is the node's source,
/// quotes included.
///
/// - `return false if src.start_with?('%', '?')`.
/// - single_quotes: `!double_quotes_required?(src)`.
/// - double_quotes: `!/" | \\[^'\\] | \#[@{$]/x.match?(src)`.
pub fn wrong_quotes(src: &[u8], style: u8) -> bool {
    if matches!(src.first(), Some(b'%') | Some(b'?')) {
        return false;
    }
    if style == STYLE_SINGLE {
        !double_quotes_required(src)
    } else {
        !double_style_keep(src)
    }
}

/// Port of the double-style `wrong_quotes?` pattern `/" | \\[^'\\] | \#[@{$]/x`:
/// true if the source contains a `"`, or a `\` followed by a byte other than
/// `'` or `\`, or a `#` followed by `@` / `{` / `$`. (A `true` here means the
/// string is *already* acceptable for double-quotes style, i.e. NOT wrong.)
fn double_style_keep(src: &[u8]) -> bool {
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'"' => return true,
            b'\\' => {
                if let Some(&next) = src.get(i + 1)
                    && next != b'\''
                    && next != b'\\'
                {
                    return true;
                }
            }
            b'#' => {
                if matches!(src.get(i + 1).copied(), Some(b'@') | Some(b'{') | Some(b'$')) {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        let mut is_interp = false;
        let mut is_ignored_dstr = false;
        // `:dstr` / `:dsym` / `:regexp` cover both the with-interpolation Prism
        // forms (`Interpolated*Node`) and the heredoc / multi-line forms of
        // `:dstr` (a Prism `InterpolatedStringNode` is also a `:dstr` to
        // parser-gem). String / Symbol / RegularExpression without interpolation
        // are *not* in stock's recognised list (`:str` / `:sym` / `:regexp` —
        // wait, `:regexp` matches both! A regexp without interpolation is still
        // a `:regexp` to parser-gem). So a `RegularExpressionNode` (no interp)
        // also counts. Similarly a heredoc string with no interpolation is a
        // `:dstr` to parser-gem — Prism keeps it an `InterpolatedStringNode`
        // when the heredoc has any escapes or always — but a single-line
        // non-interp string is `:str`, not `:dstr`.
        let is_dstr_dsym_or_regexp = matches!(
            node,
            Node::InterpolatedStringNode { .. }
                | Node::InterpolatedSymbolNode { .. }
                | Node::InterpolatedRegularExpressionNode { .. }
                | Node::RegularExpressionNode { .. }
        );
        match node {
            Node::EmbeddedStatementsNode { .. } | Node::EmbeddedVariableNode { .. } => {
                is_interp = true;
            }
            Node::InterpolatedStringNode { .. }
                if self.cfg.consistent_multiline && !self.inside_ignored_dstr() =>
            {
                let n = node.as_interpolated_string_node().unwrap();
                is_ignored_dstr = self.on_dstr(&n);
            }
            _ => {}
        }
        self.stack.push(Frame {
            is_interp,
            is_ignored_dstr,
            is_dstr_dsym_or_regexp,
        });
    }

    fn leave(&mut self) {
        self.stack.pop();
    }

    fn enter_leaf(&mut self, node: &Node<'_>) {
        if let Node::StringNode { .. } = node {
            let n = node.as_string_node().unwrap();
            self.on_str(&n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `double_quotes_required?` matches stock's
    /// `/'|(?<! \\) \\{2}* \\ (?![\\"])/x` on the node source (quotes included).
    /// Verified exhaustively against the Ruby implementation over all byte
    /// strings of length 1..=3 over a delimiter-heavy alphabet (`'`, `"`, `\`,
    /// `#`, `@`, `{`, `$`, `&`, `;`, `a`); these are the load-bearing branches.
    #[test]
    fn double_quotes_required_matches_stock() {
        // Contains a `'` -> true.
        assert!(double_quotes_required(b"'it''s'"));
        // Odd backslash run not followed by `"` -> true.
        assert!(double_quotes_required(b"'a\\b'")); // single `\`
        assert!(double_quotes_required(b"'a\\\\\\b'")); // run of 3
        // Even backslash run (`\\`) -> false.
        assert!(!double_quotes_required(b"\"a\\\\b\""));
        // Backslash run before a `"` -> false (the `(?![\\\"])` guard).
        assert!(!double_quotes_required(b"\"a\\\"b\""));
        // No `'` and no escape -> false.
        assert!(!double_quotes_required(b"\"plain\""));
    }

    /// The double-style `wrong_quotes?` pattern `/" | \\[^'\\] | \#[@{$]/x`. A
    /// `true` from `double_style_keep` means the literal is already acceptable
    /// for double quotes (NOT wrong).
    #[test]
    fn double_style_keep_matches_stock() {
        // Contains a `"`.
        assert!(double_style_keep(b"\"a\""));
        // `\` followed by a non-quote / non-backslash byte: `'\\&'` -> the second
        // backslash is followed by `&`. (Regression: a paired-skip scan missed
        // this and over-reported the literal as wrong.)
        assert!(double_style_keep(b"'\\\\&'"));
        // `#` followed by `{` / `@` / `$` (interpolation-ish).
        assert!(double_style_keep(b"'a#{b'"));
        assert!(double_style_keep(b"'a#@b'"));
        // A plain single-quoted literal -> not acceptable for double (wrong).
        assert!(!double_style_keep(b"'plain'"));
        // A backslash run that only ever abuts a quote (`\\` then the closing
        // `'`) -> the `[^'\\]` class excludes both, so no match -> not
        // acceptable for double.
        assert!(!double_style_keep(b"'a\\\\'"));
    }
}
