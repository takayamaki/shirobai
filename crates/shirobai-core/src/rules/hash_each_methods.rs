//! `Style/HashEachMethods`.

use std::mem::discriminant;

use ruby_prism::{
    BlockArgumentNode, BlockNode, BlockParametersNode, CallNode, MultiTargetNode, Node, Visit,
};

/// A `keys.each` / `values.each` chain or an `each` block with an unused
/// key/value argument. The correction is `replace_*` → `replacement` plus an
/// optional removal (`remove_end > remove_start`), mirroring stock's
/// `corrector.replace` + `corrector.remove` pair.
pub struct HashEachOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    pub message: String,
    pub replace_start: usize,
    pub replace_end: usize,
    pub replacement: String,
    pub remove_start: usize,
    pub remove_end: usize,
}

/// `ARRAY_CONVERTER_METHODS`: a receiver produced by these is an Array, not a
/// Hash, so the block form stands down (`use_array_converter_method_as_preceding?`).
const ARRAY_CONVERTER_METHODS: &[&[u8]] = &[
    b"assoc", b"chunk", b"flatten", b"rassoc", b"sort", b"sort_by", b"to_a",
];

pub fn check_hash_each_methods(
    source: &[u8],
    allowed_receivers: &[String],
) -> Vec<HashEachOffense> {
    let mut rule = build_rule(source, allowed_receivers);
    super::dispatch::run(source, &mut [&mut rule]);
    rule.offenses
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule<'a>(source: &'a [u8], allowed_receivers: &'a [String]) -> Visitor<'a> {
    Visitor {
        source,
        allowed_receivers,
        offenses: Vec::new(),
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    allowed_receivers: &'a [String],
    pub(crate) offenses: Vec<HashEachOffense>,
}

/// `each_key` for `keys`, `each_value` for `values` (`"each_#{method[0..-2]}"`).
fn prefer_for(method: &[u8]) -> &'static str {
    if method == b"keys" {
        "each_key"
    } else {
        "each_value"
    }
}

/// The `keys`/`values` method name when `call` matches the inner
/// `(call _ {:keys :values})` of the node patterns: zero arguments and no
/// block of any kind (a parser `block` wrapper or a `block_pass` child would
/// both break the pattern).
fn kv_method<'pr>(call: &CallNode<'pr>) -> Option<&'pr [u8]> {
    let name = call.name().as_slice();
    if name != b"keys" && name != b"values" {
        return None;
    }
    if call.arguments().is_some_and(|a| !a.arguments().is_empty()) || call.block().is_some() {
        return None;
    }
    Some(name)
}

/// Whether `call` corresponds to a parser `block` node (a block literal is
/// attached; a `BlockArgumentNode` is a parser `block_pass` *child* instead).
fn has_parser_block(call: &CallNode<'_>) -> bool {
    call.block().is_some_and(|b| b.as_block_node().is_some())
}

fn is_const(node: &Node<'_>) -> bool {
    node.as_constant_read_node().is_some() || node.as_constant_path_node().is_some()
}

/// `rubocop-ast`'s generic `Node#receiver`: the receiver of a send/csend or of
/// a block's inner call — all `CallNode` in Prism — and nil for anything else.
fn receiver_of<'pr>(node: &Node<'pr>) -> Option<Node<'pr>> {
    node.as_call_node().and_then(|c| c.receiver())
}

/// `Node#literal?` (parser type list) restated over Prism node kinds.
fn is_literal(node: &Node<'_>) -> bool {
    node.as_true_node().is_some()
        || node.as_false_node().is_some()
        || node.as_nil_node().is_some()
        || node.as_integer_node().is_some()
        || node.as_float_node().is_some()
        || node.as_rational_node().is_some()
        || node.as_imaginary_node().is_some()
        || node.as_string_node().is_some()
        || node.as_interpolated_string_node().is_some()
        || node.as_x_string_node().is_some()
        || node.as_interpolated_x_string_node().is_some()
        || node.as_symbol_node().is_some()
        || node.as_interpolated_symbol_node().is_some()
        || node.as_array_node().is_some()
        || node.as_hash_node().is_some()
        || node.as_regular_expression_node().is_some()
        || node.as_interpolated_regular_expression_node().is_some()
        || node.as_range_node().is_some()
}

