//! `Style/HashTransformKeys`.
//!
//! Detects four shapes of "rewriting only the keys of a hash" that stock's
//! `Style/HashTransformKeys` flags via the `HashTransformMethod` mixin, and
//! emits the exact corrector ops `Autocorrection.{strip_prefix_and_suffix,
//! set_new_method_name, set_new_arg_name, set_new_body_expression}` produce
//! on stock. The shapes are:
//!
//! 1. `_.each_with_object({}) { |(k, v), h| h[transform(k)] = v }`
//! 2. `Hash[_.map { |k, v| [transform(k), v] }]`
//! 3. `_.map { |k, v| [transform(k), v] }.to_h`
//! 4. `_.to_h { |k, v| [transform(k), v] }` (Ruby 2.6+ form)
//!
//! Stock's pattern checks `hash_receiver?` on the inner receiver and runs
//! three negative guards (`noop_transformation?`, `transformation_uses_both_args?`,
//! `use_transformed_argname?`) on the captures before adding an offense.

use ruby_prism::{BlockNode, CallNode, Location, Node, Visit};

/// One match. `edits` is the ordered list of `corrector.replace(range, text)`
/// operations the Ruby wrapper plays back verbatim; ranges never overlap by
/// construction (each touches a different substring of the offense's source
/// span).
pub struct HashTransformKeysOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: String,
    pub edits: Vec<(usize, usize, String)>,
}

pub fn check_hash_transform_keys(source: &[u8]) -> Vec<HashTransformKeysOffense> {
    let mut rule = build_rule(source);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

pub(crate) fn build_rule(source: &[u8]) -> Visitor<'_> {
    Visitor {
        source,
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    pub(crate) offenses: Vec<HashTransformKeysOffense>,
}

/// `HashTransformMethod#RESTRICT_ON_SEND = [:[], :to_h]`. We mirror that
/// gate before going pattern matching so unrelated calls pay nothing.
fn restrict_on_send(name: &[u8]) -> bool {
    name == b"[]" || name == b"to_h"
}

/// `hash_receiver?` mixin (`HashTransformMethod`): a node Stock treats as a
/// "definitely a hash" receiver — hash literals, the known hash-producing
/// senders, and their block forms (`group_by`, `to_h`, `tally`, the
/// `transform_*` family, `each_with_object({})`).
fn hash_receiver(node: &Node<'_>) -> bool {
    if node.as_hash_node().is_some() {
        return true;
    }
    if let Some(call) = node.as_call_node() {
        const PLAIN_SENDS: &[&[u8]] = &[
            b"to_h", b"to_hash", b"merge", b"merge!", b"update", b"invert", b"except", b"tally",
        ];
        if call.block().is_none() && PLAIN_SENDS.contains(&call.name().as_slice()) {
            return true;
        }
        // `(block (send _ {...}) ...)`: in prism the outer CallNode carries
        // `.block` directly. Stock's pattern uses parser's `block` view which
        // ALSO matches lambda literals — but lambdas are `LambdaNode` in prism
        // (a separate AST kind that does not present a CallNode); since this
        // helper takes a Node and we only check CallNode here, lambdas as
        // receivers are silently rejected. That matches stock in practice
        // because no realistic lambda call produces a hash-typed receiver
        // for `map.to_h`/`each_with_object` here.
        if let Some(block) = call.block()
            && block.as_block_node().is_some()
        {
            const BLOCK_SENDS: &[&[u8]] = &[
                b"group_by",
                b"to_h",
                b"tally",
                b"transform_keys",
                b"transform_keys!",
                b"transform_values",
                b"transform_values!",
            ];
            let name = call.name().as_slice();
            if BLOCK_SENDS.contains(&name) {
                return true;
            }
            if name == b"each_with_object" && eo_one_empty_hash_arg(&call) {
                return true;
            }
        }
    }
    false
}

/// `each_with_object` with a single empty `{}` literal as its sole argument.
fn eo_one_empty_hash_arg(call: &CallNode<'_>) -> bool {
    let args = call
        .arguments()
        .map(|a| a.arguments())
        .filter(|args| args.iter().count() == 1);
    let Some(args) = args else { return false };
    let first = args.iter().next().expect("len checked");
    first
        .as_hash_node()
        .is_some_and(|h| h.elements().iter().count() == 0)
}

/// Walks `root` and reports a hit on any `LocalVariableReadNode` whose
/// location starts at `target_offset` (parser identity is location-equality
/// for prism nodes that come from the same parse).
fn contains_lvar_node(root: &Node<'_>, target_offset: usize) -> bool {
    struct Scan {
        target: usize,
        found: bool,
    }
    impl<'pr> Visit<'pr> for Scan {
        fn visit_local_variable_read_node(
            &mut self,
            node: &ruby_prism::LocalVariableReadNode<'pr>,
        ) {
            if !self.found && node.location().start_offset() == self.target {
                self.found = true;
            }
        }
    }
    let mut scan = Scan {
        target: target_offset,
        found: false,
    };
    scan.visit(root);
    scan.found
}

/// `each_descendant(:lvar).any? { |n| n.source == transformed_argname.to_s }`:
/// the root node itself is excluded (we dodge it via location identity even
/// when `Visit` dispatches to it as the typed visitor entry).
fn any_strict_descendant_lvar_named(root: &Node<'_>, name: &[u8]) -> bool {
    struct Scan<'a> {
        target: &'a [u8],
        root_start: usize,
        root_end: usize,
        found: bool,
    }
    impl<'pr> Visit<'pr> for Scan<'_> {
        fn visit_local_variable_read_node(
            &mut self,
            node: &ruby_prism::LocalVariableReadNode<'pr>,
        ) {
            if self.found {
                return;
            }
            let loc = node.location();
            let is_root = loc.start_offset() == self.root_start
                && loc.end_offset() == self.root_end;
            if !is_root && node.name().as_slice() == self.target {
                self.found = true;
            }
        }
    }
    let root_loc = root.location();
    let mut scan = Scan {
        target: name,
        root_start: root_loc.start_offset(),
        root_end: root_loc.end_offset(),
        found: false,
    };
    scan.visit(root);
    scan.found
}

