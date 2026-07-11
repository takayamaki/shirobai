//! `Style/HashSyntax`.
//!
//! Enforces the configured hash key syntax (`EnforcedStyle`: `ruby19` /
//! `hash_rockets` / `no_mixed_keys` / `ruby19_no_mixed_keys`) and, on Ruby
//! 3.1+, the hash value shorthand (`EnforcedShorthandSyntax`: `always` /
//! `never` / `either` / `consistent` / `either_consistent`). A separate offense
//! is registered per problematic pair.
//!
//! Reconstructed over Prism, mirroring stock's `on_hash` (the `EnforcedStyle`
//! `check` machinery) and `HashShorthandSyntax`'s `on_hash_for_mixed_shorthand`
//! (the `consistent` / `either_consistent` breakdown) plus `on_pair` (the
//! `always` / `never` per-pair shorthand). The walk maintains an ancestor frame
//! stack so the shorthand paren-insertion logic
//! (`def_node_that_require_parentheses`, modifier-form / last-expression
//! detection) can be replayed without re-parsing.
//!
//! Division of labour with the Ruby wrapper: Rust decides which pairs offend,
//! with which message and `config_to_allow_offenses` signal, and emits the
//! exact corrector op sequence (replace / remove / insert_before /
//! insert_after) for each. The wrapper applies the ops verbatim (byte offsets
//! mapped through `SourceOffsets`), exactly like stock's correctors. Symbol
//! acceptability is decided in Rust by an ASCII byte port of stock's regexes:
//! a non-ASCII symbol name never matches `\A[_a-z]\w*[?!]?\z/i` (the gem's `\w`
//! is ASCII-only here) and is never ruby19-convertible, matching stock.
//!
//! Prism vs parser geometry for a pair (`AssocNode`):
//! - rocket `:a => 0`: key = `:a` (leading colon included), `operator_loc` = `=>`.
//! - colon `a: 1`: key = `a:` (trailing colon **included**), `operator_loc` =
//!   `None`; the parser-side key is the colon stripped, and the operator is the
//!   trailing `:`.
//! - value omission `foo:`: key = `foo:`, value range == key range.

use ruby_prism::{Location, Node};

/// A single corrector op. `kind`: 0 replace, 1 remove, 2 insert_before,
/// 3 insert_after. For remove the text is empty.
pub struct Op {
    pub kind: u8,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Detection side effect the wrapper replays through the genuine stock methods,
/// so `config_to_allow_offenses` / `detected_styles` match exactly.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Detect {
    /// `opposite_style_detected` (an `EnforcedStyle` offense was found).
    OppositeStyle,
    /// `correct_style_detected` (a non-offending pair under an `EnforcedStyle`
    /// check). Emitted as a non-offense record (`is_offense == false`).
    CorrectStyle,
    /// `self.config_to_allow_offenses = { 'Enabled' => false }` (shorthand
    /// `always`).
    Disabled,
    /// no detection call (shorthand `never` / mixed / consistent paths).
    None,
}

/// One record from the walk, in walk order. When `is_offense` is false it is a
/// pure detection marker (`correct_style_detected`) with no caret and no ops.
pub struct HashSyntaxOffense {
    pub is_offense: bool,
    pub start_offset: usize,
    pub end_offset: usize,
    /// Message selector: 0 ruby19, 1 hash_rockets, 2 no_mixed_keys, 3 omit,
    /// 4 explicit, 5 do_not_mix_omit, 6 do_not_mix_explicit.
    pub message: u8,
    pub detect: Detect,
    pub ops: Vec<Op>,
}

/// `Style/HashSyntax` configuration.
#[derive(Clone, Copy)]
pub struct Config {
    /// 0 ruby19, 1 hash_rockets, 2 no_mixed_keys, 3 ruby19_no_mixed_keys.
    pub style: u8,
    /// 0 always, 1 never, 2 either, 3 consistent, 4 either_consistent.
    pub shorthand: u8,
    pub use_hash_rockets_with_symbol_values: bool,
    pub prefer_hash_rockets_for_non_alnum_ending_symbols: bool,
    /// `target_ruby_version > 3.0` (3.1+): shorthand syntax is available.
    pub ruby31_plus: bool,
    /// `target_ruby_version > 2.1`: quoted-symbol ruby19 keys are allowed.
    pub ruby22_plus: bool,
}

const STYLE_HASH_ROCKETS: u8 = 1;
const STYLE_NO_MIXED_KEYS: u8 = 2;
const STYLE_RUBY19_NO_MIXED: u8 = 3;

const SHORT_ALWAYS: u8 = 0;
const SHORT_NEVER: u8 = 1;
const SHORT_EITHER: u8 = 2;
const SHORT_CONSISTENT: u8 = 3;
const SHORT_EITHER_CONSISTENT: u8 = 4;

pub fn check_hash_syntax(source: &[u8], cfg: &Config) -> Vec<HashSyntaxOffense> {
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

/// Which delimiter a pair uses / a check targets.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Delim {
    Colon,
    Rocket,
}

/// The correction to apply for an `EnforcedStyle` offense.
#[derive(Clone, Copy)]
enum Fix {
    /// `autocorrect_ruby19` — rocket -> `key: value`.
    Ruby19,
    /// `autocorrect_hash_rockets` — colon -> `:key => value`.
    HashRockets,
}

/// Ancestor frame.
struct Frame {
    kind: FrameKind,
    start: usize,
    end: usize,
    /// For a "statement container" node (program / block / def / begin / paren /
    /// if-or-loop clause), the start offset of its statements list's last child.
    /// `node.right_sibling.nil?` for a statement is then "its start equals its
    /// nearest container's `last_stmt_start`". Prism does not fire the branch
    /// hook for `StatementsNode` itself, so siblings are tracked on the parent.
    last_stmt_start: Option<usize>,
}