/// Structural equality, mirroring the parser gem's `Node#==` (type + children)
/// for the node kinds a receiver chain can realistically contain. Calls are
/// compared by flavor (send/csend), name, receiver, arguments and block-pass —
/// so `foo()` and `foo` are equal, exactly like `(send nil :foo)` twice.
/// Everything else (variables, constants, literals) falls back to "same node
/// kind + same source text", which matches parser equality whenever the same
/// value is spelled the same way.
fn structural_eq(source: &[u8], a: &Node<'_>, b: &Node<'_>) -> bool {
    let src_eq = |a: &Node<'_>, b: &Node<'_>| {
        let (la, lb) = (a.location(), b.location());
        source[la.start_offset()..la.end_offset()] == source[lb.start_offset()..lb.end_offset()]
    };
    if let (Some(ca), Some(cb)) = (a.as_call_node(), b.as_call_node()) {
        let (a_blk, b_blk) = (has_parser_block(&ca), has_parser_block(&cb));
        if a_blk || b_blk {
            // A parser `block` node: comparing args + body structurally is
            // overkill for a receiver chain; source equality is the practical
            // proxy.
            return a_blk == b_blk && src_eq(a, b);
        }
        if ca.is_safe_navigation() != cb.is_safe_navigation()
            || ca.name().as_slice() != cb.name().as_slice()
        {
            return false;
        }
        match (ca.receiver(), cb.receiver()) {
            (None, None) => {}
            (Some(ra), Some(rb)) => {
                if !structural_eq(source, &ra, &rb) {
                    return false;
                }
            }
            _ => return false,
        }
        let args_a: Vec<Node> = ca
            .arguments()
            .map(|x| x.arguments().iter().collect())
            .unwrap_or_default();
        let args_b: Vec<Node> = cb
            .arguments()
            .map(|x| x.arguments().iter().collect())
            .unwrap_or_default();
        if args_a.len() != args_b.len() {
            return false;
        }
        if !args_a
            .iter()
            .zip(&args_b)
            .all(|(x, y)| structural_eq(source, x, y))
        {
            return false;
        }
        // A block-pass argument is a child of the parser send node.
        return match (ca.block(), cb.block()) {
            (None, None) => true,
            (Some(x), Some(y)) => {
                let (xe, ye) = (
                    x.as_block_argument_node().and_then(|p| p.expression()),
                    y.as_block_argument_node().and_then(|p| p.expression()),
                );
                match (xe, ye) {
                    (None, None) => true,
                    (Some(xe), Some(ye)) => structural_eq(source, &xe, &ye),
                    _ => false,
                }
            }
            _ => false,
        };
    }
    discriminant(a) == discriminant(b) && src_eq(a, b)
}

/// Finds `(send %root :[]= ...)` anywhere in a subtree (`hash_mutated?`).
struct MutationScan<'a, 'pr> {
    source: &'a [u8],
    root: &'a Node<'pr>,
    found: bool,
}

impl<'a, 'pr> Visit<'pr> for MutationScan<'a, 'pr> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        if self.found {
            return;
        }
        if node.name().as_slice() == b"[]="
            && !node.is_safe_navigation()
            && node
                .receiver()
                .is_some_and(|r| structural_eq(self.source, &r, self.root))
        {
            self.found = true;
            return;
        }
        ruby_prism::visit_call_node(self, node);
    }
}

/// Collects every `lvar` read in a subtree (`each_descendant(:lvar)` sources;
/// an lvar's source is exactly its name).
#[derive(Default)]
struct LvarScan {
    names: Vec<Vec<u8>>,
}

impl<'pr> Visit<'pr> for LvarScan {
    fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
        self.names.push(node.name().as_slice().to_vec());
    }
}

/// The lvar sources of the parser body's *strict* descendants. Parser wraps a
/// multi-statement body in a `begin` node (descendants = every statement
/// subtree) but a single-statement body IS the statement — a body that is
/// exactly one `lvar` has no descendants, so that read does not count.
fn body_lvar_sources(body: &Node<'_>) -> Vec<Vec<u8>> {
    let mut scan = LvarScan::default();
    if let Some(stmts) = body.as_statements_node() {
        let children = stmts.body();
        if children.len() == 1 {
            let only = children.iter().next().expect("length checked");
            if only.as_local_variable_read_node().is_none() {
                scan.visit(&only);
            }
        } else {
            for child in children.iter() {
                scan.visit(&child);
            }
        }
    } else {
        // A `BeginNode` body (block with rescue/ensure): the parser body root
        // is the rescue/ensure node itself, never an lvar.
        scan.visit(body);
    }
    scan.names
}

