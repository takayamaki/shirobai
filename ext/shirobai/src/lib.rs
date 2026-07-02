use std::cell::RefCell;

use magnus::{Error, RArray, RString, Ruby, function};
use shirobai_core::rules::bundle::BundleConfig;

/// Wire type for `Style/RedundantSelfAssignment` result tuples:
/// `(op_start, op_end, method_name, kind, range_start, range_end, rhs_start, rhs_end)`.
type RedundantSelfAssignmentTuple = (usize, usize, String, u8, usize, usize, usize, usize);

/// Wire type for `Style/PercentLiteralDelimiters` result tuples:
/// `(start, end, begin_start, begin_end, end_start, end_end, type_index)`.
type PercentLiteralDelimitersTuple = (usize, usize, usize, usize, usize, usize, u8);

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

/// `Metrics/MethodLength` per-method results: `[start, end, head_end, length,
/// name, filterable]`. The Ruby wrapper applies the `AllowedMethods` /
/// `AllowedPatterns` filter (only to `filterable` candidates) and builds the
/// message.
fn map_method_length(
    v: Vec<shirobai_core::rules::method_length::MethodLengthCandidate>,
) -> Vec<(usize, usize, usize, usize, String, bool)> {
    v.into_iter()
        .map(|c| {
            (
                c.start_offset,
                c.end_offset,
                c.head_end,
                c.length,
                c.name,
                c.filterable,
            )
        })
        .collect()
}

/// `Metrics/ClassLength` per-class results: `[start, end, head_end, length,
/// sclass]`. The Ruby wrapper builds the message and skips `sclass`
/// candidates in LSP mode (stock errors out on those instead of reporting).
fn map_class_length(
    v: Vec<shirobai_core::rules::class_length::ClassLengthCandidate>,
) -> Vec<(usize, usize, usize, usize, bool)> {
    v.into_iter()
        .map(|c| (c.start_offset, c.end_offset, c.head_end, c.length, c.sclass))
        .collect()
}

/// `Metrics/ModuleLength` per-module results: `[start, end, head_end,
/// length]`. The Ruby wrapper builds the message.
fn map_module_length(
    v: Vec<shirobai_core::rules::module_length::ModuleLengthCandidate>,
) -> Vec<(usize, usize, usize, usize)> {
    v.into_iter()
        .map(|c| (c.start_offset, c.end_offset, c.head_end, c.length))
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

/// `Metrics/AbcSize` per-method results: `[start, end, head_end, name,
/// assignments, branches, conditions]`. The Ruby side derives the float score,
/// the `<a, b, c>` vector and the message (floats never cross the FFI).
#[allow(clippy::type_complexity)]
fn map_abc_size(
    v: Vec<shirobai_core::rules::abc_size::AbcMethod>,
) -> Vec<(usize, usize, usize, String, u64, u64, u64)> {
    v.into_iter()
        .map(|m| {
            (
                m.start_offset,
                m.end_offset,
                m.head_end,
                m.method_name,
                m.assignments,
                m.branches,
                m.conditions,
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

/// Maps to `[start, end, column_delta, message, correct_target]` where
/// `correct_target` is `-1` for a right brace (plain range correction), `0`
/// when the first pair's value begins after its key (correct the key's line
/// only), and `1` otherwise (correct the whole pair node).
fn map_first_hash_element_indentation(
    v: Vec<shirobai_core::rules::first_hash_element_indentation::FirstHashElemIndentOffense>,
) -> Vec<(usize, usize, isize, String, i64)> {
    v.into_iter()
        .map(|o| {
            let target = match o.correct_whole_pair {
                None => -1,
                Some(false) => 0,
                Some(true) => 1,
            };
            (o.start_offset, o.end_offset, o.column_delta, o.message, target)
        })
        .collect()
}

/// Maps each `Layout/HashAlignment` offense to
/// `[group, start, end, message, has_value, [key_delta, sep_delta, val_delta],
///   [key_start, key_end, key_column], [op_start, op_end],
///   [value_start, value_end]]`. `group` is the source hash's `on_hash`-order
/// index (so the wrapper can confine a `ClobberingError` to its own hash, as
/// stock's per-callback `add_offense` does); `message` selects the offense text
/// (0 key / 1 separator / 2 table / 3 kwsplat); when `has_value` is false Ruby
/// adjusts the node by `key_delta` only, else it adjusts key / separator /
/// value ranges by their deltas (with the stock `-key.column` clamp).
#[allow(clippy::type_complexity)]
fn map_hash_alignment(
    v: Vec<shirobai_core::rules::hash_alignment::HashAlignmentOffense>,
) -> Vec<(
    usize,
    usize,
    usize,
    u8,
    bool,
    (isize, isize, isize),
    (usize, usize, usize),
    (usize, usize),
    (usize, usize),
)> {
    v.into_iter()
        .map(|o| {
            (
                o.group,
                o.start_offset,
                o.end_offset,
                o.message,
                o.has_value,
                (o.key_delta, o.separator_delta, o.value_delta),
                (o.key_start, o.key_end, o.key_column),
                (o.op_start, o.op_end),
                (o.value_start, o.value_end),
            )
        })
        .collect()
}

/// `Style/HashSyntax` correction ops: `[kind, start, end, text]` —
/// 0 replace / 1 remove / 2 insert_before / 3 insert_after.
type HashSyntaxOps = Vec<(u8, usize, usize, String)>;

/// Maps each `Style/HashSyntax` walk record to
/// `[is_offense, start, end, message, detect, [[kind, s, e, text], ...]]`.
/// `message` selects the offense text (0 ruby19 / 1 hash_rockets /
/// 2 no_mixed_keys / 3 omit / 4 explicit / 5 do_not_mix_omit /
/// 6 do_not_mix_explicit); `detect` is the stock detection side effect the
/// wrapper replays (0 opposite_style / 1 correct_style / 2 disabled / 3 none).
/// When `is_offense` is false the record is a pure `correct_style_detected`
/// marker (no caret, no ops).
#[allow(clippy::type_complexity)]
fn map_hash_syntax(
    v: Vec<shirobai_core::rules::hash_syntax::HashSyntaxOffense>,
) -> Vec<(bool, usize, usize, u8, u8, HashSyntaxOps)> {
    use shirobai_core::rules::hash_syntax::Detect;
    v.into_iter()
        .map(|o| {
            let detect = match o.detect {
                Detect::OppositeStyle => 0u8,
                Detect::CorrectStyle => 1u8,
                Detect::Disabled => 2u8,
                Detect::None => 3u8,
            };
            let ops = o
                .ops
                .into_iter()
                .map(|op| (op.kind, op.start, op.end, op.text))
                .collect();
            (o.is_offense, o.start_offset, o.end_offset, o.message, detect, ops)
        })
        .collect()
}

/// `Style/StringLiterals` records. Each entry is `[is_offense, start, end,
/// message, detect, fix, content]`. `message` selects the text (0 prefer single
/// / 1 prefer double / 2 inconsistent); `detect` is the replayed detection side
/// effect (0 opposite_style / 1 correct_style / 3 none); `fix` is the
/// autocorrect kind (0 single -> `to_string_literal(content)`, 1 double ->
/// `content.inspect`, 2 none). When `is_offense` is false the record is a pure
/// `correct_style_detected` marker.
#[allow(clippy::type_complexity)]
fn map_string_literals(
    v: Vec<shirobai_core::rules::string_literals::StringLiteralsOffense>,
) -> Vec<(bool, usize, usize, u8, u8, u8, String)> {
    v.into_iter()
        .map(|o| {
            (
                o.is_offense,
                o.start_offset,
                o.end_offset,
                o.message,
                o.detect,
                o.fix,
                o.content,
            )
        })
        .collect()
}

/// `Style/StringLiteralsInInterpolation` records. Each entry is `[is_offense,
/// start, end, detect, fix, content]`. `detect` is the replayed detection side
/// effect (0 opposite_style / 1 correct_style); `fix` is the autocorrect kind
/// (0 single -> `to_string_literal(content)`, 1 double -> `content.inspect`).
/// When `is_offense` is false the record is a pure `correct_style_detected`
/// marker.
#[allow(clippy::type_complexity)]
fn map_string_literals_in_interpolation(
    v: Vec<
        shirobai_core::rules::string_literals_in_interpolation::StringLiteralsInInterpolationOffense,
    >,
) -> Vec<(bool, usize, usize, u8, u8, String)> {
    v.into_iter()
        .map(|o| {
            (
                o.is_offense,
                o.start_offset,
                o.end_offset,
                o.detect,
                o.fix,
                o.content,
            )
        })
        .collect()
}

/// `Style/TrailingCommaInArguments` records. Each entry is `[start, end,
/// message, fix]`. `message` selects the text (0 avoid/no_comma, 1 avoid/comma,
/// 2 avoid/consistent_comma, 3 avoid/diff_comma, 4 put); `fix` is the corrector
/// op (0 avoid -> remove the comma, 1 put -> insert a comma after the range).
fn map_trailing_comma_in_arguments(
    v: Vec<shirobai_core::rules::trailing_comma_in_arguments::TrailingCommaInArgumentsOffense>,
) -> Vec<(usize, usize, u8, u8)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.message, o.fix))
        .collect()
}

/// `Layout/TrailingEmptyLines`: at most one record per file, flattened to a
/// `Vec` of length 0 or 1. Each entry is `[report_start, report_end, ac_start,
/// ac_end, replacement, blank_lines]`: Ruby reports the caret range
/// `[report_start, report_end)`, replaces `[ac_start, ac_end)` with
/// `replacement`, and builds the message from `blank_lines` (`-1` final newline
/// missing, `0` trailing blank line missing, else the "N trailing blank lines"
/// form).
fn map_trailing_empty_lines(
    v: Option<shirobai_core::rules::trailing_empty_lines::TrailingEmptyLinesOffense>,
) -> Vec<(usize, usize, usize, usize, String, i64)> {
    v.into_iter()
        .map(|o| {
            (
                o.report_start,
                o.report_end,
                o.ac_start,
                o.ac_end,
                o.replacement.to_string(),
                o.blank_lines,
            )
        })
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

fn map_useless_access_modifier(
    v: Vec<shirobai_core::rules::useless_access_modifier::UselessAccessModifierOffense>,
) -> Vec<(usize, usize, String)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.name))
        .collect()
}