enum FrameKind {
    /// A parser dispatch node (`CallNode` / `SuperNode` / `YieldNode`).
    Call(CallFrame),
    /// A hash literal (`HashNode` braced, or braceless `KeywordHashNode`).
    Hash {
        braces: bool,
        /// `pairs.last` range and whether `key.source == value.source` (omittable).
        last_pair: Option<((usize, usize), bool)>,
    },
    /// Prism `ArgumentsNode`: transparent in parser, skipped when finding a
    /// node's "parser parent".
    Arguments,
    Other(OtherKind),
}

struct CallFrame {
    method_is_brackets: bool,
    is_assignment_method: bool,
    parenthesized: bool,
    selector: Option<(usize, usize)>,
    first_arg: Option<(usize, usize)>,
    last_arg: Option<(usize, usize)>,
    last_arg_pairs: Vec<(usize, usize)>,
    /// parser's `last_argument` is a hash. Prism keeps a block-pass (`&block`)
    /// out of `arguments()`, but parser counts it as the last argument, so a
    /// call ending in `&block` has a non-hash last argument.
    last_arg_is_hash: bool,
    is_call_like: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OtherKind {
    Return,
    If,
    Unless,
    While,
    Until,
    Begin,
    /// A `( ... )` grouping (`ParenthesesNode`) — parser's `begin` with `(` loc.
    Parens,
    Modifier,
    Assignment,
    Plain,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    cfg: Config,
    stack: Vec<Frame>,
    pub offenses: Vec<HashSyntaxOffense>,
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(h) = node.as_hash_node() {
            let elements: Vec<Node<'_>> = h.elements().iter().collect();
            self.on_hash(node, &elements, true);
        } else if let Some(h) = node.as_keyword_hash_node() {
            let elements: Vec<Node<'_>> = h.elements().iter().collect();
            self.on_hash(node, &elements, false);
        } else if let Some(a) = node.as_assoc_node() {
            // `on_pair` for the always / never shorthand. The parent hash is the
            // current stack top (an AssocNode is always a child of a hash).
            let pair = pair_from_assoc(self.source, &a);
            self.on_pair(&pair);
        }
        self.stack.push(self.make_frame(node));
    }

    // NOTE: shorthand ancestor analysis treats `stack.len() - 1` as the hash
    // frame. `on_pair` is reached from `enter(AssocNode)` (hash already pushed)
    // so that holds; `on_hash_for_mixed_shorthand` runs from `on_hash` (hash not
    // yet pushed) and pushes a temporary hash frame around itself.

    fn leave(&mut self) {
        self.stack.pop();
    }
}

/// Per-pair facts gathered from a prism `AssocNode`.
struct Pair {
    /// AssocNode whole range (== parser `pair.source_range`).
    range: (usize, usize),
    /// parser key range (colon stripped for colon pairs).
    key_range: (usize, usize),
    /// parser operator range (`=>` or the colon for colon pairs).
    op_range: (usize, usize),
    /// value range.
    value_range: (usize, usize),
    /// delimiter is `:` (colon) vs `=>` (rocket).
    colon: bool,
    /// value omission (`foo:`).
    value_omission: bool,
    /// key is a symbol (`any_sym_type?`: plain or interpolated).
    key_is_sym: bool,
    /// key is a plain symbol (`sym_type?`), excluding interpolated dsyms.
    key_is_plain_sym: bool,
    /// key source (parser geometry).
    key_source: String,
    /// value is a symbol.
    value_is_sym: bool,
    /// value source.
    value_source: String,
    /// value node is a send or lvar (for shorthand omittability).
    value_send_or_lvar: bool,
}

fn pair_from_assoc(source: &[u8], a: &ruby_prism::AssocNode<'_>) -> Pair {
    let range = loc(&a.location());
    let key_node = a.key();
    let key_loc = loc(&key_node.location());
    let value_node = a.value();
    let value_loc = loc(&value_node.location());
    // `word_symbol_pair?` uses `any_sym_type?` (plain `SymbolNode` *or*
    // interpolated `InterpolatedSymbolNode`, e.g. `"#{foo}":`). `require_hash_value?`
    // (shorthand) uses the narrower `sym_type?` (plain symbols only).
    let key_is_plain_sym = key_node.as_symbol_node().is_some();
    let key_is_sym = key_is_plain_sym || key_node.as_interpolated_symbol_node().is_some();

    let (colon, op_range, key_range_parser) = match a.operator_loc() {
        Some(op) => (false, loc(&op), key_loc),
        None => {
            let op = (key_loc.1 - 1, key_loc.1);
            let key = (key_loc.0, key_loc.1 - 1);
            (true, op, key)
        }
    };

    let value_omission = colon && value_loc == key_loc;

    // For value omission (`foo:`) prism's implicit value spans the whole key
    // (`foo:`, colon included); parser's implicit value node is just the
    // identifier (`foo`), which is the offense range stock uses (`node.value`).
    let value_loc = if value_omission { key_range_parser } else { value_loc };

    let key_source =
        String::from_utf8_lossy(&source[key_range_parser.0..key_range_parser.1]).into_owned();

    let value_is_sym = value_node.as_symbol_node().is_some();
    let value_send_or_lvar =
        value_node.as_call_node().is_some() || value_node.as_local_variable_read_node().is_some();
    let value_source = if value_omission {
        key_source.clone()
    } else {
        String::from_utf8_lossy(&source[value_loc.0..value_loc.1]).into_owned()
    };

    Pair {
        range,
        key_range: key_range_parser,
        op_range,
        value_range: value_loc,
        colon,
        value_omission,
        key_is_sym,
        key_is_plain_sym,
        key_source,
        value_is_sym,
        value_source,
        value_send_or_lvar,
    }
}