/// The block parameters in parser `args`-children order: positionals in source
/// order (an implicit rest from a trailing comma emits no parser child),
/// then keywords, block and shadow locals.
fn flatten_block_params<'pr>(bp: &BlockParametersNode<'pr>) -> Vec<Node<'pr>> {
    let mut out = Vec::new();
    if let Some(p) = bp.parameters() {
        for n in p.requireds().iter() {
            out.push(n);
        }
        for n in p.optionals().iter() {
            out.push(n);
        }
        if let Some(rest) = p.rest()
            && rest.as_implicit_rest_node().is_none()
        {
            out.push(rest);
        }
        for n in p.posts().iter() {
            out.push(n);
        }
        for n in p.keywords().iter() {
            out.push(n);
        }
        if let Some(kr) = p.keyword_rest() {
            out.push(kr);
        }
        if let Some(b) = p.block() {
            out.push(b.as_node());
        }
    }
    for l in bp.locals().iter() {
        out.push(l);
    }
    out
}

/// Whether every `arg`/`restarg` descendant of a destructured (mlhs) block
/// argument is unused.
fn mlhs_unused(target: &MultiTargetNode<'_>, lvars: &[Vec<u8>]) -> bool {
    let component_unused = |node: &Node<'_>| -> bool {
        if let Some(req) = node.as_required_parameter_node() {
            return !lvars.iter().any(|l| l == req.name().as_slice());
        }
        if let Some(splat) = node.as_splat_node() {
            // restarg source minus the `*` prefix; an anonymous `*` strips to
            // the empty string, which never matches an lvar.
            let name: &[u8] = splat
                .expression()
                .and_then(|e| e.as_required_parameter_node().map(|r| r.name().as_slice()))
                .unwrap_or(b"");
            return !lvars.iter().any(|l| l == name);
        }
        if let Some(nested) = node.as_multi_target_node() {
            return mlhs_unused(&nested, lvars);
        }
        true
    };
    target.lefts().iter().all(|n| component_unused(&n))
        && target.rest().is_none_or(|n| component_unused(&n))
        && target.rights().iter().all(|n| component_unused(&n))
}

impl<'a> Visitor<'a> {
    fn lossy(&self, start: usize, end: usize) -> String {
        String::from_utf8_lossy(&self.source[start..end]).into_owned()
    }

    /// `unused_block_arg_exist?` for a non-mlhs argument: its full source minus
    /// a leading `*` must match no lvar read in the body.
    fn param_unused(&self, param: &Node<'_>, lvars: &[Vec<u8>]) -> bool {
        if let Some(mt) = param.as_multi_target_node() {
            return mlhs_unused(&mt, lvars);
        }
        let loc = param.location();
        let mut src = &self.source[loc.start_offset()..loc.end_offset()];
        if src.starts_with(b"*") {
            src = &src[1..];
        }
        !lvars.iter().any(|l| l == src)
    }

    /// `AllowedReceivers#receiver_name`: walk down to the root-most receiver
    /// (stopping above a constant), then "Const.method" / bare method name for
    /// a plain send, or the node's source for everything else.
    fn receiver_name(&self, node: &Node<'_>) -> String {
        if let Some(call) = node.as_call_node() {
            if let Some(recv) = call.receiver()
                && !is_const(&recv)
            {
                return self.receiver_name(&recv);
            }
            // `send_type?`: csend and parser block nodes take the source branch.
            if !call.is_safe_navigation() && !has_parser_block(&call) {
                let name = String::from_utf8_lossy(call.name().as_slice()).into_owned();
                return match call.receiver() {
                    Some(recv) => format!("{}.{name}", self.receiver_name(&recv)),
                    None => name,
                };
            }
        }
        let loc = node.location();
        self.lossy(loc.start_offset(), loc.end_offset())
    }

    fn allowed_receiver(&self, receiver: &Node<'_>) -> bool {
        if self.allowed_receivers.is_empty() {
            return false;
        }
        self.allowed_receivers
            .contains(&self.receiver_name(receiver))
    }

