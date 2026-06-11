use std::cell::RefCell;

use magnus::{Error, RArray, RString, Ruby, function};
use shirobai_core::rules::bundle::BundleConfig;

/// Bytes of `source`, borrowed straight from the Ruby heap without copying.
///
/// SAFETY: the slice is only read inside the same extension call while the GVL
/// is held, the analysis never re-enters Ruby while the borrow is alive, and
/// everything returned to Ruby is owned — so the backing `RString` cannot be
/// mutated, moved or freed during the borrow.
fn bytes(source: &RString) -> &[u8] {
    unsafe { source.as_slice() }
}

// Tuple mappers: one per cop, converting the core result type into the tuple
// shape handed to Ruby. Shared by the standalone entry points and `check_all`
// so the per-cop wire shape is defined in exactly one place.

fn map_debugger(v: Vec<shirobai_core::rules::debugger::DebuggerOffense>) -> Vec<(usize, usize)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset))
        .collect()
}

fn map_block_length(
    v: Vec<shirobai_core::rules::block_length::BlockLengthCandidate>,
) -> Vec<(usize, usize, usize, usize, String, String)> {
    v.into_iter()
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

fn map_block_nesting(
    (offenses, deepest): (
        Vec<shirobai_core::rules::block_nesting::BlockNestingOffense>,
        usize,
    ),
) -> (Vec<(usize, usize)>, usize) {
    let offenses = offenses
        .into_iter()
        .map(|o| (o.start_offset, o.end_offset))
        .collect();
    (offenses, deepest)
}

fn map_complexity(
    v: Vec<shirobai_core::rules::complexity::MethodComplexity>,
) -> Vec<(usize, usize, usize, String, usize, usize)> {
    v.into_iter()
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

#[allow(clippy::type_complexity)]
fn map_variable_number(
    (offenses, had_correct): (
        Vec<shirobai_core::rules::variable_number::VariableNumberOffense>,
        bool,
    ),
) -> (Vec<(usize, usize, u8, String, u8)>, bool) {
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

#[allow(clippy::type_complexity)]
fn map_method_name(
    (candidates, had_valid): (
        Vec<shirobai_core::rules::method_name::MethodNameCandidate>,
        bool,
    ),
) -> (
    Vec<(usize, usize, String, bool, u8, usize, usize, String)>,
    bool,
) {
    let candidates = candidates
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
        .collect();
    (candidates, had_valid)
}

fn map_safe_navigation_chain(
    v: Vec<shirobai_core::rules::safe_navigation_chain::SafeNavChainOffense>,
) -> Vec<(usize, usize, String, usize, usize)> {
    v.into_iter()
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

fn map_multiline_operation(
    v: Vec<shirobai_core::rules::multiline_operation_indentation::OperationIndentOffense>,
) -> Vec<(usize, usize, isize, String)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.message))
        .collect()
}

#[allow(clippy::type_complexity)]
fn map_multiline_method_call(
    v: Vec<shirobai_core::rules::multiline_method_call_indentation::MethodCallIndentOffense>,
) -> Vec<(usize, usize, isize, String, usize, usize, usize, usize)> {
    v.into_iter()
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

fn map_dot_position(
    v: Vec<shirobai_core::rules::dot_position::DotPositionOffense>,
) -> Vec<(usize, usize, usize, usize, usize)> {
    v.into_iter()
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

#[allow(clippy::type_complexity)]
fn map_line_length(
    v: Vec<shirobai_core::rules::line_length::LineLengthCandidate>,
) -> Vec<(usize, usize, usize, usize, usize, Vec<String>)> {
    v.into_iter()
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

fn map_line_length_breakables(
    v: Vec<shirobai_core::rules::line_length_breakable::Breakable>,
) -> Vec<(usize, usize, String)> {
    v.into_iter()
        .map(|b| (b.line_index, b.insert_offset, b.delimiter))
        .collect()
}

fn map_line_end_concatenation(
    v: Vec<shirobai_core::rules::line_end_concatenation::LineEndConcatOffense>,
) -> Vec<(usize, usize, String, usize, usize)> {
    v.into_iter()
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

fn map_argument_alignment(
    v: Vec<shirobai_core::rules::argument_alignment::ArgAlignOffense>,
) -> Vec<(usize, usize, isize, bool)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.autocorrect))
        .collect()
}

#[allow(clippy::type_complexity)]
fn map_first_argument_indentation(
    v: Vec<shirobai_core::rules::first_argument_indentation::FirstArgIndentOffense>,
) -> Vec<(usize, usize, isize, String, bool, usize, usize)> {
    v.into_iter()
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

fn map_closing_parenthesis_indentation(
    v: Vec<shirobai_core::rules::closing_parenthesis_indentation::ClosingParenIndentOffense>,
) -> Vec<(usize, usize, isize, String)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.message))
        .collect()
}