/// `(lvar _name)`: a bare local variable read whose name matches `expected`.
fn match_lvar_named(node: &Node<'_>, expected: &[u8]) -> bool {
    node.as_local_variable_read_node()
        .is_some_and(|l| l.name().as_slice() == expected)
}

impl<'a> Visitor<'a> {
    fn loc_text<'b>(&'b self, loc: &Location<'_>) -> &'b [u8] {
        &self.source[loc.start_offset()..loc.end_offset()]
    }

    fn emit(
        &mut self,
        node_loc: Location<'_>,
        match_desc: &str,
        edits: Vec<(usize, usize, String)>,
    ) {
        self.offenses.push(HashTransformKeysOffense {
            start_offset: node_loc.start_offset(),
            end_offset: node_loc.end_offset(),
            message: format!("Prefer `transform_keys` over `{match_desc}`."),
            edits,
        });
    }

    /// Top-level CallNode dispatch. Stock's mixin has on_block / on_send /
    /// on_csend; we collapse them to a single CallNode visit:
    ///
    /// - CallNode `each_with_object` (block-bearing) ⇒ pattern 1
    /// - CallNode `to_h` with block whose receiver is `hash_receiver?` ⇒ pattern 4
    /// - CallNode `to_h` (block-bearing OR not) whose receiver is a block-
    ///   bearing `_.map { ... }` ⇒ pattern 3 (parser fires `on_send(to_h)`
    ///   even when the outer `to_h` carries a block of its own — the send
    ///   node and the block node are separate parser entities)
    /// - CallNode `[]` on `Hash` ⇒ pattern 2
    fn process_call<'pr>(&mut self, call: &CallNode<'pr>) {
        let name = call.name().as_slice();
        if name == b"each_with_object" && call.block().is_some() {
            self.try_each_with_object(call);
            return;
        }
        if name == b"to_h" {
            // Try the `to_h{}` block form first (matches when receiver is a
            // hash). Try `_.map{...}.to_h` next (matches when receiver is a
            // block-bearing `map`); the two are mutually exclusive on the
            // receiver shape so we never double-flag.
            if call.block().is_some() {
                self.try_to_h_block(call);
            }
            self.try_map_to_h(call);
            return;
        }
        if !restrict_on_send(name) {
            return;
        }
        if name == b"[]" {
            self.try_hash_brackets_map(call);
        }
    }

    // ------------------------------------------------------------------
    // Pattern 1: `each_with_object`
    // ------------------------------------------------------------------
    fn try_each_with_object<'pr>(&mut self, call: &CallNode<'pr>) {
        let Some(block_node) = call.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        let Some(check) = self.eligible_each_with_object(call, &block_node) else {
            return;
        };
        // Build the edits while `block_node` is alive.
        let edits = self.build_each_with_object_edits(&block_node, call, &check);
        self.emit(call.as_node().location(), "each_with_object", edits);
    }

    /// Walks the `each_with_object` shape end-to-end and returns the
    /// captures-derived data (`transformed_argname` + body source) when stock
    /// would flag an offense.
    fn eligible_each_with_object<'pr>(
        &self,
        call: &CallNode<'pr>,
        block: &BlockNode<'pr>,
    ) -> Option<EligibleCaptures> {
        if !eo_one_empty_hash_arg(call) {
            return None;
        }
        let recv = call.receiver()?;
        if !hash_receiver(&recv) {
            return None;
        }
        let params = block.parameters()?.as_block_parameters_node()?;
        let p = params.parameters()?;
        if p.optionals().iter().count() != 0
            || p.rest().is_some()
            || p.posts().iter().count() != 0
            || p.keywords().iter().count() != 0
            || p.keyword_rest().is_some()
            || p.block().is_some()
            || params.locals().iter().count() != 0
        {
            return None;
        }
        let requireds: Vec<Node<'pr>> = p.requireds().iter().collect();
        if requireds.len() != 2 {
            return None;
        }
        let mlhs = requireds[0].as_multi_target_node()?;
        if mlhs.rest().is_some() || mlhs.rights().iter().count() != 0 {
            return None;
        }
        let mlhs_lefts: Vec<Node<'pr>> = mlhs.lefts().iter().collect();
        if mlhs_lefts.len() != 2 {
            return None;
        }
        let k_param = mlhs_lefts[0].as_required_parameter_node()?;
        let val_param = mlhs_lefts[1].as_required_parameter_node()?;
        let memo_param = requireds[1].as_required_parameter_node()?;
        let key_argname = k_param.name().as_slice();
        let val_argname = val_param.name().as_slice();
        let memo_argname = memo_param.name().as_slice();

        let body = block.body()?;
        let stmts = body.as_statements_node()?;
        let body_calls: Vec<Node<'pr>> = stmts.body().iter().collect();
        if body_calls.len() != 1 {
            return None;
        }
        let body_call = body_calls[0].as_call_node()?;
        if body_call.name().as_slice() != b"[]=" {
            return None;
        }
        if body_call.is_safe_navigation() {
            return None;
        }
        let body_recv = body_call.receiver()?;
        if !match_lvar_named(&body_recv, memo_argname) {
            return None;
        }
        let args = body_call.arguments()?;
        let body_args: Vec<Node<'pr>> = args.arguments().iter().collect();
        if body_args.len() != 2 {
            return None;
        }
        let key_body_expr = &body_args[0];
        let val_body_capture = &body_args[1];
        // `$!`_memo` anchor: key_body_expr does NOT reference the memo lvar.
        if any_strict_descendant_lvar_named(key_body_expr, memo_argname)
            || match_lvar_named(key_body_expr, memo_argname)
        {
            return None;
        }
        if !match_lvar_named(val_body_capture, val_argname) {
            return None;
        }
        // Three negative guards.
        if let Some(lv) = key_body_expr.as_local_variable_read_node()
            && lv.name().as_slice() == key_argname
        {
            // noop
            return None;
        }
        let val_offset = val_body_capture.location().start_offset();
        if contains_lvar_node(key_body_expr, val_offset) {
            // transformation_uses_both_args?
            return None;
        }
        if !any_strict_descendant_lvar_named(key_body_expr, key_argname) {
            // use_transformed_argname? returned false
            return None;
        }
        let key_loc = key_body_expr.location();
        let body_text = self.body_source_with_braces_at(key_body_expr, &key_loc);
        Some(EligibleCaptures {
            transformed_argname: key_argname.to_vec(),
            body_text,
            body_start: key_loc.start_offset(),
            body_end: key_loc.end_offset(),
        })
    }

    fn build_each_with_object_edits<'pr>(
        &self,
        block: &BlockNode<'pr>,
        call: &CallNode<'pr>,
        check: &EligibleCaptures,
    ) -> Vec<(usize, usize, String)> {
        let mut edits = Vec::new();
        let selector_start = call
            .message_loc()
            .map(|m| m.start_offset())
            .unwrap_or(call.location().start_offset());
        let selector_end = call
            .closing_loc()
            .map(|l| l.end_offset())
            .or_else(|| call.message_loc().map(|m| m.end_offset()))
            .unwrap_or(call.location().end_offset());
        edits.push((
            selector_start,
            selector_end,
            "transform_keys".to_string(),
        ));
        let arg_replacement = format!(
            "|{}|",
            String::from_utf8_lossy(&check.transformed_argname),
        );
        let params_loc = block.parameters().expect("matched").location();
        edits.push((
            params_loc.start_offset(),
            params_loc.end_offset(),
            arg_replacement,
        ));
        let _ = check.body_start;
        let body_loc = block.body().expect("matched").location();
        let _ = check.body_end;
        edits.push((
            body_loc.start_offset(),
            body_loc.end_offset(),
            check.body_text.clone(),
        ));
        edits
    }

    // ------------------------------------------------------------------
    // Pattern 2: `Hash[_.map { ... }]`
    // ------------------------------------------------------------------
    fn try_hash_brackets_map<'pr>(&mut self, call: &CallNode<'pr>) {
        if call.is_safe_navigation() {
            return;
        }
        let Some(recv) = call.receiver() else {
            return;
        };
        if !is_hash_const(&recv) {
            return;
        }
        let Some(args) = call.arguments() else {
            return;
        };
        let arg_list: Vec<Node<'pr>> = args.arguments().iter().collect();
        if arg_list.len() != 1 {
            return;
        }
        let Some(inner) = arg_list[0].as_call_node() else {
            return;
        };
        let Some(block_node) = inner.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        let inner_name = inner.name().as_slice();
        if inner_name != b"map" && inner_name != b"collect" {
            return;
        }
        if inner.arguments().is_some() {
            return;
        }
        let Some(inner_recv) = inner.receiver() else {
            return;
        };
        if !hash_receiver(&inner_recv) {
            return;
        }
        let Some(check) = self.eligible_map_block(&block_node) else {
            return;
        };
        let edits = self.build_hash_brackets_map_edits(call, &block_node, &inner, &check);
        self.emit(call.as_node().location(), "Hash[_.map {...}]", edits);
    }

    fn build_hash_brackets_map_edits<'pr>(
        &self,
        outer: &CallNode<'pr>,
        block: &BlockNode<'pr>,
        map_call: &CallNode<'pr>,
        check: &EligibleCaptures,
    ) -> Vec<(usize, usize, String)> {
        let outer_loc = outer.as_node().location();
        let mut edits = Vec::new();
        edits.push((
            outer_loc.start_offset(),
            outer_loc.start_offset() + 5,
            String::new(),
        ));
        edits.push((
            outer_loc.end_offset() - 1,
            outer_loc.end_offset(),
            String::new(),
        ));
        self.append_block_rename_edits(&mut edits, block, map_call, check);
        edits
    }

    // ------------------------------------------------------------------
    // Pattern 3: `_.map { ... }.to_h`
    // ------------------------------------------------------------------
    fn try_map_to_h<'pr>(&mut self, call: &CallNode<'pr>) {
        if call.arguments().is_some() {
            return;
        }
        let Some(recv) = call.receiver() else {
            return;
        };
        let Some(inner) = recv.as_call_node() else {
            return;
        };
        let Some(block_node) = inner.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        let inner_name = inner.name().as_slice();
        if inner_name != b"map" && inner_name != b"collect" {
            return;
        }
        if inner.arguments().is_some() {
            return;
        }
        let Some(inner_recv) = inner.receiver() else {
            return;
        };
        if !hash_receiver(&inner_recv) {
            return;
        }
        let Some(check) = self.eligible_map_block(&block_node) else {
            return;
        };
        let edits = self.build_map_to_h_edits(call, &inner, &block_node, &check);
        // The offense range is the parser send node — i.e. the outer `.to_h`
        // call WITHOUT its trailing block (if any). When the outer to_h
        // carries no block, this is just `call.location()`. When it does
        // carry a block, parser's `(send ... :to_h)` ends at the `to_h`
        // selector; in prism the CallNode's location already includes the
        // block, so we cap at `message_loc().end` (the end of `to_h`).
        let outer_loc = call.as_node().location();
        let offense_end = if call.block().is_some() {
            call.message_loc()
                .map(|m| m.end_offset())
                .unwrap_or(outer_loc.end_offset())
        } else {
            outer_loc.end_offset()
        };
        let offense_start = outer_loc.start_offset();
        // Build a synthetic location-like via the existing Location helper
        // is awkward; we emit the offense directly with the trimmed offsets.
        self.offenses.push(HashTransformKeysOffense {
            start_offset: offense_start,
            end_offset: offense_end,
            message: "Prefer `transform_keys` over `map {...}.to_h`.".to_string(),
            edits,
        });
    }

    fn build_map_to_h_edits<'pr>(
        &self,
        outer: &CallNode<'pr>,
        map_call: &CallNode<'pr>,
        block: &BlockNode<'pr>,
        check: &EligibleCaptures,
    ) -> Vec<(usize, usize, String)> {
        let mut edits = Vec::new();
        // `from_map_to_h` trailing computation:
        // - if outer `.to_h` itself carries a block (`node.parent&.block_type?
        //   && node.parent.send_node == node` in parser-speak), trailing=0
        //   — the outer `.to_h {...}` is preserved verbatim.
        // - otherwise trailing = outer_end - inner_map_end, i.e. the
        //   trailing `.to_h` (plus any whitespace before it) is removed.
        if outer.block().is_none() {
            let outer_loc = outer.as_node().location();
            let map_range = map_call.as_node().location();
            let trail = outer_loc.end_offset() - map_range.end_offset();
            if trail > 0 {
                edits.push((
                    outer_loc.end_offset() - trail,
                    outer_loc.end_offset(),
                    String::new(),
                ));
            }
        }
        self.append_block_rename_edits(&mut edits, block, map_call, check);
        edits
    }

    // ------------------------------------------------------------------
    // Pattern 4: `_.to_h { ... }` (Ruby 2.6+)
    // ------------------------------------------------------------------
    fn try_to_h_block<'pr>(&mut self, call: &CallNode<'pr>) {
        let Some(block_node) = call.block().and_then(|b| b.as_block_node()) else {
            return;
        };
        if call.arguments().is_some() {
            return;
        }
        let Some(recv) = call.receiver() else {
            return;
        };
        if !hash_receiver(&recv) {
            return;
        }
        let Some(check) = self.eligible_map_block(&block_node) else {
            return;
        };
        let edits = self.build_to_h_block_edits(&block_node, call, &check);
        self.emit(call.as_node().location(), "to_h {...}", edits);
    }

    fn build_to_h_block_edits<'pr>(
        &self,
        block: &BlockNode<'pr>,
        call: &CallNode<'pr>,
        check: &EligibleCaptures,
    ) -> Vec<(usize, usize, String)> {
        let mut edits = Vec::new();
        self.append_block_rename_edits(&mut edits, block, call, check);
        edits
    }

    // ------------------------------------------------------------------
    // Shared: emit `set_new_method_name` + `set_new_arg_name` +
    // `set_new_body_expression`.
    // ------------------------------------------------------------------
    fn append_block_rename_edits<'pr>(
        &self,
        edits: &mut Vec<(usize, usize, String)>,
        block: &BlockNode<'pr>,
        inner_send: &CallNode<'pr>,
        check: &EligibleCaptures,
    ) {
        let selector = inner_send.message_loc().expect("send has a selector");
        let end_off = inner_send
            .closing_loc()
            .map(|c| c.end_offset())
            .unwrap_or(selector.end_offset());
        edits.push((
            selector.start_offset(),
            end_off,
            "transform_keys".to_string(),
        ));
        let arg_replacement = format!(
            "|{}|",
            String::from_utf8_lossy(&check.transformed_argname),
        );
        let params_loc = block.parameters().expect("matched").location();
        edits.push((
            params_loc.start_offset(),
            params_loc.end_offset(),
            arg_replacement,
        ));
        let body_loc = block.body().expect("matched").location();
        edits.push((
            body_loc.start_offset(),
            body_loc.end_offset(),
            check.body_text.clone(),
        ));
    }

    /// `set_new_body_expression`: returns the substring for `transforming_body_expr`,
    /// wrapped in `{ ... }` when it is a braceless hash literal.
    fn body_source_with_braces_at(&self, expr: &Node<'_>, loc: &Location<'_>) -> String {
        let text = String::from_utf8_lossy(self.loc_text(loc)).into_owned();
        if let Some(hash) = expr.as_hash_node()
            && hash.opening_loc().as_slice() != b"{"
        {
            return format!("{{ {text} }}");
        }
        text
    }

    /// `(args (arg $_) (arg _val))` + `(array $_ $(lvar _val))` body: the
    /// captures for `map` / `to_h{}` / `Hash[]` forms. Returns the data we
    /// need to build the edit list once the negative guards pass.
    fn eligible_map_block<'pr>(&self, block: &BlockNode<'pr>) -> Option<EligibleCaptures> {
        let params = block.parameters()?.as_block_parameters_node()?;
        let p = params.parameters()?;
        if p.optionals().iter().count() != 0
            || p.rest().is_some()
            || p.posts().iter().count() != 0
            || p.keywords().iter().count() != 0
            || p.keyword_rest().is_some()
            || p.block().is_some()
            || params.locals().iter().count() != 0
        {
            return None;
        }
        let requireds: Vec<Node<'pr>> = p.requireds().iter().collect();
        if requireds.len() != 2 {
            return None;
        }
        let k_param = requireds[0].as_required_parameter_node()?;
        let val_param = requireds[1].as_required_parameter_node()?;
        let key_argname = k_param.name().as_slice();
        let val_argname = val_param.name().as_slice();
        let body = block.body()?;
        let stmts = body.as_statements_node()?;
        let only: Vec<Node<'pr>> = stmts.body().iter().collect();
        if only.len() != 1 {
            return None;
        }
        let array = only[0].as_array_node()?;
        let elems: Vec<Node<'pr>> = array.elements().iter().collect();
        if elems.len() != 2 {
            return None;
        }
        let key_body_expr = &elems[0];
        let val_body_capture = &elems[1];
        if !match_lvar_named(val_body_capture, val_argname) {
            return None;
        }
        // Negative guards.
        if let Some(lv) = key_body_expr.as_local_variable_read_node()
            && lv.name().as_slice() == key_argname
        {
            return None;
        }
        let val_offset = val_body_capture.location().start_offset();
        if contains_lvar_node(key_body_expr, val_offset) {
            return None;
        }
        if !any_strict_descendant_lvar_named(key_body_expr, key_argname) {
            return None;
        }
        let key_loc = key_body_expr.location();
        let body_text = self.body_source_with_braces_at(key_body_expr, &key_loc);
        Some(EligibleCaptures {
            transformed_argname: key_argname.to_vec(),
            body_text,
            body_start: key_loc.start_offset(),
            body_end: key_loc.end_offset(),
        })
    }
}