    /// `root_receiver`: the bottom of the receiver chain.
    fn root_receiver<'pr>(&self, each: &CallNode<'pr>) -> Option<Node<'pr>> {
        let mut current = each.receiver()?;
        while let Some(inner) = receiver_of(&current) {
            current = inner;
        }
        Some(current)
    }

    fn hash_mutated(&self, each: &CallNode<'_>, root: &Node<'_>) -> bool {
        let mut scan = MutationScan {
            source: self.source,
            root,
            found: false,
        };
        scan.visit(&each.as_node());
        scan.found
    }

    /// `handleable?` for the block form.
    fn handleable(&self, each: &CallNode<'_>) -> bool {
        if let Some(recv) = each.receiver()
            && let Some(rc) = recv.as_call_node()
            && ARRAY_CONVERTER_METHODS.contains(&rc.name().as_slice())
        {
            return false;
        }
        let Some(root) = self.root_receiver(each) else {
            return false;
        };
        if self.hash_mutated(each, &root) {
            return false;
        }
        !is_literal(&root) || root.as_hash_node().is_some()
    }

    fn process_call(&mut self, call: &CallNode<'_>) {
        if call.name().as_slice() != b"each" {
            return;
        }
        // Both node patterns give `:each` zero arguments.
        if call.arguments().is_some_and(|a| !a.arguments().is_empty()) {
            return;
        }
        let Some(block) = call.block() else { return };
        if let Some(block_node) = block.as_block_node() {
            self.process_block(call, &block_node);
        } else if let Some(pass) = block.as_block_argument_node() {
            self.process_block_pass(call, &pass);
        }
    }

    /// `on_block` (+ numblock/itblock aliases): `kv_each`, falling through to
    /// the unused-block-arg check when `register_kv_offense` bails out
    /// (stock's `register_kv_offense(...) and return` only returns after an
    /// actual `add_offense`).
    fn process_block(&mut self, each: &CallNode<'_>, block: &BlockNode<'_>) {
        if !self.handleable(each) {
            return;
        }
        if let Some(receiver) = each.receiver()
            && let Some(kv) = receiver.as_call_node()
            && let Some(method) = kv_method(&kv)
            && let Some(parent) = kv.receiver()
            && !self.allowed_receiver(&parent)
        {
            self.register_kv(each, &kv, method, &parent);
            return;
        }
        self.check_unused_block_args(each, block);
    }

    /// `register_kv_offense` + `correct_key_value_each`: offense on
    /// `keys`-selector..`each`-selector, correction replaces the whole send
    /// with `receiver` + the `each` call operator + `each_key`/`each_value`.
    fn register_kv(
        &mut self,
        each: &CallNode<'_>,
        kv: &CallNode<'_>,
        method: &[u8],
        parent: &Node<'_>,
    ) {
        let (Some(kv_sel), Some(each_sel)) = (kv.message_loc(), each.message_loc()) else {
            return;
        };
        let start = kv_sel.start_offset();
        let end = each_sel.end_offset();
        // The parser send node ends at the argument-list `)` when present
        // (`each()`), else at the selector — never at the block.
        let send_end = each.closing_loc().map_or(end, |l| l.end_offset());
        let prefer = prefer_for(method);
        let current = self.lossy(start, send_end);
        let dot = each.call_operator_loc().map_or_else(String::new, |l| {
            self.lossy(l.start_offset(), l.end_offset())
        });
        let parent_loc = parent.location();
        let receiver_src = self.lossy(parent_loc.start_offset(), parent_loc.end_offset());
        self.offenses.push(HashEachOffense {
            start_offset: start,
            end_offset: end,
            message: format!("Use `{prefer}` instead of `{current}`."),
            replace_start: each.location().start_offset(),
            replace_end: send_end,
            replacement: format!("{receiver_src}{dot}{prefer}"),
            remove_start: 0,
            remove_end: 0,
        });
    }

    /// `on_block_pass` + `kv_each_with_block_pass`: `hash.keys.each(&:sym)`.
    /// Stock never gates this path through `handleable?`.
    fn process_block_pass(&mut self, each: &CallNode<'_>, pass: &BlockArgumentNode<'_>) {
        if pass
            .expression()
            .is_none_or(|e| e.as_symbol_node().is_none())
        {
            return;
        }
        let Some(receiver) = each.receiver() else {
            return;
        };
        let Some(kv) = receiver.as_call_node() else {
            return;
        };
        let Some(method) = kv_method(&kv) else { return };
        let Some(parent) = kv.receiver() else { return };
        if self.allowed_receiver(&parent) {
            return;
        }
        let (Some(kv_sel), Some(each_sel)) = (kv.message_loc(), each.message_loc()) else {
            return;
        };
        let start = kv_sel.start_offset();
        let end = each_sel.end_offset();
        let prefer = prefer_for(method);
        let current = self.lossy(start, end);
        self.offenses.push(HashEachOffense {
            start_offset: start,
            end_offset: end,
            message: format!("Use `{prefer}` instead of `{current}`."),
            replace_start: start,
            replace_end: end,
            replacement: prefer.to_string(),
            remove_start: 0,
            remove_end: 0,
        });
    }

    /// `each_arguments` + `check_unused_block_args`: an explicit two-argument
    /// `each` block where exactly one side is unused.
    fn check_unused_block_args(&mut self, each: &CallNode<'_>, block: &BlockNode<'_>) {
        // `(block ...)` only: numbered/it parameter blocks have no `args` node.
        let Some(bp) = block
            .parameters()
            .and_then(|p| p.as_block_parameters_node())
        else {
            return;
        };
        let params = flatten_block_params(&bp);
        if params.len() != 2 {
            return;
        }
        let Some(body) = block.body() else { return };
        let lvars = body_lvar_sources(&body);
        let (key, value) = (&params[0], &params[1]);
        let value_unused = self.param_unused(value, &lvars);
        let key_unused = self.param_unused(key, &lvars);
        if value_unused && key_unused {
            return;
        }
        let (key_loc, value_loc) = (key.location(), value.location());
        let (prefer, unused, remove_start, remove_end) = if value_unused {
            (
                "each_key",
                self.lossy(value_loc.start_offset(), value_loc.end_offset()),
                key_loc.end_offset(),
                value_loc.end_offset(),
            )
        } else if key_unused {
            (
                "each_value",
                self.lossy(key_loc.start_offset(), key_loc.end_offset()),
                key_loc.start_offset(),
                value_loc.start_offset(),
            )
        } else {
            return;
        };
        let Some(each_sel) = each.message_loc() else {
            return;
        };
        let loc = each.location();
        self.offenses.push(HashEachOffense {
            start_offset: loc.start_offset(),
            end_offset: loc.end_offset(),
            message: format!(
                "Use `{prefer}` instead of `each` and remove the unused `{unused}` block argument."
            ),
            replace_start: each_sel.start_offset(),
            replace_end: each_sel.end_offset(),
            replacement: prefer.to_string(),
            remove_start,
            remove_end,
        });
    }
}