impl Visitor<'_> {
    fn make_frame(&self, node: &Node<'_>) -> Frame {
        let (start, end) = loc(&node.location());
        let kind = if let Some(cf) = self.call_frame(node) {
            FrameKind::Call(cf)
        } else if let Some(hf) = self.hash_frame(node) {
            hf
        } else if node.as_arguments_node().is_some() {
            FrameKind::Arguments
        } else {
            FrameKind::Other(other_kind(node))
        };
        let last_stmt_start = container_last_stmt_start(node);
        Frame {
            kind,
            start,
            end,
            last_stmt_start,
        }
    }

    fn hash_frame(&self, node: &Node<'_>) -> Option<FrameKind> {
        let (elements, braces): (Vec<Node<'_>>, bool) = if let Some(h) = node.as_hash_node() {
            (h.elements().iter().collect(), true)
        } else {
            let h = node.as_keyword_hash_node()?;
            (h.elements().iter().collect(), false)
        };
        let last_pair = elements.iter().filter_map(|e| e.as_assoc_node()).next_back().map(|a| {
            let p = pair_from_assoc(self.source, &a);
            (p.range, p.key_source == p.value_source)
        });
        Some(FrameKind::Hash { braces, last_pair })
    }

    fn call_frame(&self, node: &Node<'_>) -> Option<CallFrame> {
        // `block_pass` is true when the call carries a `&block` argument, which
        // prism keeps out of `arguments()` but parser counts as the call's last
        // argument (so the parser `last_argument` is then not a hash).
        let (args, selector, method_brackets, is_assign, paren, block_pass) =
            if let Some(c) = node.as_call_node() {
                let m = c.name();
                (
                    c.arguments(),
                    c.message_loc().map(|l| loc(&l)),
                    m.as_slice() == b"[]" || m.as_slice() == b"[]=",
                    m.as_slice().last() == Some(&b'='),
                    c.opening_loc().is_some(),
                    c.block().map(|b| b.as_block_argument_node().is_some()).unwrap_or(false),
                )
            } else if let Some(s) = node.as_super_node() {
                let bp = s.block().map(|b| b.as_block_argument_node().is_some()).unwrap_or(false);
                (s.arguments(), Some(loc(&s.keyword_loc())), false, false, s.lparen_loc().is_some(), bp)
            } else {
                let y = node.as_yield_node()?;
                (y.arguments(), Some(loc(&y.keyword_loc())), false, false, y.lparen_loc().is_some(), false)
            };
        let arg_nodes: Vec<Node<'_>> = args
            .as_ref()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();
        let arg_ranges: Vec<(usize, usize)> = arg_nodes.iter().map(|n| loc(&n.location())).collect();
        let last_arg_pairs = arg_nodes
            .last()
            .map(|n| hash_pairs_of(n))
            .unwrap_or_default();
        let last_arg_is_hash = !block_pass
            && arg_nodes
                .last()
                .map(|n| n.as_hash_node().is_some() || n.as_keyword_hash_node().is_some())
                .unwrap_or(false);
        Some(CallFrame {
            method_is_brackets: method_brackets,
            is_assignment_method: is_assign,
            parenthesized: paren,
            selector,
            first_arg: arg_ranges.first().copied(),
            last_arg: arg_ranges.last().copied(),
            last_arg_pairs,
            last_arg_is_hash,
            is_call_like: true,
        })
    }

    // ---- EnforcedStyle (`on_hash`) ----

    fn on_hash(&mut self, hash_node: &Node<'_>, elements: &[Node<'_>], braces: bool) {
        let pairs: Vec<Pair> = elements
            .iter()
            .filter_map(|e| e.as_assoc_node())
            .map(|a| pair_from_assoc(self.source, &a))
            .collect();
        if pairs.is_empty() {
            return;
        }

        // `on_hash_for_mixed_shorthand` needs the hash on the ancestor stack
        // (so its `require_hash_value` / paren logic resolves the dispatch
        // ancestor below the hash). Push a temporary frame for the duration.
        self.stack.push(self.make_frame(hash_node));
        self.on_hash_for_mixed_shorthand(&pairs);
        self.stack.pop();

        let force_rockets = self.force_hash_rockets(&pairs);

        if self.cfg.style == STYLE_HASH_ROCKETS || force_rockets {
            // hash_rockets_check: check(pairs, ':', MSG_HASH_ROCKETS)
            self.check(hash_node, &pairs, Delim::Colon, 1, Fix::HashRockets, braces);
        } else if self.cfg.style == STYLE_RUBY19_NO_MIXED {
            self.ruby19_no_mixed_keys_check(hash_node, &pairs, braces);
        } else if self.cfg.style == STYLE_NO_MIXED_KEYS {
            self.no_mixed_keys_check(hash_node, &pairs, braces);
        } else {
            // ruby19_check
            if self.sym_indices(&pairs) {
                self.check(hash_node, &pairs, Delim::Rocket, 0, Fix::Ruby19, braces);
            }
        }
    }

    fn force_hash_rockets(&self, pairs: &[Pair]) -> bool {
        self.cfg.use_hash_rockets_with_symbol_values && pairs.iter().any(|p| p.value_is_sym)
    }

    fn sym_indices(&self, pairs: &[Pair]) -> bool {
        pairs.iter().all(|p| self.word_symbol_pair(p))
    }

    fn word_symbol_pair(&self, pair: &Pair) -> bool {
        pair.key_is_sym && self.acceptable_19_syntax_symbol(&pair.key_source)
    }

    fn acceptable_19_syntax_symbol(&self, sym_name: &str) -> bool {
        let name = sym_name.strip_prefix(':').unwrap_or(sym_name);

        if self.cfg.prefer_hash_rockets_for_non_alnum_ending_symbols
            && !ends_with_alnum_or_quote(name)
        {
            return false;
        }

        if matches_simple_symbol(name) {
            return true;
        }

        if !self.cfg.ruby22_plus {
            return false;
        }

        (name.starts_with('\'') && name.ends_with('\''))
            || (name.starts_with('"') && name.ends_with('"'))
    }

    fn ruby19_no_mixed_keys_check(&mut self, hash_node: &Node<'_>, pairs: &[Pair], braces: bool) {
        if self.force_hash_rockets(pairs) {
            self.check(hash_node, pairs, Delim::Colon, 1, Fix::HashRockets, braces);
        } else if self.sym_indices(pairs) {
            self.check(hash_node, pairs, Delim::Rocket, 0, Fix::Ruby19, braces);
        } else {
            // no_mixed: colon pairs -> rockets, rocket pairs -> ruby19.
            self.check_no_mixed(hash_node, pairs, Delim::Colon, braces);
        }
    }

    fn no_mixed_keys_check(&mut self, hash_node: &Node<'_>, pairs: &[Pair], braces: bool) {
        if self.sym_indices(pairs) {
            // pairs.first.inverse_delimiter — the opposite of the first pair's.
            let inv = if pairs[0].colon {
                Delim::Rocket
            } else {
                Delim::Colon
            };
            self.check_no_mixed(hash_node, pairs, inv, braces);
        } else {
            self.check_no_mixed(hash_node, pairs, Delim::Colon, braces);
        }
    }

    /// `check` for the no-mixed-keys messages: the fix depends on the offending
    /// pair's own delimiter (`autocorrect_no_mixed_keys`).
    fn check_no_mixed(&mut self, hash_node: &Node<'_>, pairs: &[Pair], delim: Delim, braces: bool) {
        for (i, pair) in pairs.iter().enumerate() {
            let pd = if pair.colon { Delim::Colon } else { Delim::Rocket };
            if pd == delim {
                let fix = if pair.colon { Fix::HashRockets } else { Fix::Ruby19 };
                self.emit_style_offense(hash_node, pair, 2, fix, braces, i == 0);
            } else {
                self.emit_correct_style();
            }
        }
    }

    /// `check(pairs, delim, msg)` with a fixed fix kind.
    fn check(
        &mut self,
        hash_node: &Node<'_>,
        pairs: &[Pair],
        delim: Delim,
        msg: u8,
        fix: Fix,
        braces: bool,
    ) {
        for (i, pair) in pairs.iter().enumerate() {
            let pd = if pair.colon { Delim::Colon } else { Delim::Rocket };
            if pd == delim {
                self.emit_style_offense(hash_node, pair, msg, fix, braces, i == 0);
            } else {
                self.emit_correct_style();
            }
        }
    }

    fn emit_correct_style(&mut self) {
        self.offenses.push(HashSyntaxOffense {
            is_offense: false,
            start_offset: 0,
            end_offset: 0,
            message: 0,
            detect: Detect::CorrectStyle,
            ops: Vec::new(),
        });
    }

    fn emit_style_offense(
        &mut self,
        hash_node: &Node<'_>,
        pair: &Pair,
        msg: u8,
        fix: Fix,
        braces: bool,
        is_first_pair: bool,
    ) {
        let start = pair.range.0;
        let end = pair.op_range.1;
        let ops = match fix {
            Fix::Ruby19 => self.autocorrect_ruby19(hash_node, pair, braces, is_first_pair),
            Fix::HashRockets => self.autocorrect_hash_rockets(pair),
        };
        self.offenses.push(HashSyntaxOffense {
            is_offense: true,
            start_offset: start,
            end_offset: end,
            message: msg,
            detect: Detect::OppositeStyle,
            ops,
        });
    }

    // ---- shorthand: `on_hash_for_mixed_shorthand` (consistent / either_consistent) ----

    fn on_hash_for_mixed_shorthand(&mut self, pairs: &[Pair]) {
        if !self.cfg.ruby31_plus {
            return;
        }
        if self.cfg.shorthand != SHORT_CONSISTENT && self.cfg.shorthand != SHORT_EITHER_CONSISTENT {
            return;
        }
        let mut omitted = Vec::new();
        let mut omittable = Vec::new();
        let mut needed = false;
        for p in pairs {
            if p.value_omission {
                omitted.push(p);
            } else if self.require_hash_value(p) {
                needed = true;
            } else {
                omittable.push(p);
            }
        }
        let key_count =
            (!omitted.is_empty()) as usize + (!omittable.is_empty()) as usize + needed as usize;
        let mixed = key_count > 1;

        if mixed {
            if needed {
                for p in &omitted {
                    let replacement = format!("{k}: {k}", k = p.key_source);
                    let ops = self.shorthand_ops(p, &replacement);
                    self.push_shorthand(p, 6, Detect::None, ops);
                }
            } else {
                for p in &omittable {
                    let replacement = format!("{}:", p.key_source);
                    let ops = self.shorthand_ops(p, &replacement);
                    self.push_shorthand(p, 5, Detect::None, ops);
                }
            }
        } else {
            if needed {
                return;
            }
            // ignore_explicit_omissible: keys == [:value_omittable] && either_consistent
            if omitted.is_empty()
                && !omittable.is_empty()
                && self.cfg.shorthand == SHORT_EITHER_CONSISTENT
            {
                return;
            }
            for p in &omittable {
                let replacement = format!("{}:", p.key_source);
                let ops = self.shorthand_ops(p, &replacement);
                self.push_shorthand(p, 3, Detect::None, ops);
            }
        }
    }

    // ---- shorthand: `on_pair` (always / never) ----

    fn on_pair(&mut self, pair: &Pair) {
        if self.ignore_hash_shorthand_syntax() {
            return;
        }
        if self.cfg.shorthand == SHORT_ALWAYS {
            if pair.value_omission || self.require_hash_value(pair) {
                return;
            }
            let replacement = format!("{}:", pair.key_source);
            let ops = self.shorthand_ops(pair, &replacement);
            self.push_shorthand(pair, 3, Detect::Disabled, ops);
        } else if self.cfg.shorthand == SHORT_NEVER {
            if !pair.value_omission {
                return;
            }
            let replacement = format!("{k}: {k}", k = pair.key_source);
            let ops = self.shorthand_ops(pair, &replacement);
            self.push_shorthand(pair, 4, Detect::None, ops);
        }
    }

    fn push_shorthand(&mut self, pair: &Pair, message: u8, detect: Detect, ops: Vec<Op>) {
        self.offenses.push(HashSyntaxOffense {
            is_offense: true,
            start_offset: pair.value_range.0,
            end_offset: pair.value_range.1,
            message,
            detect,
            ops,
        });
    }

    fn ignore_hash_shorthand_syntax(&self) -> bool {
        !self.cfg.ruby31_plus
            || self.cfg.shorthand == SHORT_EITHER
            || self.cfg.shorthand == SHORT_CONSISTENT
            || self.cfg.shorthand == SHORT_EITHER_CONSISTENT
    }

    /// `require_hash_value?(hash_key_source, node)`.
    fn require_hash_value(&self, pair: &Pair) -> bool {
        if !pair.key_is_plain_sym {
            return true;
        }
        if self.require_hash_value_for_around_hash_literal(pair) {
            return true;
        }
        if !pair.value_send_or_lvar {
            return true;
        }
        pair.key_source != pair.value_source
            || pair.key_source.ends_with('!')
            || pair.key_source.ends_with('?')
    }

    /// `require_hash_value_for_around_hash_literal?`.
    fn require_hash_value_for_around_hash_literal(&self, pair: &Pair) -> bool {
        let Some((mdi, hash_braces, hash_range)) = self.method_dispatch_for_pair(pair) else {
            return false;
        };
        // !node.parent.braces? && !use_element_of_hash_literal_as_receiver? &&
        // use_modifier_form_without_parenthesized_method_call?
        if hash_braces {
            return false;
        }
        if self.use_element_of_hash_literal_as_receiver(mdi, hash_range) {
            return false;
        }
        self.use_modifier_form_without_parenthesized_method_call(mdi)
    }

    /// `find_ancestor_method_dispatch_node(node)` for a pair: `node.parent.parent`
    /// (hash's parent) must be call-like and not `[]`/`[]=`. Returns the frame
    /// index of that dispatch node, whether the parent hash has braces, and the
    /// hash range.
    fn method_dispatch_for_pair(&self, pair: &Pair) -> Option<(usize, bool, (usize, usize))> {
        // The pair's parent is the hash (stack top). The hash's parent is
        // `node.parent.parent`. In prism a call's hash argument is wrapped in an
        // ArgumentsNode, but parser's `node.parent.parent` skips straight to the
        // send — so we skip Arguments frames when looking one level above the
        // hash.
        let hash_idx = self.stack.len().checked_sub(1)?;
        let (hash_braces, hash_range) = match &self.stack[hash_idx].kind {
            FrameKind::Hash { braces, .. } => {
                (*braces, (self.stack[hash_idx].start, self.stack[hash_idx].end))
            }
            _ => return None,
        };
        let _ = pair;
        // ancestor = hash's parent, skipping Arguments.
        let mut idx = hash_idx;
        while idx > 0 {
            idx -= 1;
            match &self.stack[idx].kind {
                FrameKind::Arguments => continue,
                FrameKind::Call(cf) => {
                    if cf.is_call_like && !cf.method_is_brackets {
                        return Some((idx, hash_braces, hash_range));
                    }
                    return None;
                }
                _ => return None,
            }
        }
        None
    }

    fn use_element_of_hash_literal_as_receiver(
        &self,
        mdi: usize,
        hash_range: (usize, usize),
    ) -> bool {
        // ancestor.send_type? && ancestor.receiver == parent(hash).
        // A send whose receiver is the hash: the hash starts at the call start
        // and the call is `{...}.method`. Approximate: the call's source begins
        // at the hash start (receiver position).
        if let FrameKind::Call(_) = &self.stack[mdi].kind {
            return self.stack[mdi].start == hash_range.0;
        }
        false
    }

    fn use_modifier_form_without_parenthesized_method_call(&self, mdi: usize) -> bool {
        if let FrameKind::Call(cf) = &self.stack[mdi].kind
            && cf.parenthesized
        {
            return false;
        }
        // ancestor.ancestors.any? { |n| n.modifier_form? }
        self.stack[..mdi]
            .iter()
            .any(|f| matches!(&f.kind, FrameKind::Other(OtherKind::Modifier)))
    }

    // ---- shorthand corrector (register_offense) ----

    fn shorthand_ops(&self, pair: &Pair, replacement: &str) -> Vec<Op> {
        // corrector.replace(node, replacement)
        let mut ops = vec![Op {
            kind: 0,
            start: pair.range.0,
            end: pair.range.1,
            text: replacement.to_string(),
        }];

        // def_node_that_require_parentheses(node): add parens to a no-paren call.
        if let Some(paren) = self.def_node_that_require_parentheses(pair) {
            // white_spaces = selector.end_pos .. first_argument.begin_pos
            ops.push(Op {
                kind: 0,
                start: paren.selector_end,
                end: paren.first_arg_start,
                text: "(".to_string(),
            });
            if paren.pair_is_last {
                ops.push(Op {
                    kind: 3,
                    start: paren.last_arg_end,
                    end: paren.last_arg_end,
                    text: ")".to_string(),
                });
            }
        }
        ops
    }

    fn def_node_that_require_parentheses(&self, pair: &Pair) -> Option<ParenInsert> {
        // last_pair = node.parent.pairs.last;
        // return unless last_pair.key.source == last_pair.value.source
        let hash_idx = self.stack.len().checked_sub(1)?;
        let (hash_braces, last_pair) = match &self.stack[hash_idx].kind {
            FrameKind::Hash { braces, last_pair } => (*braces, *last_pair),
            _ => return None,
        };
        let (_last_pair_range, last_pair_omittable) = last_pair?;
        if !last_pair_omittable {
            return None;
        }
        // dispatch_node = find_ancestor_method_dispatch_node(node)
        let (mdi, _hb, _hr) = self.method_dispatch_for_pair(pair)?;
        let cf = match &self.stack[mdi].kind {
            FrameKind::Call(cf) => cf,
            _ => return None,
        };
        // return if dispatch_node.assignment_method?
        if cf.is_assignment_method {
            return None;
        }
        // return if dispatch_node.parenthesized?
        if cf.parenthesized {
            return None;
        }
        // return if dispatch_node.parent && parentheses?(dispatch_node.parent)
        if self.parent_has_parentheses(mdi) {
            return None;
        }
        // return if last_expression?(dispatch_node) && !requires_parentheses_context?(dispatch_node)
        if self.last_expression(mdi) && !self.requires_parentheses_context(mdi) {
            return None;
        }
        // def_node = node.each_ancestor(:call, :super, :yield).first
        // (== mdi). `DefNode.new(def_node) unless def_node.arguments.empty?`.
        // `last_argument.nil? || !last_argument.hash_type?` → no paren insertion
        // (the call's parser last argument is not a hash, e.g. it ends in
        // `&block`).
        if !cf.last_arg_is_hash {
            return None;
        }
        // `next if node.parent.braces?` — a braced hash gets no paren insertion.
        if hash_braces {
            return None;
        }
        let first_arg = cf.first_arg?;
        let last_arg = cf.last_arg?;
        let selector_end = cf.selector?.1;
        // The paren-and-`)` insertion only happens when `last_argument` is a
        // hash; `node == last_argument.pairs.last` and `!node.parent.braces?`
        // gate the closing `)`. `hash_braces` here is whether the pair's own
        // parent hash is braced.
        let _ = hash_braces;
        let pair_is_last = cf.last_arg_pairs.last() == Some(&pair.range);
        Some(ParenInsert {
            selector_end,
            first_arg_start: first_arg.0,
            last_arg_end: last_arg.1,
            pair_is_last,
        })
    }

    fn parent_has_parentheses(&self, mdi: usize) -> bool {
        // dispatch_node.parent && parentheses?(parent): parent is begin/paren.
        if mdi == 0 {
            return false;
        }
        // find the frame just above mdi (skip Arguments / transparent Stmts).
        let mut idx = mdi;
        while idx > 0 {
            idx -= 1;
            match &self.stack[idx].kind {
                FrameKind::Arguments => continue,
                // `parentheses?`: a `begin`/paren whose `loc.begin` is `(`.
                FrameKind::Other(OtherKind::Parens) => return true,
                FrameKind::Other(OtherKind::Begin) => {
                    return self.source.get(self.stack[idx].start) == Some(&b'(');
                }
                _ => return false,
            }
        }
        false
    }

    /// `last_expression?(node)`: starting at the frame `fi`.
    /// `return false if node.right_sibling` /
    /// `return true unless assignment ancestor` /
    /// `return last_expression?(assignment.parent) if assignment.parent assignment` /
    /// `!assignment.right_sibling`.
    fn last_expression(&self, fi: usize) -> bool {
        if self.has_right_sibling(fi) {
            return false;
        }
        // find the nearest assignment ancestor of `fi`.
        let Some(asgn) = self.assignment_ancestor(fi) else {
            return true;
        };
        // if the assignment's parent is itself an assignment, recurse on it.
        if let Some(parent) = self.assignment_ancestor(asgn) {
            // only when the *immediate* parser parent is an assignment.
            if self.parent_is_assignment(asgn) {
                return self.last_expression(parent);
            }
        }
        !self.has_right_sibling(asgn)
    }

    /// `node.right_sibling`: a node has a right sibling iff its nearest enclosing
    /// statement container has a later statement — i.e. this node is *not* that
    /// container's statements list's last child. Prism does not frame
    /// `StatementsNode`, so the container parent carries `last_stmt_start`.
    fn has_right_sibling(&self, fi: usize) -> bool {
        let node_start = self.stack[fi].start;
        let mut idx = fi;
        while idx > 0 {
            idx -= 1;
            if let Some(last) = self.stack[idx].last_stmt_start {
                return last != node_start;
            }
        }
        false
    }

    /// The nearest assignment ancestor's frame index above `fi`, if any.
    fn assignment_ancestor(&self, fi: usize) -> Option<usize> {
        self.stack[..fi]
            .iter()
            .rposition(|f| matches!(&f.kind, FrameKind::Other(OtherKind::Assignment)))
    }

    /// Whether the parser parent of the node at `fi` is an assignment.
    fn parent_is_assignment(&self, fi: usize) -> bool {
        let mut idx = fi;
        while idx > 0 {
            idx -= 1;
            match &self.stack[idx].kind {
                FrameKind::Arguments => continue,
                FrameKind::Other(OtherKind::Assignment) => return true,
                _ => return false,
            }
        }
        false
    }

    fn requires_parentheses_context(&self, mdi: usize) -> bool {
        // parent.type?(:call, :if, :super, :until, :while, :yield)
        if mdi == 0 {
            return false;
        }
        // parser's `node.parent`: skip transparent `Arguments` wrappers.
        let mut idx = mdi;
        while idx > 0 {
            idx -= 1;
            match &self.stack[idx].kind {
                FrameKind::Arguments => continue,
                FrameKind::Call(_) => return true,
                FrameKind::Other(OtherKind::If)
                | FrameKind::Other(OtherKind::Unless)
                | FrameKind::Other(OtherKind::While)
                | FrameKind::Other(OtherKind::Until) => return true,
                _ => return false,
            }
        }
        false
    }

    // ---- EnforcedStyle correctors ----

    fn autocorrect_ruby19(
        &self,
        hash_node: &Node<'_>,
        pair: &Pair,
        braces: bool,
        is_first_pair: bool,
    ) -> Vec<Op> {
        let range_start = pair.key_range.0;
        // `range_with_surrounding_space(key.join(operator), side: :right)`:
        // extend right over `[ \t]*\n*` (stock defaults), then run the `sub`.
        let range_end = self.surrounding_space_end(pair.op_range.1, 1);
        let src = String::from_utf8_lossy(&self.source[range_start..range_end]).into_owned();
        let space = if self.argument_without_space(hash_node) {
            " "
        } else {
            ""
        };
        let replacement = sub_ruby19(&src, space);
        let mut ops = vec![Op {
            kind: 0,
            start: range_start,
            end: range_end,
            text: replacement,
        }];
        // rubocop#15327: the braceless-return wrap runs once per offending pair
        // but the hash must only be wrapped once, so only the first pair of the
        // hash inserts the braces (`pair_node.equal?(hash_node.pairs.first)`).
        if !braces && is_first_pair && self.parent_is_return_type(hash_node) {
            let (hs, he) = loc(&hash_node.location());
            ops.push(Op {
                kind: 2,
                start: hs,
                end: hs,
                text: "{".to_string(),
            });
            ops.push(Op {
                kind: 3,
                start: he,
                end: he,
                text: "}".to_string(),
            });
        }
        ops
    }

    fn autocorrect_hash_rockets(&self, pair: &Pair) -> Vec<Op> {
        let mut text = format!(":{} => ", pair.key_source);
        if pair.value_omission {
            text.push_str(&pair.key_source);
        }
        let mut ops = vec![Op {
            kind: 0,
            start: pair.key_range.0,
            end: pair.key_range.1,
            text,
        }];
        // `corrector.remove(range_with_surrounding_space(op))` with stock's
        // defaults (`side: :both, newlines: true, whitespace: false`): on each
        // side extend over `[ \t]*`, then over `\n*`, then STOP — it does *not*
        // continue over the indentation that follows a newline. For a
        // value-omitted pair (`uri:`) the colon abuts `\n<indent>)`, so the `\n`
        // is pulled but the `)`'s indent is kept (`:uri => uri          )`).
        let (s, e) = (
            self.surrounding_space_end(pair.op_range.0, -1),
            self.surrounding_space_end(pair.op_range.1, 1),
        );
        ops.push(Op {
            kind: 1,
            start: s,
            end: e,
            text: String::new(),
        });
        ops
    }

    /// `final_pos`: from `pos`, in `step` direction, skip `[ \t]*` then `\n*`.
    fn surrounding_space_end(&self, pos: usize, step: isize) -> usize {
        let n = self.source.len();
        // The byte under inspection is `pos-1` going left, `pos` going right.
        let peek = |p: usize| -> Option<u8> {
            if step < 0 {
                if p == 0 { None } else { Some(self.source[p - 1]) }
            } else if p < n {
                Some(self.source[p])
            } else {
                None
            }
        };
        let mut p = pos;
        while matches!(peek(p), Some(b' ') | Some(b'\t')) {
            p = (p as isize + step) as usize;
        }
        while matches!(peek(p), Some(b'\n')) {
            p = (p as isize + step) as usize;
        }
        p
    }

    fn argument_without_space(&self, hash_node: &Node<'_>) -> bool {
        let hash_start = loc(&hash_node.location()).0;
        let mut idx = self.stack.len();
        while idx > 0 {
            idx -= 1;
            match &self.stack[idx].kind {
                FrameKind::Arguments => continue,
                FrameKind::Call(cf) => {
                    let is_arg = cf.first_arg.is_some()
                        && contains(self.stack[idx].start, self.stack[idx].end, hash_start);
                    return is_arg && cf.selector.map(|s| s.1) == Some(hash_start);
                }
                _ => return false,
            }
        }
        false
    }

    fn parent_is_return_type(&self, hash_node: &Node<'_>) -> bool {
        let hash_start = loc(&hash_node.location()).0;
        // immediate parent (skip Arguments): a ReturnNode whose value is the hash.
        if let Some(top) = self.stack.last()
            && let FrameKind::Other(OtherKind::Return) = top.kind
        {
            // ensure the hash is the return value (starts after `return `).
            return top.start <= hash_start;
        }
        false
    }
}

