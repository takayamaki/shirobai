use magnus::{Error, RString, Ruby, function};

/// Bytes of `source`, borrowed straight from the Ruby heap without copying.
///
/// SAFETY: the slice is only read inside the same extension call while the GVL
/// is held, the analysis never re-enters Ruby while the borrow is alive, and
/// everything returned to Ruby is owned — so the backing `RString` cannot be
/// mutated, moved or freed during the borrow.
fn bytes(source: &RString) -> &[u8] {
    unsafe { source.as_slice() }
}

/// Ruby entry point for `Lint/Debugger`. Takes the source, the flattened
/// `DebuggerMethods` list and the flattened `DebuggerRequires` list, and
/// returns `[[start_offset, end_offset], ...]`.
fn check_debugger(
    source: RString,
    methods: Vec<String>,
    requires: Vec<String>,
) -> Vec<(usize, usize)> {
    shirobai_core::rules::debugger::check_debugger(bytes(&source), &methods, &requires)
        .into_iter()
        .map(|o| (o.start_offset, o.end_offset))
        .collect()
}

/// Ruby entry point for `Metrics/BlockLength`. Returns one entry per block
/// whose body exceeds `max`: `[[start, end, length, method_name, receiver], ...]`.
/// Config-driven allow filtering is applied on the Ruby side.
fn check_block_length(
    source: RString,
    max: usize,
    count_comments: bool,
    count_as_one: Vec<String>,
) -> Vec<(usize, usize, usize, usize, String, String)> {
    shirobai_core::rules::block_length::check_block_length(
        bytes(&source),
        max,
        count_comments,
        &count_as_one,
    )
    .into_iter()
    .map(|c| {
        (
            c.start_offset,
            c.end_offset,
            c.head_end,
            c.length,
            c.method_name,
            c.receiver,
        )
    })
    .collect()
}

/// Ruby entry point for `Metrics/BlockNesting`. Takes the source, the `Max`
/// level and the `CountBlocks` / `CountModifierForms` flags. Returns
/// `[[[start, end], ...], deepest_level]`: the reportable offense ranges and the
/// deepest nesting level that exceeded `Max` (for `ExcludeLimit` bookkeeping).
fn check_block_nesting(
    source: RString,
    max: usize,
    count_blocks: bool,
    count_modifier_forms: bool,
) -> (Vec<(usize, usize)>, usize) {
    let (offenses, deepest) = shirobai_core::rules::block_nesting::check_block_nesting(
        bytes(&source),
        max,
        count_blocks,
        count_modifier_forms,
    );
    let offenses = offenses
        .into_iter()
        .map(|o| (o.start_offset, o.end_offset))
        .collect();
    (offenses, deepest)
}

/// Ruby entry point for the complexity cops. Returns one entry per method
/// whose score exceeds either threshold (`cyclomatic > max_cyclomatic ||
/// perceived > max_perceived`; `0` disables a threshold since scores start at
/// 1): `[[start, end, head_end, name, cyclomatic, perceived], ...]`.
#[allow(clippy::type_complexity)]
fn check_complexity(
    source: RString,
    max_cyclomatic: usize,
    max_perceived: usize,
) -> Vec<(usize, usize, usize, String, usize, usize)> {
    shirobai_core::rules::complexity::check_complexity_exceeding(
        bytes(&source),
        max_cyclomatic,
        max_perceived,
    )
    .into_iter()
    .map(|m| {
        (
            m.start_offset,
            m.end_offset,
            m.head_end,
            m.method_name,
            m.cyclomatic,
            m.perceived,
        )
    })
    .collect()
}

/// Ruby entry point for `Naming/VariableNumber`. Returns
/// `[[[start, end, id_type, name, alt], ...], had_correct]`.
#[allow(clippy::type_complexity)]
fn check_variable_number(
    source: RString,
    style: u8,
    flags: u8,
    allowed_identifiers: Vec<String>,
) -> (Vec<(usize, usize, u8, String, u8)>, bool) {
    let (offenses, had_correct) = shirobai_core::rules::variable_number::check_variable_number(
        bytes(&source),
        style,
        flags,
        &allowed_identifiers,
    );
    let offenses = offenses
        .into_iter()
        .map(|o| {
            (
                o.start_offset,
                o.end_offset,
                o.identifier_type,
                o.name,
                o.alternative,
            )
        })
        .collect();
    (offenses, had_correct)
}

