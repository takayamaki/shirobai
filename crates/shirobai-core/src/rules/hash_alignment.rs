//! `Layout/HashAlignment`.
//!
//! Checks that the keys, separators, and values of a multi-line hash literal
//! are aligned according to configuration. Replicates the
//! `HashAlignmentStyles` mixin's four checker classes (`KeyAlignment`,
//! `SeparatorAlignment`, `TableAlignment`, `KeywordSplatAlignment`) and the
//! cop's `check_pairs` / `add_offenses` machinery: per pair a column-delta
//! triple `{key, separator, value}` is computed for each configured alignment;
//! a non-zero delta is an offense; for the colon/rocket flavours the
//! least-offending of multiple permitted styles wins (`offenses_by.min_by
//! { length }`), and keyword-splat offenses are always reported.
//!
//! Division of labour with the Ruby wrapper: Rust decides which pairs offend,
//! with which message, and computes the delta triple and the byte ranges of
//! the key / operator / value (or, for the no-value path, the node). Ruby
//! applies `corrector.insert_before` / `corrector.remove` exactly like stock's
//! `adjust`, including the `key_delta` clamp to `-key.column`.
//!
//! Columns are parser-gem columns (character counts from the line start), not
//! display width. Prism colon pairs (`a: 1`) carry the `:` inside the key and
//! expose no operator location, so the key is trimmed of its trailing `:` and
//! the colon synthesised as the operator, matching parser's geometry; value
//! omission (`a:`) is detected by the node source ending in `:`.

use std::rc::Rc;

use ruby_prism::{Location, Node};

use super::line_index::LineIndex;

/// One misaligned pair (or kwsplat). The Ruby wrapper applies the deltas.
pub struct HashAlignmentOffense {
    /// Index of the hash these offenses came from, in walk (`on_hash`) order.
    /// Stock registers a hash's offenses inside one `on_hash` callback; if one
    /// offense's corrector raises `ClobberingError` it aborts that callback,
    /// dropping the rest of THAT hash's offenses but leaving other hashes
    /// (separate callbacks) intact. The wrapper uses this to confine a clobber
    /// to its own hash group.
    pub group: usize,
    /// Offense range = the pair / kwsplat node's source range.
    pub start_offset: usize,
    pub end_offset: usize,
    /// `0` key, `1` separator, `2` table, `3` kwsplat — selects the message.
    pub message: u8,
    /// When false, only the node is realigned by `key_delta` (kwsplat or value
    /// omission). When true, key/operator/value are each adjusted.
    pub has_value: bool,
    pub key_delta: isize,
    pub separator_delta: isize,
    pub value_delta: isize,
    /// Key range (parser geometry: colon excluded) and its column (for the
    /// clamp). Only meaningful when `has_value`.
    pub key_start: usize,
    pub key_end: usize,
    pub key_column: usize,
    pub op_start: usize,
    pub op_end: usize,
    pub value_start: usize,
    pub value_end: usize,
}

/// A permitted alignment flavour for one separator kind.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Alignment {
    Key,
    Separator,
    Table,
}

impl Alignment {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Alignment::Separator,
            2 => Alignment::Table,
            _ => Alignment::Key,
        }
    }
    fn message(self) -> u8 {
        match self {
            Alignment::Key => 0,
            Alignment::Separator => 1,
            Alignment::Table => 2,
        }
    }
}

/// `Layout/HashAlignment` configuration.
#[derive(Clone)]
pub struct Config {
    /// `EnforcedHashRocketStyle`, in config order (deduplicated downstream).
    pub hash_rocket_styles: Vec<u8>,
    /// `EnforcedColonStyle`, in config order.
    pub colon_styles: Vec<u8>,
    /// `EnforcedLastArgumentHashStyle`: 0 always_inspect, 1 always_ignore,
    /// 2 ignore_explicit, 3 ignore_implicit.
    pub last_argument_style: u8,
    /// `Layout/ArgumentAlignment` enforces `with_fixed_indentation`.
    pub enforce_fixed_indentation: bool,
}

fn dedup_styles(raw: &[u8]) -> Vec<Alignment> {
    let mut out: Vec<Alignment> = Vec::new();
    for &v in raw {
        let a = Alignment::from_u8(v);
        if !out.contains(&a) {
            out.push(a);
        }
    }
    if out.is_empty() {
        out.push(Alignment::Key);
    }
    out
}