struct ParenInsert {
    selector_end: usize,
    first_arg_start: usize,
    last_arg_end: usize,
    pair_is_last: bool,
}

fn contains(start: usize, end: usize, inner: usize) -> bool {
    start <= inner && inner <= end
}

/// For a statement-container node, the start offset of its statements list's
/// last child (used to answer `node.right_sibling.nil?`). Covers the containers
/// a method-dispatch node can sit directly inside: program, block body, def
/// body, begin/paren body, and the branches of `if` / `unless` / `while` /
/// `until` (predicates included, since a no-paren call can be a predicate).
fn container_last_stmt_start(node: &Node<'_>) -> Option<usize> {
    let stmts = if let Some(p) = node.as_program_node() {
        Some(p.statements())
    } else if let Some(b) = node.as_block_node() {
        b.body().and_then(|n| n.as_statements_node())
    } else if let Some(d) = node.as_def_node() {
        d.body().and_then(|n| n.as_statements_node())
    } else if let Some(b) = node.as_begin_node() {
        b.statements()
    } else if let Some(p) = node.as_parentheses_node() {
        p.body().and_then(|n| n.as_statements_node())
    } else {
        None
    };
    stmts
        .as_ref()
        .and_then(|s| s.body().iter().last())
        .map(|n| n.location().start_offset())
}