/// Data needed to write the edits once the captures are validated. We move
/// the work that touches `Node<'pr>` (source-string slicing, negative-guard
/// scans) into the matching step so we never have to stash a node past the
/// CallNode lifetime.
#[allow(dead_code)]
struct EligibleCaptures {
    transformed_argname: Vec<u8>,
    body_text: String,
    body_start: usize,
    body_end: usize,
}

/// `(const _ :Hash)`: bare `Hash` or any constant path ending in `Hash`.
fn is_hash_const(node: &Node<'_>) -> bool {
    if let Some(c) = node.as_constant_read_node() {
        return c.name().as_slice() == b"Hash";
    }
    if let Some(c) = node.as_constant_path_node()
        && let Some(name) = c.name()
    {
        return name.as_slice() == b"Hash";
    }
    false
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_CALL,
        )
    }
    
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(call) = node.as_call_node() {
            self.process_call(&call);
        }
    }

    fn leave(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Vec<HashTransformKeysOffense> {
        check_hash_transform_keys(source.as_bytes())
    }

    fn apply(source: &str) -> String {
        let mut out = source.as_bytes().to_vec();
        let mut edits: Vec<(usize, usize, Vec<u8>)> = Vec::new();
        for o in run(source) {
            for (s, e, t) in o.edits {
                edits.push((s, e, t.into_bytes()));
            }
        }
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        for (start, end, text) in edits {
            out.splice(start..end, text);
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn each_with_object_inline_block() {
        let src = "{a: 1, b: 2}.each_with_object({}) {|(k, v), h| h[foo(k)] = v}";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].message,
            "Prefer `transform_keys` over `each_with_object`."
        );
        assert_eq!(apply(src), "{a: 1, b: 2}.transform_keys {|k| foo(k)}");
    }

    #[test]
    fn hash_brackets_map() {
        let src = "Hash[{a: 1, b: 2}.map {|k, v| [k.to_sym, v]}]";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(apply(src), "{a: 1, b: 2}.transform_keys {|k| k.to_sym}");
    }

    #[test]
    fn map_to_h() {
        let src = "{a: 1, b: 2}.map {|k, v| [k.to_sym, v]}.to_h";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(apply(src), "{a: 1, b: 2}.transform_keys {|k| k.to_sym}");
    }

    #[test]
    fn to_h_block() {
        let src = "{a: 1, b: 2}.to_h {|k, v| [k.to_sym, v]}";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(apply(src), "{a: 1, b: 2}.transform_keys {|k| k.to_sym}");
    }

    #[test]
    fn no_offense_when_key_is_noop() {
        assert!(run("{a: 1, b: 2}.each_with_object({}) {|(k, v), h| h[k] = v}").is_empty());
        assert!(run("{a: 1, b: 2}.map {|k, v| [k, v]}.to_h").is_empty());
    }

    #[test]
    fn no_offense_when_key_transform_uses_value() {
        assert!(run("{a: 1, b: 2}.each_with_object({}) {|(k, v), h| h[foo(v)] = v}").is_empty());
    }

    #[test]
    fn no_offense_when_unchanged_arg_unused() {
        assert!(run("{a: 1, b: 2}.map {|_k, v| [v, v]}.to_h").is_empty());
    }

    #[test]
    fn unknown_receiver_does_not_flag() {
        assert!(run("x.each_with_object({}) {|(k, v), h| h[foo(k)] = v}").is_empty());
        assert!(run("x.map {|k, v| [k.to_sym, v]}.to_h").is_empty());
        assert!(run("Hash[x.map {|k, v| [k.to_sym, v]}]").is_empty());
        assert!(run("x.to_h {|k, v| [k.to_sym, v]}").is_empty());
    }

    #[test]
    fn array_literal_receiver_does_not_flag() {
        assert!(run("[1, 2, 3].each_with_object({}) {|(k, v), h| h[foo(k)] = v}").is_empty());
        assert!(run("[1, 2, 3].map {|k, v| [k.to_sym, v]}.to_h").is_empty());
    }

    #[test]
    fn hash_literal_receiver_flags() {
        assert_eq!(
            run("{a: 1}.each_with_object({}) {|(k, v), h| h[foo(k)] = v}").len(),
            1
        );
    }

    #[test]
    fn to_h_receiver_flags() {
        let src = "x.to_h.each_with_object({}) {|(k, v), h| h[foo(k)] = v}";
        assert_eq!(run(src).len(), 1);
    }

    #[test]
    fn merge_receiver_flags() {
        let src = "x.merge(y).to_h {|k, v| [k.to_sym, v]}";
        assert_eq!(run(src).len(), 1);
    }

    #[test]
    fn group_by_block_receiver_flags() {
        let src = "x.group_by { |e| e.type }.each_with_object({}) {|(k, v), h| h[foo(k)] = v}";
        assert_eq!(run(src).len(), 1);
    }

    #[test]
    fn safe_navigation_each_with_object() {
        let src = "x.to_h&.each_with_object({}) {|(k, v), h| h[foo(k)] = v}";
        assert_eq!(run(src).len(), 1);
    }

    #[test]
    fn multiline_each_with_object_block_keeps_layout() {
        let src = "some_hash.to_h.each_with_object({}) do |(key, val), memo|\n  memo[key.to_sym] = val\nend\n";
        let out = apply(src);
        assert_eq!(
            out,
            "some_hash.to_h.transform_keys do |key|\n  key.to_sym\nend\n"
        );
    }

    #[test]
    fn map_to_h_with_trailing_newline_before_to_h() {
        let src = "{a: 1, b: 2}.map {|k, v| [k.to_sym, v]}.\n  to_h\n";
        let out = apply(src);
        assert_eq!(out, "{a: 1, b: 2}.transform_keys {|k| k.to_sym}\n");
    }

    #[test]
    fn map_to_h_outer_to_h_with_block_keeps_trailing() {
        let src = "{a: 1, b: 2}.map {|k, v| [k.to_s, v]}.to_h {|k, v| [v, k]}";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(
            apply(src),
            "{a: 1, b: 2}.transform_keys {|k| k.to_s}.to_h {|k, v| [v, k]}"
        );
    }

    #[test]
    fn hash_brackets_map_multiline_keeps_indent() {
        let src = "Hash[{a: 1, b: 2}.map do |k, v|\n  [k.to_s, v]\nend]\n";
        let out = apply(src);
        assert_eq!(out, "{a: 1, b: 2}.transform_keys do |k|\n  k.to_s\nend\n");
    }

    #[test]
    fn map_to_h_via_csend() {
        let src = "{a: 1}.map {|k, v| [k.to_sym, v]}&.to_h";
        assert_eq!(run(src).len(), 1);
    }
}