/// Shared by all six `EmptyLinesAroundBody`-family slots: `[[start, end,
/// insert, message], ...]` — `[start, end)` is the first character of the
/// offense line; `insert` selects the `EmptyLineCorrector` arm (`false` =
/// remove the range, `true` = insert `"\n"` before it).
fn map_empty_lines(
    v: Vec<shirobai_core::rules::empty_lines_around_body::EmptyLineOffense>,
) -> Vec<(usize, usize, bool, String)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.insert, o.message))
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

fn map_indentation_consistency(
    v: Vec<shirobai_core::rules::indentation_consistency::ConsistencyOffense>,
) -> Vec<(usize, usize, isize, bool)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.autocorrect))
        .collect()
}

/// `Layout/EmptyLineBetweenDefs`: `[[start, end, message, insert, pos, n], ...]`
/// — `[start, end)` is the second member's `def_location`; `insert` selects the
/// corrector arm (`true` = insert `"\n" * n` after `range_between(pos, pos+1)`,
/// `false` = remove `range_between(pos, pos + n)`).
fn map_empty_line_between_defs(
    v: Vec<shirobai_core::rules::empty_line_between_defs::EmptyLineBetweenDefsOffense>,
) -> Vec<(usize, usize, String, bool, usize, usize)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.message, o.insert, o.pos, o.n))
        .collect()
}

/// `Layout/EmptyLinesAroundArguments`: `[[start, end], ...]` — `[start, end)` is
/// the offense line range (the whole `last_line - 1` line plus its `\n`), which
/// the Ruby wrapper reports and removes verbatim.
fn map_empty_lines_around_arguments(
    v: Vec<shirobai_core::rules::empty_lines_around_arguments::EmptyLinesAroundArgumentsOffense>,
) -> Vec<(usize, usize)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset))
        .collect()
}

/// `Layout/SpaceAroundMethodCallOperator`: `[[start, end], ...]` — `[start, end)`
/// is both the offense highlight and the autocorrect removal range (the
/// whitespace run the Ruby wrapper reports and removes verbatim).
fn map_space_around_method_call_operator(
    v: Vec<
        shirobai_core::rules::space_around_method_call_operator::SpaceAroundMethodCallOperatorOffense,
    >,
) -> Vec<(usize, usize)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset))
        .collect()
}

/// `Layout/SpaceAroundKeyword`: `[[start, end, before], ...]` — `[start, end)` is
/// the keyword range (the offense highlight); `before` is `true` for a missing
/// space *before* the keyword (the Ruby wrapper inserts a space before the
/// range, `MSG_BEFORE`) and `false` for a missing space *after* it (inserts a
/// space after, `MSG_AFTER`).
fn map_space_around_keyword(
    v: Vec<shirobai_core::rules::space_around_keyword::SpaceAroundKeywordOffense>,
) -> Vec<(usize, usize, bool)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.before))
        .collect()
}

/// `Layout/SpaceInsideBlockBraces`: `[[start, end, message_code], ...]` —
/// `[start, end)` is the offense range stock reports (also the autocorrect
/// target); `message_code` selects the fixed message / corrector action the
/// Ruby wrapper reproduces from `range.source` (0 missing-inside-{, 1 inside-{
/// detected, 2 missing-inside-}, 3 inside-} detected, 4 empty-space detected,
/// 5 empty-space missing, 6 `{|` missing, 7 `{|` detected).
fn map_space_inside_block_braces(
    v: Vec<shirobai_core::rules::space_inside_block_braces::SpaceInsideBlockBracesOffense>,
) -> Vec<(usize, usize, u8)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.message.code()))
        .collect()
}

/// `Layout/EndAlignment`: `[[end_start, end_end, matching, message, align_column], ...]`
/// — `[end_start, end_end)` is the `end` keyword range; `matching` is the list
/// of style ids the `end` already aligns with, in the path's hash order
/// (`matching.keys`: 0 = keyword, 1 = variable, 2 = start_of_line);
/// `message`/`align_column` are empty/0 when `matching` includes the configured
/// style (no offense).
fn map_end_alignment(
    v: Vec<shirobai_core::rules::end_alignment::EndAlignmentRecord>,
) -> Vec<(usize, usize, Vec<u8>, String, usize)> {
    v.into_iter()
        .map(|r| {
            let (message, align) = r
                .offense
                .map(|o| (o.message, o.align_column))
                .unwrap_or_default();
            (r.end_start, r.end_end, r.matching, message, align)
        })
        .collect()
}

/// `Layout/DefEndAlignment`: `[[end_start, end_end, matching, message, align_column], ...]`
/// — same shape as `map_end_alignment`. `matching` lists the style ids the
/// `end` already aligns with (0 = start_of_line, 1 = def), in the path's hash
/// order; `message`/`align_column` are empty/0 when the configured style is
/// matched (no offense).
fn map_def_end_alignment(
    v: Vec<shirobai_core::rules::def_end_alignment::DefEndAlignmentRecord>,
) -> Vec<(usize, usize, Vec<u8>, String, usize)> {
    v.into_iter()
        .map(|r| {
            let (message, align) = r
                .offense
                .map(|o| (o.message, o.align_column))
                .unwrap_or_default();
            (r.end_start, r.end_end, r.matching, message, align)
        })
        .collect()
}

/// `Layout/AccessModifierIndentation`: `[[start, end, message, column_delta],
/// ...]`. `[start, end)` is the bare `private` / `protected` / `public` /
/// `module_function` modifier's source range (offense highlight). `message`
/// is the formatted stock `MSG` (empty when the modifier matches the
/// configured style — the wrapper then calls `correct_style_detected`).
/// `column_delta` is the signed `expected - actual` shift the wrapper feeds
/// to `AlignmentCorrector.correct`.
fn map_access_modifier_indentation(
    v: Vec<shirobai_core::rules::access_modifier_indentation::AccessModifierIndentationRecord>,
) -> Vec<(usize, usize, String, i64)> {
    v.into_iter()
        .map(|r| {
            let (message, delta) = r
                .offense
                .map(|o| (o.message, o.column_delta))
                .unwrap_or_default();
            (r.start, r.end, message, delta)
        })
        .collect()
}

/// `Style/StabbyLambdaParentheses`: `[[start, end, paren_open_start,
/// paren_open_end, paren_close_start, paren_close_end, message], ...]`.
/// `[start, end)` is the `args.loc.expression` source range (offense highlight
/// = stock's `add_offense(arguments)`). Under `require_parentheses`
/// `paren_*` fields are zero and the wrapper calls
/// `corrector.wrap(args_range, '(', ')')`. Under `require_no_parentheses`
/// `[paren_open_start, paren_open_end)` is the `(` 1-byte range (replaced
/// with `''`) and `[paren_close_start, paren_close_end)` is the `)` 1-byte
/// range (removed). `message` is the formatted stock `MSG_*`.
fn map_stabby_lambda_parentheses(
    v: Vec<shirobai_core::rules::stabby_lambda_parentheses::StabbyLambdaParenthesesOffense>,
) -> Vec<(usize, usize, usize, usize, usize, usize, String)> {
    v.into_iter()
        .map(|o| {
            (
                o.start,
                o.end,
                o.paren_open_start,
                o.paren_open_end,
                o.paren_close_start,
                o.paren_close_end,
                o.message.to_string(),
            )
        })
        .collect()
}

/// `Lint/AmbiguousBlockAssociation` wire tuple: see
/// `map_ambiguous_block_association` for field semantics. A `type` alias keeps
/// clippy's `type_complexity` lint quiet without an `#[allow]`.
type AmbiguousBlockAssociationTuple = (
    usize, usize, usize, usize, usize, usize, usize, usize, usize,
);

/// `Lint/AmbiguousBlockAssociation`: `[[start, end, param_start, param_end,
/// inner_send_start, inner_send_end, ac_open_start, ac_open_end, ac_close_pos],
/// ...]`. `[start, end)` is the outer call's full source range (offense
/// highlight). `[param_start, param_end)` is the last argument's source range
/// (the block-bearing inner call — substituted into MSG as `%<param>s`).
/// `[inner_send_start, inner_send_end)` is the inner block sender's source
/// range (substituted into MSG as `%<method>s`). Autocorrect: replace
/// `[ac_open_start, ac_open_end)` with `(` (`corrector.remove(range)` +
/// `corrector.insert_before(range, '(')` in stock) and insert `)` at
/// `ac_close_pos` (the last argument's end).
fn map_ambiguous_block_association(
    v: Vec<shirobai_core::rules::ambiguous_block_association::AmbiguousBlockAssociationOffense>,
) -> Vec<AmbiguousBlockAssociationTuple> {
    v.into_iter()
        .map(|o| {
            (
                o.start_offset,
                o.end_offset,
                o.param_start,
                o.param_end,
                o.inner_send_start,
                o.inner_send_end,
                o.ac_open_start,
                o.ac_open_end,
                o.ac_close_pos,
            )
        })
        .collect()
}