fn hash_pairs_of(node: &Node<'_>) -> Vec<(usize, usize)> {
    let elems: Vec<Node<'_>> = if let Some(h) = node.as_hash_node() {
        h.elements().iter().collect()
    } else if let Some(h) = node.as_keyword_hash_node() {
        h.elements().iter().collect()
    } else {
        return Vec::new();
    };
    elems
        .iter()
        .filter_map(|e| e.as_assoc_node())
        .map(|a| loc(&a.location()))
        .collect()
}

fn other_kind(node: &Node<'_>) -> OtherKind {
    if node.as_return_node().is_some() {
        OtherKind::Return
    } else if node.as_if_node().is_some() {
        // modifier form: an `if` with no `then`/`end` keyword — prism IfNode has
        // an `end_keyword_loc`; modifier ifs have none.
        if node.as_if_node().and_then(|n| n.end_keyword_loc()).is_none() {
            OtherKind::Modifier
        } else {
            OtherKind::If
        }
    } else if let Some(u) = node.as_unless_node() {
        if u.end_keyword_loc().is_none() {
            OtherKind::Modifier
        } else {
            OtherKind::Unless
        }
    } else if let Some(w) = node.as_while_node() {
        if w.closing_loc().is_none() {
            OtherKind::Modifier
        } else {
            OtherKind::While
        }
    } else if let Some(u) = node.as_until_node() {
        if u.closing_loc().is_none() {
            OtherKind::Modifier
        } else {
            OtherKind::Until
        }
    } else if node.as_parentheses_node().is_some() {
        // `( ... )` — parser represents this as a `begin` whose `loc.begin` is
        // `(`, which `parentheses?` matches.
        OtherKind::Parens
    } else if node.as_begin_node().is_some() {
        OtherKind::Begin
    } else if is_assignment_node(node) {
        OtherKind::Assignment
    } else {
        OtherKind::Plain
    }
}

