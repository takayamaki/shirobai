//! `Style/StabbyLambdaParentheses`.
//!
//! Stock's logic (`vendor/rubocop/lib/rubocop/cop/style/stabby_lambda_parentheses.rb`):
//!
//! - `on_send` filters to `node.lambda_literal? && node.block_node.arguments?`:
//!   a parser-gem `(block (send nil :lambda) ...)` whose `send` is exactly the
//!   `->` literal and whose `arguments` `(args ...)` has at least one child.
//! - For each, compares the configured `EnforcedStyle` (`require_parentheses`
//!   default vs `require_no_parentheses`) against `args.loc.begin` truthiness:
//!     - `require_parentheses` + no `(` → offense; autocorrect `corrector.wrap(args, '(', ')')`
//!     - `require_no_parentheses` + has `(` → offense; autocorrect replaces
//!       `args.loc.begin` with `''` (the `(`) and removes `args.loc.end` (the `)`).
//! - The offense highlight is the `arguments` source range (`args.loc.expression`),
//!   not the lambda or the surrounding `()`.
//!
//! Reproduced here on prism's separate node type: every `LambdaNode` whose
//! `parameters` is a `BlockParametersNode` with a non-nil inner `parameters`
//! mirrors stock's `block_node.arguments?` true. The `BlockParametersNode`'s
//! `opening_loc` (truthy `(` location) mirrors stock's `args.loc.begin` — we
//! flag and emit autocorrect anchors accordingly.
//!
//! Edge cases (probed against stock):
//! - `-> {}` — `parameters` is nil (no args) → no offense, regardless of style.
//! - `-> () {}` — `parameters` is `BlockParametersNode` with inner nil → stock's
//!   `arguments?` is `false` (an empty `(args)`), so stock does NOT flag it
//!   under `require_no_parentheses` either.
//! - `lambda { |a| a }` — prism `CallNode` with a `BlockNode` block, not a
//!   `LambdaNode`. Stock's `lambda_literal?` only matches the stabby `->` form,
//!   so this is never flagged. Falls through naturally here (we only `enter`
//!   on `LambdaNode`).
//! - `o.lambda` / `lambda(&:nil?)` — bare `CallNode`s with no block, never
//!   matched.

use ruby_prism::{BlockParametersNode, LambdaNode, Node, Visit};

/// `EnforcedStyle`: 0 = `require_parentheses` (default), 1 = `require_no_parentheses`.
pub const STYLE_REQUIRE_PARENTHESES: u8 = 0;
pub const STYLE_REQUIRE_NO_PARENTHESES: u8 = 1;

#[derive(Clone, Copy)]
pub struct Config {
    pub style: u8,
}

/// One offense. Stock highlights `args.loc.expression` (the `(...)` source or
/// the bare `a,b,c` source when missing). Autocorrect anchors depend on the
/// style:
///
/// - `require_parentheses` (missing `(`): the wrapper calls
///   `corrector.wrap(args_range, '(', ')')` — using `wrap_start`/`wrap_end`
///   here, which equal `start`/`end`.
/// - `require_no_parentheses` (unwanted `(`): `paren_open_start`/`paren_open_end`
///   is the `(` 1-byte range (replaced with `''`), `paren_close_start`/`paren_close_end`
///   is the `)` 1-byte range (removed).
#[derive(Debug)]
pub struct StabbyLambdaParenthesesOffense {
    /// `args.loc.expression` start (offense highlight).
    pub start: usize,
    /// `args.loc.expression` end.
    pub end: usize,
    /// `args.loc.begin` (`(`) start/end — only set under `require_no_parentheses`.
    pub paren_open_start: usize,
    pub paren_open_end: usize,
    /// `args.loc.end` (`)`) start/end — only set under `require_no_parentheses`.
    pub paren_close_start: usize,
    pub paren_close_end: usize,
    /// Stock `MSG`: `Wrap stabby lambda arguments with parentheses.` or
    /// `Do not wrap stabby lambda arguments with parentheses.`.
    pub message: &'static str,
}

const MSG_REQUIRE: &str = "Wrap stabby lambda arguments with parentheses.";
const MSG_NO_REQUIRE: &str = "Do not wrap stabby lambda arguments with parentheses.";