/// `Style/HashTransformKeys`: `[[start, end, message, edits], ...]` where
/// `edits` is `[[edit_start, edit_end, replacement], ...]`. Stock's mixin
/// emits four `corrector.replace` calls per offense (strip the `Hash[..]`
/// brackets or the trailing `.to_h`, swap the selector to `transform_keys`,
/// rewrite the block argument list, replace the block body). We pack them
/// flat so the wrapper just iterates and replays each one.
#[allow(clippy::type_complexity)]
fn map_hash_transform_keys(
    v: Vec<shirobai_core::rules::hash_transform_keys::HashTransformKeysOffense>,
) -> Vec<(usize, usize, String, Vec<(usize, usize, String)>)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.message, o.edits))
        .collect()
}

/// `Lint/RequireParentheses`: `[[start_offset, end_offset], ...]`.
fn map_require_parentheses(
    v: Vec<shirobai_core::rules::require_parentheses::RequireParenthesesOffense>,
) -> Vec<(usize, usize)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset))
        .collect()
}

/// `Lint/SelfAssignment`: `[[start_offset, end_offset, rbs_anchor_offset], ...]`.
/// `rbs_anchor_offset` is the end byte of the subnode stock would key into
/// `processed_source.ast_with_comments` for an `#:` RBS inline annotation —
/// the Ruby wrapper only consults it when `AllowRBSInlineAnnotation: true`.
fn map_self_assignment(
    v: Vec<shirobai_core::rules::self_assignment::SelfAssignmentOffense>,
) -> Vec<(usize, usize, usize)> {
    v.into_iter()
        .map(|o| (o.start_offset, o.end_offset, o.rbs_anchor_offset))
        .collect()
}

/// `Style/NestedParenthesizedCalls`: `[[start, end, ac_open_start, ac_open_end,
/// ac_close_pos], ...]`. `[start, end)` is the inner unparenthesized call's
/// full source range (offense highlight and message source); the wrapper
/// replaces `[ac_open_start, ac_open_end)` with `(` (stock's surrounding-space
/// `replace` op) and inserts `)` at `ac_close_pos` (right after the last inner
/// argument).
fn map_nested_parenthesized_calls(
    v: Vec<shirobai_core::rules::nested_parenthesized_calls::NestedParenthesizedCallsOffense>,
) -> Vec<(usize, usize, usize, usize, usize)> {
    v.into_iter()
        .map(|o| {
            (
                o.start_offset,
                o.end_offset,
                o.ac_open_start,
                o.ac_open_end,
                o.ac_close_pos,
            )
        })
        .collect()
}

/// `Lint/UnreachableCode`: `[[start, end], ...]`. `[start, end)` is the
/// byte range of the unreachable expression — exactly stock's
/// `add_offense(expression2)` range. The message is fixed (`MSG` constant)
/// and no autocorrect is attached, matching stock.
fn map_unreachable_code(
    v: Vec<shirobai_core::rules::unreachable_code::UnreachableCodeOffense>,
) -> Vec<(usize, usize)> {
    v.into_iter().map(|o| (o.start_offset, o.end_offset)).collect()
}

/// `Lint/ParenthesesAsGroupedExpression`: `[[space_start, space_end, arg_start,
/// arg_end], ...]`. `[space_start, space_end)` is the whitespace run between
/// the selector and the `(` — this is both the offense highlight and the
/// autocorrect `remove` range (matches stock's `corrector.remove(range)`).
/// `[arg_start, arg_end)` is the first argument's full source range; the
/// wrapper reads `source[arg_start..arg_end]` for the `MSG` substitution.
fn map_parentheses_as_grouped_expression(
    v: Vec<
        shirobai_core::rules::parentheses_as_grouped_expression::ParenthesesAsGroupedExpressionOffense,
    >,
) -> Vec<(usize, usize, usize, usize)> {
    v.into_iter()
        .map(|o| (o.space_start, o.space_end, o.arg_start, o.arg_end))
        .collect()
}

/// `Style/PercentLiteralDelimiters`: `[[start, end, begin_start, begin_end,
/// end_start, end_end, type_index], ...]`. `[start, end)` is the literal's
/// full source range (offense highlight); `[begin_start, begin_end)` is the
/// opening token (e.g. `%w(`) the wrapper replaces with `<type><open>`;
/// `[end_start, end_end)` is the **single-byte** closer (`)` of `%r(.*)i`),
/// so options are preserved when the wrapper substitutes the close byte.
/// `type_index` is the `[%, %i, %I, %q, %Q, %r, %s, %w, %W, %x]` slot the
/// Ruby side uses for the `MSG` token and the preferred-pair lookup.
fn map_percent_literal_delimiters(
    v: Vec<shirobai_core::rules::percent_literal_delimiters::PercentLiteralDelimitersOffense>,
) -> Vec<PercentLiteralDelimitersTuple> {
    v.into_iter()
        .map(|o| {
            (
                o.start_offset,
                o.end_offset,
                o.begin_start,
                o.begin_end,
                o.end_start,
                o.end_end,
                o.type_index,
            )
        })
        .collect()
}

/// `Layout/MultilineMethodCallBraceLayout`:
/// `[[offense_start, offense_end, message_code, send_node_start, send_node_end,
/// correctable], ...]` — `[offense_start, offense_end)` is the closing `)`
/// token; `message_code` is the 0-3 message selector; `(send_node_start,
/// send_node_end)` pins the inner brace-bearing send (a chained `foo(...).bar`
/// shares `begin_pos` with the outer call, so the end_pos disambiguates). The
/// wrapper relocates the parser-gem node via `processed_source.ast.each_node`
/// and hands it to stock's `MultilineLiteralBraceCorrector`. `correctable` is
/// false when the `new_line_needed_before_closing_brace?` guard fires
/// (comment + chained / arg).
fn map_multiline_method_call_brace_layout(
    v: Vec<shirobai_core::rules::multiline_method_call_brace_layout::MmcblOffense>,
) -> Vec<(usize, usize, u8, usize, usize, bool)> {
    v.into_iter()
        .map(|o| {
            (
                o.offense_start,
                o.offense_end,
                o.message_code,
                o.send_node_start,
                o.send_node_end,
                o.correctable,
            )
        })
        .collect()
}

/// `Layout/AssignmentIndentation`:
/// `[[rhs_start, rhs_end, column_delta], ...]` — `[rhs_start, rhs_end)` is the
/// RHS's source range (the offense location). `column_delta` is the signed
/// `expected_column - actual_column`; the wrapper relocates the matching
/// `Parser::AST::Node` by `rhs_start` and hands it to stock's
/// `AlignmentCorrector#correct` with that delta.
fn map_assignment_indentation(
    v: Vec<shirobai_core::rules::assignment_indentation::AssignmentIndentationOffense>,
) -> Vec<(usize, usize, i64)> {
    v.into_iter()
        .map(|o| (o.rhs_start, o.rhs_end, o.column_delta))
        .collect()
}

/// `Style/RedundantSelfAssignment`: `[[op_start, op_end, method_name,
/// kind, range_start, range_end, rhs_start, rhs_end], ...]`.
///
/// `kind` is `0` for variable-assignment style (the wrapper replaces
/// `[range_start, range_end)` with `source[rhs_start..rhs_end]`) and `1`
/// for setter style (the wrapper removes `[range_start, range_end)`; the
/// `rhs_*` fields are unused / zero). `method_name` is the destructive
/// method (`concat`, `delete_if`, ...) the offense message reports.
fn map_redundant_self_assignment(
    v: Vec<shirobai_core::rules::redundant_self_assignment::RedundantSelfAssignmentOffense>,
    source: &[u8],
) -> Vec<RedundantSelfAssignmentTuple> {
    v.into_iter()
        .map(|o| {
            let method_name = std::str::from_utf8(&source[o.method_name_start..o.method_name_end])
                .unwrap_or("")
                .to_string();
            (
                o.op_start,
                o.op_end,
                method_name,
                o.kind,
                o.range_start,
                o.range_end,
                o.rhs_start,
                o.rhs_end,
            )
        })
        .collect()
}

/// `Style/ColonMethodCall`: `[[dot_start, dot_end], ...]` — the `::` token
/// range. Both the offense highlight and the autocorrect replace range are the
/// same `[dot_start, dot_end)` (matches stock's
/// `add_offense(node.loc.dot) { |c| c.replace(node.loc.dot, '.') }`). Always
/// two bytes wide for a well-formed `::`; the wrapper takes both ends so the
/// `Parser::Source::Range` and the autocorrect target are byte-identical.
fn map_colon_method_call(
    v: Vec<shirobai_core::rules::colon_method_call::ColonMethodCallOffense>,
) -> Vec<(usize, usize)> {
    v.into_iter().map(|o| (o.dot_start, o.dot_end)).collect()
}

/// `Layout/BlockAlignment`: `[[end_start, end_end, message, align_column], ...]`
/// — `[end_start, end_end)` is the closing-token range (`end` / `}`);
/// `message`/`align_column` carry the offense detail (only misaligned blocks
/// are emitted).
fn map_block_alignment(
    v: Vec<shirobai_core::rules::block_alignment::BlockAlignmentOffense>,
) -> Vec<(usize, usize, String, usize)> {
    v.into_iter()
        .map(|o| (o.end_start, o.end_end, o.message, o.align_column))
        .collect()
}

/// `Layout/ElseAlignment`: `[[else_start, else_end, message, column_delta], ...]`
/// — `[else_start, else_end)` is the keyword range (`else` / `elsif` / ...);
/// `message` is the formatted offense; `column_delta` is the signed shift the
/// autocorrect applies to the keyword's line. Only misaligned keywords emitted.
fn map_else_alignment(
    v: Vec<shirobai_core::rules::else_alignment::ElseAlignmentOffense>,
) -> Vec<(usize, usize, String, i64)> {
    v.into_iter()
        .map(|o| (o.else_start, o.else_end, o.message, o.column_delta))
        .collect()
}

/// `Style/BlockDelimiters` correction ops: `[kind, start, end, text]` —
/// kind 0 = replace, 1 = remove, 2 = insert_before, 3 = insert_after,
/// 4 = wrap in `begin\n` / `\nend` (text empty).
type BlockDelimitersOps = Vec<(u8, usize, usize, String)>;

