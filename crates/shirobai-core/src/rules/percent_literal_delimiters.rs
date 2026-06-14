//! `Style/PercentLiteralDelimiters`: enforces consistent delimiters for
//! `%`-literals (`%w`, `%i`, `%r`, `%s`, `%x`, `%q`, `%Q`, `%W`, `%I`, `%`).
//!
//! Mirrors `vendor/rubocop/lib/rubocop/cop/style/percent_literal_delimiters.rb`
//! (plus its `PercentLiteral` mixin and the `PreferredDelimiters` helper class):
//!
//! - Identify the literal's type from `node.loc.begin.source[0..-2]`:
//!   `%`, `%q`, `%Q`, `%w`, `%W`, `%i`, `%I`, `%r`, `%s`, `%x`. In prism every
//!   percent literal's `opening_loc` starts with `%` and ends with the actual
//!   begin-delimiter byte (e.g. `%w[`, `%(`); the type prefix is everything
//!   except that last byte.
//! - For each type the per-cop config `PreferredDelimiters` map gives a
//!   2-char string `"<open><close>"` (e.g. `"[]"`, `"()"`). The wrapper passes
//!   those 10 strings (in [`PERCENT_LITERAL_TYPES`] order) verbatim.
//! - Skip the literal when:
//!   1. its actual begin delimiter equals the preferred open byte; OR
//!   2. any string-source child contains the preferred open or close byte
//!      (i.e. switching to the preferred pair would force escaping); OR
//!   3. for `%w` / `%i` only, any string-source child contains the begin
//!      delimiter's own pair (e.g. `%w(\(some\))` has `(`/`)` in the bytes,
//!      so flipping to `[]` is safe, but flipping to anything else would force
//!      escaping the existing `(`).
//! - Otherwise emit the offense and the autocorrect:
//!   - replace `opening_loc` (`%w(`) with `<type><open>` (`%w[`); and
//!   - replace the first byte of `closing_loc` with `<close>` (`)i` → `]i`
//!     keeps the regex options because stock's `node.loc.end` is the closer
//!     byte only).
//!
//! Children handed to the contains-delimiter check mirror the parser-gem AST
//! shape that stock walks (see the table in the implementation):
//!
//! | parser AST                | prism node                              |
//! |---------------------------|-----------------------------------------|
//! | `(str "...")` child       | `StringNode.content_loc` slice          |
//! | `(sym "...")` child       | `SymbolNode.value_loc` slice            |
//! | `(str "...")` content     | `StringNode.content_loc` slice          |
//! | `(sym ...)` plain symbol  | (not scanned — stock returns `nil`)     |
//! | `(regexp ... (regopt))`   | `RegularExpressionNode.content_loc`     |
//! | `(xstr (str "..."))`      | `XStringNode.content_loc`               |
//! | `(dstr (str "..."))` etc. | `InterpolatedXNode.parts` (StringNode only) |
//!
//! See [`PERCENT_LITERAL_TYPES`] for the canonical 10-entry type order shared
//! with [`crate::rules::bundle::BundleConfig`] / Ruby's `PreferredDelimiters`.

use ruby_prism::{Node, Visit};

/// The ten percent-literal types the cop recognises, in the canonical order
/// shared with stock's `PreferredDelimiters::PERCENT_LITERAL_TYPES` and the
/// `PreferredDelimiters` array packed into [`Config`].
pub const PERCENT_LITERAL_TYPES: [&str; 10] =
    ["%", "%i", "%I", "%q", "%Q", "%r", "%s", "%w", "%W", "%x"];

/// Per-type preferred delimiter pair. `0` is the opening byte, `1` the closing
/// byte. Both bytes are always ASCII in practice (the config strings stock
/// accepts are restricted to the four matching pairs `()`, `[]`, `{}`, `<>`
/// and a handful of single-byte mirror delimiters like `||`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelimPair {
    pub open: u8,
    pub close: u8,
}

/// Packed per-cop config: a 10-entry array of preferred delimiter pairs in
/// [`PERCENT_LITERAL_TYPES`] order. The Ruby wrapper resolves `default` plus
/// per-type overrides and hands us this fixed table.
#[derive(Debug, Clone)]
pub struct Config {
    /// `[%, %i, %I, %q, %Q, %r, %s, %w, %W, %x]` preferred pairs.
    pub pairs: [DelimPair; 10],
}