pub fn check_hash_alignment(source: &[u8], cfg: &Config) -> Vec<HashAlignmentOffense> {
    let mut rule = build_rule(source, cfg);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

pub(crate) fn build_rule<'a>(source: &'a [u8], cfg: &Config) -> Visitor<'a> {
    Visitor {
        source,
        line_index: super::line_index::with_line_index(source, |li| li.clone()),
        hash_rocket: dedup_styles(&cfg.hash_rocket_styles),
        colon: dedup_styles(&cfg.colon_styles),
        last_argument_style: cfg.last_argument_style,
        enforce_fixed: cfg.enforce_fixed_indentation,
        stack: Vec::new(),
        ignored: Vec::new(),
        group: 0,
        offenses: Vec::new(),
    }
}

/// Ancestor frame: enough to find a hash's containing call (for the
/// last-argument-hash ignore and the paren-claim incompatibility check).
enum FrameKind {
    Call {
        /// `loc.selector` (the method name) range, if any.
        selector: Option<(usize, usize)>,
        /// `loc.expression` (the whole call) range.
        expression: (usize, usize),
        /// Ranges of the call's positional/keyword arguments in source order.
        args: Vec<(usize, usize)>,
    },
    /// Prism's `ArgumentsNode`, the transparent wrapper between a call and its
    /// arguments. parser has no such node — a hash argument's parent there is
    /// the `send` directly — so this level is skipped when finding a hash's
    /// "parser parent".
    Arguments,
    Other,
}

struct Frame {
    kind: FrameKind,
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    line_index: Rc<LineIndex>,
    hash_rocket: Vec<Alignment>,
    colon: Vec<Alignment>,
    last_argument_style: u8,
    enforce_fixed: bool,
    stack: Vec<Frame>,
    /// `ignore_node`d hash ranges (last-argument hashes excluded from `on_hash`).
    ignored: Vec<(usize, usize)>,
    /// Monotonic hash-group id, one per checked hash (`on_hash` call).
    group: usize,
    pub(crate) offenses: Vec<HashAlignmentOffense>,
}

fn loc(l: &Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

/// The `ArgumentsNode` of a call-like node (`CallNode` / `SuperNode` /
/// `YieldNode`), for the last-argument-hash ignore that stock applies via
/// `on_send` aliased to `on_super` / `on_yield`. `ForwardingSuperNode`
/// (`super` with no parens) carries no arguments and is excluded.
fn call_like_arguments<'a>(node: &Node<'a>) -> Option<ruby_prism::ArgumentsNode<'a>> {
    if let Some(c) = node.as_call_node() {
        c.arguments()
    } else if let Some(s) = node.as_super_node() {
        s.arguments()
    } else if let Some(y) = node.as_yield_node() {
        y.arguments()
    } else {
        None
    }
}

/// `(arguments, selector)` for a call-like node. The selector is the message
/// (`CallNode`) or the keyword (`super` / `yield`), mirroring parser's
/// `loc.selector`. Returns `None` for non-call-like nodes.
#[allow(clippy::type_complexity)]
fn call_like_meta<'a>(
    node: &Node<'a>,
) -> Option<(Option<ruby_prism::ArgumentsNode<'a>>, Option<(usize, usize)>)> {
    if let Some(c) = node.as_call_node() {
        Some((c.arguments(), c.message_loc().map(|l| loc(&l))))
    } else if let Some(s) = node.as_super_node() {
        Some((s.arguments(), Some(loc(&s.keyword_loc()))))
    } else {
        node.as_yield_node()
            .map(|y| (y.arguments(), Some(loc(&y.keyword_loc()))))
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        // `on_send` / `on_csend` / `on_super` / `on_yield` all alias to the same
        // last-argument-hash ignore.
        if let Some(args) = call_like_arguments(node) {
            self.process_call(&args);
        }
        // `on_hash` fires for parser `hash` nodes, which include both braced
        // hash literals (prism `HashNode`) and braceless keyword hashes (prism
        // `KeywordHashNode`, a method call's trailing `a: 1, b: 2`).
        if let Some(h) = node.as_hash_node() {
            let elements: Vec<Node<'_>> = h.elements().iter().collect();
            self.process_hash(node, &elements, true);
        } else if let Some(h) = node.as_keyword_hash_node() {
            let elements: Vec<Node<'_>> = h.elements().iter().collect();
            self.process_hash(node, &elements, false);
        }
        self.stack.push(self.make_frame(node));
    }

    fn leave(&mut self) {
        self.stack.pop();
    }
}