/// Ruby entry point for `Naming/MethodName`. Returns one entry per method-name
/// site: `[[start, end, name, valid, alt, fb_start, fb_end, fb_name], ...]`.
/// AllowedPatterns / Forbidden filtering and style bookkeeping stay on the Ruby
/// side.
#[allow(clippy::type_complexity)]
fn check_method_name(
    source: RString,
    style: u8,
) -> Vec<(usize, usize, String, bool, u8, usize, usize, String)> {
    shirobai_core::rules::method_name::check_method_name(bytes(&source), style)
        .into_iter()
        .map(|c| {
            (
                c.start_offset,
                c.end_offset,
                c.name,
                c.valid,
                c.alternative,
                c.forbidden_start,
                c.forbidden_end,
                c.forbidden_name,
            )
        })
        .collect()
}

/// Ruby entry point for `Lint/SafeNavigationChain`. Returns
/// `[[start, end, replacement, wrap_start, wrap_end], ...]`.
fn check_safe_navigation_chain(
    source: RString,
    nil_methods: Vec<String>,
) -> Vec<(usize, usize, String, usize, usize)> {
    shirobai_core::rules::safe_navigation_chain::check_safe_navigation_chain(
        bytes(&source),
        &nil_methods,
    )
    .into_iter()
    .map(|o| {
        (
            o.start_offset,
            o.end_offset,
            o.replacement,
            o.wrap_start,
            o.wrap_end,
        )
    })
    .collect()
}

/// Ruby entry point for `Layout/MultilineOperationIndentation`. Takes the
/// source, the enforced style (0=aligned, 1=indented), the configured
/// indentation width and the base `Layout/IndentationWidth` width. Returns
/// `[[start, end, column_delta, message], ...]`.
fn check_multiline_operation_indentation(
    source: RString,
    style: u8,
    indent_width: usize,
    base_indent_width: usize,
) -> Vec<(usize, usize, isize, String)> {
    shirobai_core::rules::multiline_operation_indentation::check_multiline_operation_indentation(
        bytes(&source),
        style,
        indent_width,
        base_indent_width,
    )
    .into_iter()
    .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.message))
    .collect()
}

/// Ruby entry point for `Layout/MultilineMethodCallIndentation`. Takes the
/// source, the enforced style (0=aligned, 1=indented,
/// 2=indented_relative_to_receiver), the configured indentation width and the
/// base `Layout/IndentationWidth` width. Returns `[[start, end, column_delta,
/// message, block_body_start, block_body_end, block_end_start, block_end_end],
/// ...]` (block fields are `0` when the call has no multiline block).
#[allow(clippy::type_complexity)]
fn check_multiline_method_call_indentation(
    source: RString,
    style: u8,
    indent_width: usize,
    base_indent_width: usize,
) -> Vec<(usize, usize, isize, String, usize, usize, usize, usize)> {
    shirobai_core::rules::multiline_method_call_indentation::check_multiline_method_call_indentation(
        bytes(&source),
        style,
        indent_width,
        base_indent_width,
    )
    .into_iter()
    .map(|o| {
        (
            o.start_offset,
            o.end_offset,
            o.column_delta,
            o.message,
            o.block_body_start,
            o.block_body_end,
            o.block_end_start,
            o.block_end_end,
        )
    })
    .collect()
}

/// Ruby entry point for `Layout/DotPosition`. Takes the source and the enforced
/// style (0=leading, 1=trailing). Returns `[[dot_start, dot_end, remove_start,
/// remove_end, insert_pos], ...]`.
fn check_dot_position(source: RString, style: u8) -> Vec<(usize, usize, usize, usize, usize)> {
    shirobai_core::rules::dot_position::check_dot_position(bytes(&source), style)
        .into_iter()
        .map(|o| {
            (
                o.start_offset,
                o.end_offset,
                o.remove_start,
                o.remove_end,
                o.insert_pos,
            )
        })
        .collect()
}

/// Ruby entry point for the multiline-indentation bundle: runs
/// `Layout/MultilineOperationIndentation` and
/// `Layout/MultilineMethodCallIndentation` in one shared AST walk. Each config
/// is `[style, indent_width, base_indent_width]`. Returns
/// `[op_offenses, mc_offenses]` with the same per-cop tuple shapes as the
/// standalone entry points.
#[allow(clippy::type_complexity)]
fn check_multiline_bundle(
    source: RString,
    op: (u8, usize, usize),
    mc: (u8, usize, usize),
) -> (
    Vec<(usize, usize, isize, String)>,
    Vec<(usize, usize, isize, String, usize, usize, usize, usize)>,
) {
    let (op_off, mc_off) = shirobai_core::rules::bundle::check_multiline_bundle(
        bytes(&source),
        op.0,
        op.1,
        op.2,
        mc.0,
        mc.1,
        mc.2,
    );
    let op_off = op_off
        .into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.message))
        .collect();
    let mc_off = mc_off
        .into_iter()
        .map(|o| {
            (
                o.start_offset,
                o.end_offset,
                o.column_delta,
                o.message,
                o.block_body_start,
                o.block_body_end,
                o.block_end_start,
                o.block_end_end,
            )
        })
        .collect();
    (op_off, mc_off)
}