/// One offense, with everything the Ruby wrapper needs to build the offense
/// range + autocorrect ops.
#[derive(Debug, Clone)]
pub struct PercentLiteralDelimitersOffense {
    /// Inclusive start byte of the literal (= `node.loc.begin.start`).
    /// Also doubles as the offense range start (stock highlights the whole
    /// `node.loc.expression`, but caret-aligned spec checks usually point at
    /// `node.loc.expression.begin..end` — we hand back the full literal range
    /// so the wrapper can build it once).
    pub start_offset: usize,
    /// Exclusive end byte of the literal (= `node.loc.end.end` *including*
    /// regex options, mirroring `node.loc.expression.end_pos`).
    pub end_offset: usize,
    /// `[begin_start, begin_end)` of the opening token (`%w(` etc.), the
    /// autocorrect `replace` range for the new `<type><open>` text.
    pub begin_start: usize,
    pub begin_end: usize,
    /// `[end_start, end_end)` of the closing token's *first byte only*
    /// (stock's `node.loc.end` is the closer alone; for a regex with options
    /// like `%r(.*)i`, prism's `closing_loc` would also include the `i`, so we
    /// trim it back to one byte to preserve the options).
    pub end_start: usize,
    pub end_end: usize,
    /// Index into [`PERCENT_LITERAL_TYPES`] (and [`Config::pairs`]). The Ruby
    /// wrapper uses it to format the message (`` `%w`-literals should be …``)
    /// and look up the autocorrect text.
    pub type_index: u8,
}

/// Standalone entry: walk and collect offenses for one source.
pub fn check_percent_literal_delimiters(
    source: &[u8],
    cfg: &Config,
) -> Vec<PercentLiteralDelimitersOffense> {
    let mut rule = build_rule(source, cfg.clone());
    super::parse_cache::with_parsed(source, |_, node| rule.visit(node));
    rule.offenses
}

pub(crate) fn build_rule<'s>(source: &'s [u8], cfg: Config) -> Visitor<'s> {
    Visitor {
        source,
        cfg,
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'s> {
    source: &'s [u8],
    cfg: Config,
    pub(crate) offenses: Vec<PercentLiteralDelimitersOffense>,
}

impl<'s> Visitor<'s> {
    /// `process(node, *types)` in stock: only proceed if the node is a percent
    /// literal of a type we recognise. Returns the matched type index.
    fn percent_type(source: &[u8], opening_start: usize, opening_end: usize) -> Option<u8> {
        if opening_end <= opening_start || opening_end > source.len() {
            return None;
        }
        let opening = &source[opening_start..opening_end];
        if opening.first() != Some(&b'%') {
            return None;
        }
        // type = opening.source[0..-2]  (everything except the last byte)
        if opening.len() < 2 {
            return None;
        }
        let prefix = &opening[..opening.len() - 1];
        // Match the prefix against PERCENT_LITERAL_TYPES bytewise (every entry
        // is ASCII).
        PERCENT_LITERAL_TYPES
            .iter()
            .position(|t| t.as_bytes() == prefix)
            .map(|i| i as u8)
    }

    /// Common decision after we've identified a percent literal: skip /
    /// emit-offense. `children` are pre-collected str/sym source slices.
    /// Static method so the caller can borrow `self.source` for `children`.
    #[allow(clippy::too_many_arguments)]
    fn check_literal(
        offenses: &mut Vec<PercentLiteralDelimitersOffense>,
        cfg: &Config,
        source: &[u8],
        node_start: usize,
        node_end: usize,
        opening_start: usize,
        opening_end: usize,
        closing_start: usize,
        closing_end: usize,
        type_index: u8,
        children: &[&[u8]],
    ) {
        let pair = cfg.pairs[type_index as usize];
        // 1. uses_preferred_delimiter?  -- opening's last byte == preferred.open
        let begin_byte = source[opening_end - 1];
        if begin_byte == pair.open {
            return;
        }

        // 2. contains_preferred_delimiter? -- any child source contains preferred[0] or [1].
        let prefs: &[u8] = if pair.open == pair.close {
            // Avoid wasted work if the pair is the same byte (rare, e.g.
            // `||`); we still only need to scan for one distinct byte.
            std::slice::from_ref(&pair.open)
        } else {
            // Two distinct bytes packed in a 2-element ad-hoc array — we just
            // pass both.
            &[pair.open, pair.close][..]
        };
        if children.iter().any(|c| c.iter().any(|b| prefs.contains(b))) {
            return;
        }

        // 3. include_same_character_as_used_for_delimiter? — `%w` / `%i` only.
        if type_index == 7 /* %w */ || type_index == 1 /* %i */ {
            let used = matchpairs(begin_byte);
            if children
                .iter()
                .any(|c| c.iter().any(|b| used.contains(b)))
            {
                return;
            }
        }

        // Stock's `node.loc.end` is the closer byte (regex options excluded)
        // even though prism's `closing_loc` may also span them. Snap the
        // end-replacement range to a single byte.
        let end_byte_end = (closing_start + 1).min(closing_end);
        offenses.push(PercentLiteralDelimitersOffense {
            start_offset: node_start,
            end_offset: node_end,
            begin_start: opening_start,
            begin_end: opening_end,
            end_start: closing_start,
            end_end: end_byte_end,
            type_index,
        });
    }