fn map_first_array_element_indentation(
    v: Vec<shirobai_core::rules::first_array_element_indentation::FirstArrayElemIndentOffense>,
) -> Vec<(usize, usize, isize, String)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.message))
        .collect()
}

#[allow(clippy::type_complexity)]
fn map_hash_each_methods(
    v: Vec<shirobai_core::rules::hash_each_methods::HashEachOffense>,
) -> Vec<(usize, usize, String, usize, usize, String, usize, usize)> {
    v.into_iter()
        .map(|o| {
            (
                o.start_offset,
                o.end_offset,
                o.message,
                o.replace_start,
                o.replace_end,
                o.replacement,
                o.remove_start,
                o.remove_end,
            )
        })
        .collect()
}

#[allow(clippy::type_complexity)]
fn map_void(
    v: Vec<shirobai_core::rules::void::VoidOffense>,
) -> Vec<(usize, usize, String, usize, usize, String, usize, usize)> {
    v.into_iter()
        .map(|o| {
            (
                o.start_offset,
                o.end_offset,
                o.message,
                o.replace_start,
                o.replace_end,
                o.replacement,
                o.remove_start,
                o.remove_end,
            )
        })
        .collect()
}

fn map_predicate_prefix(
    v: Vec<shirobai_core::rules::predicate_prefix::PredicatePrefixCandidate>,
) -> Vec<(usize, usize, String, bool, bool)> {
    v.into_iter()
        .map(|c| {
            (
                c.start_offset,
                c.end_offset,
                c.name,
                c.is_def,
                c.sorbet_boolean_sig,
            )
        })
        .collect()
}

fn map_redundant_self(
    v: Vec<shirobai_core::rules::redundant_self::RedundantSelfOffense>,
) -> Vec<(usize, usize, usize, usize)> {
    v.into_iter()
        .map(|o| (o.self_start, o.self_end, o.dot_start, o.dot_end))
        .collect()
}