impl Visitor<'_> {
    fn make_frame(&self, node: &Node<'_>) -> Frame {
        let kind = if let Some((args, selector)) = call_like_meta(node) {
            FrameKind::Call {
                selector,
                expression: loc(&node.location()),
                args: args
                    .map(|a| a.arguments().iter().map(|n| loc(&n.location())).collect())
                    .unwrap_or_default(),
            }
        } else if node.as_arguments_node().is_some() {
            FrameKind::Arguments
        } else {
            FrameKind::Other
        };
        Frame { kind }
    }

    fn line_of(&self, off: usize) -> usize {
        self.line_index.line_of(off)
    }

    fn column(&self, off: usize) -> usize {
        self.line_index.column(self.source, off)
    }

    fn col(&self, off: usize) -> isize {
        self.column(off) as isize
    }

    /// `on_send` / `on_csend` / `on_super` / `on_yield`: a call whose last
    /// argument is a hash to be ignored by `EnforcedLastArgumentHashStyle`. In
    /// parser a trailing braceless keyword hash is also a `hash` node, so both
    /// a prism `HashNode` (`braces`) and a prism `KeywordHashNode` (braceless)
    /// last argument get an ignore entry when the style says so.
    fn process_call(&mut self, args: &ruby_prism::ArgumentsNode<'_>) {
        let Some(last) = args.arguments().iter().last() else {
            return;
        };
        let (range, braces) = if let Some(h) = last.as_hash_node() {
            (loc(&h.as_node().location()), true)
        } else if let Some(h) = last.as_keyword_hash_node() {
            (loc(&h.as_node().location()), false)
        } else {
            return;
        };
        if self.ignore_hash_argument(braces) {
            self.ignored.push(range);
        }
    }

    fn ignore_hash_argument(&self, braces: bool) -> bool {
        match self.last_argument_style {
            1 => true,    // always_ignore
            2 => braces,  // ignore_explicit
            3 => !braces, // ignore_implicit
            _ => false,   // always_inspect
        }
    }

    /// `on_hash`, for both a braced `HashNode` and a braceless
    /// `KeywordHashNode` (parser models both as `hash` nodes).
    fn process_hash(&mut self, node: &Node<'_>, elements: &[Node<'_>], braces: bool) {
        let range = loc(&node.location());
        let pairs: Vec<Pair> = elements.iter().map(|e| self.make_pair(e)).collect();
        // `node.pairs.empty?` — at least one AssocNode (kwsplat excluded).
        if !pairs.iter().any(|p| p.is_pair()) {
            return;
        }
        // `node.single_line?`.
        if self.line_of(range.0) == self.line_of(range.1.saturating_sub(1)) {
            return;
        }
        if self.ignored.contains(&range) {
            return;
        }
        if self.autocorrect_incompatible(node, elements, braces, &pairs) {
            return;
        }
        // `alignment_for_hash_rockets.any?(checkable) && alignment_for_colons.any?(checkable)`.
        if !self.hash_rocket.iter().any(|&a| self.checkable(a, &pairs))
            || !self.colon.iter().any(|&a| self.checkable(a, &pairs))
        {
            return;
        }
        self.check_pairs(&pairs);
    }

    /// The call frame that is this hash's parser parent, if any. parser has no
    /// `ArgumentsNode`, so a hash argument's parent is the `send` directly:
    /// when the prism parent is an `ArgumentsNode`, the call is one level up.
    fn parent_call(&self) -> Option<&Frame> {
        match self.stack.last() {
            Some(Frame {
                kind: FrameKind::Call { .. },
            }) => self.stack.last(),
            Some(Frame {
                kind: FrameKind::Arguments,
            }) => match self.stack.get(self.stack.len().wrapping_sub(2)) {
                Some(f @ Frame {
                    kind: FrameKind::Call { .. },
                }) => Some(f),
                _ => None,
            },
            _ => None,
        }
    }

    /// `autocorrect_incompatible_with_other_cops?`.
    fn autocorrect_incompatible(
        &self,
        node: &Node<'_>,
        elements: &[Node<'_>],
        _braces: bool,
        pairs: &[Pair],
    ) -> bool {
        if !self.enforce_fixed {
            return false;
        }
        let Some(Frame {
            kind:
                FrameKind::Call {
                    selector,
                    expression,
                    args,
                },
        }) = self.parent_call()
        else {
            return false;
        };
        let left = self.argument_before_hash(node, elements, args);
        let selector_range = left.or(*selector).unwrap_or(*expression);
        // `same_line?` override: `Util#same_line?(a, b)` (a.line == b.line) OR
        // `a.last_line == b.line`. `node.pairs.first` is the first pair.
        let first = pairs.iter().find(|p| p.is_pair()).unwrap();
        let sel_first_line = self.line_of(selector_range.0);
        let sel_last_line = self.line_of(selector_range.1.saturating_sub(1));
        let pair_line = self.line_of(first.range.0);
        sel_first_line == pair_line || sel_last_line == pair_line
    }

    /// `argument_before_hash`: the kwsplat value if the hash's first child is a
    /// kwsplat, else the hash's left sibling in the parent call's arguments.
    fn argument_before_hash(
        &self,
        node: &Node<'_>,
        elements: &[Node<'_>],
        args: &[(usize, usize)],
    ) -> Option<(usize, usize)> {
        if let Some(splat) = elements.first().and_then(|e| e.as_assoc_splat_node()) {
            return splat.value().map(|v| loc(&v.location()));
        }
        let hash_start = node.location().start_offset();
        let idx = args.iter().position(|&(s, _)| s == hash_start)?;
        idx.checked_sub(1).map(|i| args[i])
    }

    /// Build pair geometry, normalising prism colon pairs to parser geometry.
    fn make_pair(&self, node: &Node<'_>) -> Pair {
        let range = loc(&node.location());
        let Some(a) = node.as_assoc_node() else {
            return Pair {
                range,
                kind: PairKind::Kwsplat,
            };
        };
        let key = a.key().location();
        let value = a.value().location();
        let (ks, ke) = loc(&key);
        if let Some(op) = a.operator_loc() {
            // Rocket pair: prism gives the operator directly.
            let (os, oe) = loc(&op);
            Pair {
                range,
                kind: PairKind::Pair {
                    hash_rocket: true,
                    value_omission: false,
                    key_start: ks,
                    key_end: ke,
                    op_start: os,
                    op_end: oe,
                    value_start: value.start_offset(),
                    value_end: value.end_offset(),
                },
            }
        } else {
            // Colon pair: `:` is prism's key's last byte; parser's key excludes
            // it and the operator is that colon. Value omission when the source
            // ends with `:` and the value node coincides with the key.
            let omission =
                self.source.get(range.1 - 1) == Some(&b':') && value.start_offset() == ks;
            Pair {
                range,
                kind: PairKind::Pair {
                    hash_rocket: false,
                    value_omission: omission,
                    key_start: ks,
                    key_end: ke - 1,
                    op_start: ke - 1,
                    op_end: ke,
                    value_start: value.start_offset(),
                    value_end: value.end_offset(),
                },
            }
        }
    }

    /// `checkable_layout?`.
    fn checkable(&self, alignment: Alignment, pairs: &[Pair]) -> bool {
        if alignment == Alignment::Key {
            return true;
        }
        !self.pairs_on_same_line(pairs) && !self.mixed_delimiters(pairs)
    }

    fn pairs_on_same_line(&self, pairs: &[Pair]) -> bool {
        let assoc: Vec<&Pair> = pairs.iter().filter(|p| p.is_pair()).collect();
        assoc.windows(2).any(|w| self.same_line(w[0], w[1]))
    }

    fn mixed_delimiters(&self, pairs: &[Pair]) -> bool {
        let mut rocket = false;
        let mut colon = false;
        for p in pairs.iter().filter(|p| p.is_pair()) {
            if p.hash_rocket() {
                rocket = true;
            } else {
                colon = true;
            }
        }
        rocket && colon
    }

    /// `HashElementNode#same_line?`: `loc.last_line == other.loc.line ||
    /// loc.line == other.loc.last_line`.
    fn same_line(&self, a: &Pair, b: &Pair) -> bool {
        let a_first = self.line_of(a.range.0);
        let a_last = self.line_of(a.range.1.saturating_sub(1));
        let b_first = self.line_of(b.range.0);
        let b_last = self.line_of(b.range.1.saturating_sub(1));
        a_last == b_first || a_first == b_last
    }

    /// `begins_its_line?`: the range's column equals the first non-blank
    /// column of its line.
    fn begins_its_line(&self, off: usize) -> bool {
        let ls = self.line_index.line_start(off);
        let nonws = self.source[ls..]
            .iter()
            .take_while(|&&b| matches!(b, b' ' | b'\t'))
            .count();
        ls + nonws == off
    }

    fn check_pairs(&mut self, pairs: &[Pair]) {
        // New `on_hash` callback => new hash group for clobber confinement.
        self.group += 1;
        let first = pairs.iter().find(|p| p.is_pair()).unwrap();
        let ctx = HashCtx {
            max_key_width: self.max_key_width(pairs),
            max_delimiter_width: self.max_delimiter_width(pairs),
        };

        // `offenses_by` (class -> list of offending pair indices) and
        // `column_deltas` (class -> {pair index -> delta}). Every class on
        // which `check_delta` runs gets an entry even with zero offenses, so
        // `min_by length` ranges over all of them; a later write to the same
        // (class, pair) wins (the first-pair double-check).
        let mut by_class: Vec<ClassEntry> = Vec::new();
        let mut kwsplat: Vec<(usize, Delta)> = Vec::new();

        // First pair: `alignment_for(first_pair).each { deltas_for_first_pair }`.
        for &alignment in self.alignment_for(first) {
            let delta = self.delta_for_first_pair(alignment, first, &ctx);
            self.record(&mut by_class, alignment, 0, delta);
        }

        // `node.children.each` — every element, in source order.
        for (idx, current) in pairs.iter().enumerate() {
            if current.is_pair() {
                for &alignment in self.alignment_for(current) {
                    let delta = self.delta_current(alignment, first, current, &ctx);
                    self.record(&mut by_class, alignment, idx, delta);
                }
            } else {
                // `KeywordSplatAlignment#deltas`: `first.key_delta(current)`
                // (`:left`) is NOT short-circuited by the keyword-splat guard
                // (that only fires for `:right`), so it is `first.key.column -
                // kwsplat.column` (a kwsplat's `key` is the whole node), unless
                // they share a line. Reported via `correct_no_value`.
                if self.begins_its_line(current.range.0) {
                    let k = if self.same_line(first, current) {
                        0
                    } else {
                        self.col(first.key_start()) - self.col(current.range.0)
                    };
                    if k != 0 {
                        kwsplat.push((
                            idx,
                            Delta {
                                key: k,
                                separator: 0,
                                value: 0,
                            },
                        ));
                    }
                }
            }
        }

        // `add_offenses`: kwsplat offenses always, then the colon/rocket class
        // with the fewest offenses (`min_by length`, first wins on ties).
        for (idx, delta) in kwsplat {
            self.offenses
                .push(self.build_offense(&pairs[idx], 3, delta));
        }
        if let Some(winner) = by_class.iter().min_by_key(|e| e.offenders.len()) {
            let message = winner.class.message();
            // `register_offenses_with_format` iterates the (possibly
            // duplicated) node list; `add_offense` dedups by range. Emit each
            // pair once, in first-offending order.
            let mut seen: Vec<usize> = Vec::new();
            for &idx in &winner.offenders {
                if seen.contains(&idx) {
                    continue;
                }
                seen.push(idx);
                let pair = &pairs[idx];
                // The applied delta is read from `alignment_for(pair).first`'s
                // class (`column_deltas[alignment_for(offense).first.class]`),
                // i.e. the FIRST configured style for the pair's flavour — NOT
                // the winning class. Only membership and message come from the
                // winner.
                let first_class = self.alignment_for(pair)[0];
                let delta = by_class
                    .iter()
                    .find(|e| e.class == first_class)
                    .and_then(|e| e.deltas.iter().find(|(i, _)| *i == idx))
                    .map(|(_, d)| *d)
                    .unwrap_or(Delta {
                        key: 0,
                        separator: 0,
                        value: 0,
                    });
                self.offenses.push(self.build_offense(pair, message, delta));
            }
        }
    }

    /// Insert/replace a `(class, pair)` delta and update the class's offender
    /// list. Mirrors `check_delta`: the class entry is created even when the
    /// delta is good (zero), and a later delta for the same pair overwrites.
    fn record(&self, by_class: &mut Vec<ClassEntry>, class: Alignment, idx: usize, delta: Delta) {
        let entry = if let Some(e) = by_class.iter_mut().find(|e| e.class == class) {
            e
        } else {
            by_class.push(ClassEntry {
                class,
                offenders: Vec::new(),
                deltas: Vec::new(),
            });
            by_class.last_mut().unwrap()
        };
        let good = delta.key == 0 && delta.separator == 0 && delta.value == 0;
        // `check_delta` writes `column_deltas[class][node]` ONLY for a
        // non-good delta (it `return`s before the write otherwise). So a good
        // delta leaves no stored delta — and if such a pair is later reported
        // under a different winning class, `correct_node ... unless delta.nil?`
        // applies no correction. It also `push`es the node to
        // `offenses_by[class]` per offending check (so the first pair, checked
        // twice, can appear twice), and `min_by length` counts those
        // duplicates; the duplicates collapse only at `add_offense` time.
        if !good {
            if let Some(slot) = entry.deltas.iter_mut().find(|(i, _)| *i == idx) {
                slot.1 = delta;
            } else {
                entry.deltas.push((idx, delta));
            }
            entry.offenders.push(idx);
        }
    }

    fn build_offense(&self, pair: &Pair, message: u8, delta: Delta) -> HashAlignmentOffense {
        // `correct_node` key_value path: a pair with a value that is not value
        // omission. A kwsplat (no value) and value omission take the
        // `correct_no_value` path (adjust the node by the key delta).
        let has_value = pair.is_pair() && !pair.value_omission();
        HashAlignmentOffense {
            group: self.group,
            start_offset: pair.range.0,
            end_offset: pair.range.1,
            message,
            has_value,
            key_delta: delta.key,
            separator_delta: delta.separator,
            value_delta: delta.value,
            key_start: pair.key_start(),
            key_end: pair.key_end(),
            key_column: self.column(pair.key_start()),
            op_start: pair.op_start(),
            op_end: pair.op_end(),
            value_start: pair.value_start(),
            value_end: pair.value_end(),
        }
    }

    /// `alignment_for(pair)` for pairs (kwsplat handled separately).
    fn alignment_for(&self, pair: &Pair) -> &[Alignment] {
        if pair.hash_rocket() {
            &self.hash_rocket
        } else {
            &self.colon
        }
    }

    fn delta_for_first_pair(&self, alignment: Alignment, first: &Pair, ctx: &HashCtx) -> Delta {
        match alignment {
            Alignment::Key => Delta {
                key: 0,
                separator: self.key_sep_delta(first),
                value: self.key_val_delta(first),
            },
            Alignment::Separator => Delta {
                key: 0,
                separator: 0,
                value: 0,
            },
            Alignment::Table => {
                let sep = self.value_sep_delta(Alignment::Table, first, first, 0, ctx);
                Delta {
                    key: 0,
                    separator: sep,
                    value: self.table_val_delta(first, first, ctx) - sep,
                }
            }
        }
    }

    fn delta_current(
        &self,
        alignment: Alignment,
        first: &Pair,
        current: &Pair,
        ctx: &HashCtx,
    ) -> Delta {
        match alignment {
            Alignment::Key => {
                if !self.begins_its_line(current.range.0) {
                    return Delta {
                        key: 0,
                        separator: 0,
                        value: 0,
                    };
                }
                Delta {
                    key: self.key_delta_left(first, current),
                    separator: self.key_sep_delta(current),
                    value: self.key_val_delta(current),
                }
            }
            Alignment::Table => {
                let k = self.key_delta_left(first, current);
                let s = self.value_sep_delta(alignment, first, current, k, ctx);
                Delta {
                    key: k,
                    separator: s,
                    value: self.table_val_delta(first, current, ctx) - k - s,
                }
            }
            Alignment::Separator => {
                let k = self.sep_key_delta(first, current);
                let s = self.value_sep_delta(alignment, first, current, k, ctx);
                Delta {
                    key: k,
                    separator: s,
                    value: self.sep_val_delta(first, current) - k - s,
                }
            }
        }
    }

    // ---- delta primitives (parser columns) ----

    /// `KeyAlignment#separator_delta`.
    fn key_sep_delta(&self, pair: &Pair) -> isize {
        if pair.hash_rocket() {
            self.col(pair.key_end()) + 1 - self.col(pair.op_start())
        } else {
            0
        }
    }

    /// `KeyAlignment#value_delta`.
    fn key_val_delta(&self, pair: &Pair) -> isize {
        if pair.value_on_new_line(self) || pair.value_omission() {
            return 0;
        }
        self.col(pair.op_end()) + 1 - self.col(pair.value_start())
    }

    /// `HashElementDelta#key_delta(:left)`.
    fn key_delta_left(&self, first: &Pair, current: &Pair) -> isize {
        if self.same_line(first, current) {
            return 0;
        }
        self.col(first.key_start()) - self.col(current.key_start())
    }

    /// SeparatorAlignment `key_delta`: `:right`.
    fn sep_key_delta(&self, first: &Pair, current: &Pair) -> isize {
        if self.same_line(first, current) {
            return 0;
        }
        self.col(first.key_end()) - self.col(current.key_end())
    }

    /// `ValueAlignment#separator_delta`.
    fn value_sep_delta(
        &self,
        alignment: Alignment,
        first: &Pair,
        current: &Pair,
        key_delta: isize,
        ctx: &HashCtx,
    ) -> isize {
        if current.hash_rocket() {
            let hrd = match alignment {
                Alignment::Table => self.table_rocket_delta(first, current, ctx),
                Alignment::Separator => self.sep_rocket_delta(first, current),
                Alignment::Key => 0,
            };
            hrd - key_delta
        } else {
            0
        }
    }

    /// TableAlignment `hash_rocket_delta`.
    fn table_rocket_delta(&self, first: &Pair, current: &Pair, ctx: &HashCtx) -> isize {
        self.col(first.range.0) + ctx.max_key_width + 1 - self.col(current.op_start())
    }

    /// SeparatorAlignment `hash_rocket_delta`: `first.delimiter_delta(current)`.
    fn sep_rocket_delta(&self, first: &Pair, current: &Pair) -> isize {
        if self.same_line(first, current) {
            return 0;
        }
        if first.hash_rocket() != current.hash_rocket() {
            return 0;
        }
        self.col(first.op_start()) - self.col(current.op_start())
    }

    /// TableAlignment `value_delta`.
    fn table_val_delta(&self, first: &Pair, current: &Pair, ctx: &HashCtx) -> isize {
        if current.value_omission() {
            return 0;
        }
        let correct = self.col(first.key_start()) + ctx.max_key_width + ctx.max_delimiter_width;
        correct - self.col(current.value_start())
    }

    /// SeparatorAlignment `value_delta`: `first.value_delta(current)`.
    fn sep_val_delta(&self, first: &Pair, current: &Pair) -> isize {
        if current.value_omission() {
            return 0;
        }
        if self.same_line(first, current) {
            return 0;
        }
        self.col(first.value_start()) - self.col(current.value_start())
    }

    /// `max_key_width`: `keys.map { |k| k.source.length }.max` (parser keys, so
    /// colon excluded). Returns 0 for an empty pair set (never happens here).
    fn max_key_width(&self, pairs: &[Pair]) -> isize {
        pairs
            .iter()
            .filter(|p| p.is_pair())
            .map(|p| (p.key_end() - p.key_start()) as isize)
            .max()
            .unwrap_or(0)
    }

    /// `max_delimiter_width`: `pairs.map { |p| p.delimiter(true).length }.max`.
    /// `' => '` is 4, `': '` is 2.
    fn max_delimiter_width(&self, pairs: &[Pair]) -> isize {
        pairs
            .iter()
            .filter(|p| p.is_pair())
            .map(|p| if p.hash_rocket() { 4 } else { 2 })
            .max()
            .unwrap_or(0)
    }

}

/// `column_deltas` triple for one pair under one alignment.
#[derive(Clone, Copy)]
struct Delta {
    key: isize,
    separator: isize,
    value: isize,
}

/// `offenses_by[class]` + `column_deltas[class]` for one alignment class.
struct ClassEntry {
    class: Alignment,
    /// Offending pair indices, with the same duplicates stock's
    /// `offenses_by[class]` accumulates (the first pair is checked twice).
    offenders: Vec<usize>,
    /// Stored delta per offending pair index (last write wins).
    deltas: Vec<(usize, Delta)>,
}

/// Per-hash precomputed widths for table alignment.
struct HashCtx {
    max_key_width: isize,
    max_delimiter_width: isize,
}

/// Per-pair geometry.
struct Pair {
    range: (usize, usize),
    kind: PairKind,
}

enum PairKind {
    Pair {
        hash_rocket: bool,
        value_omission: bool,
        key_start: usize,
        key_end: usize,
        op_start: usize,
        op_end: usize,
        value_start: usize,
        value_end: usize,
    },
    Kwsplat,
}

impl Pair {
    fn is_pair(&self) -> bool {
        matches!(self.kind, PairKind::Pair { .. })
    }
    fn hash_rocket(&self) -> bool {
        matches!(
            self.kind,
            PairKind::Pair {
                hash_rocket: true,
                ..
            }
        )
    }
    fn value_omission(&self) -> bool {
        matches!(
            self.kind,
            PairKind::Pair {
                value_omission: true,
                ..
            }
        )
    }
    fn key_start(&self) -> usize {
        match self.kind {
            PairKind::Pair { key_start, .. } => key_start,
            PairKind::Kwsplat => self.range.0,
        }
    }
    fn key_end(&self) -> usize {
        match self.kind {
            PairKind::Pair { key_end, .. } => key_end,
            PairKind::Kwsplat => self.range.1,
        }
    }
    fn op_start(&self) -> usize {
        match self.kind {
            PairKind::Pair { op_start, .. } => op_start,
            PairKind::Kwsplat => self.range.0,
        }
    }
    fn op_end(&self) -> usize {
        match self.kind {
            PairKind::Pair { op_end, .. } => op_end,
            PairKind::Kwsplat => self.range.1,
        }
    }
    fn value_start(&self) -> usize {
        match self.kind {
            PairKind::Pair { value_start, .. } => value_start,
            PairKind::Kwsplat => self.range.0,
        }
    }
    fn value_end(&self) -> usize {
        match self.kind {
            PairKind::Pair { value_end, .. } => value_end,
            PairKind::Kwsplat => self.range.1,
        }
    }
    fn value_on_new_line(&self, v: &Visitor<'_>) -> bool {
        v.line_of(self.key_start()) != v.line_of(self.value_start())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(hr: u8, co: u8) -> Config {
        Config {
            hash_rocket_styles: vec![hr],
            colon_styles: vec![co],
            last_argument_style: 0,
            enforce_fixed_indentation: false,
        }
    }

    fn run(src: &str, cfg: &Config) -> Vec<(usize, usize, u8, isize, isize, isize)> {
        check_hash_alignment(src.as_bytes(), cfg)
            .into_iter()
            .map(|o| {
                (
                    o.start_offset,
                    o.end_offset,
                    o.message,
                    o.key_delta,
                    o.separator_delta,
                    o.value_delta,
                )
            })
            .collect()
    }

    #[test]
    fn key_style_misaligned_key() {
        // `   bb: 1` indented one too far: key delta -1.
        let got = run("hash1 = {\n  a: 0,\n   bb: 1\n}\n", &cfg(0, 0));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, 0); // key message
        assert_eq!(got[0].3, -1); // key delta
    }

    #[test]
    fn accepts_aligned() {
        assert!(run("h = {\n  a: 0,\n  bb: 1\n}\n", &cfg(0, 0)).is_empty());
        assert!(run("h = {}\n", &cfg(0, 0)).is_empty());
        assert!(run("h = { a: 0, bb: 1 }\n", &cfg(0, 0)).is_empty());
    }

    #[test]
    fn separator_style_rockets() {
        // separator: rockets aligned, keys right-aligned.
        let got = run(
            "hash = {\n    'a'  => 0,\n  'bbb' =>  1\n}\n",
            &cfg(1, 1),
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, 1); // separator message
    }

    #[test]
    fn table_style() {
        let got = run("hash = {\n  'a'   =>  0,\n  'bbb' => 1\n}\n", &cfg(2, 2));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, 2); // table message
    }

    #[test]
    fn kwsplat_alignment() {
        // misaligned kwsplat registers a kwsplat offense (key delta != 0).
        let got = run("{foo: 'bar',\n       **extra\n}\n", &cfg(0, 0));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].2, 3); // kwsplat message
    }

    #[test]
    fn distinct_hashes_get_distinct_groups() {
        // Two separate offending hashes => distinct group ids, so a clobber in
        // one cannot drop the other on the Ruby side.
        let offs = check_hash_alignment(
            "a = {\n  x: 0,\n   y: 1\n}\nb = {\n  p: 0,\n   q: 1\n}\n".as_bytes(),
            &cfg(0, 0),
        );
        assert_eq!(offs.len(), 2);
        assert_ne!(offs[0].group, offs[1].group);
        // Pairs of one hash share a group.
        let same = check_hash_alignment(
            "a = {\n  x: 0,\n   y: 1,\n    z: 2\n}\n".as_bytes(),
            &cfg(0, 0),
        );
        assert_eq!(same.len(), 2);
        assert_eq!(same[0].group, same[1].group);
    }
}