    fn check_string_node(&mut self, n: &ruby_prism::StringNode<'_>) {
        let Some(opening) = n.opening_loc() else { return };
        let opening_start = opening.start_offset();
        let opening_end = opening.end_offset();
        let Some(type_index) = Self::percent_type(self.source, opening_start, opening_end)
        else {
            return;
        };
        // Match stock's `on_str` registration: only `%`, `%Q`, `%q`.
        let allowed: &[u8] = &[0 /* % */, 3 /* %q */, 4 /* %Q */];
        if !allowed.contains(&type_index) {
            return;
        }
        let Some(closing) = n.closing_loc() else { return };
        let content = if let Some(cloc) = Some(n.content_loc()) {
            // Stock string_source returns scrub'd raw `String` content; the
            // raw bytes between opening and closing are what we sweep for
            // preferred delimiters.
            &self.source[cloc.start_offset()..cloc.end_offset()]
        } else {
            &[]
        };
        let children: [&[u8]; 1] = [content];
        let loc = n.location();
        Self::check_literal(
            &mut self.offenses,
            &self.cfg,
            self.source,
            loc.start_offset(),
            loc.end_offset(),
            opening_start,
            opening_end,
            closing.start_offset(),
            closing.end_offset(),
            type_index,
            &children,
        );
    }

    fn check_interpolated_string_node(&mut self, n: &ruby_prism::InterpolatedStringNode<'_>) {
        // Stock dispatches `on_dstr` (aliased from `on_str`) for `%(...)` /
        // `%Q(...)` with interpolation. `%q` never interpolates so it cannot
        // reach this branch.
        let Some(opening) = n.opening_loc() else { return };
        let opening_start = opening.start_offset();
        let opening_end = opening.end_offset();
        let Some(type_index) = Self::percent_type(self.source, opening_start, opening_end)
        else {
            return;
        };
        let allowed: &[u8] = &[0 /* % */, 4 /* %Q */];
        if !allowed.contains(&type_index) {
            return;
        }
        let Some(closing) = n.closing_loc() else { return };
        let mut buf: Vec<&[u8]> = Vec::new();
        for part in n.parts().iter() {
            if let Some(s) = part.as_string_node() {
                let cloc = s.content_loc();
                buf.push(&self.source[cloc.start_offset()..cloc.end_offset()]);
            }
        }
        let loc = n.location();
        Self::check_literal(
            &mut self.offenses,
            &self.cfg,
            self.source,
            loc.start_offset(),
            loc.end_offset(),
            opening_start,
            opening_end,
            closing.start_offset(),
            closing.end_offset(),
            type_index,
            &buf,
        );
    }

    fn check_symbol_node(&mut self, n: &ruby_prism::SymbolNode<'_>) {
        // Stock `on_sym` only registers `%s`. A `%s[symbol]` literal in parser
        // is `(sym :symbol)` whose `children = [:symbol]` — a plain Symbol
        // primitive that `string_source` returns `nil` for. So we scan no
        // children at all for the contains check.
        let Some(opening) = n.opening_loc() else { return };
        let opening_start = opening.start_offset();
        let opening_end = opening.end_offset();
        let Some(type_index) = Self::percent_type(self.source, opening_start, opening_end)
        else {
            return;
        };
        if type_index != 6 /* %s */ {
            return;
        }
        let Some(closing) = n.closing_loc() else { return };
        let loc = n.location();
        Self::check_literal(
            &mut self.offenses,
            &self.cfg,
            self.source,
            loc.start_offset(),
            loc.end_offset(),
            opening_start,
            opening_end,
            closing.start_offset(),
            closing.end_offset(),
            type_index,
            &[],
        );
    }