/// Ruby entry point for `Layout/LineLength`. Walks every line and returns one
/// entry per line whose visible length exceeds `max`: `[[line_index, length,
/// line_start, line_end, indentation_difference, heredoc_delimiters], ...]`
/// (`heredoc_delimiters` is the list of end delimiters of every heredoc whose
/// body covers the line). Regex-based exemptions (AllowedPatterns / AllowURI /
/// cop directives) and the `AllowHeredoc` delimiter filtering stay on the Ruby
/// side.
#[allow(clippy::type_complexity)]
fn check_line_length(
    source: RString,
    max: usize,
    tab_width: usize,
) -> Vec<(usize, usize, usize, usize, usize, Vec<String>)> {
    shirobai_core::rules::line_length::check_line_length(bytes(&source), max, tab_width)
        .into_iter()
        .map(|c| {
            (
                c.line_index,
                c.length,
                c.line_start,
                c.line_end,
                c.indentation_difference,
                c.heredoc_delimiters,
            )
        })
        .collect()
}

/// Ruby entry point for `Layout/LineLength` auto-correction. Returns one entry
/// per source line that can be broken: `[[line_index, insert_offset,
/// delimiter], ...]`. `insert_offset` is the byte offset before which a break is
/// inserted; `delimiter` is empty for an ordinary newline break or the string
/// quote for a `SplitStrings` continuation. `candidate_lines` is the set of
/// 0-based line indexes that exceed `Max` (the `LineLength` candidates); only
/// those lines' breakables are computed, since a non-candidate line can never
/// become an offense and therefore never consumes a breakable range.
fn check_line_length_breakables(
    source: RString,
    max: usize,
    split_strings: bool,
    candidate_lines: Vec<usize>,
) -> Vec<(usize, usize, String)> {
    let candidates: std::collections::HashSet<usize> = candidate_lines.into_iter().collect();
    shirobai_core::rules::line_length_breakable::compute_breakables_filtered(
        bytes(&source),
        max,
        split_strings,
        Some(&candidates),
    )
    .into_iter()
    .map(|b| (b.line_index, b.insert_offset, b.delimiter))
    .collect()
}

/// Ruby entry point for `Style/LineEndConcatenation`. Returns one entry per
/// offense: `[[op_start, op_end, operator, replace_start, replace_end], ...]`.
/// `[op_start, op_end)` is the offense range; `[replace_start, replace_end)` is
/// the range Ruby replaces with `\`.
fn check_line_end_concatenation(source: RString) -> Vec<(usize, usize, String, usize, usize)> {
    shirobai_core::rules::line_end_concatenation::check_line_end_concatenation(bytes(&source))
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

/// Ruby entry point for `Layout/ArgumentAlignment`. Takes the source, the
/// enforced style (0=with_first_argument, 1=with_fixed_indentation), the
/// configured indentation width and whether autocorrect is incompatible with
/// `Layout/HashAlignment`'s separator styles (which disables this cop's
/// autocorrect). Returns `[[start, end, column_delta, autocorrect], ...]`.
fn check_argument_alignment(
    source: RString,
    style: u8,
    indent_width: usize,
    incompatible: bool,
) -> Vec<(usize, usize, isize, bool)> {
    shirobai_core::rules::argument_alignment::check_argument_alignment(
        bytes(&source),
        style,
        indent_width,
        incompatible,
    )
    .into_iter()
    .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.autocorrect))
    .collect()
}

/// Ruby entry point for `Layout/FirstArgumentIndentation`. Takes the source,
/// the enforced style (0=special_for_inner_method_call_in_parentheses,
/// 1=consistent, 2=consistent_relative_to_receiver,
/// 3=special_for_inner_method_call), the configured indentation width and
/// whether the cop is disabled because `Layout/ArgumentAlignment` enforces
/// `with_fixed_indentation` while `Layout/FirstMethodArgumentLineBreak` is off.
/// Returns `[[start, end, column_delta, message, autocorrect, correct_start,
/// correct_end], ...]`.
#[allow(clippy::type_complexity)]
fn check_first_argument_indentation(
    source: RString,
    style: u8,
    indent_width: usize,
    enforce_fixed_with_no_line_break: bool,
) -> Vec<(usize, usize, isize, String, bool, usize, usize)> {
    shirobai_core::rules::first_argument_indentation::check_first_argument_indentation(
        bytes(&source),
        style,
        indent_width,
        enforce_fixed_with_no_line_break,
    )
    .into_iter()
    .map(|o| {
        (
            o.start_offset,
            o.end_offset,
            o.column_delta,
            o.message,
            o.autocorrect,
            o.correct_start,
            o.correct_end,
        )
    })
    .collect()
}

