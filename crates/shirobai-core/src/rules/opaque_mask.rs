//! Opaque-region collection shared by the byte-scan cops
//! (`punctuation_spacing`, `space_inside_parens`, `space_before_first_arg`).
//!
//! A byte inside one of these regions is never (part of) a parser token that
//! the byte-scan cops care about: string / symbol / regexp / xstring
//! opening+content+closing (heredoc bodies and quoted terminators included),
//! character literals, global variable names (`$,` / `$;`), comments and the
//! `__END__` data segment. Interpolation code inside strings is NOT opaque.
//!
//! The collection hooks are called from each rule's `enter` / `enter_leaf`
//! (the rules keep their own interest masks); [`merge`] folds the collected
//! ranges together with the comments and the data segment into one sorted,
//! disjoint list for scan-time lookups.

use ruby_prism::Node;

/// Branch-node hook: interpolated literal delimiters, percent-array
/// delimiters and gvar-write names. Call from `Rule::enter`; needs the
/// `ENTER_ISTRING`, `ENTER_LITERAL`, `ENTER_WRITE` and `ENTER_OTHER`
/// interest bits (`InterpolatedMatchLastLineNode` is OTHER, `ArrayNode` is
/// LITERAL).
pub(crate) fn collect_enter(node: &Node<'_>, masks: &mut Vec<(usize, usize)>) {
    let mut push = |loc: ruby_prism::Location<'_>| {
        masks.push((loc.start_offset(), loc.end_offset()));
    };
    match node {
        // Interpolated literals: the delimiters are opaque (a quoted heredoc
        // terminator can contain any byte); the string parts and embedded
        // statements are visited on their own.
        Node::InterpolatedStringNode { .. } => {
            let n = node.as_interpolated_string_node().unwrap();
            if let Some(o) = n.opening_loc() {
                push(o);
            }
            if let Some(c) = n.closing_loc() {
                push(c);
            }
        }
        Node::InterpolatedXStringNode { .. } => {
            let n = node.as_interpolated_x_string_node().unwrap();
            push(n.opening_loc());
            push(n.closing_loc());
        }
        Node::InterpolatedRegularExpressionNode { .. } => {
            let n = node.as_interpolated_regular_expression_node().unwrap();
            push(n.opening_loc());
            push(n.closing_loc());
        }
        Node::InterpolatedMatchLastLineNode { .. } => {
            let n = node.as_interpolated_match_last_line_node().unwrap();
            push(n.opening_loc());
            push(n.closing_loc());
        }
        Node::InterpolatedSymbolNode { .. } => {
            let n = node.as_interpolated_symbol_node().unwrap();
            if let Some(o) = n.opening_loc() {
                push(o);
            }
            if let Some(c) = n.closing_loc() {
                push(c);
            }
        }
        // Percent-array delimiters (`%w( … )`, `%i,a b,`): the opener and
        // closer are tSTRING_BEG / tSTRING_END tokens whose bytes can be any
        // punctuation. Plain `[ … ]` arrays keep their real bracket tokens.
        Node::ArrayNode { .. } => {
            let n = node.as_array_node().unwrap();
            if let (Some(o), Some(c)) = (n.opening_loc(), n.closing_loc())
                && o.as_slice().starts_with(b"%")
            {
                push(o);
                push(c);
            }
        }
        // Global variable assignment names (`$, = …`); the read / target
        // forms are leaves.
        Node::GlobalVariableWriteNode { .. } => {
            push(node.as_global_variable_write_node().unwrap().name_loc());
        }
        Node::GlobalVariableOrWriteNode { .. } => {
            push(node.as_global_variable_or_write_node().unwrap().name_loc());
        }
        Node::GlobalVariableAndWriteNode { .. } => {
            push(node.as_global_variable_and_write_node().unwrap().name_loc());
        }
        Node::GlobalVariableOperatorWriteNode { .. } => {
            push(
                node.as_global_variable_operator_write_node()
                    .unwrap()
                    .name_loc(),
            );
        }
        _ => {}
    }
}

/// Leaf-node hook: plain literals and gvar reads / targets. Call from
/// `Rule::enter_leaf`; needs the `LEAF` interest bit.
pub(crate) fn collect_leaf(node: &Node<'_>, masks: &mut Vec<(usize, usize)>) {
    match node {
        // Opening / content / closing pushed separately: heredocs keep their
        // body and terminator away from the opener.
        Node::StringNode { .. } => {
            let n = node.as_string_node().unwrap();
            if let Some(o) = n.opening_loc() {
                masks.push((o.start_offset(), o.end_offset()));
            }
            let c = n.content_loc();
            masks.push((c.start_offset(), c.end_offset()));
            if let Some(cl) = n.closing_loc() {
                masks.push((cl.start_offset(), cl.end_offset()));
            }
        }
        Node::XStringNode { .. } => {
            let n = node.as_x_string_node().unwrap();
            let (o, c, cl) = (n.opening_loc(), n.content_loc(), n.closing_loc());
            masks.push((o.start_offset(), o.end_offset()));
            masks.push((c.start_offset(), c.end_offset()));
            masks.push((cl.start_offset(), cl.end_offset()));
        }
        // Symbols (quoted / `%s` forms), regexps and gvar names are
        // contiguous: mask the whole node.
        Node::SymbolNode { .. }
        | Node::RegularExpressionNode { .. }
        | Node::MatchLastLineNode { .. }
        | Node::GlobalVariableReadNode { .. }
        | Node::GlobalVariableTargetNode { .. } => {
            let loc = node.location();
            masks.push((loc.start_offset(), loc.end_offset()));
        }
        _ => {}
    }
}

/// Fold walk-collected `masks` with the comments and the `__END__` data
/// segment into one sorted, non-overlapping range list.
pub(crate) fn merge(
    mut masks: Vec<(usize, usize)>,
    comments: &[(usize, usize)],
    data_start: Option<usize>,
    source_len: usize,
) -> Vec<(usize, usize)> {
    masks.extend_from_slice(comments);
    if let Some(ds) = data_start {
        masks.push((ds, source_len));
    }
    masks.sort_unstable();
    masks.retain(|r| r.0 < r.1);
    // Collapse overlaps so binary-search lookups see disjoint ranges.
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(masks.len());
    for (s, e) in masks {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// Whether `pos` lies inside one of the (sorted, disjoint) `masks`.
pub(crate) fn contains(masks: &[(usize, usize)], pos: usize) -> bool {
    let i = masks.partition_point(|r| r.1 <= pos);
    masks.get(i).is_some_and(|r| r.0 <= pos)
}