    fn check_xstring_node(&mut self, n: &ruby_prism::XStringNode<'_>) {
        let Some(type_index) = Self::percent_type(
            self.source,
            n.opening_loc().start_offset(),
            n.opening_loc().end_offset(),
        ) else {
            return;
        };
        if type_index != 9 /* %x */ {
            return;
        }
        let cloc = n.content_loc();
        let content = &self.source[cloc.start_offset()..cloc.end_offset()];
        let children: [&[u8]; 1] = [content];
        let opening = n.opening_loc();
        let closing = n.closing_loc();
        let loc = n.location();
        Self::check_literal(
            &mut self.offenses,
            &self.cfg,
            self.source,
            loc.start_offset(),
            loc.end_offset(),
            opening.start_offset(),
            opening.end_offset(),
            closing.start_offset(),
            closing.end_offset(),
            type_index,
            &children,
        );
    }

    fn check_interpolated_xstring_node(
        &mut self,
        n: &ruby_prism::InterpolatedXStringNode<'_>,
    ) {
        let opening = n.opening_loc();
        let Some(type_index) =
            Self::percent_type(self.source, opening.start_offset(), opening.end_offset())
        else {
            return;
        };
        if type_index != 9 /* %x */ {
            return;
        }
        let closing = n.closing_loc();
        let mut buf: Vec<&[u8]> = Vec::new();
        for part in n.parts().iter() {
            if let Some(s) = part.as_string_node() {
                let cloc = s.content_loc();
                buf.push(&self.source[cloc.start_offset()..cloc.end_offset()]);
            }
        }
        let loc = n.location();
        Self::check_literal(
            &mut self.offenses,
            &self.cfg,
            self.source,
            loc.start_offset(),
            loc.end_offset(),
            opening.start_offset(),
            opening.end_offset(),
            closing.start_offset(),
            closing.end_offset(),
            type_index,
            &buf,
        );
    }

    fn check_regexp_node(&mut self, n: &ruby_prism::RegularExpressionNode<'_>) {
        let opening = n.opening_loc();
        let Some(type_index) =
            Self::percent_type(self.source, opening.start_offset(), opening.end_offset())
        else {
            return;
        };
        if type_index != 5 /* %r */ {
            return;
        }
        let closing = n.closing_loc();
        let cloc = n.content_loc();
        let content = &self.source[cloc.start_offset()..cloc.end_offset()];
        let children: [&[u8]; 1] = [content];
        let loc = n.location();
        Self::check_literal(
            &mut self.offenses,
            &self.cfg,
            self.source,
            loc.start_offset(),
            loc.end_offset(),
            opening.start_offset(),
            opening.end_offset(),
            closing.start_offset(),
            closing.end_offset(),
            type_index,
            &children,
        );
    }

    fn check_interpolated_regexp_node(
        &mut self,
        n: &ruby_prism::InterpolatedRegularExpressionNode<'_>,
    ) {
        let opening = n.opening_loc();
        let Some(type_index) =
            Self::percent_type(self.source, opening.start_offset(), opening.end_offset())
        else {
            return;
        };
        if type_index != 5 /* %r */ {
            return;
        }
        let closing = n.closing_loc();
        let mut buf: Vec<&[u8]> = Vec::new();
        for part in n.parts().iter() {
            if let Some(s) = part.as_string_node() {
                let cloc = s.content_loc();
                buf.push(&self.source[cloc.start_offset()..cloc.end_offset()]);
            }
        }
        let loc = n.location();
        Self::check_literal(
            &mut self.offenses,
            &self.cfg,
            self.source,
            loc.start_offset(),
            loc.end_offset(),
            opening.start_offset(),
            opening.end_offset(),
            closing.start_offset(),
            closing.end_offset(),
            type_index,
            &buf,
        );
    }