fn is_assignment_node(node: &Node<'_>) -> bool {
    node.as_local_variable_write_node().is_some()
        || node.as_instance_variable_write_node().is_some()
        || node.as_class_variable_write_node().is_some()
        || node.as_global_variable_write_node().is_some()
        || node.as_constant_write_node().is_some()
        || node.as_constant_path_write_node().is_some()
        || node.as_multi_write_node().is_some()
        || node.as_call_node().map(|c| c.name().as_slice().last() == Some(&b'=')).unwrap_or(false)
}

fn ends_with_alnum_or_quote(name: &str) -> bool {
    match name.chars().last() {
        Some(c) => c.is_alphanumeric() || c == '"' || c == '\'',
        None => false,
    }
}

fn matches_simple_symbol(name: &str) -> bool {
    let b = name.as_bytes();
    if b.is_empty() {
        return false;
    }
    let first = b[0];
    if !(first == b'_' || first.is_ascii_alphabetic()) {
        return false;
    }
    let mut i = 1;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphanumeric() || c == b'_' {
            i += 1;
        } else {
            break;
        }
    }
    if i < b.len() && (b[i] == b'?' || b[i] == b'!') {
        i += 1;
    }
    i == b.len()
}

fn sub_ruby19(src: &str, space: &str) -> String {
    if let Some(rest) = src.strip_prefix(':')
        && let Some(arrow) = rest.rfind("=>")
    {
        let before = &rest[..arrow];
        let capture = before.trim_end();
        let after = &rest[arrow + 2..];
        if after.trim().is_empty() && !capture.is_empty() {
            return format!("{space}{capture}: ");
        }
    }
    src.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(style: u8, shorthand: u8) -> Config {
        Config {
            style,
            shorthand,
            use_hash_rockets_with_symbol_values: false,
            prefer_hash_rockets_for_non_alnum_ending_symbols: false,
            ruby31_plus: true,
            ruby22_plus: true,
        }
    }

    fn run(src: &str, c: &Config) -> Vec<HashSyntaxOffense> {
        check_hash_syntax(src.as_bytes(), c)
    }

    fn offenses(src: &str, c: &Config) -> Vec<(usize, usize, u8)> {
        run(src, c)
            .into_iter()
            .filter(|o| o.is_offense)
            .map(|o| (o.start_offset, o.end_offset, o.message))
            .collect()
    }

    #[test]
    fn ruby19_flags_rocket_pairs() {
        // `:a =>` offense range = pair start .. operator end.
        let c = cfg(0, 2);
        assert_eq!(offenses("{ :a => 0 }", &c), vec![(2, 7, 0)]);
    }

    #[test]
    fn ruby19_skips_non_word_symbols() {
        let c = cfg(0, 2);
        assert!(offenses("{ :[] => 0 }", &c).is_empty());
        assert!(offenses("{ :a= => 0 }", &c).is_empty());
    }

    #[test]
    fn ruby19_accepts_quoted_symbols() {
        let c = cfg(0, 2);
        assert_eq!(offenses(r#"{ :"s t" => 0 }"#, &c), vec![(2, 11, 0)]);
    }

    #[test]
    fn non_ascii_symbol_never_converts() {
        let c = cfg(0, 2);
        assert!(offenses("{ :\u{00e9} => 0 }", &c).is_empty());
    }

    #[test]
    fn hash_rockets_flags_colon_pairs() {
        let c = cfg(1, 2);
        assert_eq!(offenses("{ a: 0 }", &c), vec![(2, 4, 1)]);
    }

    #[test]
    fn shorthand_always_omits_matching_value() {
        let c = cfg(0, 0);
        // `{foo: foo}` -> offense on the value `foo` (3 = omit).
        let offs = offenses("{foo: foo}", &c);
        assert_eq!(offs.len(), 1);
        assert_eq!(offs[0].2, 3);
    }

    #[test]
    fn shorthand_never_includes_omitted_value() {
        let c = cfg(0, 1);
        let offs = offenses("{foo:, bar:}", &c);
        assert_eq!(offs.len(), 2);
        assert!(offs.iter().all(|o| o.2 == 4));
    }

    #[test]
    fn either_skips_shorthand() {
        let c = cfg(0, 2);
        assert!(offenses("{foo: foo}", &c).is_empty());
        assert!(offenses("{foo:}", &c).is_empty());
    }
}