impl super::dispatch::Rule for Visitor<'_> {
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

    fn run(source: &str) -> Vec<HashEachOffense> {
        check_hash_each_methods(source.as_bytes(), &["Thread.current".to_string()])
    }

    fn apply(source: &str) -> String {
        let mut out = source.as_bytes().to_vec();
        let mut edits: Vec<(usize, usize, Vec<u8>)> = Vec::new();
        for o in run(source) {
            edits.push((o.replace_start, o.replace_end, o.replacement.into_bytes()));
            if o.remove_end > o.remove_start {
                edits.push((o.remove_start, o.remove_end, Vec::new()));
            }
        }
        edits.sort_by_key(|e| std::cmp::Reverse(e.0));
        for (start, end, text) in edits {
            out.splice(start..end, text);
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn kv_each_block() {
        let src = "foo.keys.each { |k| p k }";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(&src[got[0].start_offset..got[0].end_offset], "keys.each");
        assert_eq!(got[0].message, "Use `each_key` instead of `keys.each`.");
        assert_eq!(apply(src), "foo.each_key { |k| p k }");
    }

    #[test]
    fn kv_each_safe_navigation() {
        let src = "foo&.values&.each { |v| p v }";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].message,
            "Use `each_value` instead of `values&.each`."
        );
        assert_eq!(apply(src), "foo&.each_value { |v| p v }");
    }

    #[test]
    fn kv_each_block_pass() {
        let src = "foo.keys.each(&:bar)";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(&src[got[0].start_offset..got[0].end_offset], "keys.each");
        assert_eq!(apply(src), "foo.each_key(&:bar)");
    }

    #[test]
    fn block_pass_skips_handleable() {
        // Literal array receiver and converter receivers don't gate the
        // block-pass path in stock.
        assert_eq!(run("[[1,2]].keys.each(&:bar)").len(), 1);
        assert_eq!(run("foo.to_a.keys.each(&:bar)").len(), 1);
    }

    #[test]
    fn unused_value_argument() {
        let src = "foo.each { |k, unused_value| do_something(k) }";
        let got = run(src);
        assert_eq!(got.len(), 1);
        assert_eq!(&src[got[0].start_offset..got[0].end_offset], src);
        assert_eq!(
            got[0].message,
            "Use `each_key` instead of `each` and remove the unused `unused_value` block argument."
        );
        assert_eq!(apply(src), "foo.each_key { |k| do_something(k) }");
    }

    #[test]
    fn unused_key_argument() {
        let src = "foo.each { |unused_key, v| do_something(v) }";
        assert_eq!(apply(src), "foo.each_value { |v| do_something(v) }");
    }

    #[test]
    fn used_or_both_unused_arguments() {
        assert!(run("foo.each { |k, v| do_something(k, v) }").is_empty());
        assert!(run("foo.each { |k, v| do_something }").is_empty());
        assert!(run("foo.each { |k, v| }").is_empty());
        // A single-lvar body has no descendants in parser terms.
        assert!(run("foo.each { |k, v| v }").is_empty());
    }

    #[test]
    fn implicit_receiver() {
        assert!(run("keys.each { |k| p k }").is_empty());
        assert!(run("each { |k, v| p k }").is_empty());
        assert!(run("keys.each(&:bar)").is_empty());
        // ...but the unused-arg fall-through still fires on `keys.each`.
        let got = run("keys.each { |k, v| p v }");
        assert_eq!(got.len(), 1);
        assert!(got[0].message.contains("each_value"));
    }

    #[test]
    fn allowed_receiver_falls_through() {
        assert!(run("Thread.current.keys.each { |k| p k }").is_empty());
        let got = check_hash_each_methods(
            b"execute.keys.each { |k, v| p v }",
            &["execute".to_string()],
        );
        assert_eq!(got.len(), 1);
        assert!(got[0].message.contains("unused `k`"));
    }

    #[test]
    fn array_converter_receiver() {
        assert!(run("foo.to_a.each { |unused_key, v| do_something(v) }").is_empty());
        assert!(
            run("foo.sort_by { |k, v| v }.each { |unused_key, v| do_something(v) }").is_empty()
        );
    }

    #[test]
    fn mutated_hash() {
        assert!(run("foo.keys.each { |k| foo[k] = 1 }").is_empty());
        // Structural equality: `foo()` and `foo` are the same parser node.
        assert!(run("foo().keys.each { |k| foo[k] = 1 }").is_empty());
        // A different hash being written keeps the offense.
        assert_eq!(run("foo.keys.each { |k| bar[k] = 1 }").len(), 1);
    }

    #[test]
    fn literal_receivers() {
        assert!(run("[[1, 2], [3, 4]].each { |a, _| p a }").is_empty());
        assert_eq!(run("{hash: :literal}.keys.each { |k| p k }").len(), 1);
        assert_eq!(apply("{}.keys.each { |k| p k }"), "{}.each_key { |k| p k }");
    }

    #[test]
    fn trailing_comma_param_is_single() {
        // parser drops the implicit rest: `|k,|` has one args child.
        assert!(run("foo.each { |k,| p k }").is_empty());
    }

    #[test]
    fn shadow_arg_counts_as_value() {
        let got = run("foo.each { |k; x| p k }");
        assert_eq!(got.len(), 1);
        assert!(got[0].message.contains("unused `x`"));
    }

    #[test]
    fn numbered_param_block() {
        assert_eq!(apply("foo.keys.each { p _1 }"), "foo.each_key { p _1 }");
        // ...but no `each_arguments` match for numbered blocks.
        assert!(run("foo.each { p _1 }").is_empty());
    }
}