    fn check_array_node(&mut self, n: &ruby_prism::ArrayNode<'_>) {
        let Some(opening) = n.opening_loc() else { return };
        let opening_start = opening.start_offset();
        let opening_end = opening.end_offset();
        let Some(type_index) = Self::percent_type(self.source, opening_start, opening_end)
        else {
            return;
        };
        // Stock `on_array` registers `%w`, `%W`, `%i`, `%I`.
        let allowed: &[u8] = &[7 /* %w */, 8 /* %W */, 1 /* %i */, 2 /* %I */];
        if !allowed.contains(&type_index) {
            return;
        }
        let Some(closing) = n.closing_loc() else { return };
        let mut buf: Vec<&[u8]> = Vec::new();
        for el in n.elements().iter() {
            match el {
                Node::StringNode { .. } => {
                    let s = el.as_string_node().unwrap();
                    let cloc = s.content_loc();
                    buf.push(&self.source[cloc.start_offset()..cloc.end_offset()]);
                }
                Node::SymbolNode { .. } => {
                    let s = el.as_symbol_node().unwrap();
                    if let Some(vloc) = s.value_loc() {
                        buf.push(&self.source[vloc.start_offset()..vloc.end_offset()]);
                    }
                }
                // Stock's `string_source` returns `nil` for a `(dstr ...)` /
                // `(dsym ...)` child whose `type` is neither `:str` nor
                // `:sym` — even when one of its parts is a literal string with
                // a preferred-delimiter byte. So an array element that *as a
                // whole* is interpolated (e.g. `%W{ #{from}[0] }`) is invisible
                // to the contains check, and the offense must fire.
                //
                // We must NOT descend into the parts here. Walking the parts
                // and pushing their `StringNode` content used to make
                // `%W{ #{from}[0] }` look like it contained `[`, suppressing
                // the offense (Discourse `app/models/optimized_image.rb`
                // L269/L300 — fixed 2026-06-15).
                Node::InterpolatedStringNode { .. }
                | Node::InterpolatedSymbolNode { .. } => {}
                _ => {}
            }
        }
        let loc = n.location();
        Self::check_literal(
            &mut self.offenses,
            &self.cfg,
            self.source,
            loc.start_offset(),
            loc.end_offset(),
            opening_start,
            opening_end,
            closing.start_offset(),
            closing.end_offset(),
            type_index,
            &buf,
        );
    }
}

/// `PercentLiteralDelimiters#matchpairs`: the pair characters used by
/// `include_same_character_as_used_for_delimiter?` for `%w` / `%i`.
fn matchpairs(begin: u8) -> [u8; 2] {
    match begin {
        b'(' => [b'(', b')'],
        b'[' => [b'[', b']'],
        b'{' => [b'{', b'}'],
        b'<' => [b'<', b'>'],
        c => [c, c],
    }
}