#[allow(clippy::type_complexity)]
fn map_indentation_width(
    v: Vec<shirobai_core::rules::indentation_width::IndentationOffense>,
) -> Vec<(usize, usize, isize, String, bool, usize, usize)> {
    v.into_iter()
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

thread_local! {
    /// Bundle configs registered by `register_bundle_config` (token = index,
    /// no eviction: a lint run registers one entry per distinct `Config`
    /// object, i.e. a handful). Thread-local under the same assumption as
    /// `parse_cache`: an in-process RuboCop run drives its cops from a single
    /// thread (`--parallel` forks processes instead).
    static BUNDLE_CONFIGS: RefCell<Vec<BundleConfig>> = const { RefCell::new(Vec::new()) };
}

/// Ruby entry point registering a packed [`BundleConfig`] for `check_all`.
/// `nums` / `lists` follow the packing order documented on `BundleConfig`.
/// Returns the token to pass to `check_all`.
fn register_bundle_config(
    ruby: &Ruby,
    nums: Vec<i64>,
    lists: Vec<Vec<String>>,
) -> Result<usize, Error> {
    let cfg = BundleConfig::from_packed(&nums, lists)
        .map_err(|e| Error::new(ruby.exception_arg_error(), e))?;
    Ok(BUNDLE_CONFIGS.with(|cell| {
        let mut configs = cell.borrow_mut();
        configs.push(cfg);
        configs.len() - 1
    }))
}

/// Ruby entry point for the all-cop bundle: computes every cop's result for
/// `source` in one call, using the config registered under `token`. Returns a
/// fixed-order 22-slot Array; each slot carries that cop's existing tuple-array
/// shape (identical to the standalone entry point's return value). The slot
/// order is mirrored by `Shirobai::Dispatch::SLOTS` on the Ruby side:
///
/// 0 debugger / 1 block_length / 2 block_nesting / 3 complexity /
/// 4 variable_number / 5 method_name / 6 safe_navigation_chain /
/// 7 multiline_operation / 8 multiline_method_call / 9 dot_position /
/// 10 line_length / 11 line_length_breakables / 12 line_end_concatenation /
/// 13 argument_alignment / 14 first_argument_indentation / 15 redundant_self /
/// 16 indentation_width / 17 predicate_prefix /
/// 18 closing_parenthesis_indentation / 19 first_array_element_indentation /
/// 20 hash_each_methods / 21 void
fn check_all(ruby: &Ruby, source: RString, token: usize) -> Result<RArray, Error> {
    BUNDLE_CONFIGS.with(|cell| {
        let configs = cell.borrow();
        let cfg = configs.get(token).ok_or_else(|| {
            Error::new(
                ruby.exception_arg_error(),
                format!("unknown bundle config token {token}"),
            )
        })?;
        let r = shirobai_core::rules::bundle::check_all_bundle(bytes(&source), cfg);
        let ary = ruby.ary_new_capa(22);
        ary.push(map_debugger(r.debugger))?;
        ary.push(map_block_length(r.block_length))?;
        ary.push(map_block_nesting(r.block_nesting))?;
        ary.push(map_complexity(r.complexity))?;
        ary.push(map_variable_number(r.variable_number))?;
        ary.push(map_method_name(r.method_name))?;
        ary.push(map_safe_navigation_chain(r.safe_navigation_chain))?;
        ary.push(map_multiline_operation(r.multiline_operation))?;
        ary.push(map_multiline_method_call(r.multiline_method_call))?;
        ary.push(map_dot_position(r.dot_position))?;
        ary.push(map_line_length(r.line_length))?;
        ary.push(map_line_length_breakables(r.line_length_breakables))?;
        ary.push(map_line_end_concatenation(r.line_end_concatenation))?;
        ary.push(map_argument_alignment(r.argument_alignment))?;
        ary.push(map_first_argument_indentation(r.first_argument_indentation))?;
        ary.push(map_redundant_self(r.redundant_self))?;
        ary.push(map_indentation_width(r.indentation_width))?;
        ary.push(map_predicate_prefix(r.predicate_prefix))?;
        ary.push(map_closing_parenthesis_indentation(
            r.closing_parenthesis_indentation,
        ))?;
        ary.push(map_first_array_element_indentation(
            r.first_array_element_indentation,
        ))?;
        ary.push(map_hash_each_methods(r.hash_each_methods))?;
        ary.push(map_void(r.void))?;
        Ok(ary)
    })
}

/// Ruby entry point for `Lint/Debugger`. Takes the source, the flattened
/// `DebuggerMethods` list and the flattened `DebuggerRequires` list, and
/// returns `[[start_offset, end_offset], ...]`.
fn check_debugger(
    source: RString,
    methods: Vec<String>,
    requires: Vec<String>,
) -> Vec<(usize, usize)> {
    map_debugger(shirobai_core::rules::debugger::check_debugger(
        bytes(&source),
        &methods,
        &requires,
    ))
}

/// Ruby entry point for `Metrics/BlockLength`. Returns one entry per block
/// whose body exceeds `max`: `[[start, end, head_end, length, method_name,
/// receiver], ...]`. With `filtered` (no AllowedPatterns configured) the
/// `AllowedMethods` exclusion is applied in Rust from the String entries in
/// `allowed_methods`; otherwise all allow filtering stays on the Ruby side.
fn check_block_length(
    source: RString,
    max: usize,
    count_comments: bool,
    count_as_one: Vec<String>,
    allowed_methods: Vec<String>,
    filtered: bool,
) -> Vec<(usize, usize, usize, usize, String, String)> {
    map_block_length(
        shirobai_core::rules::block_length::check_block_length_filtered(
            bytes(&source),
            max,
            count_comments,
            &count_as_one,
            &allowed_methods,
            filtered,
        ),
    )
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
    map_block_nesting(shirobai_core::rules::block_nesting::check_block_nesting(
        bytes(&source),
        max,
        count_blocks,
        count_modifier_forms,
    ))
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
    map_complexity(
        shirobai_core::rules::complexity::check_complexity_exceeding(
            bytes(&source),
            max_cyclomatic,
            max_perceived,
        ),
    )
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
    map_variable_number(
        shirobai_core::rules::variable_number::check_variable_number(
            bytes(&source),
            style,
            flags,
            &allowed_identifiers,
        ),
    )
}

/// Ruby entry point for `Naming/MethodName`. Returns `[candidates, had_valid]`
/// where each candidate is `[start, end, name, valid, alt, fb_start, fb_end,
/// fb_name]`. With `filtered` (no AllowedPatterns / Forbidden* config) only the
/// invalid sites are returned and `had_valid` carries the
/// `correct_style_detected` bookkeeping; otherwise every site is returned.
#[allow(clippy::type_complexity)]
fn check_method_name(
    source: RString,
    style: u8,
    filtered: bool,
) -> (
    Vec<(usize, usize, String, bool, u8, usize, usize, String)>,
    bool,
) {
    map_method_name(
        shirobai_core::rules::method_name::check_method_name_filtered(
            bytes(&source),
            style,
            filtered,
        ),
    )
}

/// Ruby entry point for `Lint/SafeNavigationChain`. Returns
/// `[[start, end, replacement, wrap_start, wrap_end], ...]`.
fn check_safe_navigation_chain(
    source: RString,
    nil_methods: Vec<String>,
) -> Vec<(usize, usize, String, usize, usize)> {
    map_safe_navigation_chain(
        shirobai_core::rules::safe_navigation_chain::check_safe_navigation_chain(
            bytes(&source),
            &nil_methods,
        ),
    )
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
    map_multiline_operation(
        shirobai_core::rules::multiline_operation_indentation::check_multiline_operation_indentation(
            bytes(&source),
            style,
            indent_width,
            base_indent_width,
        ),
    )
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
    map_multiline_method_call(
        shirobai_core::rules::multiline_method_call_indentation::check_multiline_method_call_indentation(
            bytes(&source),
            style,
            indent_width,
            base_indent_width,
        ),
    )
}

/// Ruby entry point for `Layout/DotPosition`. Takes the source and the enforced
/// style (0=leading, 1=trailing). Returns `[[dot_start, dot_end, remove_start,
/// remove_end, insert_pos], ...]`.
fn check_dot_position(source: RString, style: u8) -> Vec<(usize, usize, usize, usize, usize)> {
    map_dot_position(shirobai_core::rules::dot_position::check_dot_position(
        bytes(&source),
        style,
    ))
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
    (
        map_multiline_operation(op_off),
        map_multiline_method_call(mc_off),
    )
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
    map_line_length(shirobai_core::rules::line_length::check_line_length(
        bytes(&source),
        max,
        tab_width,
    ))
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
    map_line_length_breakables(
        shirobai_core::rules::line_length_breakable::compute_breakables_filtered(
            bytes(&source),
            max,
            split_strings,
            Some(&candidates),
        ),
    )
}

/// Ruby entry point for `Style/LineEndConcatenation`. Returns one entry per
/// offense: `[[op_start, op_end, operator, replace_start, replace_end], ...]`.
/// `[op_start, op_end)` is the offense range; `[replace_start, replace_end)` is
/// the range Ruby replaces with `\`.
fn check_line_end_concatenation(source: RString) -> Vec<(usize, usize, String, usize, usize)> {
    map_line_end_concatenation(
        shirobai_core::rules::line_end_concatenation::check_line_end_concatenation(bytes(&source)),
    )
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
    map_argument_alignment(
        shirobai_core::rules::argument_alignment::check_argument_alignment(
            bytes(&source),
            style,
            indent_width,
            incompatible,
        ),
    )
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
    map_first_argument_indentation(
        shirobai_core::rules::first_argument_indentation::check_first_argument_indentation(
            bytes(&source),
            style,
            indent_width,
            enforce_fixed_with_no_line_break,
        ),
    )
}

/// Ruby entry point for `Naming/PredicatePrefix`. Takes the source, the
/// `NamePrefix` list and the `MethodDefinitionMacros` list, and returns the
/// definition sites whose name literally starts with a configured prefix:
/// `[[start, end, name, is_def, sorbet_boolean_sig], ...]`. The per-prefix
/// filtering (`allowed_method_name?`, `AllowedMethods`, `UseSorbetSigs`) stays
/// on the Ruby side, applied verbatim to these rare candidates.
fn check_predicate_prefix(
    source: RString,
    prefixes: Vec<String>,
    macros: Vec<String>,
) -> Vec<(usize, usize, String, bool, bool)> {
    map_predicate_prefix(
        shirobai_core::rules::predicate_prefix::check_predicate_prefix(
            bytes(&source),
            &prefixes,
            &macros,
        ),
    )
}

/// Ruby entry point for `Layout/ClosingParenthesisIndentation`. Takes the
/// source and the configured indentation width. Returns one entry per hanging
/// `)` that is misindented: `[[start, end, column_delta, message], ...]` —
/// `[start, end)` is the closing paren token, which Ruby both reports and
/// realigns by `column_delta` via `AlignmentCorrector`.
fn check_closing_parenthesis_indentation(
    source: RString,
    indent_width: usize,
) -> Vec<(usize, usize, isize, String)> {
    map_closing_parenthesis_indentation(
        shirobai_core::rules::closing_parenthesis_indentation::check_closing_parenthesis_indentation(
            bytes(&source),
            indent_width,
        ),
    )
}

/// Ruby entry point for `Layout/FirstArrayElementIndentation`. Takes the
/// source, the enforced style (0=special_inside_parentheses, 1=consistent,
/// 2=align_brackets), the configured indentation width and whether the cop is
/// disabled because `Layout/ArrayAlignment` enforces `with_fixed_indentation`
/// (which gates every style except `consistent`). Returns one entry per
/// misindented first element or hanging right bracket: `[[start, end,
/// column_delta, message], ...]` — `[start, end)` is the offense range, which
/// Ruby both reports and realigns by `column_delta` via `AlignmentCorrector`.
fn check_first_array_element_indentation(
    source: RString,
    style: u8,
    indent_width: usize,
    enforce_fixed_indentation: bool,
) -> Vec<(usize, usize, isize, String)> {
    map_first_array_element_indentation(
        shirobai_core::rules::first_array_element_indentation::check_first_array_element_indentation(
            bytes(&source),
            style,
            indent_width,
            enforce_fixed_indentation,
        ),
    )
}

/// Ruby entry point for `Style/HashEachMethods`. Takes the source and the
/// `AllowedReceivers` list (matched by receiver source name, like the
/// `AllowedReceivers` mixin). Returns one entry per offense: `[[start, end,
/// message, replace_start, replace_end, replacement, remove_start,
/// remove_end], ...]` — Ruby replaces `[replace_start, replace_end)` with
/// `replacement` and removes `[remove_start, remove_end)` when non-empty.
#[allow(clippy::type_complexity)]
fn check_hash_each_methods(
    source: RString,
    allowed_receivers: Vec<String>,
) -> Vec<(usize, usize, String, usize, usize, String, usize, usize)> {
    map_hash_each_methods(
        shirobai_core::rules::hash_each_methods::check_hash_each_methods(
            bytes(&source),
            &allowed_receivers,
        ),
    )
}

/// Ruby entry point for `Lint/Void`. Takes the source and the
/// `CheckForMethodsWithNoSideEffects` flag. Returns one entry per offense:
/// `[[start, end, message, replace_start, replace_end, replacement,
/// remove_start, remove_end], ...]` — Ruby replaces `[replace_start,
/// replace_end)` with `replacement` and removes `[remove_start, remove_end)`
/// when non-empty (both empty for the stock no-correction cases).
#[allow(clippy::type_complexity)]
fn check_void(
    source: RString,
    check_nonmutating: bool,
) -> Vec<(usize, usize, String, usize, usize, String, usize, usize)> {
    map_void(shirobai_core::rules::void::check_void(
        bytes(&source),
        check_nonmutating,
    ))
}

/// Ruby entry point for `Style/RedundantSelf`. Returns one entry per redundant
/// `self` receiver: `[[self_start, self_end, dot_start, dot_end], ...]`. The
/// `Kernel` method allow-list is supplied by Ruby.
fn check_redundant_self(
    source: RString,
    kernel_methods: Vec<String>,
) -> Vec<(usize, usize, usize, usize)> {
    map_redundant_self(shirobai_core::rules::redundant_self::check_redundant_self(
        bytes(&source),
        &kernel_methods,
    ))
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
    map_indentation_width(
        shirobai_core::rules::indentation_width::check_indentation_width(
            bytes(&source),
            cfg,
            &allowed_lines,
            &prior_ranges,
        ),
    )
}

#[magnus::init(name = "shirobai")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Shirobai")?;
    module.define_module_function(
        "register_bundle_config",
        function!(register_bundle_config, 2),
    )?;
    module.define_module_function("check_all", function!(check_all, 2))?;
    module.define_module_function("check_debugger", function!(check_debugger, 3))?;
    module.define_module_function("check_block_length", function!(check_block_length, 6))?;
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
    module.define_module_function("check_method_name", function!(check_method_name, 3))?;
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
        "check_predicate_prefix",
        function!(check_predicate_prefix, 3),
    )?;
    module.define_module_function(
        "check_first_argument_indentation",
        function!(check_first_argument_indentation, 4),
    )?;
    module.define_module_function(
        "check_indentation_width",
        function!(check_indentation_width, 4),
    )?;
    module.define_module_function(
        "check_closing_parenthesis_indentation",
        function!(check_closing_parenthesis_indentation, 2),
    )?;
    module.define_module_function(
        "check_first_array_element_indentation",
        function!(check_first_array_element_indentation, 4),
    )?;
    module.define_module_function(
        "check_hash_each_methods",
        function!(check_hash_each_methods, 2),
    )?;
    module.define_module_function("check_void", function!(check_void, 2))?;
    Ok(())
}