pub fn check_stabby_lambda_parentheses(
    source: &[u8],
    config: Config,
) -> Vec<StabbyLambdaParenthesesOffense> {
    let mut rule = build_rule(source, config);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

pub(crate) fn build_rule(source: &[u8], config: Config) -> Visitor<'_> {
    Visitor {
        source,
        config,
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    #[allow(dead_code)]
    source: &'a [u8],
    config: Config,
    pub(crate) offenses: Vec<StabbyLambdaParenthesesOffense>,
}

impl Visitor<'_> {
    fn handle_lambda(&mut self, node: &LambdaNode<'_>) {
        // `block_node.arguments?` in stock: there is an `(args ...)` with at
        // least one child. In prism this is `LambdaNode.parameters` being a
        // `BlockParametersNode` whose inner `parameters` is non-nil. An empty
        // `(args)` (`-> () {}`) maps to `inner parameters` = nil and stock
        // reports `arguments?` false — no offense in either style.
        let Some(params_node) = node.parameters() else {
            return;
        };
        let Some(bp) = params_node.as_block_parameters_node() else {
            return;
        };
        if bp.parameters().is_none() {
            return;
        }
        let has_parens = bp.opening_loc().is_some();
        let want_parens = self.config.style == STYLE_REQUIRE_PARENTHESES;
        if has_parens == want_parens {
            return;
        }
        if !want_parens && !self.parens_removable(&bp) {
            return;
        }
        self.emit(&bp, want_parens);
    }

    fn emit(&mut self, bp: &BlockParametersNode<'_>, want_parens: bool) {
        // Offense highlight = `args.loc.expression` = `BlockParametersNode.location`.
        let loc = bp.location();
        let start = loc.start_offset();
        let end = loc.end_offset();
        let (paren_open_start, paren_open_end, paren_close_start, paren_close_end) =
            if want_parens {
                // Missing `(`: wrap entire highlight. The wrapper uses
                // `(start, end)` to wrap; paren_* fields are unused but set to
                // zero for stable defaults.
                (0, 0, 0, 0)
            } else {
                // Unwanted `(`: report `(` and `)` 1-byte locations.
                // 1.92: `(` through the leading padding, and the trailing
                // padding through `)`.
                let open = bp.opening_loc().expect("has_parens => opening_loc set");
                let close = bp.closing_loc().expect("has_parens => closing_loc set");
                let (first, last) = arg_bounds(bp);
                (open.start_offset(), first, last, close.end_offset())
            };
        let message = if want_parens { MSG_REQUIRE } else { MSG_NO_REQUIRE };
        self.offenses.push(StabbyLambdaParenthesesOffense {
            start,
            end,
            paren_open_start,
            paren_open_end,
            paren_close_start,
            paren_close_end,
            message,
        });
    }
}

impl Visitor<'_> {
    /// 1.92 `redundant_parentheses?` extras: the parentheses stay when an
    /// argument needs them (`arguments_require_parentheses?`: a block or a
    /// braced hash anywhere inside) or when the padding between them and the
    /// arguments holds anything but whitespace (a comment).
    fn parens_removable(&self, bp: &BlockParametersNode<'_>) -> bool {
        let mut scan = NeedsParens { found: false };
        if let Some(p) = bp.parameters() {
            scan.visit(&p.as_node());
        }
        if scan.found {
            return false;
        }
        let (first, last) = arg_bounds(bp);
        let open_end = bp.opening_loc().map_or(first, |l| l.end_offset());
        let close_start = bp.closing_loc().map_or(last, |l| l.start_offset());
        let blank = |s: &[u8]| s.iter().all(|&b| matches!(b, b' ' | b'\t' | b'\n' | b'\x0B' | b'\x0C' | b'\r' | 0));
        blank(&self.source[open_end..first]) && blank(&self.source[last..close_start])
    }
}

/// Start of the first and end of the last parser `args` child (parameters
/// and `;` block-local variables).
fn arg_bounds(bp: &BlockParametersNode<'_>) -> (usize, usize) {
    let mut first = usize::MAX;
    let mut last = 0;
    let mut see = |n: &Node<'_>| {
        let l = n.location();
        first = first.min(l.start_offset());
        last = last.max(l.end_offset());
    };
    if let Some(p) = bp.parameters() {
        p.requireds().iter().for_each(|n| see(&n));
        p.optionals().iter().for_each(|n| see(&n));
        if let Some(n) = p.rest() {
            see(&n);
        }
        p.posts().iter().for_each(|n| see(&n));
        p.keywords().iter().for_each(|n| see(&n));
        if let Some(n) = p.keyword_rest() {
            see(&n);
        }
        if let Some(n) = p.block() {
            see(&n.as_node());
        }
    }
    bp.locals().iter().for_each(|n| see(&n));
    (first, last)
}