fn map_block_delimiters_ops(
    ops: Vec<shirobai_core::rules::block_delimiters::CorrectionOp>,
) -> BlockDelimitersOps {
    ops.into_iter()
        .map(|o| (o.kind, o.start, o.end, o.text))
        .collect()
}

/// The resolved `Style/BlockDelimiters` result:
/// `[[[token_start, token_end, block_start, block_end, message, ops], ...],
/// [[ignore_start, ignore_end], ...], has_conditional]`.
#[allow(clippy::type_complexity)]
fn map_block_delimiters(
    r: shirobai_core::rules::block_delimiters::BlockDelimitersResult,
) -> (
    Vec<(usize, usize, usize, usize, String, BlockDelimitersOps)>,
    Vec<(usize, usize)>,
    bool,
) {
    let offenses = r
        .offenses
        .into_iter()
        .map(|c| {
            (
                c.token.0,
                c.token.1,
                c.block.0,
                c.block.1,
                c.message,
                map_block_delimiters_ops(c.ops),
            )
        })
        .collect();
    (offenses, r.send_ignores, r.has_conditional)
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
/// fixed-order 30-slot Array; each slot carries that cop's existing tuple-array
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
/// 20 hash_each_methods / 21 void / 22 useless_access_modifier /
/// 23 empty_lines_around_method_body / 24 empty_lines_around_class_body /
/// 25 empty_lines_around_module_body / 26 empty_lines_around_block_body /
/// 27 empty_lines_around_begin_body /
/// 28 empty_lines_around_exception_handling_keywords /
/// 29 block_delimiters / 30 abc_size / 31 indentation_consistency /
/// 32 empty_line_between_defs / 33 end_alignment / 34 block_alignment /
/// 35 else_alignment / 36 first_hash_element_indentation / 37 hash_alignment /
/// 38 empty_lines_around_arguments / 39 hash_syntax / 40 string_literals /
/// 41 trailing_comma_in_arguments / 42 string_literals_in_interpolation /
/// 43 trailing_empty_lines / 44 space_around_method_call_operator /
/// 45 space_around_keyword / 46 space_inside_block_braces /
/// 47 method_length / 48 def_end_alignment / 49 require_parentheses /
/// 50 self_assignment / 51 nested_parenthesized_calls /
/// 52 parentheses_as_grouped_expression / 53 percent_literal_delimiters /
/// 54 multiline_method_call_brace_layout / 55 access_modifier_indentation /
/// 56 assignment_indentation / 57 redundant_self_assignment /
/// 58 colon_method_call / 59 stabby_lambda_parentheses /
/// 60 unreachable_code / 61 hash_transform_keys /
/// 62 ambiguous_block_association /
/// 63 empty_line_after_guard_clause /
/// 64 empty_comment / 65 empty_line_after_magic_comment /
/// 66 empty_lines / 67 leading_empty_lines /
/// 68 class_length / 69 module_length
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
        let ary = ruby.ary_new_capa(70);
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
        ary.push(map_useless_access_modifier(r.useless_access_modifier))?;
        let elab = r.empty_lines_around_body;
        ary.push(map_empty_lines(elab.method_body))?;
        ary.push(map_empty_lines(elab.class_body))?;
        ary.push(map_empty_lines(elab.module_body))?;
        ary.push(map_empty_lines(elab.block_body))?;
        ary.push(map_empty_lines(elab.begin_body))?;
        ary.push(map_empty_lines(elab.exception_keywords))?;
        ary.push(map_block_delimiters(r.block_delimiters))?;
        ary.push(map_abc_size(r.abc_size))?;
        ary.push(map_indentation_consistency(r.indentation_consistency))?;
        ary.push(map_empty_line_between_defs(r.empty_line_between_defs))?;
        ary.push(map_end_alignment(r.end_alignment))?;
        ary.push(map_block_alignment(r.block_alignment))?;
        ary.push(map_else_alignment(r.else_alignment))?;
        ary.push(map_first_hash_element_indentation(
            r.first_hash_element_indentation,
        ))?;
        ary.push(map_hash_alignment(r.hash_alignment))?;
        ary.push(map_empty_lines_around_arguments(
            r.empty_lines_around_arguments,
        ))?;
        ary.push(map_hash_syntax(r.hash_syntax))?;
        ary.push(map_string_literals(r.string_literals))?;
        ary.push(map_trailing_comma_in_arguments(
            r.trailing_comma_in_arguments,
        ))?;
        ary.push(map_string_literals_in_interpolation(
            r.string_literals_in_interpolation,
        ))?;
        ary.push(map_trailing_empty_lines(r.trailing_empty_lines))?;
        ary.push(map_space_around_method_call_operator(
            r.space_around_method_call_operator,
        ))?;
        ary.push(map_space_around_keyword(r.space_around_keyword))?;
        ary.push(map_space_inside_block_braces(r.space_inside_block_braces))?;
        ary.push(map_method_length(r.method_length))?;
        ary.push(map_def_end_alignment(r.def_end_alignment))?;
        ary.push(map_require_parentheses(r.require_parentheses))?;
        ary.push(map_self_assignment(r.self_assignment))?;
        ary.push(map_nested_parenthesized_calls(r.nested_parenthesized_calls))?;
        ary.push(map_parentheses_as_grouped_expression(
            r.parentheses_as_grouped_expression,
        ))?;
        ary.push(map_percent_literal_delimiters(
            r.percent_literal_delimiters,
        ))?;
        ary.push(map_multiline_method_call_brace_layout(
            r.multiline_method_call_brace_layout,
        ))?;
        ary.push(map_access_modifier_indentation(
            r.access_modifier_indentation,
        ))?;
        ary.push(map_assignment_indentation(r.assignment_indentation))?;
        ary.push(map_redundant_self_assignment(
            r.redundant_self_assignment,
            bytes(&source),
        ))?;
        ary.push(map_colon_method_call(r.colon_method_call))?;
        ary.push(map_stabby_lambda_parentheses(
            r.stabby_lambda_parentheses,
        ))?;
        ary.push(map_unreachable_code(r.unreachable_code))?;
        ary.push(map_hash_transform_keys(r.hash_transform_keys))?;
        ary.push(map_ambiguous_block_association(
            r.ambiguous_block_association,
        ))?;
        ary.push(map_empty_line_after_guard_clause(
            r.empty_line_after_guard_clause,
        ))?;
        ary.push(map_empty_comment(r.empty_comment))?;
        ary.push(map_empty_line_after_magic_comment(
            r.empty_line_after_magic_comment,
        ))?;
        ary.push(map_empty_lines_offenses(r.empty_lines))?;
        ary.push(map_leading_empty_lines(r.leading_empty_lines))?;
        ary.push(map_class_length(r.class_length))?;
        ary.push(map_module_length(r.module_length))?;
        Ok(ary)
    })
}

/// `Layout/EmptyLineAfterGuardClause`: `[[offense_start, offense_end,
/// ac_anchor_first_line_start, ac_anchor_last_line], ...]`.
fn map_empty_line_after_guard_clause(
    v: Vec<shirobai_core::rules::empty_line_after_guard_clause::GuardClauseCandidate>,
) -> Vec<(usize, usize, usize, usize)> {
    v.into_iter()
        .map(|c| {
            (
                c.offense_start,
                c.offense_end,
                c.ac_anchor_first_line_start,
                c.ac_anchor_last_line,
            )
        })
        .collect()
}

/// `Layout/EmptyComment`: `[[offense_start, offense_end, ac_start, ac_end],
/// ...]`. `[offense_start, offense_end)` is the comment range stock reports
/// (matches `comment.source_range`; CRLF trailing `\r` already snapped off);
/// `[ac_start, ac_end)` is the range the wrapper passes to
/// `corrector.remove` (whole-line including the final newline, OR the
/// comment with its leading horizontal whitespace when the comment shares a
/// line with earlier code).
fn map_empty_comment(
    v: Vec<shirobai_core::rules::empty_comment::EmptyCommentOffense>,
) -> Vec<(usize, usize, usize, usize)> {
    v.into_iter()
        .map(|o| (o.offense_start, o.offense_end, o.ac_start, o.ac_end))
        .collect()
}

/// `Layout/EmptyLineAfterMagicComment`: `[[start, end, line_1based], ...]`
/// for every comment that appears before the file's first AST statement (or
/// for every comment when the AST is empty). `[start, end)` is the comment's
/// parser-gem `source_range` (CRLF trailing `\r` already snapped off). The
/// Ruby wrapper filters by `MagicComment.parse(text).any?`, picks the last
/// matching candidate, and builds the offense / corrector around it.
fn map_empty_line_after_magic_comment(
    v: Vec<shirobai_core::rules::empty_line_after_magic_comment::MagicCommentCandidate>,
) -> Vec<(usize, usize, usize)> {
    v.into_iter()
        .map(|c| (c.start, c.end, c.line))
        .collect()
}

/// `Layout/EmptyLines`: `[[start, end], ...]`. `[start, end)` is the 1-byte
/// `source_range(buffer, line, 0)` the wrapper passes to both `add_offense`
/// and `corrector.remove`. Named with the `_offenses` suffix because the
/// other `map_empty_lines` in this file maps the `empty_lines_around_body`
/// family's offense shape.
fn map_empty_lines_offenses(
    v: Vec<shirobai_core::rules::empty_lines::EmptyLinesOffense>,
) -> Vec<(usize, usize)> {
    v.into_iter().map(|o| (o.start, o.end)).collect()
}