/// Ruby entry point for `Style/RedundantSelf`. Returns one entry per redundant
/// `self` receiver: `[[self_start, self_end, dot_start, dot_end], ...]`. The
/// `Kernel` method allow-list is supplied by Ruby.
fn check_redundant_self(
    source: RString,
    kernel_methods: Vec<String>,
) -> Vec<(usize, usize, usize, usize)> {
    shirobai_core::rules::redundant_self::check_redundant_self(bytes(&source), &kernel_methods)
        .into_iter()
        .map(|o| (o.self_start, o.self_end, o.dot_start, o.dot_end))
        .collect()
}

/// Ruby entry point for `Layout/IndentationWidth`. `config` packs
/// `[width, relative_to_receiver, access_modifier_outdent,
/// indented_internal_methods, end_align, def_end_align_def, use_tabs]`.
/// `allowed_lines` is the set of 1-based line numbers whose content matches an
/// `AllowedPatterns` entry (regex matching stays in Ruby). `prior_ranges` are the
/// correction ranges already registered by this cop instance in earlier
/// autocorrect iterations (`other_offense_in_same_range?` persists across passes).
/// Returns `[[start, end, column_delta, message, autocorrect, correct_start, correct_end], ...]`.
#[allow(clippy::type_complexity)]
fn check_indentation_width(
    source: RString,
    config: Vec<i64>,
    allowed_lines: Vec<usize>,
    prior_ranges: Vec<(usize, usize)>,
) -> Vec<(usize, usize, isize, String, bool, usize, usize)> {
    let cfg = shirobai_core::rules::indentation_width::Config {
        width: config[0] as usize,
        relative_to_receiver: config[1] != 0,
        access_modifier_outdent: config[2] != 0,
        indented_internal_methods: config[3] != 0,
        end_align: config[4] as u8,
        def_end_align_def: config[5] != 0,
        use_tabs: config[6] != 0,
    };
    shirobai_core::rules::indentation_width::check_indentation_width(
        bytes(&source),
        cfg,
        &allowed_lines,
        &prior_ranges,
    )
    .into_iter()
    .map(|o| {
        (
            o.start_offset,
            o.end_offset,
            o.column_delta,
            o.message,
            o.autocorrect,
            o.correct_start,
            o.correct_end,
        )
    })
    .collect()
}

#[magnus::init(name = "shirobai")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Shirobai")?;
    module.define_module_function("check_debugger", function!(check_debugger, 3))?;
    module.define_module_function("check_block_length", function!(check_block_length, 4))?;
    module.define_module_function("check_complexity", function!(check_complexity, 3))?;
    module.define_module_function("check_block_nesting", function!(check_block_nesting, 4))?;
    module.define_module_function("check_variable_number", function!(check_variable_number, 4))?;
    module.define_module_function(
        "check_safe_navigation_chain",
        function!(check_safe_navigation_chain, 2),
    )?;
    module.define_module_function(
        "check_multiline_operation_indentation",
        function!(check_multiline_operation_indentation, 4),
    )?;
    module.define_module_function(
        "check_multiline_method_call_indentation",
        function!(check_multiline_method_call_indentation, 4),
    )?;
    module.define_module_function("check_dot_position", function!(check_dot_position, 2))?;
    module.define_module_function("check_line_length", function!(check_line_length, 3))?;
    module.define_module_function(
        "check_line_length_breakables",
        function!(check_line_length_breakables, 4),
    )?;
    module.define_module_function("check_method_name", function!(check_method_name, 2))?;
    module.define_module_function(
        "check_line_end_concatenation",
        function!(check_line_end_concatenation, 1),
    )?;
    module.define_module_function(
        "check_multiline_bundle",
        function!(check_multiline_bundle, 3),
    )?;
    module.define_module_function(
        "check_argument_alignment",
        function!(check_argument_alignment, 4),
    )?;
    module.define_module_function("check_redundant_self", function!(check_redundant_self, 2))?;
    module.define_module_function(
        "check_first_argument_indentation",
        function!(check_first_argument_indentation, 4),
    )?;
    module.define_module_function(
        "check_indentation_width",
        function!(check_indentation_width, 4),
    )?;
    Ok(())
}