/// `arguments.each_descendant(:any_block, :hash).any? { !hash || braces? }`.
struct NeedsParens {
    found: bool,
}

impl<'pr> Visit<'pr> for NeedsParens {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        let block = |b: Option<Node<'_>>| b.is_some_and(|b| b.as_block_node().is_some());
        if node.as_hash_node().is_some()
            || node.as_lambda_node().is_some()
            || node.as_call_node().is_some_and(|c| block(c.block()))
            || node.as_super_node().is_some_and(|s| block(s.block()))
            || node.as_forwarding_super_node().is_some_and(|s| s.block().is_some())
        {
            self.found = true;
        }
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_LAMBDA,
        )
    }
    
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(n) = node.as_lambda_node() {
            self.handle_lambda(&n);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, style: u8) -> Vec<(usize, usize, &'static str)> {
        check_stabby_lambda_parentheses(source.as_bytes(), Config { style })
            .into_iter()
            .map(|o| (o.start, o.end, o.message))
            .collect()
    }

    // --- typical: most representative cases first ---

    #[test]
    fn require_parens_missing_parens_offense() {
        // Most typical bad: `->a,b,c { ... }` under default style.
        let r = run("->a,b,c { a + b + c }\n", STYLE_REQUIRE_PARENTHESES);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].2, MSG_REQUIRE);
        // Highlight = `a,b,c` at offsets [2, 7).
        assert_eq!(&r[0].0..&r[0].1, &2..&7);
    }

    #[test]
    fn require_parens_with_parens_clean() {
        // Most typical good under default style.
        assert!(run("->(a,b,c) { a + b + c }\n", STYLE_REQUIRE_PARENTHESES).is_empty());
    }

    #[test]
    fn require_no_parens_with_parens_offense() {
        // Most typical bad under the opposite style: `->(a,b,c) { ... }`.
        let r = run("->(a,b,c) { a + b + c }\n", STYLE_REQUIRE_NO_PARENTHESES);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].2, MSG_NO_REQUIRE);
        // Highlight = `(a,b,c)` at offsets [2, 9).
        assert_eq!(&r[0].0..&r[0].1, &2..&9);
    }

    #[test]
    fn require_no_parens_without_parens_clean() {
        assert!(run("->a,b,c { a + b + c }\n", STYLE_REQUIRE_NO_PARENTHESES).is_empty());
    }

    // --- common no-arg cases (shared between both styles) ---

    #[test]
    fn no_args_no_offense_under_either_style() {
        // `-> { ... }` — no `args` at all, neither style flags it.
        assert!(run("-> { true }\n", STYLE_REQUIRE_PARENTHESES).is_empty());
        assert!(run("-> { true }\n", STYLE_REQUIRE_NO_PARENTHESES).is_empty());
    }

    #[test]
    fn empty_parens_no_args_no_offense() {
        // `-> () { ... }` — stock's `arguments?` is false (empty args),
        // neither style flags it.
        assert!(run("-> () { true }\n", STYLE_REQUIRE_PARENTHESES).is_empty());
        assert!(run("-> () { true }\n", STYLE_REQUIRE_NO_PARENTHESES).is_empty());
    }

    // --- not-a-stabby-lambda cases (must not match) ---

    #[test]
    fn lambda_method_call_with_block_no_offense() {
        // `lambda { |a| a }` is a CallNode + BlockNode, not a LambdaNode.
        assert!(run("lambda { |a| a }\n", STYLE_REQUIRE_PARENTHESES).is_empty());
        assert!(run("lambda { |a| a }\n", STYLE_REQUIRE_NO_PARENTHESES).is_empty());
    }

    #[test]
    fn lambda_method_call_with_block_pass_no_offense() {
        // `lambda(&:nil?)` is a bare CallNode, no block.
        assert!(run("lambda(&:nil?)\n", STYLE_REQUIRE_PARENTHESES).is_empty());
    }

    #[test]
    fn receiver_lambda_method_no_offense() {
        // `o.lambda` is a CallNode, no block.
        assert!(run("o.lambda\n", STYLE_REQUIRE_PARENTHESES).is_empty());
    }

    // --- argument shape variety (each should be detected as offense under the wrong style) ---

    #[test]
    fn single_arg_missing_parens() {
        let r = run("->a { a }\n", STYLE_REQUIRE_PARENTHESES);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn single_arg_missing_parens_leading_space() {
        // `-> a { a }` — same shape, leading space is not part of the args range.
        let r = run("-> a { a }\n", STYLE_REQUIRE_PARENTHESES);
        assert_eq!(r.len(), 1);
        // Args expression = `a` at offsets [3, 4).
        assert_eq!(&r[0].0..&r[0].1, &3..&4);
    }

    #[test]
    fn keyword_args_with_parens_under_no_parens() {
        // `->(a:, b:) { a }` — kwargs always require parens syntactically; under
        // `require_no_parentheses` stock STILL flags it (the user wrote `()`
        // around kwargs, which the rule says don't do). Autocorrect would
        // produce a syntax error, but that is stock's behavior. Pin it.
        let r = run("->(a:, b:) { a }\n", STYLE_REQUIRE_NO_PARENTHESES);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn splat_arg_with_parens_no_parens_style() {
        let r = run("->(*a) { a }\n", STYLE_REQUIRE_NO_PARENTHESES);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn block_arg_with_parens_no_parens_style() {
        // `&blk` — like kwargs, requires `()` syntactically. Stock still flags.
        let r = run("->(&blk) { blk }\n", STYLE_REQUIRE_NO_PARENTHESES);
        assert_eq!(r.len(), 1);
    }

    // --- offense paren ranges (anchors used by autocorrect) ---

    #[test]
    fn no_parens_offense_carries_paren_ranges() {
        // Anchors used by `corrector.replace(args.loc.begin, '')` and
        // `corrector.remove(args.loc.end)`.
        let offs = check_stabby_lambda_parentheses(
            b"->(a,b,c) { a + b + c }\n",
            Config { style: STYLE_REQUIRE_NO_PARENTHESES },
        );
        assert_eq!(offs.len(), 1);
        let o = &offs[0];
        assert_eq!((o.paren_open_start, o.paren_open_end), (2, 3));
        assert_eq!((o.paren_close_start, o.paren_close_end), (8, 9));
    }

    // --- nesting / multiple ---

    #[test]
    fn nested_lambdas_each_checked_independently() {
        // Outer good, inner bad (mixed in one source).
        let r = run(
            "->(x) { ->y { x + y } }\n",
            STYLE_REQUIRE_PARENTHESES,
        );
        assert_eq!(r.len(), 1);
        // Inner offense covers `y`.
        let (s, e, _) = r[0];
        assert_eq!(&"->(x) { ->y { x + y } }\n"[s..e], "y");
    }

    // 1.92 (expected values from the stock 1.92.1 CLI).

    fn corrected_no_parens(source: &str) -> String {
        let mut out = source.to_string();
        let offs = check_stabby_lambda_parentheses(
            source.as_bytes(),
            Config { style: STYLE_REQUIRE_NO_PARENTHESES },
        );
        for o in offs.iter().rev() {
            out.replace_range(o.paren_close_start..o.paren_close_end, "");
            out.replace_range(o.paren_open_start..o.paren_open_end, "");
        }
        out
    }

    #[test]
    fn keeps_parens_needed_by_a_default_value_or_a_comment() {
        for src in [
            "a = ->(x = [{}]) { x }\n",
            "b = ->(x = foo { 1 }) { x }\n",
            "c = ->(x = -> { 1 }) { x }\n",
            "d = ->(k: {}) { k }\n",
            "m = ->(x # c\n) { x }\n",
        ] {
            assert!(run(src, STYLE_REQUIRE_NO_PARENTHESES).is_empty(), "{src}");
        }
    }

    #[test]
    fn removes_padding_with_the_parens() {
        assert_eq!(corrected_no_parens("e = ->( x, y ) { x }\n"), "e = ->x, y { x }\n");
        assert_eq!(corrected_no_parens("f = ->(\n  x\n) { x }\n"), "f = ->x { x }\n");
        assert_eq!(corrected_no_parens("g = ->(x; y) { x }\n"), "g = ->x; y { x }\n");
        assert_eq!(
            corrected_no_parens("b = ->(x = foo(a: 1)) { x }\n"),
            "b = ->x = foo(a: 1) { x }\n"
        );
    }
}