/// `Layout/LeadingEmptyLines`: at most one offense per file. Tuple shape
/// `(start, end, ac_start, ac_end)`: `[start, end)` is the first lexical
/// token's source range (the offense range stock yields to `add_offense`);
/// `[ac_start, ac_end)` = `[0, token.begin_pos)` is the leading-blank range
/// the corrector removes. Wrapped in a 0-or-1-element `Vec` for a uniform
/// Ruby-side shape.
fn map_leading_empty_lines(
    v: Option<shirobai_core::rules::leading_empty_lines::LeadingEmptyLinesOffense>,
) -> Vec<(usize, usize, usize, usize)> {
    v.into_iter()
        .map(|o| (o.start, o.end, o.ac_start, o.ac_end))
        .collect()
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

/// Ruby entry point for `Metrics/MethodLength`. Returns one entry per method
/// whose body exceeds `max`: `[[start, end, head_end, length, name,
/// filterable], ...]`. Allow filtering stays on the Ruby side.
fn check_method_length(
    source: RString,
    max: usize,
    count_comments: bool,
    count_as_one: Vec<String>,
) -> Vec<(usize, usize, usize, usize, String, bool)> {
    map_method_length(shirobai_core::rules::method_length::check_method_length(
        bytes(&source),
        max,
        count_comments,
        &count_as_one,
    ))
}

/// Ruby entry point for `Metrics/ClassLength`. Returns one entry per class
/// definition whose measured length exceeds `max`: `[[start, end, head_end,
/// length, sclass], ...]`.
fn check_class_length(
    source: RString,
    max: usize,
    count_comments: bool,
    count_as_one: Vec<String>,
) -> Vec<(usize, usize, usize, usize, bool)> {
    map_class_length(shirobai_core::rules::class_length::check_class_length(
        bytes(&source),
        max,
        count_comments,
        &count_as_one,
    ))
}

/// Ruby entry point for `Metrics/ModuleLength`. Returns one entry per module
/// definition whose measured length exceeds `max`: `[[start, end, head_end,
/// length], ...]`.
fn check_module_length(
    source: RString,
    max: usize,
    count_comments: bool,
    count_as_one: Vec<String>,
) -> Vec<(usize, usize, usize, usize)> {
    map_module_length(shirobai_core::rules::module_length::check_module_length(
        bytes(&source),
        max,
        count_comments,
        &count_as_one,
    ))
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

/// Ruby entry point for `Metrics/AbcSize`. Reports one entry per method whose
/// squared ABC vector exceeds `max_floor**2` (`max_floor` is the configured
/// `Max.floor`; a negative value reports every method): `[[start, end,
/// head_end, name, assignments, branches, conditions], ...]`. The Ruby side
/// re-applies the exact `score > Max` filter and builds the message.
#[allow(clippy::type_complexity)]
fn check_abc_size(
    source: RString,
    max_floor: i64,
    discount_repeated: bool,
) -> Vec<(usize, usize, usize, String, u64, u64, u64)> {
    map_abc_size(shirobai_core::rules::abc_size::check_abc_size(
        bytes(&source),
        max_floor,
        discount_repeated,
    ))
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

/// Ruby entry point for `Layout/FirstHashElementIndentation`. Takes the
/// source, the enforced style (0=special_inside_parentheses, 1=consistent,
/// 2=align_braces), the configured indentation width, whether
/// `Layout/ArgumentAlignment` enforces `with_fixed_indentation` (which only
/// suppresses the paren claim, never the cop), and the two
/// `Layout/HashAlignment` separator flags (colon / hash-rocket). Returns one
/// entry per misindented first pair or hanging right brace: `[[start, end,
/// column_delta, message, correct_target], ...]`.
fn check_first_hash_element_indentation(
    source: RString,
    style: u8,
    indent_width: usize,
    enforce_fixed_indentation: bool,
    colon_separator: bool,
    rocket_separator: bool,
) -> Vec<(usize, usize, isize, String, i64)> {
    map_first_hash_element_indentation(
        shirobai_core::rules::first_hash_element_indentation::check_first_hash_element_indentation(
            bytes(&source),
            style,
            indent_width,
            enforce_fixed_indentation,
            shirobai_core::rules::first_hash_element_indentation::SeparatorConfig {
                colon_separator,
                rocket_separator,
            },
        ),
    )
}

/// Ruby entry point for `Layout/HashAlignment`. Takes the source, the
/// comma-joined `EnforcedHashRocketStyle` and `EnforcedColonStyle` (each
/// `key`/`separator`/`table`, in config order), the
/// `EnforcedLastArgumentHashStyle` code (0 always_inspect, 1 always_ignore,
/// 2 ignore_explicit, 3 ignore_implicit), and whether `Layout/ArgumentAlignment`
/// enforces `with_fixed_indentation`. Returns one entry per misaligned pair or
/// kwsplat (see `map_hash_alignment`).
#[allow(clippy::type_complexity)]
fn check_hash_alignment(
    source: RString,
    hash_rocket_styles: String,
    colon_styles: String,
    last_argument_style: u8,
    enforce_fixed_indentation: bool,
) -> Vec<(
    usize,
    usize,
    usize,
    u8,
    bool,
    (isize, isize, isize),
    (usize, usize, usize),
    (usize, usize),
    (usize, usize),
)> {
    let parse = |s: &str| -> Vec<u8> {
        s.split(',')
            .filter(|p| !p.is_empty())
            .map(|p| match p {
                "separator" => 1,
                "table" => 2,
                _ => 0,
            })
            .collect()
    };
    let cfg = shirobai_core::rules::hash_alignment::Config {
        hash_rocket_styles: parse(&hash_rocket_styles),
        colon_styles: parse(&colon_styles),
        last_argument_style,
        enforce_fixed_indentation,
    };
    map_hash_alignment(shirobai_core::rules::hash_alignment::check_hash_alignment(
        bytes(&source),
        &cfg,
    ))
}

/// Unpacks the `Style/HashSyntax` wire config from packed nums.
/// `[style, shorthand, use_hash_rockets_with_symbol_values,
///   prefer_hash_rockets_for_non_alnum_ending_symbols, ruby31_plus, ruby22_plus]`.
fn hash_syntax_config(nums: &[i64]) -> shirobai_core::rules::hash_syntax::Config {
    shirobai_core::rules::hash_syntax::Config {
        style: nums[0] as u8,
        shorthand: nums[1] as u8,
        use_hash_rockets_with_symbol_values: nums[2] != 0,
        prefer_hash_rockets_for_non_alnum_ending_symbols: nums[3] != 0,
        ruby31_plus: nums[4] != 0,
        ruby22_plus: nums[5] != 0,
    }
}

/// Ruby entry point for `Style/HashSyntax`. Takes the source and the packed
/// config nums (see `hash_syntax_config`). Returns one entry per offending pair
/// (see `map_hash_syntax`).
fn check_hash_syntax(
    source: RString,
    nums: Vec<i64>,
) -> Vec<(bool, usize, usize, u8, u8, HashSyntaxOps)> {
    let cfg = hash_syntax_config(&nums);
    map_hash_syntax(shirobai_core::rules::hash_syntax::check_hash_syntax(
        bytes(&source),
        &cfg,
    ))
}

/// Ruby entry point for `Style/StringLiterals`. Takes the source and the packed
/// config nums (`[style, consistent_multiline]`). Returns one entry per record
/// (see `map_string_literals`).
fn check_string_literals(
    source: RString,
    nums: Vec<i64>,
) -> Vec<(bool, usize, usize, u8, u8, u8, String)> {
    let cfg = shirobai_core::rules::string_literals::Config {
        style: nums[0] as u8,
        consistent_multiline: nums[1] != 0,
    };
    map_string_literals(shirobai_core::rules::string_literals::check_string_literals(
        bytes(&source),
        &cfg,
    ))
}

/// Ruby entry point for `Style/StringLiteralsInInterpolation`. Takes the source
/// and the packed config nums (`[style]`). Returns one entry per record (see
/// `map_string_literals_in_interpolation`).
fn check_string_literals_in_interpolation(
    source: RString,
    nums: Vec<i64>,
) -> Vec<(bool, usize, usize, u8, u8, String)> {
    let cfg = shirobai_core::rules::string_literals_in_interpolation::Config {
        style: nums[0] as u8,
    };
    map_string_literals_in_interpolation(
        shirobai_core::rules::string_literals_in_interpolation::check_string_literals_in_interpolation(
            bytes(&source),
            &cfg,
        ),
    )
}

/// Ruby entry point for `Style/TrailingCommaInArguments`. Takes the source and
/// the packed config nums (`[style]`). Returns one entry per record (see
/// `map_trailing_comma_in_arguments`).
fn check_trailing_comma_in_arguments(
    source: RString,
    nums: Vec<i64>,
) -> Vec<(usize, usize, u8, u8)> {
    let cfg = shirobai_core::rules::trailing_comma_in_arguments::Config {
        style: nums[0] as u8,
    };
    map_trailing_comma_in_arguments(
        shirobai_core::rules::trailing_comma_in_arguments::check_trailing_comma_in_arguments(
            bytes(&source),
            &cfg,
        ),
    )
}

/// Ruby entry point for `Layout/TrailingEmptyLines`. Takes the source and the
/// packed config nums (`[style]`: 0 final_newline, 1 final_blank_line). Returns
/// a 0-or-1-element Vec (see `map_trailing_empty_lines`).
fn check_trailing_empty_lines(
    source: RString,
    nums: Vec<i64>,
) -> Vec<(usize, usize, usize, usize, String, i64)> {
    let cfg = shirobai_core::rules::trailing_empty_lines::Config {
        style: nums[0] as u8,
    };
    map_trailing_empty_lines(
        shirobai_core::rules::trailing_empty_lines::check_trailing_empty_lines(bytes(&source), &cfg),
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

/// Ruby entry point for `Style/HashTransformKeys`. Config-less.
/// Returns one entry per offense: `[[start, end, message, edits], ...]`
/// where `edits` is `[[edit_start, edit_end, replacement], ...]`.
#[allow(clippy::type_complexity)]
fn check_hash_transform_keys(
    source: RString,
) -> Vec<(usize, usize, String, Vec<(usize, usize, String)>)> {
    map_hash_transform_keys(
        shirobai_core::rules::hash_transform_keys::check_hash_transform_keys(bytes(&source)),
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

/// Ruby entry point for `Lint/UselessAccessModifier`. Takes the source, the
/// `ContextCreatingMethods` and `MethodCreatingMethods` lists and the
/// `AllCops/ActiveSupportExtensionsEnabled` flag. Returns one entry per
/// useless modifier: `[[start, end, name], ...]` — `[start, end)` is the
/// offense range (the modifier send) and `name` the modifier interpolated
/// into the message; Ruby derives the whole-line removal from the range with
/// the stock `range_by_whole_lines` helper.
fn check_useless_access_modifier(
    source: RString,
    context_creating: Vec<String>,
    method_creating: Vec<String>,
    active_support_extensions: bool,
) -> Vec<(usize, usize, String)> {
    map_useless_access_modifier(
        shirobai_core::rules::useless_access_modifier::check_useless_access_modifier(
            bytes(&source),
            &context_creating,
            &method_creating,
            active_support_extensions,
        ),
    )
}

/// Ruby entry point for the `EmptyLinesAroundBody` family (standalone
/// fallback; the wrappers normally read the bundled run). Takes the source
/// and the `EnforcedStyle` ordinals of the three configurable members
/// (class / module / block). Returns the six cops' offense arrays in slot
/// order (method, class, module, block, begin, exception keywords), each
/// `[[start, end, insert, message], ...]` (see `map_empty_lines`).
#[allow(clippy::type_complexity)]
fn check_empty_lines_around_body(
    source: RString,
    class_style: u8,
    module_style: u8,
    block_style: u8,
) -> (
    Vec<(usize, usize, bool, String)>,
    Vec<(usize, usize, bool, String)>,
    Vec<(usize, usize, bool, String)>,
    Vec<(usize, usize, bool, String)>,
    Vec<(usize, usize, bool, String)>,
    Vec<(usize, usize, bool, String)>,
) {
    let r = shirobai_core::rules::empty_lines_around_body::check_empty_lines_around_body(
        bytes(&source),
        shirobai_core::rules::empty_lines_around_body::Config {
            class_style,
            module_style,
            block_style,
        },
    );
    (
        map_empty_lines(r.method_body),
        map_empty_lines(r.class_body),
        map_empty_lines(r.module_body),
        map_empty_lines(r.block_body),
        map_empty_lines(r.begin_body),
        map_empty_lines(r.exception_keywords),
    )
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

/// Ruby entry point for `Layout/IndentationConsistency` (the fallback path for
/// autocorrect re-passes, when the bundle is not eligible). `internal` is true
/// for the `indented_internal_methods` EnforcedStyle. Returns
/// `[[start, end, column_delta, autocorrect], ...]`.
fn check_indentation_consistency(
    source: RString,
    internal: bool,
) -> Vec<(usize, usize, isize, bool)> {
    let cfg = shirobai_core::rules::indentation_consistency::Config {
        indented_internal_methods: internal,
    };
    map_indentation_consistency(
        shirobai_core::rules::indentation_consistency::check_indentation_consistency(
            bytes(&source),
            cfg,
        ),
    )
}

/// Ruby entry point for `Layout/EmptyLineBetweenDefs`. `nums` is
/// `[method_defs, class_defs, module_defs, allow_adjacent_one_line_defs,
/// minimum_empty_lines, maximum_empty_lines]`, `lists` is `[def_like_macros]`.
/// Returns the shape documented on `map_empty_line_between_defs`.
fn check_empty_line_between_defs(
    source: RString,
    nums: Vec<i64>,
    lists: Vec<Vec<String>>,
) -> Vec<(usize, usize, String, bool, usize, usize)> {
    let cfg = shirobai_core::rules::empty_line_between_defs::Config {
        method_defs: nums.first().copied().unwrap_or(0) != 0,
        class_defs: nums.get(1).copied().unwrap_or(0) != 0,
        module_defs: nums.get(2).copied().unwrap_or(0) != 0,
        allow_adjacent_one_line_defs: nums.get(3).copied().unwrap_or(0) != 0,
        minimum_empty_lines: nums.get(4).copied().unwrap_or(1) as usize,
        maximum_empty_lines: nums.get(5).copied().unwrap_or(1) as usize,
        def_like_macros: lists.into_iter().next().unwrap_or_default(),
    };
    map_empty_line_between_defs(
        shirobai_core::rules::empty_line_between_defs::check_empty_line_between_defs(
            bytes(&source),
            cfg,
        ),
    )
}

/// Ruby entry point for `Layout/EmptyLineAfterGuardClause`. Returns one entry
/// per candidate offense: `[[offense_start, offense_end,
/// ac_anchor_first_line_start, ac_anchor_last_line], ...]`. The Ruby wrapper
/// finishes the directive-comment check (`# rubocop:enable` / `# :nocov:` /
/// `# simplecov:disable`|`enable`) using `processed_source.comment_at_line`
/// and `DirectiveComment#enabled?`.
fn check_empty_line_after_guard_clause(
    source: RString,
) -> Vec<(usize, usize, usize, usize)> {
    shirobai_core::rules::empty_line_after_guard_clause::check_empty_line_after_guard_clause(
        bytes(&source),
    )
    .into_iter()
    .map(|c| {
        (
            c.offense_start,
            c.offense_end,
            c.ac_anchor_first_line_start,
            c.ac_anchor_last_line,
        )
    })
    .collect()
}

/// Ruby entry point for `Layout/EmptyComment` (standalone fallback). Takes
/// the source and the two config flags (`AllowBorderComment` /
/// `AllowMarginComment`). Returns the shape documented on `map_empty_comment`.
fn check_empty_comment(
    source: RString,
    allow_border_comment: bool,
    allow_margin_comment: bool,
) -> Vec<(usize, usize, usize, usize)> {
    let cfg = shirobai_core::rules::empty_comment::Config {
        allow_border_comment,
        allow_margin_comment,
    };
    map_empty_comment(shirobai_core::rules::empty_comment::check_empty_comment(
        bytes(&source),
        cfg,
    ))
}

/// Ruby entry point for `Layout/EmptyLineAfterMagicComment` (standalone
/// fallback, config-less). Returns the shape documented on
/// `map_empty_line_after_magic_comment`.
fn check_empty_line_after_magic_comment(
    source: RString,
) -> Vec<(usize, usize, usize)> {
    map_empty_line_after_magic_comment(
        shirobai_core::rules::empty_line_after_magic_comment::check_empty_line_after_magic_comment(
            bytes(&source),
        ),
    )
}

/// Ruby entry point for `Layout/EmptyLines` (standalone fallback,
/// config-less). Returns the shape documented on `map_empty_lines`.
fn check_empty_lines(source: RString) -> Vec<(usize, usize)> {
    map_empty_lines_offenses(shirobai_core::rules::empty_lines::check_empty_lines(
        bytes(&source),
    ))
}

/// Ruby entry point for `Layout/LeadingEmptyLines` (no config). Returns a
/// 0-or-1-element Vec (see `map_leading_empty_lines`).
fn check_leading_empty_lines(source: RString) -> Vec<(usize, usize, usize, usize)> {
    map_leading_empty_lines(
        shirobai_core::rules::leading_empty_lines::check_leading_empty_lines(bytes(&source)),
    )
}

/// Ruby entry point for `Layout/EmptyLinesAroundArguments` (no config). Returns
/// the shape documented on `map_empty_lines_around_arguments`.
fn check_empty_lines_around_arguments(source: RString) -> Vec<(usize, usize)> {
    map_empty_lines_around_arguments(
        shirobai_core::rules::empty_lines_around_arguments::check_empty_lines_around_arguments(
            bytes(&source),
        ),
    )
}

/// Ruby entry point for `Layout/SpaceAroundMethodCallOperator` (no config).
/// Returns the shape documented on `map_space_around_method_call_operator`.
fn check_space_around_method_call_operator(source: RString) -> Vec<(usize, usize)> {
    map_space_around_method_call_operator(
        shirobai_core::rules::space_around_method_call_operator::check_space_around_method_call_operator(
            bytes(&source),
        ),
    )
}

/// Ruby entry point for `Layout/SpaceAroundKeyword` (no config). Returns the
/// shape documented on `map_space_around_keyword`.
fn check_space_around_keyword(source: RString) -> Vec<(usize, usize, bool)> {
    map_space_around_keyword(
        shirobai_core::rules::space_around_keyword::check_space_around_keyword(bytes(&source)),
    )
}

/// Ruby entry point for `Layout/SpaceInsideBlockBraces` (fallback path; the
/// bundle is the usual path). `style` / `empty_style` are 0 = space,
/// 1 = no_space; `sbbp` is `SpaceBeforeBlockParameters`. Returns the shape
/// documented on `map_space_inside_block_braces`.
fn check_space_inside_block_braces(
    source: RString,
    style: u8,
    empty_style: u8,
    sbbp: bool,
) -> Vec<(usize, usize, u8)> {
    use shirobai_core::rules::space_inside_block_braces as sibb;
    let to_style = |v: u8| {
        if v != 0 {
            sibb::Style::NoSpace
        } else {
            sibb::Style::Space
        }
    };
    let cfg = sibb::Config {
        style: to_style(style),
        empty_braces_style: to_style(empty_style),
        space_before_block_parameters: sbbp,
    };
    map_space_inside_block_braces(sibb::check_space_inside_block_braces(bytes(&source), cfg))
}

/// Ruby entry point for `Layout/EndAlignment`. `style` is the
/// `EnforcedStyleAlignWith` selector (0 = keyword, 1 = variable,
/// 2 = start_of_line). Returns the shape documented on `map_end_alignment`.
fn check_end_alignment(source: RString, style: u8) -> Vec<(usize, usize, Vec<u8>, String, usize)> {
    let cfg = shirobai_core::rules::end_alignment::Config { style };
    map_end_alignment(shirobai_core::rules::end_alignment::check_end_alignment(
        bytes(&source),
        cfg,
    ))
}

/// Ruby entry point for `Layout/DefEndAlignment`. `style` is the
/// `EnforcedStyleAlignWith` selector (0 = start_of_line, 1 = def). Returns the
/// shape documented on `map_def_end_alignment`.
fn check_def_end_alignment(
    source: RString,
    style: u8,
) -> Vec<(usize, usize, Vec<u8>, String, usize)> {
    let cfg = shirobai_core::rules::def_end_alignment::Config { style };
    map_def_end_alignment(shirobai_core::rules::def_end_alignment::check_def_end_alignment(
        bytes(&source),
        cfg,
    ))
}

/// Ruby entry point for `Layout/AccessModifierIndentation`. `style` is the
/// `EnforcedStyle` selector (0 = indent, 1 = outdent); `indentation_width` is
/// the cop's own `IndentationWidth` override (or `Layout/IndentationWidth`'s
/// `Width` when not overridden). Returns the shape documented on
/// `map_access_modifier_indentation`.
fn check_access_modifier_indentation(
    source: RString,
    style: u8,
    indentation_width: usize,
) -> Vec<(usize, usize, String, i64)> {
    let cfg = shirobai_core::rules::access_modifier_indentation::Config {
        style,
        indentation_width,
    };
    map_access_modifier_indentation(
        shirobai_core::rules::access_modifier_indentation::check_access_modifier_indentation(
            bytes(&source),
            cfg,
        ),
    )
}

/// Ruby entry point for `Style/StabbyLambdaParentheses`. `style` is the
/// `EnforcedStyle` selector (0 = require_parentheses, 1 = require_no_parentheses).
/// Returns the shape documented on `map_stabby_lambda_parentheses`.
fn check_stabby_lambda_parentheses(
    source: RString,
    style: u8,
) -> Vec<(usize, usize, usize, usize, usize, usize, String)> {
    let cfg = shirobai_core::rules::stabby_lambda_parentheses::Config { style };
    map_stabby_lambda_parentheses(
        shirobai_core::rules::stabby_lambda_parentheses::check_stabby_lambda_parentheses(
            bytes(&source),
            cfg,
        ),
    )
}

/// Ruby entry point for `Lint/AmbiguousBlockAssociation`. Takes the source,
/// `AllowedMethods` (matched verbatim against the INNER block sender's name),
/// and `AllowedInnerSources` (a pre-applied list of inner-sender source bytes
/// the wrapper already matched against `AllowedPatterns` regexps — fed in
/// directly so the Rust crate never has to embed a regexp engine). Returns
/// the shape documented on `map_ambiguous_block_association`.
fn check_ambiguous_block_association(
    source: RString,
    allowed_methods: Vec<String>,
    allowed_inner_sources: Vec<String>,
) -> Vec<AmbiguousBlockAssociationTuple> {
    map_ambiguous_block_association(
        shirobai_core::rules::ambiguous_block_association::check_ambiguous_block_association(
            bytes(&source),
            &allowed_methods,
            &allowed_inner_sources,
        ),
    )
}

/// Ruby entry point for `Lint/RequireParentheses`. Config-less. Returns
/// `[[start_offset, end_offset], ...]`.
fn check_require_parentheses(source: RString) -> Vec<(usize, usize)> {
    map_require_parentheses(
        shirobai_core::rules::require_parentheses::check_require_parentheses(bytes(&source)),
    )
}

/// Ruby entry point for `Lint/SelfAssignment`. Config-less from the Rust side
/// (the `AllowRBSInlineAnnotation` filter is applied in the Ruby wrapper using
/// `processed_source.ast_with_comments`, exactly like stock). Returns
/// `[[start_offset, end_offset, rbs_anchor_offset], ...]`.
fn check_self_assignment(source: RString) -> Vec<(usize, usize, usize)> {
    map_self_assignment(shirobai_core::rules::self_assignment::check_self_assignment(
        bytes(&source),
    ))
}

/// Ruby entry point for `Style/RedundantSelfAssignment`. Config-less. Returns
/// the shape documented on `map_redundant_self_assignment`.
fn check_redundant_self_assignment(
    source: RString,
) -> Vec<RedundantSelfAssignmentTuple> {
    let src_bytes = bytes(&source);
    map_redundant_self_assignment(
        shirobai_core::rules::redundant_self_assignment::check_redundant_self_assignment(
            src_bytes,
        ),
        src_bytes,
    )
}

/// Ruby entry point for `Style/NestedParenthesizedCalls`. Takes the source and
/// the `AllowedMethods` list (matched verbatim against the inner call's name).
/// Returns `[[start, end, ac_open_start, ac_open_end, ac_close_pos], ...]`.
fn check_nested_parenthesized_calls(
    source: RString,
    allowed_methods: Vec<String>,
) -> Vec<(usize, usize, usize, usize, usize)> {
    map_nested_parenthesized_calls(
        shirobai_core::rules::nested_parenthesized_calls::check_nested_parenthesized_calls(
            bytes(&source),
            &allowed_methods,
        ),
    )
}

/// Ruby entry point for `Lint/ParenthesesAsGroupedExpression`. Config-less.
/// Returns `[[space_start, space_end, arg_start, arg_end], ...]`.
fn check_parentheses_as_grouped_expression(
    source: RString,
) -> Vec<(usize, usize, usize, usize)> {
    map_parentheses_as_grouped_expression(
        shirobai_core::rules::parentheses_as_grouped_expression::check_parentheses_as_grouped_expression(
            bytes(&source),
        ),
    )
}

/// Ruby entry point for `Style/ColonMethodCall`. Config-less. Returns
/// `[[dot_start, dot_end], ...]` — the `::` token range used for both the
/// offense highlight and the autocorrect replacement.
fn check_colon_method_call(source: RString) -> Vec<(usize, usize)> {
    map_colon_method_call(
        shirobai_core::rules::colon_method_call::check_colon_method_call(bytes(&source)),
    )
}

/// Ruby entry point for `Lint/UnreachableCode`. Config-less. Returns
/// `[[start, end], ...]` — the byte range of each unreachable expression.
fn check_unreachable_code(source: RString) -> Vec<(usize, usize)> {
    map_unreachable_code(shirobai_core::rules::unreachable_code::check_unreachable_code(
        bytes(&source),
    ))
}

/// Unpacks the `Style/PercentLiteralDelimiters` config: `pairs` is a 10-entry
/// list of 2-byte strings in `[%, %i, %I, %q, %Q, %r, %s, %w, %W, %x]` order.
/// The Ruby side resolves `PreferredDelimiters` (default + per-type overrides)
/// down to this fixed array.
fn percent_literal_delimiters_config(
    ruby: &Ruby,
    pairs: Vec<String>,
) -> Result<shirobai_core::rules::percent_literal_delimiters::Config, Error> {
    if pairs.len() != 10 {
        return Err(Error::new(
            ruby.exception_arg_error(),
            format!(
                "Style/PercentLiteralDelimiters expects 10 pair strings, got {}",
                pairs.len()
            ),
        ));
    }
    use shirobai_core::rules::percent_literal_delimiters::{Config, DelimPair};
    let mut out: [DelimPair; 10] = [DelimPair { open: b'(', close: b')' }; 10];
    for (i, p) in pairs.iter().enumerate() {
        let bytes = p.as_bytes();
        if bytes.len() != 2 {
            return Err(Error::new(
                ruby.exception_arg_error(),
                format!(
                    "Style/PercentLiteralDelimiters pair #{i} must be 2 bytes, got {:?}",
                    p
                ),
            ));
        }
        out[i] = DelimPair {
            open: bytes[0],
            close: bytes[1],
        };
    }
    Ok(Config { pairs: out })
}

/// Ruby entry point for `Style/PercentLiteralDelimiters`. Takes the source and
/// the resolved per-type preferred-delimiter list (10 entries).
fn check_percent_literal_delimiters(
    ruby: &Ruby,
    source: RString,
    pairs: Vec<String>,
) -> Result<Vec<PercentLiteralDelimitersTuple>, Error> {
    let cfg = percent_literal_delimiters_config(ruby, pairs)?;
    Ok(map_percent_literal_delimiters(
        shirobai_core::rules::percent_literal_delimiters::check_percent_literal_delimiters(
            bytes(&source),
            &cfg,
        ),
    ))
}

/// Ruby entry point for `Layout/MultilineMethodCallBraceLayout`. `style` is
/// the `EnforcedStyle` selector (0 = symmetrical, 1 = new_line, 2 = same_line).
/// Returns the shape documented on `map_multiline_method_call_brace_layout`.
fn check_multiline_method_call_brace_layout(
    source: RString,
    style: u8,
) -> Vec<(usize, usize, u8, usize, usize, bool)> {
    map_multiline_method_call_brace_layout(
        shirobai_core::rules::multiline_method_call_brace_layout::check_multiline_method_call_brace_layout(
            bytes(&source),
            style,
        ),
    )
}

/// Ruby entry point for `Layout/AssignmentIndentation`. `indentation_width` is
/// `Layout/AssignmentIndentation.IndentationWidth` falling back to
/// `Layout/IndentationWidth.Width` falling back to 2. Returns the shape
/// documented on `map_assignment_indentation`.
fn check_assignment_indentation(source: RString, indentation_width: usize) -> Vec<(usize, usize, i64)> {
    let cfg = shirobai_core::rules::assignment_indentation::Config { indentation_width };
    map_assignment_indentation(
        shirobai_core::rules::assignment_indentation::check_assignment_indentation(
            bytes(&source),
            cfg,
        ),
    )
}

/// Ruby entry point for `Layout/BlockAlignment`. `style` is the
/// `EnforcedStyleAlignWith` selector (0 = either, 1 = start_of_block,
/// 2 = start_of_line). Returns the shape documented on `map_block_alignment`.
fn check_block_alignment(source: RString, style: u8) -> Vec<(usize, usize, String, usize)> {
    let cfg = shirobai_core::rules::block_alignment::Config { style };
    map_block_alignment(shirobai_core::rules::block_alignment::check_block_alignment(
        bytes(&source),
        cfg,
    ))
}

/// Ruby entry point for `Layout/ElseAlignment`. `style` is `Layout/EndAlignment`'s
/// `EnforcedStyleAlignWith` selector (0 = keyword, 1 = variable,
/// 2 = start_of_line). Returns the shape documented on `map_else_alignment`.
fn check_else_alignment(source: RString, style: u8) -> Vec<(usize, usize, String, i64)> {
    let cfg = shirobai_core::rules::else_alignment::Config { style };
    map_else_alignment(shirobai_core::rules::else_alignment::check_else_alignment(
        bytes(&source),
        cfg,
    ))
}

/// Unpacks the `Style/BlockDelimiters` wire config: `nums` is
/// `[style, allow_braces_on_procedural_oneliners]`, `lists` is
/// `[procedural, functional, allowed, braces_required]` (the same shapes the
/// bundle carries).
fn block_delimiters_config(
    nums: &[i64],
    lists: Vec<Vec<String>>,
) -> shirobai_core::rules::block_delimiters::Config {
    let mut lists = lists.into_iter();
    let mut next_list = || lists.next().unwrap_or_default();
    shirobai_core::rules::block_delimiters::Config {
        style: nums.first().copied().unwrap_or(0) as u8,
        allow_braces_on_procedural_oneliners: nums.get(1).copied().unwrap_or(0) != 0,
        procedural_methods: next_list(),
        functional_methods: next_list(),
        allowed_methods: next_list(),
        braces_required_methods: next_list(),
    }
}

/// Ruby entry point for `Style/BlockDelimiters` (resolved form). `prior`
/// carries the block ranges ignored in earlier autocorrect iterations
/// (stock's `@ignored_nodes` persists across passes on one cop instance).
/// Returns `[offenses, send_ignores, has_conditional]` (see
/// `map_block_delimiters`).
#[allow(clippy::type_complexity)]
fn check_block_delimiters(
    source: RString,
    nums: Vec<i64>,
    lists: Vec<Vec<String>>,
    prior: Vec<(usize, usize)>,
) -> (
    Vec<(usize, usize, usize, usize, String, BlockDelimitersOps)>,
    Vec<(usize, usize)>,
    bool,
) {
    let cfg = block_delimiters_config(&nums, lists);
    map_block_delimiters(
        shirobai_core::rules::block_delimiters::check_block_delimiters(
            bytes(&source),
            &cfg,
            &prior,
        ),
    )
}

/// Ruby entry point for the `Style/BlockDelimiters` raw event stream, for the
/// wrapper's exact replay (configured `AllowedPatterns`, or offenses whose
/// suppression depends on disable directives). Returns
/// `[[is_ignore, block_start, block_end, token_start, token_end, method_name,
/// message, ops], ...]` in walk order; ignore events carry zeros/empties in
/// the candidate fields.
#[allow(clippy::type_complexity)]
fn check_block_delimiters_events(
    source: RString,
    nums: Vec<i64>,
    lists: Vec<Vec<String>>,
) -> Vec<(
    bool,
    usize,
    usize,
    usize,
    usize,
    String,
    String,
    BlockDelimitersOps,
)> {
    let cfg = block_delimiters_config(&nums, lists);
    shirobai_core::rules::block_delimiters::check_block_delimiters_events(bytes(&source), &cfg)
        .into_iter()
        .map(|event| match event {
            shirobai_core::rules::block_delimiters::Event::Ignore((s, e)) => {
                (true, s, e, 0, 0, String::new(), String::new(), Vec::new())
            }
            shirobai_core::rules::block_delimiters::Event::Candidate(c) => (
                false,
                c.block.0,
                c.block.1,
                c.token.0,
                c.token.1,
                c.method_name,
                c.message,
                map_block_delimiters_ops(c.ops),
            ),
        })
        .collect()
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
    module.define_module_function("check_method_length", function!(check_method_length, 4))?;
    module.define_module_function("check_class_length", function!(check_class_length, 4))?;
    module.define_module_function("check_module_length", function!(check_module_length, 4))?;
    module.define_module_function("check_complexity", function!(check_complexity, 3))?;
    module.define_module_function("check_abc_size", function!(check_abc_size, 3))?;
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
        "check_indentation_consistency",
        function!(check_indentation_consistency, 2),
    )?;
    module.define_module_function(
        "check_empty_line_between_defs",
        function!(check_empty_line_between_defs, 3),
    )?;
    module.define_module_function(
        "check_empty_lines_around_arguments",
        function!(check_empty_lines_around_arguments, 1),
    )?;
    module.define_module_function(
        "check_empty_line_after_guard_clause",
        function!(check_empty_line_after_guard_clause, 1),
    )?;
    module.define_module_function(
        "check_empty_comment",
        function!(check_empty_comment, 3),
    )?;
    module.define_module_function(
        "check_empty_line_after_magic_comment",
        function!(check_empty_line_after_magic_comment, 1),
    )?;
    module.define_module_function(
        "check_empty_lines",
        function!(check_empty_lines, 1),
    )?;
    module.define_module_function(
        "check_leading_empty_lines",
        function!(check_leading_empty_lines, 1),
    )?;
    module.define_module_function(
        "check_space_around_method_call_operator",
        function!(check_space_around_method_call_operator, 1),
    )?;
    module.define_module_function(
        "check_space_around_keyword",
        function!(check_space_around_keyword, 1),
    )?;
    module.define_module_function(
        "check_space_inside_block_braces",
        function!(check_space_inside_block_braces, 4),
    )?;
    module.define_module_function("check_end_alignment", function!(check_end_alignment, 2))?;
    module.define_module_function(
        "check_def_end_alignment",
        function!(check_def_end_alignment, 2),
    )?;
    module.define_module_function(
        "check_require_parentheses",
        function!(check_require_parentheses, 1),
    )?;
    module.define_module_function(
        "check_self_assignment",
        function!(check_self_assignment, 1),
    )?;
    module.define_module_function(
        "check_redundant_self_assignment",
        function!(check_redundant_self_assignment, 1),
    )?;
    module.define_module_function(
        "check_nested_parenthesized_calls",
        function!(check_nested_parenthesized_calls, 2),
    )?;
    module.define_module_function(
        "check_parentheses_as_grouped_expression",
        function!(check_parentheses_as_grouped_expression, 1),
    )?;
    module.define_module_function(
        "check_unreachable_code",
        function!(check_unreachable_code, 1),
    )?;
    module.define_module_function(
        "check_percent_literal_delimiters",
        function!(check_percent_literal_delimiters, 2),
    )?;
    module.define_module_function(
        "check_multiline_method_call_brace_layout",
        function!(check_multiline_method_call_brace_layout, 2),
    )?;
    module.define_module_function(
        "check_access_modifier_indentation",
        function!(check_access_modifier_indentation, 3),
    )?;
    module.define_module_function(
        "check_assignment_indentation",
        function!(check_assignment_indentation, 2),
    )?;
    module.define_module_function(
        "check_colon_method_call",
        function!(check_colon_method_call, 1),
    )?;
    module.define_module_function(
        "check_stabby_lambda_parentheses",
        function!(check_stabby_lambda_parentheses, 2),
    )?;
    module.define_module_function(
        "check_ambiguous_block_association",
        function!(check_ambiguous_block_association, 3),
    )?;
    module.define_module_function(
        "check_block_alignment",
        function!(check_block_alignment, 2),
    )?;
    module.define_module_function(
        "check_else_alignment",
        function!(check_else_alignment, 2),
    )?;
    module.define_module_function(
        "check_indentation_width",
        function!(check_indentation_width, 4),
    )?;
    module.define_module_function(
        "check_block_delimiters",
        function!(check_block_delimiters, 4),
    )?;
    module.define_module_function(
        "check_block_delimiters_events",
        function!(check_block_delimiters_events, 3),
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
        "check_first_hash_element_indentation",
        function!(check_first_hash_element_indentation, 6),
    )?;
    module.define_module_function(
        "check_hash_alignment",
        function!(check_hash_alignment, 5),
    )?;
    module.define_module_function(
        "check_hash_each_methods",
        function!(check_hash_each_methods, 2),
    )?;
    module.define_module_function("check_hash_syntax", function!(check_hash_syntax, 2))?;
    module.define_module_function(
        "check_hash_transform_keys",
        function!(check_hash_transform_keys, 1),
    )?;
    module.define_module_function(
        "check_string_literals",
        function!(check_string_literals, 2),
    )?;
    module.define_module_function(
        "check_string_literals_in_interpolation",
        function!(check_string_literals_in_interpolation, 2),
    )?;
    module.define_module_function(
        "check_trailing_comma_in_arguments",
        function!(check_trailing_comma_in_arguments, 2),
    )?;
    module.define_module_function(
        "check_trailing_empty_lines",
        function!(check_trailing_empty_lines, 2),
    )?;
    module.define_module_function("check_void", function!(check_void, 2))?;
    module.define_module_function(
        "check_empty_lines_around_body",
        function!(check_empty_lines_around_body, 4),
    )?;
    module.define_module_function(
        "check_useless_access_modifier",
        function!(check_useless_access_modifier, 4),
    )?;
    Ok(())
}