impl<'pr> Visit<'pr> for Visitor<'_> {
    fn visit_string_node(&mut self, n: &ruby_prism::StringNode<'pr>) {
        self.check_string_node(n);
        ruby_prism::visit_string_node(self, n);
    }
    fn visit_interpolated_string_node(
        &mut self,
        n: &ruby_prism::InterpolatedStringNode<'pr>,
    ) {
        self.check_interpolated_string_node(n);
        ruby_prism::visit_interpolated_string_node(self, n);
    }
    fn visit_symbol_node(&mut self, n: &ruby_prism::SymbolNode<'pr>) {
        self.check_symbol_node(n);
        ruby_prism::visit_symbol_node(self, n);
    }
    fn visit_x_string_node(&mut self, n: &ruby_prism::XStringNode<'pr>) {
        self.check_xstring_node(n);
        ruby_prism::visit_x_string_node(self, n);
    }
    fn visit_interpolated_x_string_node(
        &mut self,
        n: &ruby_prism::InterpolatedXStringNode<'pr>,
    ) {
        self.check_interpolated_xstring_node(n);
        ruby_prism::visit_interpolated_x_string_node(self, n);
    }
    fn visit_regular_expression_node(
        &mut self,
        n: &ruby_prism::RegularExpressionNode<'pr>,
    ) {
        self.check_regexp_node(n);
        ruby_prism::visit_regular_expression_node(self, n);
    }
    fn visit_interpolated_regular_expression_node(
        &mut self,
        n: &ruby_prism::InterpolatedRegularExpressionNode<'pr>,
    ) {
        self.check_interpolated_regexp_node(n);
        ruby_prism::visit_interpolated_regular_expression_node(self, n);
    }
    fn visit_array_node(&mut self, n: &ruby_prism::ArrayNode<'pr>) {
        self.check_array_node(n);
        ruby_prism::visit_array_node(self, n);
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn enter(&mut self, node: &Node<'_>) {
        match node {
            Node::InterpolatedStringNode { .. } => {
                let n = node.as_interpolated_string_node().unwrap();
                self.check_interpolated_string_node(&n);
            }
            Node::InterpolatedXStringNode { .. } => {
                let n = node.as_interpolated_x_string_node().unwrap();
                self.check_interpolated_xstring_node(&n);
            }
            Node::InterpolatedRegularExpressionNode { .. } => {
                let n = node.as_interpolated_regular_expression_node().unwrap();
                self.check_interpolated_regexp_node(&n);
            }
            Node::ArrayNode { .. } => {
                let n = node.as_array_node().unwrap();
                self.check_array_node(&n);
            }
            Node::RegularExpressionNode { .. } => {
                let n = node.as_regular_expression_node().unwrap();
                self.check_regexp_node(&n);
            }
            Node::XStringNode { .. } => {
                let n = node.as_x_string_node().unwrap();
                self.check_xstring_node(&n);
            }
            Node::SymbolNode { .. } => {
                let n = node.as_symbol_node().unwrap();
                self.check_symbol_node(&n);
            }
            _ => {}
        }
    }
    fn leave(&mut self) {}
    fn enter_leaf(&mut self, node: &Node<'_>) {
        match node {
            Node::StringNode { .. } => {
                let n = node.as_string_node().unwrap();
                self.check_string_node(&n);
            }
            Node::SymbolNode { .. } => {
                let n = node.as_symbol_node().unwrap();
                self.check_symbol_node(&n);
            }
            Node::RegularExpressionNode { .. } => {
                let n = node.as_regular_expression_node().unwrap();
                self.check_regexp_node(&n);
            }
            Node::XStringNode { .. } => {
                let n = node.as_x_string_node().unwrap();
                self.check_xstring_node(&n);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> Config {
        // RuboCop default: `default => ()`, %i/%I/%w/%W => [], %r => {}.
        // Order = PERCENT_LITERAL_TYPES.
        let p = |o, c| DelimPair { open: o, close: c };
        Config {
            pairs: [
                p(b'(', b')'), // %
                p(b'[', b']'), // %i
                p(b'[', b']'), // %I
                p(b'(', b')'), // %q
                p(b'(', b')'), // %Q
                p(b'{', b'}'), // %r
                p(b'(', b')'), // %s
                p(b'[', b']'), // %w
                p(b'[', b']'), // %W
                p(b'(', b')'), // %x
            ],
        }
    }

    fn all_brackets() -> Config {
        let p = |o, c| DelimPair { open: o, close: c };
        Config {
            pairs: [p(b'[', b']'); 10],
        }
    }

    fn run_one(src: &str, cfg: &Config) -> Vec<PercentLiteralDelimitersOffense> {
        check_percent_literal_delimiters(src.as_bytes(), cfg)
    }

    #[test]
    fn flags_w_with_paren_when_brackets_preferred() {
        let off = run_one("%w(a b)\n", &all_brackets());
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].type_index, 7);
        assert_eq!(off[0].begin_start, 0);
        assert_eq!(off[0].begin_end, 3);
        assert_eq!(off[0].end_start, 6);
        assert_eq!(off[0].end_end, 7);
    }

    #[test]
    fn accepts_already_preferred() {
        assert!(run_one("%w[a b]\n", &all_brackets()).is_empty());
    }

    #[test]
    fn accepts_when_preferred_char_is_inside() {
        // `%w([some] [words])` contains `[` and `]` so flipping to `[]` would
        // require escaping.
        assert!(run_one("%w([some] [words])\n", &all_brackets()).is_empty());
        assert!(run_one("%([string])\n", &all_brackets()).is_empty());
    }

    #[test]
    fn accepts_when_pair_char_already_inside_for_w() {
        // matchpairs(`(`) = (`(`, `)`), content has `\(` and `\)` => skip.
        assert!(run_one("%w(\\(some words\\))\n", &all_brackets()).is_empty());
        assert!(run_one("%i(\\(\\) each)\n", &all_brackets()).is_empty());
    }

    #[test]
    fn flags_string_with_interpolation() {
        let off = run_one("%(string)\n", &all_brackets());
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].type_index, 0);
        // Closing replacement range is one byte.
        assert_eq!(off[0].end_end - off[0].end_start, 1);
    }

    #[test]
    fn flags_regexp_with_options_preserves_options() {
        let off = run_one("%r(.*)i\n", &all_brackets());
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].type_index, 5);
        // end_start points at `)`; end_end is one byte after, leaving `i` alone.
        assert_eq!(off[0].end_end - off[0].end_start, 1);
    }

    #[test]
    fn flags_symbol_literal() {
        let off = run_one("%s(symbol)\n", &all_brackets());
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].type_index, 6);
    }

    #[test]
    fn flags_xstring() {
        let off = run_one("%x(command)\n", &all_brackets());
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].type_index, 9);
    }

    #[test]
    fn defaults_w_brackets_no_offense() {
        assert!(run_one("%w[a b]\n", &default_config()).is_empty());
        assert!(run_one("%i[a]\n", &default_config()).is_empty());
        assert!(run_one("%r{re}\n", &default_config()).is_empty());
    }

    #[test]
    fn defaults_q_paren_no_offense() {
        assert!(run_one("%q(s)\n", &default_config()).is_empty());
        assert!(run_one("%Q(s)\n", &default_config()).is_empty());
        assert!(run_one("%(s)\n", &default_config()).is_empty());
    }

    #[test]
    fn flags_q_with_brackets_against_default() {
        let off = run_one("%q[s]\n", &default_config());
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].type_index, 3);
    }

    #[test]
    fn empty_string_with_interpolation_form_is_a_dstr() {
        // `%()` is parsed by prism as InterpolatedStringNode (no parts). The
        // contains check should see no children and the offense should still
        // fire.
        let off = run_one("%()\n", &all_brackets());
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].type_index, 0);
    }

    #[test]
    fn interpolation_preferred_chars_inside_does_not_skip() {
        // `%(#{[1].first})` has only an EmbeddedStatementsNode part — no
        // StringNode parts to scan, so contains is false and the offense fires.
        // `[`/`]` inside the *interpolation* don't count (stock's
        // `string_source` returns nil for a `:begin` child).
        let off = run_one("%(#{[1].first})\n", &all_brackets());
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].type_index, 0);
    }

    #[test]
    fn dstr_element_in_w_array_does_not_skip_via_inner_string() {
        // Regression: `%W{ #{x}[0] }` is `(array (dstr (begin ...) (str "[0]")))`
        // in parser. Stock's `string_source` returns `nil` for the dstr child
        // (type is :dstr, not :str/:sym), so `[` inside the dstr is invisible
        // to `contains_preferred_delimiter?` — the offense fires.
        // Discourse `app/models/optimized_image.rb` L269/L300.
        let off = run_one("%W{ #{x}[0] }\n", &all_brackets());
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].type_index, 8 /* %W */);
    }

    #[test]
    fn dsym_element_in_i_capital_array_does_not_skip() {
        // Same shape for %I (interpolated symbol array).
        let off = run_one("%I{ #{x}[0] }\n", &all_brackets());
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].type_index, 2 /* %I */);
    }

    #[test]
    fn plain_string_element_in_w_array_still_skips_when_inside() {
        // Non-regression: a plain `str` element with `[` IS scanned (stock's
        // string_source returns the source for :str). The offense is skipped.
        assert!(run_one("%w{ a[b] c }\n", &all_brackets()).is_empty());
    }

    #[test]
    fn dstr_element_does_not_block_matchpairs_check_either() {
        // include_same_character_as_used_for_delimiter? for %w / %i also runs
        // through the same `string_source`-based scan, so a dstr element with
        // `{` (the begin delimiter's pair) likewise stays invisible — offense
        // still fires.
        let off = run_one("%W{ #{x}{ }\n", &all_brackets());
        assert_eq!(off.len(), 1);
        assert_eq!(off[0].type_index, 8 /* %W */);
    }

    #[test]
    fn types_indexed_correctly() {
        for (i, ty) in PERCENT_LITERAL_TYPES.iter().enumerate() {
            // Build a literal like `%w(x)`, parse, expect type_index == i.
            let src = format!("{ty}(x)\n");
            let off = run_one(&src, &all_brackets());
            assert_eq!(off.len(), 1, "type {ty}");
            assert_eq!(off[0].type_index as usize, i, "type {ty}");
        }
    }
}
