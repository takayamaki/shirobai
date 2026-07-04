//! Bundled single-walk runs: drive several cops over one parse + one AST walk.
//!
//! When multiple shirobai cops run on the same file, routing them through one
//! [`dispatch::run`](super::dispatch::run) collapses their N traversals into one
//! shared walk (the parse is already shared by `parse_cache`).

use std::collections::HashSet;

use super::multiline_method_call_indentation::{self as mc, MethodCallIndentOffense};
use super::multiline_operation_indentation::{self as op, OperationIndentOffense};
use super::{
    abc_size, access_modifier_indentation, ambiguous_block_association, argument_alignment,
    array_alignment, assignment_indentation,
    block_delimiters, block_length, block_nesting,
    class_length,
    closing_parenthesis_indentation, colon_method_call, complexity, debugger, def_end_alignment,
    dot_position,
    duplicate_magic_comment, duplicate_methods,
    empty_comment,
    empty_line_after_guard_clause,
    empty_line_after_magic_comment,
    empty_line_between_defs,
    empty_lines,
    block_alignment, else_alignment, empty_lines_around_arguments, empty_lines_around_body,
    end_alignment,
    first_argument_indentation, first_array_element_indentation, first_hash_element_indentation,
    hash_alignment, hash_each_methods, hash_syntax, hash_transform_keys,
    if_unless_modifier,
    indentation_consistency, indentation_width,
    leading_empty_lines,
    line_end_concatenation, line_length,
    line_length_breakable, method_length, method_name, module_length,
    multiline_method_call_brace_layout, nested_parenthesized_calls,
    parentheses_as_grouped_expression,
    percent_literal_delimiters,
    predicate_prefix, punctuation_spacing, redundant_self, redundant_self_assignment,
    require_parentheses, safe_navigation_chain, self_assignment,
    space_around_keyword, space_around_method_call_operator, space_before_block_braces,
    space_before_first_arg,
    space_inside_array_literal_brackets, space_inside_block_braces,
    space_inside_hash_literal_braces, space_inside_parens, space_inside_reference_brackets,
    stabby_lambda_parentheses,
    string_literals,
    string_literals_in_interpolation,
    trailing_comma, trailing_comma_in_arguments, trailing_comma_in_array_literal,
    trailing_comma_in_hash_literal, trailing_empty_lines, unreachable_code,
    useless_access_modifier,
    variable_number, void,
};

/// Run `Layout/MultilineOperationIndentation` and
/// `Layout/MultilineMethodCallIndentation` together in one walk.
#[allow(clippy::type_complexity)]
pub fn check_multiline_bundle(
    source: &[u8],
    op_style: u8,
    op_indent: usize,
    op_base: usize,
    mc_style: u8,
    mc_indent: usize,
    mc_base: usize,
) -> (Vec<OperationIndentOffense>, Vec<MethodCallIndentOffense>) {
    let mut op_rule = op::build_rule(source, op_style, op_indent, op_base);
    let mut mc_rule = mc::build_rule(source, mc_style, mc_indent, mc_base);
    super::dispatch::run(source, &mut [&mut op_rule, &mut mc_rule]);
    (op_rule.offenses, mc_rule.offenses)
}

/// Every cop's packed configuration for [`check_all_bundle`]: exactly the
/// values the per-cop entry points receive from the Ruby wrappers today.
///
/// Built from the flat `(nums, lists)` wire format via [`BundleConfig::from_packed`].
/// This is the single place that documents the packing order; the Ruby side
/// (`Shirobai::Dispatch.packed_config`) assembles the two arrays in the same
/// order.
///
/// `nums` (`i64`, booleans are `0`/`1`), 38 entries:
///
/// | idx | field |
/// |-----|-------|
/// |  0  | block_length_max |
/// |  1  | block_length_count_comments |
/// |  2  | block_length_filtered |
/// |  3  | block_nesting_max |
/// |  4  | block_nesting_count_blocks |
/// |  5  | block_nesting_count_modifier_forms |
/// |  6  | max_cyclomatic |
/// |  7  | max_perceived |
/// |  8  | variable_number_style |
/// |  9  | variable_number_flags |
/// | 10  | method_name_style |
/// | 11  | dot_position_style |
/// | 12  | line_length_max |
/// | 13  | line_length_tab_width |
/// | 14  | line_length_split_strings |
/// | 15-17 | multiline_operation style / indent / base |
/// | 18-20 | multiline_method_call style / indent / base |
/// | 21-23 | argument_alignment style / indent / incompatible |
/// | 24-26 | first_argument_indentation style / indent / enforce_fixed_no_line_break |
/// | 27-33 | indentation_width width / relative_to_receiver / access_modifier_outdent / indented_internal_methods / end_align / def_end_align_def / use_tabs |
/// | 34  | closing_paren_indent |
/// | 35-37 | first_array_element style / indent / enforce_fixed_indentation |
/// | 38  | void_check_nonmutating (`CheckForMethodsWithNoSideEffects`) |
/// | 39  | useless_access_modifier_active_support (`AllCops/ActiveSupportExtensionsEnabled`) |
/// | 40-42 | empty_lines_around_body class / module / block styles (`EmptyLinesAroundClassBody` / `...ModuleBody` / `...BlockBody` EnforcedStyle) |
/// | 43  | block_delimiters_style (`EnforcedStyle`: 0 = line_count_based, 1 = semantic, 2 = braces_for_chaining, 3 = always_braces) |
/// | 44  | block_delimiters_oneliners (`AllowBracesOnProceduralOneLiners`) |
/// | 45  | abc_size_max_floor (`Metrics/AbcSize` `Max.floor`, prefilter; `-1` reports every method) |
/// | 46  | abc_size_discount_repeated (`!CountRepeatedAttributes`) |
/// | 47  | indentation_consistency_internal (`Layout/IndentationConsistency` EnforcedStyle == 'indented_internal_methods') |
/// | 48-50 | empty_line_between_defs method / class / module defs (`EmptyLineBetweenMethodDefs` / `...ClassDefs` / `...ModuleDefs`) |
/// | 51  | empty_line_between_defs_allow_adjacent_one_line_defs (`AllowAdjacentOneLineDefs`) |
/// | 52-53 | empty_line_between_defs min / max empty lines (`NumberOfEmptyLines` as `[min, max]`) |
/// | 54  | end_alignment style (`EnforcedStyleAlignWith`: 0 = keyword, 1 = variable, 2 = start_of_line) |
/// | 55  | block_alignment style (`EnforcedStyleAlignWith`: 0 = either, 1 = start_of_block, 2 = start_of_line) |
/// | 56  | else_alignment style (`Layout/EndAlignment`'s `EnforcedStyleAlignWith`: 0 = keyword, 1 = variable, 2 = start_of_line) |
/// | 57-61 | first_hash_element style / indent / enforce_fixed (`Layout/ArgumentAlignment` with_fixed_indentation) / colon_separator / rocket_separator (`Layout/HashAlignment` Enforced{Colon,HashRocket}Style == 'separator') |
/// | 62  | hash_alignment last_argument_style (`EnforcedLastArgumentHashStyle`: 0 always_inspect, 1 always_ignore, 2 ignore_explicit, 3 ignore_implicit) |
/// | 63  | hash_alignment enforce_fixed (`Layout/ArgumentAlignment` `with_fixed_indentation`) |
/// | 64-69 | hash_syntax style / shorthand / UseHashRocketsWithSymbolValues / PreferHashRocketsForNonAlnumEndingSymbols / ruby31_plus (`TargetRubyVersion > 3.0`) / ruby22_plus (`TargetRubyVersion > 2.1`) |
/// | 70-71 | string_literals style (`EnforcedStyle`: 0 single_quotes, 1 double_quotes) / consistent_multiline (`ConsistentQuotesInMultiline`) |
/// | 72  | trailing_comma_in_arguments style (`EnforcedStyleForMultiline`: 0 no_comma, 1 comma, 2 consistent_comma, 3 diff_comma) |
/// | 73  | string_literals_in_interpolation style (`EnforcedStyle`: 0 single_quotes, 1 double_quotes) |
/// | 74  | trailing_empty_lines style (`EnforcedStyle`: 0 final_newline, 1 final_blank_line) |
/// | 75  | space_inside_block_braces style (`EnforcedStyle`: 0 space, 1 no_space) |
/// | 76  | space_inside_block_braces empty style (`EnforcedStyleForEmptyBraces`: 0 space, 1 no_space) |
/// | 77  | space_inside_block_braces space_before_block_parameters (`SpaceBeforeBlockParameters`) |
/// | 78  | method_length_max (`Metrics/MethodLength` `Max`, default 10) |
/// | 79  | method_length_count_comments (`CountComments`) |
/// | 80  | def_end_alignment style (`Layout/DefEndAlignment` `EnforcedStyleAlignWith`: 0 = start_of_line, 1 = def) |
/// | 81  | multiline_method_call_brace_layout style (`Layout/MultilineMethodCallBraceLayout` `EnforcedStyle`: 0 = symmetrical, 1 = new_line, 2 = same_line) |
/// | 82  | access_modifier_indentation style (`Layout/AccessModifierIndentation` `EnforcedStyle`: 0 = indent, 1 = outdent) |
/// | 83  | access_modifier_indentation indentation_width (`Layout/AccessModifierIndentation` `IndentationWidth` override, falling back to `Layout/IndentationWidth` `Width`) |
/// | 84  | assignment_indentation indentation_width (`Layout/AssignmentIndentation` `IndentationWidth` falling back to `Layout/IndentationWidth.Width` falling back to 2) |
/// | 85  | stabby_lambda_parentheses style (`Style/StabbyLambdaParentheses` `EnforcedStyle`: 0 = require_parentheses, 1 = require_no_parentheses) |
/// | 86  | empty_comment_allow_border (`Layout/EmptyComment` `AllowBorderComment`) |
/// | 87  | empty_comment_allow_margin (`Layout/EmptyComment` `AllowMarginComment`) |
/// | 88-89 | class_length max / count_comments (`Metrics/ClassLength` `Max` / `CountComments`) |
/// | 90-91 | module_length max / count_comments (`Metrics/ModuleLength` `Max` / `CountComments`) |
/// | 92  | trailing_comma_in_hash_literal style (`EnforcedStyleForMultiline`: 0 no_comma, 1 comma, 2 consistent_comma, 3 diff_comma) |
/// | 93  | trailing_comma_in_array_literal style (`EnforcedStyleForMultiline`, same coding) |
/// | 94  | space_inside_hash_literal_braces style (`EnforcedStyle`: 0 space, 1 no_space, 2 compact) |
/// | 95  | space_inside_hash_literal_braces empty no_space (`EnforcedStyleForEmptyBraces == 'no_space'`) |
/// | 96  | space_inside_array_literal_brackets style (`EnforcedStyle`: 0 no_space, 1 space, 2 compact) |
/// | 97  | space_inside_array_literal_brackets empty space (`EnforcedStyleForEmptyBrackets == 'space'`) |
///
/// | 98  | if_unless_modifier_max (`Style/IfUnlessModifier`'s view of `Layout/LineLength` `Max`; `-1` when that cop is disabled) |
/// | 99  | if_unless_modifier_tab_width (`LineLengthHelp#tab_indentation_width` for this cop) |
/// | 100 | space_before_block_braces style (`EnforcedStyle`: 0 space, 1 no_space) |
/// | 101 | space_before_block_braces empty style (`EnforcedStyleForEmptyBraces` resolved: 0 space, 1 no_space, 2 invalid — `nil` follows `EnforcedStyle`) |
/// | 102 | space_before_block_braces bd_line_count_based (`Style/BlockDelimiters` `EnforcedStyle == 'line_count_based'`) |
/// | 103 | space_before_comma lcurly_space (`Layout/SpaceBeforeComma`'s view of `Layout/SpaceInsideBlockBraces` `EnforcedStyle == 'space'`) |
/// | 104 | space_after_comma rcurly_no_space (`Layout/SpaceAfterComma`'s view of `Layout/SpaceInsideHashLiteralBraces` `EnforcedStyle == 'no_space'`) |
/// | 105 | space_before_semicolon lcurly_space (`Layout/SpaceBeforeSemicolon`'s view of `Layout/SpaceInsideBlockBraces` `EnforcedStyle == 'space'`) |
/// | 106 | space_after_semicolon rcurly_no_space (`Layout/SpaceAfterSemicolon`'s view of `Layout/SpaceInsideBlockBraces` `EnforcedStyle == 'no_space'`) |
/// | 107 | space_inside_parens style (`EnforcedStyle`: 0 no_space, 1 space, 2 compact) |
/// | 108 | space_inside_reference_brackets style (`EnforcedStyle`: 0 no_space, 1 space) |
/// | 109 | space_inside_reference_brackets empty space (`EnforcedStyleForEmptyBrackets == 'space'`) |
/// | 110 | space_before_first_arg allow_for_alignment (`AllowForAlignment`) |
/// | 111 | duplicate_methods_active_support (`AllCops/ActiveSupportExtensionsEnabled`; `Lint/DuplicateMagicComment` is config-less) |
/// | 112 | array_alignment style (`EnforcedStyle`: 0 = with_first_element, 1 = with_fixed_indentation) |
/// | 113 | array_alignment indentation width (`IndentationWidth` falling back to `Layout/IndentationWidth.Width` falling back to 2) |
///
/// `lists` (`Vec<String>`), 20 entries:
///
/// | idx | field |
/// |-----|-------|
/// |  0  | debugger_methods |
/// |  1  | debugger_requires |
/// |  2  | block_length_count_as_one |
/// |  3  | block_length_allowed_methods |
/// |  4  | variable_number_allowed_identifiers |
/// |  5  | safe_navigation_nil_methods |
/// |  6  | redundant_self_kernel_methods |
/// |  7  | predicate_prefix_name_prefixes |
/// |  8  | predicate_prefix_macros |
/// |  9  | hash_each_allowed_receivers |
/// | 10  | useless_access_modifier_context_creating (`ContextCreatingMethods`) |
/// | 11  | useless_access_modifier_method_creating (`MethodCreatingMethods`) |
/// | 12  | block_delimiters_procedural (`ProceduralMethods`) |
/// | 13  | block_delimiters_functional (`FunctionalMethods`) |
/// | 14  | block_delimiters_allowed (`AllowedMethods`, deprecated lists merged) |
/// | 15  | block_delimiters_braces_required (`BracesRequiredMethods`) |
/// | 16  | empty_line_between_defs_def_like_macros (`DefLikeMacros`, verbatim names) |
/// | 17  | hash_alignment_rocket_styles (`EnforcedHashRocketStyle`, comma-joined `key`/`separator`/`table`) |
/// | 18  | hash_alignment_colon_styles (`EnforcedColonStyle`, comma-joined) |
/// | 19  | method_length_count_as_one (`Metrics/MethodLength` `CountAsOne`) |
/// | 20  | nested_parenthesized_calls_allowed_methods (`Style/NestedParenthesizedCalls` `AllowedMethods`) |
/// | 21  | percent_literal_delimiters_pairs (`Style/PercentLiteralDelimiters` `PreferredDelimiters`, resolved to 10 two-byte strings in `[%, %i, %I, %q, %Q, %r, %s, %w, %W, %x]` order) |
/// | 22  | ambiguous_block_association_allowed_methods (`Lint/AmbiguousBlockAssociation` `AllowedMethods`, regexp entries dropped by the Ruby wrapper which falls back to standalone when any regexp is present) |
/// | 23  | class_length_count_as_one (`Metrics/ClassLength` `CountAsOne`) |
/// | 24  | module_length_count_as_one (`Metrics/ModuleLength` `CountAsOne`) |
///
/// `Layout/IndentationWidth`'s `allowed_lines` and `prior_ranges` are fixed to
/// empty in the bundle: the non-empty cases (configured `AllowedPatterns`,
/// autocorrect re-passes) take the per-cop fallback path on the Ruby side.
/// `Style/BlockDelimiters`' prior ignored ranges are likewise fixed to empty
/// (autocorrect re-passes go standalone), and its `AllowedPatterns` force the
/// raw-event standalone path.
pub struct BundleConfig {
    pub debugger_methods: Vec<String>,
    pub debugger_requires: Vec<String>,
    pub block_length_max: usize,
    pub block_length_count_comments: bool,
    pub block_length_count_as_one: Vec<String>,
    pub block_length_allowed_methods: Vec<String>,
    pub block_length_filtered: bool,
    pub block_nesting_max: usize,
    pub block_nesting_count_blocks: bool,
    pub block_nesting_count_modifier_forms: bool,
    pub max_cyclomatic: usize,
    pub max_perceived: usize,
    pub variable_number_style: u8,
    pub variable_number_flags: u8,
    pub variable_number_allowed_identifiers: Vec<String>,
    pub method_name_style: u8,
    pub safe_navigation_nil_methods: Vec<String>,
    pub dot_position_style: u8,
    pub line_length_max: usize,
    pub line_length_tab_width: usize,
    pub line_length_split_strings: bool,
    pub multiline_operation: (u8, usize, usize),
    pub multiline_method_call: (u8, usize, usize),
    pub argument_alignment_style: u8,
    pub argument_alignment_indent: usize,
    pub argument_alignment_incompatible: bool,
    pub array_alignment_style: u8,
    pub array_alignment_indent: usize,
    pub first_argument_style: u8,
    pub first_argument_indent: usize,
    pub first_argument_enforce_fixed_no_line_break: bool,
    pub indentation_width: indentation_width::Config,
    pub closing_paren_indent: usize,
    pub first_array_element_style: u8,
    pub first_array_element_indent: usize,
    pub first_array_element_enforce_fixed: bool,
    pub redundant_self_kernel_methods: Vec<String>,
    pub predicate_prefix_name_prefixes: Vec<String>,
    pub predicate_prefix_macros: Vec<String>,
    pub hash_each_allowed_receivers: Vec<String>,
    pub void_check_nonmutating: bool,
    pub useless_access_modifier_context_creating: Vec<String>,
    pub useless_access_modifier_method_creating: Vec<String>,
    pub useless_access_modifier_active_support: bool,
    pub empty_lines_around_body: empty_lines_around_body::Config,
    pub block_delimiters: block_delimiters::Config,
    pub abc_size_max_floor: i64,
    pub abc_size_discount_repeated: bool,
    pub indentation_consistency: indentation_consistency::Config,
    pub empty_line_between_defs: empty_line_between_defs::Config,
    pub end_alignment: end_alignment::Config,
    pub block_alignment: block_alignment::Config,
    pub else_alignment: else_alignment::Config,
    pub first_hash_element_style: u8,
    pub first_hash_element_indent: usize,
    pub first_hash_element_enforce_fixed: bool,
    pub first_hash_element_separators: first_hash_element_indentation::SeparatorConfig,
    pub hash_alignment: hash_alignment::Config,
    pub hash_syntax: hash_syntax::Config,
    pub string_literals: string_literals::Config,
    pub string_literals_in_interpolation: string_literals_in_interpolation::Config,
    pub trailing_comma_in_arguments: trailing_comma_in_arguments::Config,
    pub trailing_comma_in_hash_literal: trailing_comma::Config,
    pub trailing_comma_in_array_literal: trailing_comma::Config,
    pub trailing_empty_lines: trailing_empty_lines::Config,
    pub space_inside_block_braces: space_inside_block_braces::Config,
    pub method_length_max: usize,
    pub method_length_count_comments: bool,
    pub method_length_count_as_one: Vec<String>,
    pub class_length_max: usize,
    pub class_length_count_comments: bool,
    pub class_length_count_as_one: Vec<String>,
    pub module_length_max: usize,
    pub module_length_count_comments: bool,
    pub module_length_count_as_one: Vec<String>,
    pub def_end_alignment: def_end_alignment::Config,
    pub multiline_method_call_brace_style: u8,
    pub nested_parenthesized_calls_allowed_methods: Vec<String>,
    pub percent_literal_delimiters: percent_literal_delimiters::Config,
    pub access_modifier_indentation: access_modifier_indentation::Config,
    pub assignment_indentation: assignment_indentation::Config,
    pub stabby_lambda_parentheses: stabby_lambda_parentheses::Config,
    pub ambiguous_block_association: ambiguous_block_association::Config,
    pub empty_comment: empty_comment::Config,
    pub space_inside_hash_literal_braces: space_inside_hash_literal_braces::Config,
    pub space_inside_array_literal_brackets: space_inside_array_literal_brackets::Config,
    pub if_unless_modifier: if_unless_modifier::Config,
    pub space_before_block_braces: space_before_block_braces::Config,
    pub punctuation_spacing: punctuation_spacing::Config,
    pub space_inside_parens: space_inside_parens::Config,
    pub space_inside_reference_brackets: space_inside_reference_brackets::Config,
    pub space_before_first_arg: space_before_first_arg::Config,
    pub duplicate_methods: duplicate_methods::Config,
}

// `Lint/ParenthesesAsGroupedExpression` carries no config so it doesn't appear
// in the `nums` / `lists` packing; the bundle still computes its slot in the
// shared walk (see `check_all_bundle` below).

const NUMS_LEN: usize = 114;
const LISTS_LEN: usize = 25;

impl BundleConfig {
    /// Build a config from the flat wire format (see the struct docs for the
    /// packing order). Errors on a length mismatch so a Ruby/Rust drift fails
    /// loudly instead of silently misassigning fields.
    pub fn from_packed(nums: &[i64], lists: Vec<Vec<String>>) -> Result<Self, String> {
        if nums.len() != NUMS_LEN {
            return Err(format!(
                "bundle config expects {NUMS_LEN} nums, got {}",
                nums.len()
            ));
        }
        if lists.len() != LISTS_LEN {
            return Err(format!(
                "bundle config expects {LISTS_LEN} lists, got {}",
                lists.len()
            ));
        }
        let mut lists = lists.into_iter();
        let mut next_list = || lists.next().expect("length checked above");
        Ok(BundleConfig {
            debugger_methods: next_list(),
            debugger_requires: next_list(),
            block_length_max: nums[0] as usize,
            block_length_count_comments: nums[1] != 0,
            block_length_count_as_one: next_list(),
            block_length_allowed_methods: next_list(),
            block_length_filtered: nums[2] != 0,
            block_nesting_max: nums[3] as usize,
            block_nesting_count_blocks: nums[4] != 0,
            block_nesting_count_modifier_forms: nums[5] != 0,
            max_cyclomatic: nums[6] as usize,
            max_perceived: nums[7] as usize,
            variable_number_style: nums[8] as u8,
            variable_number_flags: nums[9] as u8,
            variable_number_allowed_identifiers: next_list(),
            method_name_style: nums[10] as u8,
            safe_navigation_nil_methods: next_list(),
            dot_position_style: nums[11] as u8,
            line_length_max: nums[12] as usize,
            line_length_tab_width: nums[13] as usize,
            line_length_split_strings: nums[14] != 0,
            multiline_operation: (nums[15] as u8, nums[16] as usize, nums[17] as usize),
            multiline_method_call: (nums[18] as u8, nums[19] as usize, nums[20] as usize),
            argument_alignment_style: nums[21] as u8,
            argument_alignment_indent: nums[22] as usize,
            argument_alignment_incompatible: nums[23] != 0,
            array_alignment_style: nums[112] as u8,
            array_alignment_indent: nums[113] as usize,
            first_argument_style: nums[24] as u8,
            first_argument_indent: nums[25] as usize,
            first_argument_enforce_fixed_no_line_break: nums[26] != 0,
            indentation_width: indentation_width::Config {
                width: nums[27] as usize,
                relative_to_receiver: nums[28] != 0,
                access_modifier_outdent: nums[29] != 0,
                indented_internal_methods: nums[30] != 0,
                end_align: nums[31] as u8,
                def_end_align_def: nums[32] != 0,
                use_tabs: nums[33] != 0,
            },
            closing_paren_indent: nums[34] as usize,
            first_array_element_style: nums[35] as u8,
            first_array_element_indent: nums[36] as usize,
            first_array_element_enforce_fixed: nums[37] != 0,
            redundant_self_kernel_methods: next_list(),
            predicate_prefix_name_prefixes: next_list(),
            predicate_prefix_macros: next_list(),
            hash_each_allowed_receivers: next_list(),
            void_check_nonmutating: nums[38] != 0,
            useless_access_modifier_context_creating: next_list(),
            useless_access_modifier_method_creating: next_list(),
            useless_access_modifier_active_support: nums[39] != 0,
            empty_lines_around_body: empty_lines_around_body::Config {
                class_style: nums[40] as u8,
                module_style: nums[41] as u8,
                block_style: nums[42] as u8,
            },
            block_delimiters: block_delimiters::Config {
                style: nums[43] as u8,
                allow_braces_on_procedural_oneliners: nums[44] != 0,
                procedural_methods: next_list(),
                functional_methods: next_list(),
                allowed_methods: next_list(),
                braces_required_methods: next_list(),
            },
            abc_size_max_floor: nums[45],
            abc_size_discount_repeated: nums[46] != 0,
            indentation_consistency: indentation_consistency::Config {
                indented_internal_methods: nums[47] != 0,
            },
            empty_line_between_defs: empty_line_between_defs::Config {
                method_defs: nums[48] != 0,
                class_defs: nums[49] != 0,
                module_defs: nums[50] != 0,
                allow_adjacent_one_line_defs: nums[51] != 0,
                minimum_empty_lines: nums[52] as usize,
                maximum_empty_lines: nums[53] as usize,
                def_like_macros: next_list(),
            },
            end_alignment: end_alignment::Config {
                style: nums[54] as u8,
            },
            block_alignment: block_alignment::Config {
                style: nums[55] as u8,
            },
            else_alignment: else_alignment::Config {
                style: nums[56] as u8,
            },
            first_hash_element_style: nums[57] as u8,
            first_hash_element_indent: nums[58] as usize,
            first_hash_element_enforce_fixed: nums[59] != 0,
            first_hash_element_separators: first_hash_element_indentation::SeparatorConfig {
                colon_separator: nums[60] != 0,
                rocket_separator: nums[61] != 0,
            },
            hash_alignment: hash_alignment::Config {
                hash_rocket_styles: parse_hash_styles(&next_list()[..]),
                colon_styles: parse_hash_styles(&next_list()[..]),
                last_argument_style: nums[62] as u8,
                enforce_fixed_indentation: nums[63] != 0,
            },
            hash_syntax: hash_syntax::Config {
                style: nums[64] as u8,
                shorthand: nums[65] as u8,
                use_hash_rockets_with_symbol_values: nums[66] != 0,
                prefer_hash_rockets_for_non_alnum_ending_symbols: nums[67] != 0,
                ruby31_plus: nums[68] != 0,
                ruby22_plus: nums[69] != 0,
            },
            string_literals: string_literals::Config {
                style: nums[70] as u8,
                consistent_multiline: nums[71] != 0,
            },
            trailing_comma_in_arguments: trailing_comma_in_arguments::Config {
                style: nums[72] as u8,
            },
            trailing_comma_in_hash_literal: trailing_comma::Config {
                style: nums[92] as u8,
            },
            trailing_comma_in_array_literal: trailing_comma::Config {
                style: nums[93] as u8,
            },
            string_literals_in_interpolation: string_literals_in_interpolation::Config {
                style: nums[73] as u8,
            },
            trailing_empty_lines: trailing_empty_lines::Config {
                style: nums[74] as u8,
            },
            space_inside_block_braces: space_inside_block_braces::Config {
                style: if nums[75] != 0 {
                    space_inside_block_braces::Style::NoSpace
                } else {
                    space_inside_block_braces::Style::Space
                },
                empty_braces_style: if nums[76] != 0 {
                    space_inside_block_braces::Style::NoSpace
                } else {
                    space_inside_block_braces::Style::Space
                },
                space_before_block_parameters: nums[77] != 0,
            },
            method_length_max: nums[78] as usize,
            method_length_count_comments: nums[79] != 0,
            method_length_count_as_one: next_list(),
            class_length_max: nums[88] as usize,
            class_length_count_comments: nums[89] != 0,
            module_length_max: nums[90] as usize,
            module_length_count_comments: nums[91] != 0,
            def_end_alignment: def_end_alignment::Config {
                style: nums[80] as u8,
            },
            multiline_method_call_brace_style: nums[81] as u8,
            nested_parenthesized_calls_allowed_methods: next_list(),
            percent_literal_delimiters: parse_percent_pairs(&next_list()[..])?,
            access_modifier_indentation: access_modifier_indentation::Config {
                style: nums[82] as u8,
                indentation_width: nums[83] as usize,
            },
            assignment_indentation: assignment_indentation::Config {
                indentation_width: nums[84] as usize,
            },
            stabby_lambda_parentheses: stabby_lambda_parentheses::Config {
                style: nums[85] as u8,
            },
            ambiguous_block_association: ambiguous_block_association::Config {
                allowed_methods: next_list(),
            },
            empty_comment: empty_comment::Config {
                allow_border_comment: nums[86] != 0,
                allow_margin_comment: nums[87] != 0,
            },
            // Struct literal order is evaluation order: these two must come
            // after every earlier `next_list()` so they read lists 23 / 24
            // (the space cop configs below consume no lists).
            class_length_count_as_one: next_list(),
            module_length_count_as_one: next_list(),
            space_inside_hash_literal_braces: space_inside_hash_literal_braces::Config {
                style: match nums[94] {
                    1 => space_inside_hash_literal_braces::Style::NoSpace,
                    2 => space_inside_hash_literal_braces::Style::Compact,
                    _ => space_inside_hash_literal_braces::Style::Space,
                },
                no_space_empty: nums[95] != 0,
            },
            space_inside_array_literal_brackets: space_inside_array_literal_brackets::Config {
                style: match nums[96] {
                    1 => space_inside_array_literal_brackets::Style::Space,
                    2 => space_inside_array_literal_brackets::Style::Compact,
                    _ => space_inside_array_literal_brackets::Style::NoSpace,
                },
                space_empty: nums[97] != 0,
            },
            if_unless_modifier: if_unless_modifier::Config {
                max_line_length: if nums[98] < 0 { None } else { Some(nums[98]) },
                tab_width: nums[99],
            },
            space_before_block_braces: space_before_block_braces::Config {
                style: if nums[100] != 0 {
                    space_before_block_braces::Style::NoSpace
                } else {
                    space_before_block_braces::Style::Space
                },
                empty_style: match nums[101] {
                    1 => space_before_block_braces::EmptyStyle::NoSpace,
                    2 => space_before_block_braces::EmptyStyle::Invalid,
                    _ => space_before_block_braces::EmptyStyle::Space,
                },
                bd_line_count_based: nums[102] != 0,
            },
            punctuation_spacing: punctuation_spacing::Config {
                before_comma_lcurly_space: nums[103] != 0,
                after_comma_rcurly_no_space: nums[104] != 0,
                before_semi_lcurly_space: nums[105] != 0,
                after_semi_rcurly_no_space: nums[106] != 0,
            },
            space_inside_parens: space_inside_parens::Config {
                style: match nums[107] {
                    1 => space_inside_parens::Style::Space,
                    2 => space_inside_parens::Style::Compact,
                    _ => space_inside_parens::Style::NoSpace,
                },
            },
            space_inside_reference_brackets: space_inside_reference_brackets::Config {
                style: if nums[108] != 0 {
                    space_inside_reference_brackets::Style::Space
                } else {
                    space_inside_reference_brackets::Style::NoSpace
                },
                space_empty: nums[109] != 0,
            },
            space_before_first_arg: space_before_first_arg::Config {
                allow_for_alignment: nums[110] != 0,
            },
            duplicate_methods: duplicate_methods::Config {
                active_support_extensions_enabled: nums[111] != 0,
            },
        })
    }
}

/// Parse the 10-entry `PreferredDelimiters` list into a [`percent_literal_delimiters::Config`].
/// Every entry must be exactly two ASCII bytes (open, close); anything else is
/// a Ruby-side packing bug and fails loudly.
fn parse_percent_pairs(
    pairs: &[String],
) -> Result<percent_literal_delimiters::Config, String> {
    if pairs.len() != 10 {
        return Err(format!(
            "Style/PercentLiteralDelimiters expects 10 pair strings, got {}",
            pairs.len()
        ));
    }
    use percent_literal_delimiters::DelimPair;
    let mut out: [DelimPair; 10] = [DelimPair { open: b'(', close: b')' }; 10];
    for (i, p) in pairs.iter().enumerate() {
        let bytes = p.as_bytes();
        if bytes.len() != 2 {
            return Err(format!(
                "Style/PercentLiteralDelimiters pair #{i} must be 2 bytes, got {p:?}"
            ));
        }
        out[i] = DelimPair { open: bytes[0], close: bytes[1] };
    }
    Ok(percent_literal_delimiters::Config { pairs: out })
}

/// Parse a `Layout/HashAlignment` style list (`key` / `separator` / `table`)
/// into the `0`/`1`/`2` codes, in config order. An empty list yields an empty
/// vector (the rule then defaults to `key`).
fn parse_hash_styles(styles: &[String]) -> Vec<u8> {
    styles
        .iter()
        .map(|s| match s.as_str() {
            "separator" => 1,
            "table" => 2,
            _ => 0,
        })
        .collect()
}

/// Every cop's results for one source, in each cop's existing result type.
pub struct BundleResult {
    pub debugger: Vec<debugger::DebuggerOffense>,
    pub block_length: Vec<block_length::BlockLengthCandidate>,
    pub block_nesting: (Vec<block_nesting::BlockNestingOffense>, usize),
    pub complexity: Vec<complexity::MethodComplexity>,
    pub variable_number: (Vec<variable_number::VariableNumberOffense>, bool),
    pub method_name: (Vec<method_name::MethodNameCandidate>, bool),
    pub safe_navigation_chain: Vec<safe_navigation_chain::SafeNavChainOffense>,
    pub multiline_operation: Vec<OperationIndentOffense>,
    pub multiline_method_call: Vec<MethodCallIndentOffense>,
    pub dot_position: Vec<dot_position::DotPositionOffense>,
    pub line_length: Vec<line_length::LineLengthCandidate>,
    pub line_length_breakables: Vec<line_length_breakable::Breakable>,
    pub line_end_concatenation: Vec<line_end_concatenation::LineEndConcatOffense>,
    pub argument_alignment: Vec<argument_alignment::ArgAlignOffense>,
    pub array_alignment: Vec<array_alignment::ArrayAlignOffense>,
    pub first_argument_indentation: Vec<first_argument_indentation::FirstArgIndentOffense>,
    pub redundant_self: Vec<redundant_self::RedundantSelfOffense>,
    pub indentation_width: Vec<indentation_width::IndentationOffense>,
    pub predicate_prefix: Vec<predicate_prefix::PredicatePrefixCandidate>,
    pub closing_parenthesis_indentation:
        Vec<closing_parenthesis_indentation::ClosingParenIndentOffense>,
    pub first_array_element_indentation:
        Vec<first_array_element_indentation::FirstArrayElemIndentOffense>,
    pub hash_each_methods: Vec<hash_each_methods::HashEachOffense>,
    pub void: Vec<void::VoidOffense>,
    pub useless_access_modifier: Vec<useless_access_modifier::UselessAccessModifierOffense>,
    pub empty_lines_around_body: empty_lines_around_body::FamilyOffenses,
    pub empty_lines_around_arguments:
        Vec<empty_lines_around_arguments::EmptyLinesAroundArgumentsOffense>,
    pub block_delimiters: block_delimiters::BlockDelimitersResult,
    pub abc_size: Vec<abc_size::AbcMethod>,
    pub indentation_consistency: Vec<indentation_consistency::ConsistencyOffense>,
    pub empty_line_between_defs: Vec<empty_line_between_defs::EmptyLineBetweenDefsOffense>,
    pub end_alignment: Vec<end_alignment::EndAlignmentRecord>,
    pub block_alignment: Vec<block_alignment::BlockAlignmentOffense>,
    pub else_alignment: Vec<else_alignment::ElseAlignmentOffense>,
    pub first_hash_element_indentation:
        Vec<first_hash_element_indentation::FirstHashElemIndentOffense>,
    pub hash_alignment: Vec<hash_alignment::HashAlignmentOffense>,
    pub hash_syntax: Vec<hash_syntax::HashSyntaxOffense>,
    pub string_literals: Vec<string_literals::StringLiteralsOffense>,
    pub string_literals_in_interpolation:
        Vec<string_literals_in_interpolation::StringLiteralsInInterpolationOffense>,
    pub trailing_comma_in_arguments:
        Vec<trailing_comma_in_arguments::TrailingCommaInArgumentsOffense>,
    pub trailing_comma_in_hash_literal: Vec<trailing_comma::TrailingCommaOffense>,
    pub trailing_comma_in_array_literal: Vec<trailing_comma::TrailingCommaOffense>,
    /// At most one offense per file (the final-newline / trailing-blank check).
    pub trailing_empty_lines: Option<trailing_empty_lines::TrailingEmptyLinesOffense>,
    pub space_around_method_call_operator:
        Vec<space_around_method_call_operator::SpaceAroundMethodCallOperatorOffense>,
    pub space_around_keyword: Vec<space_around_keyword::SpaceAroundKeywordOffense>,
    pub space_inside_block_braces:
        Vec<space_inside_block_braces::SpaceInsideBlockBracesOffense>,
    pub method_length: Vec<method_length::MethodLengthCandidate>,
    pub class_length: Vec<class_length::ClassLengthCandidate>,
    pub module_length: Vec<module_length::ModuleLengthCandidate>,
    pub def_end_alignment: Vec<def_end_alignment::DefEndAlignmentRecord>,
    pub require_parentheses: Vec<require_parentheses::RequireParenthesesOffense>,
    pub self_assignment: Vec<self_assignment::SelfAssignmentOffense>,
    pub nested_parenthesized_calls:
        Vec<nested_parenthesized_calls::NestedParenthesizedCallsOffense>,
    pub parentheses_as_grouped_expression:
        Vec<parentheses_as_grouped_expression::ParenthesesAsGroupedExpressionOffense>,
    pub percent_literal_delimiters:
        Vec<percent_literal_delimiters::PercentLiteralDelimitersOffense>,
    pub multiline_method_call_brace_layout:
        Vec<multiline_method_call_brace_layout::MmcblOffense>,
    pub access_modifier_indentation:
        Vec<access_modifier_indentation::AccessModifierIndentationRecord>,
    pub assignment_indentation: Vec<assignment_indentation::AssignmentIndentationOffense>,
    pub redundant_self_assignment:
        Vec<redundant_self_assignment::RedundantSelfAssignmentOffense>,
    pub colon_method_call: Vec<colon_method_call::ColonMethodCallOffense>,
    pub stabby_lambda_parentheses:
        Vec<stabby_lambda_parentheses::StabbyLambdaParenthesesOffense>,
    pub unreachable_code: Vec<unreachable_code::UnreachableCodeOffense>,
    pub hash_transform_keys: Vec<hash_transform_keys::HashTransformKeysOffense>,
    pub ambiguous_block_association:
        Vec<ambiguous_block_association::AmbiguousBlockAssociationOffense>,
    pub empty_line_after_guard_clause:
        Vec<empty_line_after_guard_clause::GuardClauseCandidate>,
    pub if_unless_modifier: Vec<if_unless_modifier::IfUnlessModifierCandidate>,
    pub empty_comment: Vec<empty_comment::EmptyCommentOffense>,
    pub empty_line_after_magic_comment:
        Vec<empty_line_after_magic_comment::MagicCommentCandidate>,
    pub empty_lines: Vec<empty_lines::EmptyLinesOffense>,
    /// At most one offense per file (the leading-blank-line offense).
    pub leading_empty_lines: Option<leading_empty_lines::LeadingEmptyLinesOffense>,
    pub space_inside_hash_literal_braces:
        Vec<space_inside_hash_literal_braces::SpaceInsideHashLiteralBracesOffense>,
    pub space_inside_array_literal_brackets:
        space_inside_array_literal_brackets::ArrayBracketsResult,
    pub space_before_block_braces: space_before_block_braces::SpaceBeforeBlockBracesResult,
    /// The whole punctuation-spacing family (six cops, six wire slots).
    pub punctuation_spacing: punctuation_spacing::PunctuationSpacingOffenses,
    pub space_inside_parens: Vec<space_inside_parens::SpaceInsideParensOffense>,
    pub space_inside_reference_brackets: space_inside_reference_brackets::ReferenceBracketsResult,
    pub space_before_first_arg: Vec<space_before_first_arg::SpaceBeforeFirstArgOffense>,
    /// 1-based lines of duplicate magic comments (encoding bucket then
    /// frozen-string-literal bucket, document order within each).
    pub duplicate_magic_comment: Vec<duplicate_magic_comment::DuplicateLine>,
    /// Per-file `found_method` event stream in stock callback order (the
    /// Ruby wrapper replays the cross-file bookkeeping).
    pub duplicate_methods: Vec<duplicate_methods::DupMethodEvent>,
}

/// Run every cop over one source in a single call, sharing one parse *and*
/// (for most cops) one AST walk.
///
/// The walk-merged cops are built as [`dispatch::Rule`](super::dispatch::Rule)s
/// and driven together through one `dispatch::run`. Each rule is the same
/// implementation the cop's standalone `check_*` entry point drives, so the
/// bundled results are identical to the per-cop results. Cops that cannot share
/// the generic walk keep their own traversal:
///
/// - `Naming/MethodName` deliberately prunes its walk (it skips `def`
///   parameters and a class' constant path / superclass), so the full shared
///   walk would feed it nodes its standalone walk never sees.
/// - The `LineLength` breakables depend on the `LineLength` candidates and
///   skip their walk entirely when no line is over the limit (the common case).
///
/// The line-based `LineLength` scan is not a walk at all; its heredoc
/// collection (the only AST-dependent part) joins the shared walk. The
/// breakables are derived from the `LineLength` candidates (the `line_index`
/// of every candidate), exactly like the Ruby wrapper does on the direct path.
pub fn check_all_bundle(source: &[u8], cfg: &BundleConfig) -> BundleResult {
    // --- Shared-walk rules, one per merged cop. ---
    let (op_cfg, mc_cfg) = (cfg.multiline_operation, cfg.multiline_method_call);
    let mut op_rule = op::build_rule(source, op_cfg.0, op_cfg.1, op_cfg.2);
    let mut mc_rule = mc::build_rule(source, mc_cfg.0, mc_cfg.1, mc_cfg.2);
    let mut aa_rule = argument_alignment::build_rule(
        source,
        cfg.argument_alignment_style,
        cfg.argument_alignment_indent,
        cfg.argument_alignment_incompatible,
    );
    let mut ara_rule = array_alignment::build_rule(
        source,
        cfg.array_alignment_style,
        cfg.array_alignment_indent,
    );
    let mut fa_rule = first_argument_indentation::build_rule(
        source,
        cfg.first_argument_style,
        cfg.first_argument_indent,
        cfg.first_argument_enforce_fixed_no_line_break,
    );
    let mut snc_rule = safe_navigation_chain::build_rule(source, &cfg.safe_navigation_nil_methods);
    let mut iw_rule = indentation_width::build_rule(source, cfg.indentation_width, &[], &[]);
    let mut debugger_rule =
        debugger::build_rule(source, &cfg.debugger_methods, &cfg.debugger_requires);
    let mut rs_rule = redundant_self::build_rule(&cfg.redundant_self_kernel_methods);
    let mut vn_rule = variable_number::build_rule(
        source,
        cfg.variable_number_style,
        cfg.variable_number_flags,
        &cfg.variable_number_allowed_identifiers,
    );
    let mut dp_rule = dot_position::build_rule(source, cfg.dot_position_style);
    let mut lec_rule = line_end_concatenation::build_rule(source);
    let mut bl_rule = block_length::build_rule(
        source,
        cfg.block_length_max,
        cfg.block_length_count_comments,
        &cfg.block_length_count_as_one,
        &cfg.block_length_allowed_methods,
        cfg.block_length_filtered,
    );
    let mut cx_rule = complexity::build_rule(source, cfg.max_cyclomatic, cfg.max_perceived);
    let mut bn_rule = block_nesting::build_rule(
        source,
        cfg.block_nesting_max,
        cfg.block_nesting_count_blocks,
        cfg.block_nesting_count_modifier_forms,
    );
    let mut heredoc_rule = line_length::build_heredoc_rule(source);
    let mut pp_rule = predicate_prefix::build_rule(
        source,
        &cfg.predicate_prefix_name_prefixes,
        &cfg.predicate_prefix_macros,
    );
    let mut cpi_rule =
        closing_parenthesis_indentation::build_rule(source, cfg.closing_paren_indent);
    let mut fae_rule = first_array_element_indentation::build_rule(
        source,
        cfg.first_array_element_style,
        cfg.first_array_element_indent,
        cfg.first_array_element_enforce_fixed,
    );
    let mut hem_rule = hash_each_methods::build_rule(source, &cfg.hash_each_allowed_receivers);
    let mut void_rule = void::build_rule(source, cfg.void_check_nonmutating);
    let mut uam_rule = useless_access_modifier::build_rule(
        &cfg.useless_access_modifier_context_creating,
        &cfg.useless_access_modifier_method_creating,
        cfg.useless_access_modifier_active_support,
    );
    let mut elab_rule = empty_lines_around_body::build_rule(source, cfg.empty_lines_around_body);
    let mut elaa_rule = empty_lines_around_arguments::build_rule(source);
    let mut bd_rule = block_delimiters::build_rule(source, cfg.block_delimiters.clone());
    let mut abc_rule = abc_size::build_rule(
        source,
        cfg.abc_size_max_floor,
        cfg.abc_size_discount_repeated,
    );
    let mut ic_rule = indentation_consistency::build_rule(source, cfg.indentation_consistency);
    let mut elbd_rule =
        empty_line_between_defs::build_rule(source, cfg.empty_line_between_defs.clone());
    let mut ea_rule = end_alignment::build_rule(source, cfg.end_alignment);
    let mut ba_rule = block_alignment::build_rule(source, cfg.block_alignment);
    let mut elsea_rule = else_alignment::build_rule(source, cfg.else_alignment);
    let mut fhe_rule = first_hash_element_indentation::build_rule(
        source,
        cfg.first_hash_element_style,
        cfg.first_hash_element_indent,
        cfg.first_hash_element_enforce_fixed,
        cfg.first_hash_element_separators,
    );
    let mut ha_rule = hash_alignment::build_rule(source, &cfg.hash_alignment);
    let mut hs_rule = hash_syntax::build_rule(source, &cfg.hash_syntax);
    let mut sl_rule = string_literals::build_rule(source, &cfg.string_literals);
    let mut sli_rule = string_literals_in_interpolation::build_rule(
        source,
        &cfg.string_literals_in_interpolation,
    );
    let mut tca_rule =
        trailing_comma_in_arguments::build_rule(source, &cfg.trailing_comma_in_arguments);
    let mut tchl_rule =
        trailing_comma_in_hash_literal::build_rule(source, &cfg.trailing_comma_in_hash_literal);
    let mut tcal_rule =
        trailing_comma_in_array_literal::build_rule(source, &cfg.trailing_comma_in_array_literal);
    let mut samco_rule = space_around_method_call_operator::build_rule(source);
    let mut sak_rule = space_around_keyword::build_rule(source);
    let mut sibb_rule = space_inside_block_braces::build_rule(source, cfg.space_inside_block_braces);
    let mut sihlb_rule = space_inside_hash_literal_braces::build_rule(
        source,
        cfg.space_inside_hash_literal_braces,
    );
    let mut sialb_rule = space_inside_array_literal_brackets::build_rule(
        source,
        cfg.space_inside_array_literal_brackets,
    );
    let mut sbbb_rule =
        space_before_block_braces::build_rule(source, cfg.space_before_block_braces);
    let mut ps_rule = punctuation_spacing::build_rule(source, cfg.punctuation_spacing);
    let mut sip_rule = space_inside_parens::build_rule(source, cfg.space_inside_parens);
    let mut sirb_rule =
        space_inside_reference_brackets::build_rule(source, cfg.space_inside_reference_brackets);
    let mut sbfa_rule = space_before_first_arg::build_rule(source, cfg.space_before_first_arg);
    let mut ml_rule = method_length::build_rule(
        source,
        cfg.method_length_max,
        cfg.method_length_count_comments,
        &cfg.method_length_count_as_one,
    );
    let mut cl_rule = class_length::build_rule(
        source,
        cfg.class_length_max,
        cfg.class_length_count_comments,
        &cfg.class_length_count_as_one,
    );
    let mut mol_rule = module_length::build_rule(
        source,
        cfg.module_length_max,
        cfg.module_length_count_comments,
        &cfg.module_length_count_as_one,
    );
    let mut dea_rule = def_end_alignment::build_rule(source, cfg.def_end_alignment);
    let mut rp_rule = require_parentheses::build_rule();
    let mut sa_rule = self_assignment::build_rule(source);
    let mut npc_rule = nested_parenthesized_calls::build_rule(
        source,
        &cfg.nested_parenthesized_calls_allowed_methods,
    );
    let mut pag_rule = parentheses_as_grouped_expression::build_rule();
    let mut pld_rule =
        percent_literal_delimiters::build_rule(source, cfg.percent_literal_delimiters.clone());
    let mut mmcbl_rule = multiline_method_call_brace_layout::build_rule(
        source,
        cfg.multiline_method_call_brace_style,
    );
    let mut ami_rule =
        access_modifier_indentation::build_rule(source, cfg.access_modifier_indentation);
    let mut ai_rule = assignment_indentation::build_rule(source, cfg.assignment_indentation);
    let mut rsa_rule = redundant_self_assignment::build_rule(source);
    let mut cmc_rule = colon_method_call::build_rule();
    let mut slp_rule =
        stabby_lambda_parentheses::build_rule(source, cfg.stabby_lambda_parentheses);
    let mut uc_rule = unreachable_code::build_rule();
    let mut htk_rule = hash_transform_keys::build_rule(source);
    let mut aba_rule =
        ambiguous_block_association::build_rule(source, cfg.ambiguous_block_association.clone());
    let mut ium_rule = if_unless_modifier::build_rule(source, cfg.if_unless_modifier);
    let mut dm_rule = duplicate_methods::build_rule(source, &cfg.duplicate_methods);
    // `Layout/EmptyLines` joins the shared walk only when the file actually
    // contains `\n\n\n` (stock's prefilter); otherwise the rule's collected
    // lines are unused and we skip both the walk push and the finalize. The
    // rule has to be constructed even when ineligible to keep the borrow
    // shape simple — `el_rule` is consumed only on the eligible path.
    let empty_lines_eligible = empty_lines::contains_newline_triple(source);
    let mut el_rule = empty_lines::build_rule(source);

    let mut rules: Vec<&mut dyn super::dispatch::Rule> = vec![
        &mut op_rule,
        &mut mc_rule,
        &mut snc_rule,
        &mut iw_rule,
        &mut debugger_rule,
        &mut rs_rule,
        &mut vn_rule,
        &mut dp_rule,
        &mut lec_rule,
        &mut bl_rule,
        &mut cx_rule,
        &mut bn_rule,
        &mut heredoc_rule,
        &mut pp_rule,
        &mut cpi_rule,
        &mut hem_rule,
        &mut void_rule,
        &mut uam_rule,
        &mut elab_rule,
        &mut elaa_rule,
        &mut bd_rule,
        &mut abc_rule,
        &mut ic_rule,
        &mut elbd_rule,
        &mut ea_rule,
        &mut ba_rule,
        &mut elsea_rule,
        &mut fhe_rule,
        &mut ha_rule,
        &mut hs_rule,
        &mut sl_rule,
        &mut sli_rule,
        &mut tca_rule,
        &mut tchl_rule,
        &mut tcal_rule,
        &mut samco_rule,
        &mut sak_rule,
        &mut sibb_rule,
        &mut sihlb_rule,
        &mut sialb_rule,
        &mut sbbb_rule,
        &mut ps_rule,
        &mut sip_rule,
        &mut sirb_rule,
        &mut sbfa_rule,
        &mut ml_rule,
        &mut cl_rule,
        &mut mol_rule,
        &mut dea_rule,
        &mut rp_rule,
        &mut sa_rule,
        &mut npc_rule,
        &mut pag_rule,
        &mut pld_rule,
        &mut mmcbl_rule,
        &mut ami_rule,
        &mut ai_rule,
        &mut rsa_rule,
        &mut cmc_rule,
        &mut slp_rule,
        &mut uc_rule,
        &mut htk_rule,
        &mut aba_rule,
        &mut ium_rule,
        &mut dm_rule,
        &mut ara_rule,
    ];
    if empty_lines_eligible {
        rules.push(&mut el_rule);
    }
    if let Some(rule) = aa_rule.as_mut() {
        rules.push(rule);
    }
    if let Some(rule) = fa_rule.as_mut() {
        rules.push(rule);
    }
    if let Some(rule) = fae_rule.as_mut() {
        rules.push(rule);
    }
    super::dispatch::run(source, &mut rules);

    let multiline_operation = op_rule.offenses;
    let multiline_method_call = mc_rule.offenses;
    let argument_alignment = aa_rule.map(|r| r.offenses).unwrap_or_default();
    let array_alignment = ara_rule.offenses;
    let first_argument_indentation = fa_rule.map(|r| r.offenses).unwrap_or_default();
    let safe_navigation_chain = snc_rule.offenses;
    let indentation_width = iw_rule.offenses;
    let debugger = debugger_rule.offenses;
    let redundant_self = rs_rule.offenses;
    let variable_number = (vn_rule.offenses, vn_rule.had_correct);
    let dot_position = dp_rule.offenses;
    let line_end_concatenation = lec_rule.offenses;
    let block_length = bl_rule.out;
    let complexity = cx_rule.out;
    let block_nesting = (bn_rule.out, bn_rule.deepest);
    let predicate_prefix = pp_rule.offenses;
    let closing_parenthesis_indentation = cpi_rule.offenses;
    let first_array_element_indentation = fae_rule.map(|r| r.offenses).unwrap_or_default();
    let hash_each_methods = hem_rule.offenses;
    let void = void_rule.offenses;
    let useless_access_modifier = uam_rule.into_offenses();
    let empty_lines_around_body = elab_rule.into_offenses();
    let empty_lines_around_arguments = elaa_rule.offenses;
    // The bundle runs with no prior ignored ranges (autocorrect re-passes
    // take the standalone path on the Ruby side).
    let block_delimiters = block_delimiters::resolve(bd_rule.events, &[]);
    let abc_size = abc_rule.out;
    let indentation_consistency = ic_rule.offenses;
    let empty_line_between_defs = elbd_rule.offenses;
    let end_alignment = ea_rule.records;
    let block_alignment = ba_rule.offenses;
    let else_alignment = elsea_rule.offenses;
    let first_hash_element_indentation = fhe_rule.offenses;
    let hash_alignment = ha_rule.offenses;
    let hash_syntax = hs_rule.offenses;
    let string_literals = sl_rule.offenses;
    let string_literals_in_interpolation = sli_rule.offenses;
    let trailing_comma_in_arguments = tca_rule.offenses;
    let trailing_comma_in_hash_literal = tchl_rule.checker.offenses;
    let trailing_comma_in_array_literal = tcal_rule.checker.offenses;
    let space_around_method_call_operator = samco_rule.offenses;
    let space_around_keyword = sak_rule.offenses;
    let space_inside_block_braces = sibb_rule.offenses;
    let space_inside_hash_literal_braces = sihlb_rule.into_offenses();
    let space_inside_array_literal_brackets = sialb_rule.into_result();
    let space_before_block_braces = sbbb_rule.into_result();
    let punctuation_spacing = ps_rule.into_offenses();
    let space_inside_parens = sip_rule.into_offenses();
    let space_inside_reference_brackets = sirb_rule.result;
    let space_before_first_arg = sbfa_rule.into_offenses();
    let method_length = ml_rule.out;
    let class_length = cl_rule.out;
    let module_length = mol_rule.out;
    let def_end_alignment = dea_rule.records;
    let require_parentheses = rp_rule.offenses;
    let self_assignment = sa_rule.offenses;
    let nested_parenthesized_calls = npc_rule.offenses;
    let parentheses_as_grouped_expression = pag_rule.offenses;
    let percent_literal_delimiters = pld_rule.offenses;
    let multiline_method_call_brace_layout = mmcbl_rule.offenses;
    let access_modifier_indentation = ami_rule.records;
    let assignment_indentation = ai_rule.offenses;
    let redundant_self_assignment = rsa_rule.offenses;
    let colon_method_call = cmc_rule.offenses;
    let stabby_lambda_parentheses = slp_rule.offenses;
    let unreachable_code = uc_rule.offenses;
    let hash_transform_keys = htk_rule.offenses;
    let ambiguous_block_association = aba_rule.offenses;
    // `Layout/EmptyLineAfterGuardClause` walks the AST on its own (separate
    // `dispatch::run`); joining the shared walk is future work.
    let empty_line_after_guard_clause =
        empty_line_after_guard_clause::check_empty_line_after_guard_clause(source);
    // `Layout/EmptyComment` is a comment-only check (no AST walk); it pulls
    // comment ranges from the shared parse cache.
    let empty_comment = empty_comment::check_empty_comment(source, cfg.empty_comment);
    // `Layout/EmptyLineAfterMagicComment` is also a comment-only check (no
    // AST walk); it pulls comments and the program first-statement line from
    // the shared parse cache.
    let empty_line_after_magic_comment =
        empty_line_after_magic_comment::check_empty_line_after_magic_comment(source);
    // `Layout/LeadingEmptyLines` is a comment + first AST statement lookup
    // from the cached parse, no AST walk.
    let leading_empty_lines =
        leading_empty_lines::check_leading_empty_lines(source);
    // `Lint/DuplicateMagicComment` is a leading-line scan (comments + the
    // first non-comment token position from the cached parse), no AST walk.
    let duplicate_magic_comment =
        duplicate_magic_comment::check_duplicate_magic_comment(source);

    // --- Cops off the shared walk (see the doc comment above). ---
    // The bundle always computes the filtered flavor; a `MethodName` whose
    // config needs the unfiltered one takes the fallback path on the Ruby side.
    let method_name = method_name::check_method_name_filtered(source, cfg.method_name_style, true);
    let line_length = line_length::check_line_length_with_heredocs(
        source,
        cfg.line_length_max,
        cfg.line_length_tab_width,
        &heredoc_rule.ranges,
    );
    let candidate_lines: HashSet<usize> = line_length.iter().map(|c| c.line_index).collect();
    let line_length_breakables = line_length_breakable::compute_breakables_filtered(
        source,
        cfg.line_length_max,
        cfg.line_length_split_strings,
        Some(&candidate_lines),
    );
    // `Layout/TrailingEmptyLines` is also a pure source scan (no walk).
    let trailing_empty_lines =
        trailing_empty_lines::check_trailing_empty_lines(source, &cfg.trailing_empty_lines);
    // `Layout/EmptyLines` finalizes the shared walk's collected token-bearing
    // lines (or stays empty when the prefilter skipped the walk).
    let empty_lines = if empty_lines_eligible {
        empty_lines::finalize(source, el_rule)
    } else {
        Vec::new()
    };
    BundleResult {
        debugger,
        block_length,
        block_nesting,
        complexity,
        variable_number,
        method_name,
        safe_navigation_chain,
        multiline_operation,
        multiline_method_call,
        dot_position,
        line_length,
        line_length_breakables,
        line_end_concatenation,
        argument_alignment,
        array_alignment,
        first_argument_indentation,
        redundant_self,
        indentation_width,
        predicate_prefix,
        closing_parenthesis_indentation,
        first_array_element_indentation,
        hash_each_methods,
        void,
        useless_access_modifier,
        empty_lines_around_body,
        empty_lines_around_arguments,
        block_delimiters,
        abc_size,
        indentation_consistency,
        empty_line_between_defs,
        end_alignment,
        block_alignment,
        else_alignment,
        first_hash_element_indentation,
        hash_alignment,
        hash_syntax,
        string_literals,
        string_literals_in_interpolation,
        trailing_comma_in_arguments,
        trailing_comma_in_hash_literal,
        trailing_comma_in_array_literal,
        trailing_empty_lines,
        space_around_method_call_operator,
        space_around_keyword,
        space_inside_block_braces,
        method_length,
        class_length,
        module_length,
        def_end_alignment,
        require_parentheses,
        self_assignment,
        nested_parenthesized_calls,
        parentheses_as_grouped_expression,
        percent_literal_delimiters,
        multiline_method_call_brace_layout,
        access_modifier_indentation,
        assignment_indentation,
        redundant_self_assignment,
        colon_method_call,
        stabby_lambda_parentheses,
        unreachable_code,
        hash_transform_keys,
        ambiguous_block_association,
        empty_line_after_guard_clause,
        if_unless_modifier: ium_rule.candidates,
        empty_comment,
        empty_line_after_magic_comment,
        empty_lines,
        leading_empty_lines,
        space_inside_hash_literal_braces,
        space_inside_array_literal_brackets,
        space_before_block_braces,
        punctuation_spacing,
        space_inside_parens,
        space_inside_reference_brackets,
        space_before_first_arg,
        duplicate_magic_comment,
        duplicate_methods: dm_rule.events,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_matches_standalone() {
        // A file that triggers both cops at once.
        let src = "if a +\n    b\n  something\nend\nFoo.a\n     .b\n      .c\n";
        let (op_off, mc_off) = check_multiline_bundle(src.as_bytes(), 0, 2, 2, 0, 2, 2);
        let op_alone = op::check_multiline_operation_indentation(src.as_bytes(), 0, 2, 2);
        let mc_alone = mc::check_multiline_method_call_indentation(src.as_bytes(), 0, 2, 2);
        assert_eq!(op_off.len(), op_alone.len());
        assert_eq!(mc_off.len(), mc_alone.len());
        assert!(!op_off.is_empty());
        assert!(!mc_off.is_empty());
        for (a, b) in op_off.iter().zip(&op_alone) {
            assert_eq!(
                (a.start_offset, a.column_delta),
                (b.start_offset, b.column_delta)
            );
        }
        for (a, b) in mc_off.iter().zip(&mc_alone) {
            assert_eq!(
                (a.start_offset, a.column_delta),
                (b.start_offset, b.column_delta)
            );
        }
    }

    /// A packed config with RuboCop-default-ish values, mirroring what the
    /// Ruby side sends for an all-defaults run.
    fn default_packed() -> (Vec<i64>, Vec<Vec<String>>) {
        let nums = vec![
            25, 0, 1, // block_length: max / count_comments / filtered
            3, 0, 0, // block_nesting: max / count_blocks / count_modifier_forms
            7, 8, // complexity: max_cyclomatic / max_perceived
            1, 3, // variable_number: style(normalcase) / flags
            0, // method_name style
            0, // dot_position style
            120, 2, 0, // line_length: max / tab_width / split_strings
            0, 2, 2, // multiline_operation
            0, 2, 2, // multiline_method_call
            0, 2, 0, // argument_alignment
            0, 2, 0, // first_argument_indentation
            2, 0, 0, 0, 0, 0, 0, // indentation_width
            2, // closing_paren_indent
            0, 2, 0, // first_array_element_indentation
            0, // void_check_nonmutating
            0, // useless_access_modifier_active_support
            0, 0, 0, // empty_lines_around_body class / module / block styles
            0, 0, // block_delimiters style / oneliners
            17, 0, // abc_size: max_floor (default Max 17) / discount_repeated
            0,     // indentation_consistency: indented_internal_methods
            1, 1, 1, // empty_line_between_defs: method / class / module defs
            1, // empty_line_between_defs: allow_adjacent_one_line_defs (default true)
            1, 1, // empty_line_between_defs: min / max empty lines
            0, // end_alignment: style (keyword)
            0, // block_alignment: style (either)
            0, // else_alignment: style (keyword)
            0, 2, 0, 0, 0, // first_hash_element: style / indent / enforce / colon_sep / rocket_sep
            0, 0, // hash_alignment: last_argument_style (always_inspect) / enforce_fixed
            0, 2, 0, 0, 1, 1, // hash_syntax: style(ruby19) / shorthand(either) / urswsv / prfnaes / ruby31 / ruby22
            0, 0, // string_literals: style(single_quotes) / consistent_multiline
            0, // trailing_comma_in_arguments: style (no_comma)
            0, // string_literals_in_interpolation: style (single_quotes)
            0, // trailing_empty_lines: style (final_newline)
            0, 1, 1, // space_inside_block_braces: style(space) / empty(no_space) / sbbp(true)
            10, 0, // method_length: max(10) / count_comments
            0, // def_end_alignment: style (start_of_line)
            0, // multiline_method_call_brace_layout: style (symmetrical)
            0, 2, // access_modifier_indentation: style (indent) / indentation_width (2)
            2, // assignment_indentation: indentation_width (default 2)
            0, // stabby_lambda_parentheses: style (require_parentheses)
            1, 1, // empty_comment: allow_border / allow_margin (defaults)
            100, 0, // class_length: max(100) / count_comments
            100, 0, // module_length: max(100) / count_comments
            0, // trailing_comma_in_hash_literal: style (no_comma)
            0, // trailing_comma_in_array_literal: style (no_comma)
            0, 1, // space_inside_hash_literal_braces: style(space) / empty(no_space)
            0, 0, // space_inside_array_literal_brackets: style(no_space) / empty(no_space)
            120, 2, // if_unless_modifier: max_line_length / tab_width (defaults)
            0, 0, 1, // space_before_block_braces: style(space) / empty(space) / bd(line_count_based)
            1, 0, 1, 0, // punctuation_spacing: bc lcurly_space / ac rcurly_no_space / bs lcurly_space / as rcurly_no_space
            0, // space_inside_parens: style (no_space)
            0, 0, // space_inside_reference_brackets: style(no_space) / empty(no_space)
            1, // space_before_first_arg: allow_for_alignment
            0, // duplicate_methods: active_support_extensions_enabled
            0, 2, // array_alignment: style(with_first_element) / indent
        ];
        let lists = vec![
            vec!["binding.pry".to_string(), "debugger".to_string()],
            vec![],
            vec![],
            vec![],
            vec![],
            vec!["to_s".to_string()],
            vec!["puts".to_string()],
            ["is_", "has_", "have_", "does_"].map(String::from).to_vec(),
            ["define_method", "define_singleton_method"]
                .map(String::from)
                .to_vec(),
            vec!["Thread.current".to_string()],
            vec![], // useless_access_modifier_context_creating
            vec![], // useless_access_modifier_method_creating
            // block_delimiters: procedural / functional / allowed /
            // braces_required (RuboCop defaults).
            vec![],
            vec![],
            ["lambda", "proc", "it"].map(String::from).to_vec(),
            vec![],
            vec![], // empty_line_between_defs: def_like_macros
            vec!["key".to_string()], // hash_alignment: rocket styles
            vec!["key".to_string()], // hash_alignment: colon styles
            vec![],                  // method_length: count_as_one
            ["be", "be_a", "be_an", "be_between", "be_falsey", "be_kind_of", "be_instance_of",
             "be_truthy", "be_within", "eq", "eql", "end_with", "include", "match",
             "raise_error", "respond_to", "start_with"].map(String::from).to_vec(),
            // percent_literal_delimiters: 10 PreferredDelimiters entries in
            // `[%, %i, %I, %q, %Q, %r, %s, %w, %W, %x]` order. RuboCop default
            // is `default: ()` with `%i/%I/%w/%W => []` and `%r => {}`.
            vec![
                "()".to_string(),
                "[]".to_string(),
                "[]".to_string(),
                "()".to_string(),
                "()".to_string(),
                "{}".to_string(),
                "()".to_string(),
                "[]".to_string(),
                "[]".to_string(),
                "()".to_string(),
            ],
            // ambiguous_block_association: AllowedMethods (default empty).
            vec![],
            vec![], // class_length: count_as_one
            vec![], // module_length: count_as_one
        ];
        (nums, lists)
    }

    #[test]
    fn from_packed_assigns_fields() {
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        assert_eq!(cfg.block_length_max, 25);
        assert!(!cfg.block_length_count_comments);
        assert!(cfg.block_length_filtered);
        assert_eq!(cfg.block_nesting_max, 3);
        assert_eq!(cfg.max_cyclomatic, 7);
        assert_eq!(cfg.max_perceived, 8);
        assert_eq!(cfg.variable_number_style, 1);
        assert_eq!(cfg.variable_number_flags, 3);
        assert_eq!(cfg.line_length_max, 120);
        assert_eq!(cfg.multiline_operation, (0, 2, 2));
        assert_eq!(cfg.multiline_method_call, (0, 2, 2));
        assert_eq!(cfg.indentation_width.width, 2);
        assert_eq!(cfg.indentation_width.end_align, 0);
        assert_eq!(cfg.closing_paren_indent, 2);
        assert_eq!(cfg.first_array_element_style, 0);
        assert_eq!(cfg.first_array_element_indent, 2);
        assert!(!cfg.first_array_element_enforce_fixed);
        assert_eq!(cfg.debugger_methods, vec!["binding.pry", "debugger"]);
        assert_eq!(cfg.safe_navigation_nil_methods, vec!["to_s"]);
        assert_eq!(cfg.redundant_self_kernel_methods, vec!["puts"]);
        assert_eq!(
            cfg.predicate_prefix_name_prefixes,
            vec!["is_", "has_", "have_", "does_"]
        );
        assert_eq!(
            cfg.predicate_prefix_macros,
            vec!["define_method", "define_singleton_method"]
        );
        assert_eq!(cfg.hash_each_allowed_receivers, vec!["Thread.current"]);
    }

    #[test]
    fn from_packed_rejects_wrong_lengths() {
        let (nums, lists) = default_packed();
        assert!(BundleConfig::from_packed(&nums[..NUMS_LEN - 1], lists).is_err());
        let (nums, lists) = default_packed();
        assert!(BundleConfig::from_packed(&nums, lists[..LISTS_LEN - 1].to_vec()).is_err());
    }

    /// The six dispatch-family cops merged into the shared walk must report
    /// exactly what their standalone entry points report, over a source that
    /// triggers each of them (multiline operation, multiline method chain,
    /// misaligned arguments, misindented first argument, safe-navigation chain
    /// and body misindentation).
    #[test]
    fn shared_walk_matches_standalone_dispatch_family() {
        let src = "def f(x)\n\
                   \x20     y = x&.a.b\n\
                   \x20 foo(bar,\n\
                   \x20       baz)\n\
                   \x20 z = a +\n\
                   \x20       b\n\
                   \x20 qux(\n\
                   \x20         arg)\n\
                   end\n\
                   Foo.a\n\
                   \x20    .b\n\
                   \x20     .c\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let op_alone = op::check_multiline_operation_indentation(src.as_bytes(), 0, 2, 2);
        assert!(!op_alone.is_empty());
        assert_eq!(bundle.multiline_operation.len(), op_alone.len());
        for (a, b) in bundle.multiline_operation.iter().zip(&op_alone) {
            assert_eq!(
                (a.start_offset, a.column_delta),
                (b.start_offset, b.column_delta)
            );
        }

        let mc_alone = mc::check_multiline_method_call_indentation(src.as_bytes(), 0, 2, 2);
        assert!(!mc_alone.is_empty());
        assert_eq!(bundle.multiline_method_call.len(), mc_alone.len());
        for (a, b) in bundle.multiline_method_call.iter().zip(&mc_alone) {
            assert_eq!(
                (a.start_offset, a.column_delta),
                (b.start_offset, b.column_delta)
            );
        }

        let aa_alone =
            super::argument_alignment::check_argument_alignment(src.as_bytes(), 0, 2, false);
        assert!(!aa_alone.is_empty());
        assert_eq!(bundle.argument_alignment.len(), aa_alone.len());
        for (a, b) in bundle.argument_alignment.iter().zip(&aa_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.column_delta, a.autocorrect),
                (b.start_offset, b.end_offset, b.column_delta, b.autocorrect)
            );
        }

        let fa_alone = super::first_argument_indentation::check_first_argument_indentation(
            src.as_bytes(),
            0,
            2,
            false,
        );
        assert!(!fa_alone.is_empty());
        assert_eq!(bundle.first_argument_indentation.len(), fa_alone.len());
        for (a, b) in bundle.first_argument_indentation.iter().zip(&fa_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.column_delta, &a.message),
                (b.start_offset, b.end_offset, b.column_delta, &b.message)
            );
        }

        let snc_alone = super::safe_navigation_chain::check_safe_navigation_chain(
            src.as_bytes(),
            &cfg.safe_navigation_nil_methods,
        );
        assert!(!snc_alone.is_empty());
        assert_eq!(bundle.safe_navigation_chain.len(), snc_alone.len());
        for (a, b) in bundle.safe_navigation_chain.iter().zip(&snc_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, &a.replacement),
                (b.start_offset, b.end_offset, &b.replacement)
            );
        }

        let iw_alone = super::indentation_width::check_indentation_width(
            src.as_bytes(),
            cfg.indentation_width,
            &[],
            &[],
        );
        assert!(!iw_alone.is_empty());
        assert_eq!(bundle.indentation_width.len(), iw_alone.len());
        for (a, b) in bundle.indentation_width.iter().zip(&iw_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.column_delta, &a.message),
                (b.start_offset, b.end_offset, b.column_delta, &b.message)
            );
        }
    }

    /// The punctuation-spacing family merged into the shared walk must report
    /// exactly what the standalone entry point reports, over a source that
    /// triggers all six cops (space before/after comma and semicolon, tight
    /// colons, adjacent comment) plus masked look-alikes (string commas,
    /// block-brace semicolon skip).
    #[test]
    fn shared_walk_matches_standalone_punctuation_spacing() {
        let src = "f(a ,b)\n\
                   x = 1 ;y = 2\n\
                   h = {a:1, b: \"x,y\"}\n\
                   loop { ; h }\n\
                   z = 1# c\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);
        let alone = super::punctuation_spacing::check_punctuation_spacing(
            src.as_bytes(),
            cfg.punctuation_spacing,
        );
        assert!(!alone.space_before_comma.is_empty());
        assert!(!alone.space_after_comma.is_empty());
        assert!(!alone.space_before_semicolon.is_empty());
        assert!(!alone.space_after_semicolon.is_empty());
        assert!(!alone.space_after_colon.is_empty());
        assert!(!alone.space_before_comment.is_empty());
        let b = &bundle.punctuation_spacing;
        assert_eq!(b.space_before_comma, alone.space_before_comma);
        assert_eq!(b.space_after_comma, alone.space_after_comma);
        assert_eq!(b.space_before_semicolon, alone.space_before_semicolon);
        assert_eq!(b.space_after_semicolon, alone.space_after_semicolon);
        assert_eq!(b.space_after_colon, alone.space_after_colon);
        assert_eq!(b.space_before_comment, alone.space_before_comment);
    }

    /// The Cluster B space cops merged into the shared walk must report
    /// exactly what their standalone entry points report, over a source that
    /// triggers all three (spaced parens, spaced reference brackets, a
    /// two-space first argument).
    #[test]
    fn shared_walk_matches_standalone_cluster_b_space_cops() {
        let src = "f( 3 )\nh[ :k ]\nsomething  x\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let sip_alone = super::space_inside_parens::check_space_inside_parens(
            src.as_bytes(),
            cfg.space_inside_parens,
        );
        assert!(!sip_alone.is_empty());
        assert_eq!(bundle.space_inside_parens.len(), sip_alone.len());
        for (a, b) in bundle.space_inside_parens.iter().zip(&sip_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.message.code()),
                (b.start_offset, b.end_offset, b.message.code())
            );
        }

        let sirb_alone =
            super::space_inside_reference_brackets::check_space_inside_reference_brackets(
                src.as_bytes(),
                cfg.space_inside_reference_brackets,
            );
        assert!(!sirb_alone.offenses.is_empty());
        assert_eq!(
            bundle.space_inside_reference_brackets.offenses.len(),
            sirb_alone.offenses.len()
        );
        for (a, b) in bundle
            .space_inside_reference_brackets
            .offenses
            .iter()
            .zip(&sirb_alone.offenses)
        {
            assert_eq!(
                (a.start_offset, a.end_offset, a.message.code(), a.node),
                (b.start_offset, b.end_offset, b.message.code(), b.node)
            );
        }
        assert_eq!(
            bundle.space_inside_reference_brackets.node_ops,
            sirb_alone.node_ops
        );

        let sbfa_alone = super::space_before_first_arg::check_space_before_first_arg(
            src.as_bytes(),
            cfg.space_before_first_arg,
        );
        assert!(!sbfa_alone.is_empty());
        assert_eq!(bundle.space_before_first_arg, sbfa_alone);
    }

    /// The ancestor-stack cops merged into the shared walk (`Lint/Debugger`,
    /// `Style/RedundantSelf`, `Naming/VariableNumber`) must report exactly what
    /// their standalone entry points report, over a source exercising their
    /// context-sensitive paths: debugger calls as arguments vs. statements,
    /// `self.` with parameter/local/condition shadowing, and identifiers in
    /// branch (writes) and leaf (symbols, params) positions.
    #[test]
    fn shared_walk_matches_standalone_stack_family() {
        let src = "def f(allowed1, other)\n\
                   \x20 self.allowed1\n\
                   \x20 self.flagged\n\
                   \x20 local_1 = self.other\n\
                   \x20 binding.pry\n\
                   \x20 take(binding.pry)\n\
                   \x20 list.each { custom_debugger }\n\
                   \x20 if (self.cond_var)\n\
                   \x20   cond_var = 1\n\
                   \x20 end\n\
                   \x20 :sym1\n\
                   end\n\
                   x.y = custom_debugger\n\
                   bad1 = 1\n\
                   @ivar2 = 2\n";
        let (nums, lists) = default_packed();
        let mut cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        cfg.debugger_methods.push("custom_debugger".to_string());
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let debugger_alone = super::debugger::check_debugger(
            src.as_bytes(),
            &cfg.debugger_methods,
            &cfg.debugger_requires,
        );
        assert!(!debugger_alone.is_empty());
        assert_eq!(bundle.debugger.len(), debugger_alone.len());
        for (a, b) in bundle.debugger.iter().zip(&debugger_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset),
                (b.start_offset, b.end_offset)
            );
        }

        let rs_alone = super::redundant_self::check_redundant_self(
            src.as_bytes(),
            &cfg.redundant_self_kernel_methods,
        );
        assert!(!rs_alone.is_empty());
        assert_eq!(bundle.redundant_self.len(), rs_alone.len());
        for (a, b) in bundle.redundant_self.iter().zip(&rs_alone) {
            assert_eq!(
                (a.self_start, a.self_end, a.dot_start, a.dot_end),
                (b.self_start, b.self_end, b.dot_start, b.dot_end)
            );
        }

        let (vn_alone, vn_had_correct) = super::variable_number::check_variable_number(
            src.as_bytes(),
            cfg.variable_number_style,
            cfg.variable_number_flags,
            &cfg.variable_number_allowed_identifiers,
        );
        assert!(!vn_alone.is_empty());
        assert_eq!(bundle.variable_number.0.len(), vn_alone.len());
        assert_eq!(bundle.variable_number.1, vn_had_correct);
        for (a, b) in bundle.variable_number.0.iter().zip(&vn_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.identifier_type, &a.name),
                (b.start_offset, b.end_offset, b.identifier_type, &b.name)
            );
        }
    }

    /// The typed-visitor cops merged into the shared walk (`Layout/DotPosition`,
    /// `Style/LineEndConcatenation`, `Metrics/BlockLength`, the complexity
    /// pair) must report exactly what their standalone entry points report,
    /// over a source exercising each: a trailing-dot multiline chain, a
    /// line-end string concatenation, an over-long block and lambda, and a
    /// branchy method (`def` + `define_method`).
    #[test]
    fn shared_walk_matches_standalone_typed_family() {
        let src = "foo = bar.\n\
                   \x20 baz\n\
                   msg = 'a' +\n\
                   \x20 'b'\n\
                   big.each do |x|\n\
                   \x20 a1\n\
                   \x20 a2\n\
                   \x20 a3\n\
                   end\n\
                   small = -> { tiny }\n\
                   def branchy(x)\n\
                   \x20 if x then a elsif y then b end\n\
                   \x20 x ? c : d\n\
                   end\n\
                   define_method :dyn do\n\
                   \x20 z && w || v\n\
                   end\n\
                   /(?<m>a)/ =~ 'a'\n";
        let (mut nums, lists) = default_packed();
        nums[0] = 2; // block_length max
        nums[6] = 1; // max_cyclomatic
        nums[7] = 1; // max_perceived
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let dp_alone = super::dot_position::check_dot_position(src.as_bytes(), 0);
        assert!(!dp_alone.is_empty());
        assert_eq!(bundle.dot_position.len(), dp_alone.len());
        for (a, b) in bundle.dot_position.iter().zip(&dp_alone) {
            assert_eq!(
                (
                    a.start_offset,
                    a.end_offset,
                    a.remove_start,
                    a.remove_end,
                    a.insert_pos
                ),
                (
                    b.start_offset,
                    b.end_offset,
                    b.remove_start,
                    b.remove_end,
                    b.insert_pos
                )
            );
        }

        let lec_alone = super::line_end_concatenation::check_line_end_concatenation(src.as_bytes());
        assert!(!lec_alone.is_empty());
        assert_eq!(bundle.line_end_concatenation.len(), lec_alone.len());
        for (a, b) in bundle.line_end_concatenation.iter().zip(&lec_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, &a.operator),
                (b.start_offset, b.end_offset, &b.operator)
            );
        }

        let bl_alone = super::block_length::check_block_length_filtered(
            src.as_bytes(),
            cfg.block_length_max,
            cfg.block_length_count_comments,
            &cfg.block_length_count_as_one,
            &cfg.block_length_allowed_methods,
            cfg.block_length_filtered,
        );
        assert!(!bl_alone.is_empty());
        assert_eq!(bundle.block_length.len(), bl_alone.len());
        for (a, b) in bundle.block_length.iter().zip(&bl_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.length, &a.method_name),
                (b.start_offset, b.end_offset, b.length, &b.method_name)
            );
        }

        let cx_alone = super::complexity::check_complexity_exceeding(
            src.as_bytes(),
            cfg.max_cyclomatic,
            cfg.max_perceived,
        );
        // Both `branchy` and the `define_method` block must exceed max 1.
        assert!(cx_alone.len() >= 2);
        assert_eq!(bundle.complexity.len(), cx_alone.len());
        for (a, b) in bundle.complexity.iter().zip(&cx_alone) {
            assert_eq!(
                (
                    a.start_offset,
                    a.end_offset,
                    a.cyclomatic,
                    a.perceived,
                    &a.method_name
                ),
                (
                    b.start_offset,
                    b.end_offset,
                    b.cyclomatic,
                    b.perceived,
                    &b.method_name
                )
            );
        }
    }

    /// `Metrics/BlockNesting` merged into the shared walk must report exactly
    /// what its standalone entry reports, over a source exercising the rescue
    /// hooks (chained `rescue` clauses are siblings, each a counted level) and
    /// plain nesting (if/while/case with an over-deep chain).
    #[test]
    fn shared_walk_matches_standalone_block_nesting() {
        let src = "def f\n\
                   \x20 begin\n\
                   \x20   x\n\
                   \x20 rescue A\n\
                   \x20   if a\n\
                   \x20     while b\n\
                   \x20       case c\n\
                   \x20       when 1 then d\n\
                   \x20       end\n\
                   \x20     end\n\
                   \x20   end\n\
                   \x20 rescue B\n\
                   \x20   y\n\
                   \x20 end\n\
                   end\n";
        let (mut nums, lists) = default_packed();
        nums[3] = 2; // block_nesting max
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let (bn_alone, deepest_alone) = super::block_nesting::check_block_nesting(
            src.as_bytes(),
            cfg.block_nesting_max,
            cfg.block_nesting_count_blocks,
            cfg.block_nesting_count_modifier_forms,
        );
        assert!(!bn_alone.is_empty());
        assert_eq!(bundle.block_nesting.0.len(), bn_alone.len());
        assert_eq!(bundle.block_nesting.1, deepest_alone);
        for (a, b) in bundle.block_nesting.0.iter().zip(&bn_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset),
                (b.start_offset, b.end_offset)
            );
        }
    }

    /// `Layout/LineLength` with its heredoc collection on the shared walk must
    /// report exactly what the standalone entry reports, including the heredoc
    /// delimiters of over-long lines inside plain, interpolated and stacked
    /// heredoc bodies.
    #[test]
    fn shared_walk_matches_standalone_line_length_heredocs() {
        let long = "x".repeat(130);
        let src = format!(
            "a = <<~SQL\n\
             \x20 {long}\n\
             SQL\n\
             b = <<~MSG\n\
             \x20 #{{q}} {long}\n\
             MSG\n\
             c = [<<~ONE, <<~TWO]\n\
             \x20 {long}\n\
             ONE\n\
             \x20 plain\n\
             TWO\n\
             d = '{long}'\n"
        );
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let ll_alone = super::line_length::check_line_length(
            src.as_bytes(),
            cfg.line_length_max,
            cfg.line_length_tab_width,
        );
        // The three heredoc lines and the plain string line are all over Max.
        assert!(ll_alone.len() >= 4);
        assert!(ll_alone.iter().any(|c| !c.heredoc_delimiters.is_empty()));
        assert_eq!(bundle.line_length.len(), ll_alone.len());
        for (a, b) in bundle.line_length.iter().zip(&ll_alone) {
            assert_eq!(
                (
                    a.line_index,
                    a.length,
                    a.line_start,
                    a.line_end,
                    a.indentation_difference,
                    &a.heredoc_delimiters
                ),
                (
                    b.line_index,
                    b.length,
                    b.line_start,
                    b.line_end,
                    b.indentation_difference,
                    &b.heredoc_delimiters
                )
            );
        }
    }

    /// `Naming/PredicatePrefix` merged into the shared walk must report exactly
    /// what its standalone entry point reports, over a source exercising the
    /// sibling-sensitive Sorbet-sig pairing (sig'd and unsig'd defs), a nested
    /// class body and a `MethodDefinitionMacros` call.
    #[test]
    fn shared_walk_matches_standalone_predicate_prefix() {
        let src = "sig { returns(T::Boolean) }\n\
                   def is_attr; end\n\
                   def has_attr; end\n\
                   class Foo\n\
                   \x20 sig { returns(String) }\n\
                   \x20 def have_name; end\n\
                   end\n\
                   define_method(:does_work) do |x|\n\
                   end\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let pp_alone = super::predicate_prefix::check_predicate_prefix(
            src.as_bytes(),
            &cfg.predicate_prefix_name_prefixes,
            &cfg.predicate_prefix_macros,
        );
        assert_eq!(pp_alone.len(), 4);
        assert!(pp_alone.iter().any(|c| c.sorbet_boolean_sig));
        assert!(pp_alone.iter().any(|c| !c.is_def));
        assert_eq!(bundle.predicate_prefix.len(), pp_alone.len());
        for (a, b) in bundle.predicate_prefix.iter().zip(&pp_alone) {
            assert_eq!(
                (
                    a.start_offset,
                    a.end_offset,
                    &a.name,
                    a.is_def,
                    a.sorbet_boolean_sig
                ),
                (
                    b.start_offset,
                    b.end_offset,
                    &b.name,
                    b.is_def,
                    b.sorbet_boolean_sig
                )
            );
        }
    }

    /// `Layout/ClosingParenthesisIndentation` merged into the shared walk must
    /// report exactly what its standalone entry point reports, over a source
    /// exercising all three node families (method call, def parameters,
    /// grouped expression) and both message flavors.
    #[test]
    fn shared_walk_matches_standalone_closing_parenthesis_indentation() {
        let src = "some_method(\n\
                   \x20 a\n\
                   \x20 )\n\
                   foo = other_method(a\n\
                   )\n\
                   def f(b\n\
                   \x20 )\n\
                   end\n\
                   w = x * (\n\
                   \x20 y + z\n\
                   \x20   )\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let cpi_alone =
            super::closing_parenthesis_indentation::check_closing_parenthesis_indentation(
                src.as_bytes(),
                cfg.closing_paren_indent,
            );
        assert_eq!(cpi_alone.len(), 4);
        assert!(cpi_alone.iter().any(|o| o.message.starts_with("Align")));
        assert!(cpi_alone.iter().any(|o| o.message.starts_with("Indent")));
        assert_eq!(
            bundle.closing_parenthesis_indentation.len(),
            cpi_alone.len()
        );
        for (a, b) in bundle
            .closing_parenthesis_indentation
            .iter()
            .zip(&cpi_alone)
        {
            assert_eq!(
                (a.start_offset, a.end_offset, a.column_delta, &a.message),
                (b.start_offset, b.end_offset, b.column_delta, &b.message)
            );
        }
    }

    /// `Layout/ArrayAlignment` merged into the shared walk must report
    /// exactly what its standalone entry point reports, over a source
    /// exercising the parent-intercept paths: a bracketed literal, an
    /// implicit assignment array, a skipped masgn RHS, a rescue exception
    /// list and a nested array losing autocorrect via `within?`.
    #[test]
    fn shared_walk_matches_standalone_array_alignment() {
        let src = "array = [a,\n\
                   \x20  b,\n\
                   \x20 c]\n\
                   imp = 1,\n\
                   \x20 2\n\
                   m, n = 1,\n\
                   \x20       2\n\
                   begin\n\
                   \x20 x\n\
                   rescue FooError,\n\
                   \x20   BarError\n\
                   \x20 y\n\
                   end\n\
                   nested = [[1,\n\
                   \x20  2],\n\
                   \x20 [3,\n\
                   4]]\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let ara_alone = super::array_alignment::check_array_alignment(
            src.as_bytes(),
            cfg.array_alignment_style,
            cfg.array_alignment_indent,
        );
        assert!(ara_alone.len() >= 6);
        assert!(ara_alone.iter().any(|o| !o.autocorrect));
        assert_eq!(bundle.array_alignment.len(), ara_alone.len());
        for (a, b) in bundle.array_alignment.iter().zip(&ara_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.column_delta, a.autocorrect),
                (b.start_offset, b.end_offset, b.column_delta, b.autocorrect)
            );
        }
    }

    /// `Layout/FirstArrayElementIndentation` merged into the shared walk must
    /// report exactly what its standalone entry point reports, over a source
    /// exercising the ancestor-sensitive paths: a paren-claimed array, a
    /// parent-hash-key base, a start-of-line operand array and a hanging
    /// right bracket.
    #[test]
    fn shared_walk_matches_standalone_first_array_element_indentation() {
        let src = "func([\n\
                   \x20 1\n\
                   ])\n\
                   func(x: [\n\
                   \x20 :a,\n\
                   \x20      :b\n\
                   ],\n\
                   \x20    y: [\n\
                   \x20      :c\n\
                   \x20    ])\n\
                   a << [\n\
                   \x201\n\
                   \x20 ]\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let fae_alone =
            super::first_array_element_indentation::check_first_array_element_indentation(
                src.as_bytes(),
                cfg.first_array_element_style,
                cfg.first_array_element_indent,
                cfg.first_array_element_enforce_fixed,
            );
        assert!(fae_alone.len() >= 5);
        assert!(
            fae_alone
                .iter()
                .any(|o| o.message.contains("parent hash key"))
        );
        assert!(
            fae_alone
                .iter()
                .any(|o| o.message.contains("preceding left parenthesis"))
        );
        assert!(
            fae_alone
                .iter()
                .any(|o| o.message.starts_with("Indent the right bracket"))
        );
        assert_eq!(
            bundle.first_array_element_indentation.len(),
            fae_alone.len()
        );
        for (a, b) in bundle
            .first_array_element_indentation
            .iter()
            .zip(&fae_alone)
        {
            assert_eq!(
                (a.start_offset, a.end_offset, a.column_delta, &a.message),
                (b.start_offset, b.end_offset, b.column_delta, &b.message)
            );
        }
    }

    /// `Style/HashEachMethods` merged into the shared walk must report exactly
    /// what its standalone entry point reports, over a source exercising every
    /// branch: a `keys.each` block, a `values.each` block-pass, an unused
    /// key argument, an allowed receiver and a mutated hash.
    #[test]
    fn shared_walk_matches_standalone_hash_each_methods() {
        let src = "foo.keys.each { |k| p k }\n\
                   bar.values.each(&:baz)\n\
                   qux.each { |unused_key, v| p v }\n\
                   Thread.current.keys.each { |k| p k }\n\
                   mut.keys.each { |k| mut[k] = 1 }\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let hem_alone = super::hash_each_methods::check_hash_each_methods(
            src.as_bytes(),
            &cfg.hash_each_allowed_receivers,
        );
        assert_eq!(hem_alone.len(), 3);
        assert!(hem_alone.iter().any(|o| o.remove_end > o.remove_start));
        assert_eq!(bundle.hash_each_methods.len(), hem_alone.len());
        for (a, b) in bundle.hash_each_methods.iter().zip(&hem_alone) {
            assert_eq!(
                (
                    a.start_offset,
                    a.end_offset,
                    &a.message,
                    a.replace_start,
                    a.replace_end,
                    &a.replacement,
                    a.remove_start,
                    a.remove_end
                ),
                (
                    b.start_offset,
                    b.end_offset,
                    &b.message,
                    b.replace_start,
                    b.replace_end,
                    &b.replacement,
                    b.remove_start,
                    b.remove_end
                )
            );
        }
    }

    /// `Lint/Void` merged into the shared walk must report exactly what its
    /// standalone entry point reports, over a source exercising the
    /// context-sensitive paths: a void operator and literal in a sequence, an
    /// `initialize` body (void context), an `each` block (operator checks
    /// suppressed) and a conditional branch body (offense without correction).
    #[test]
    fn shared_walk_matches_standalone_void() {
        let src = "a == b\n\
                   42\n\
                   def initialize\n\
                   \x20 @x\n\
                   \x20 @x\n\
                   end\n\
                   arr.each do |x|\n\
                   \x20 x == 1\n\
                   \x20 7\n\
                   \x20 done\n\
                   end\n\
                   8 unless cond\n\
                   top\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let void_alone = super::void::check_void(src.as_bytes(), cfg.void_check_nonmutating);
        assert!(void_alone.len() >= 5);
        assert!(void_alone.iter().any(|o| o.message.contains("Operator")));
        assert!(void_alone.iter().any(|o| o.message.contains("Variable")));
        assert!(
            void_alone
                .iter()
                .any(|o| o.remove_end == 0 && o.replace_end == 0)
        );
        assert_eq!(bundle.void.len(), void_alone.len());
        for (a, b) in bundle.void.iter().zip(&void_alone) {
            assert_eq!(
                (
                    a.start_offset,
                    a.end_offset,
                    &a.message,
                    a.replace_start,
                    a.replace_end,
                    &a.replacement,
                    a.remove_start,
                    a.remove_end
                ),
                (
                    b.start_offset,
                    b.end_offset,
                    &b.message,
                    b.replace_start,
                    b.replace_end,
                    &b.replacement,
                    b.remove_start,
                    b.remove_end
                )
            );
        }
    }

    /// `Lint/UselessAccessModifier` merged into the shared walk must report
    /// exactly what its standalone entry point reports, over a source
    /// exercising its context-sensitive paths: a class scope with a useless
    /// trailing modifier, a repeated modifier, a singleton-class scope inside
    /// a def (handler-only frame), a `class_eval` block and a top-level
    /// modifier.
    #[test]
    fn shared_walk_matches_standalone_useless_access_modifier() {
        let src = "class A\n\
                   \x20 def m1\n\
                   \x20 end\n\
                   \x20 private\n\
                   \x20 private\n\
                   \x20 def m2\n\
                   \x20 end\n\
                   \x20 protected\n\
                   end\n\
                   def outer\n\
                   \x20 class << self\n\
                   \x20   private\n\
                   \x20 end\n\
                   end\n\
                   B.class_eval do\n\
                   \x20 public\n\
                   \x20 def m3\n\
                   \x20 end\n\
                   end\n\
                   module_function\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::useless_access_modifier::check_useless_access_modifier(
            src.as_bytes(),
            &cfg.useless_access_modifier_context_creating,
            &cfg.useless_access_modifier_method_creating,
            cfg.useless_access_modifier_active_support,
        );
        assert!(alone.len() >= 5);
        assert!(alone.iter().any(|o| o.name == "module_function"));
        assert!(alone.iter().any(|o| o.name == "public"));
        assert_eq!(bundle.useless_access_modifier.len(), alone.len());
        for (a, b) in bundle.useless_access_modifier.iter().zip(&alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, &a.name),
                (b.start_offset, b.end_offset, &b.name)
            );
        }
    }

    /// The `EmptyLinesAroundBody` family merged into the shared walk must
    /// report exactly what the standalone entry point reports, over a source
    /// exercising every member: a method body blank, a class body blank, a
    /// module body blank, a block body blank, begin-body blanks and a blank
    /// before a `rescue` keyword.
    #[test]
    fn shared_walk_matches_standalone_empty_lines_family() {
        let src = "def m\n\
                   \x20 x\n\
                   \n\
                   end\n\
                   class C\n\
                   \n\
                   \x20 y\n\
                   end\n\
                   module M\n\
                   \n\
                   \x20 z\n\
                   end\n\
                   foo do\n\
                   \n\
                   \x20 w\n\
                   end\n\
                   begin\n\
                   \n\
                   \x20 v\n\
                   \n\
                   rescue\n\
                   \x20 u\n\
                   end\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::empty_lines_around_body::check_empty_lines_around_body(
            src.as_bytes(),
            cfg.empty_lines_around_body,
        );
        let pairs = [
            (
                &bundle.empty_lines_around_body.method_body,
                &alone.method_body,
            ),
            (
                &bundle.empty_lines_around_body.class_body,
                &alone.class_body,
            ),
            (
                &bundle.empty_lines_around_body.module_body,
                &alone.module_body,
            ),
            (
                &bundle.empty_lines_around_body.block_body,
                &alone.block_body,
            ),
            (
                &bundle.empty_lines_around_body.begin_body,
                &alone.begin_body,
            ),
            (
                &bundle.empty_lines_around_body.exception_keywords,
                &alone.exception_keywords,
            ),
        ];
        for (got, want) in pairs {
            assert!(!want.is_empty());
            assert_eq!(got.len(), want.len());
            for (a, b) in got.iter().zip(want.iter()) {
                assert_eq!(
                    (a.start_offset, a.end_offset, a.insert, &a.message),
                    (b.start_offset, b.end_offset, b.insert, &b.message)
                );
            }
        }
    }

    /// `Style/BlockDelimiters` merged into the shared walk must report exactly
    /// what its standalone entry point reports, over a source exercising the
    /// in-order ignore machinery: a multi-line brace block inside
    /// unparenthesized arguments (send-ignored), a single-line `do`..`end`
    /// offense, a multi-line brace offense with a nested suppressed block
    /// (conditional), and an allowed method.
    #[test]
    fn shared_walk_matches_standalone_block_delimiters() {
        let src = "puts [1, 2, 3].map { |n|\n\
                   \x20 n * n\n\
                   }, 1\n\
                   each do |x| end\n\
                   foo {\n\
                   \x20 bar do |y| y end\n\
                   }\n\
                   foo = proc do\n\
                   \x20 puts 42\n\
                   end\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::block_delimiters::check_block_delimiters(
            src.as_bytes(),
            &cfg.block_delimiters,
            &[],
        );
        assert_eq!(alone.offenses.len(), 2);
        assert_eq!(alone.send_ignores.len(), 1);
        assert!(alone.has_conditional);
        assert_eq!(bundle.block_delimiters.offenses.len(), alone.offenses.len());
        assert_eq!(bundle.block_delimiters.send_ignores, alone.send_ignores);
        assert_eq!(
            bundle.block_delimiters.has_conditional,
            alone.has_conditional
        );
        for (a, b) in bundle
            .block_delimiters
            .offenses
            .iter()
            .zip(&alone.offenses)
        {
            assert_eq!(
                (a.token, a.block, &a.message, &a.method_name, &a.ops),
                (b.token, b.block, &b.message, &b.method_name, &b.ops)
            );
        }
    }

    /// `Metrics/AbcSize` merged into the shared walk must report exactly what
    /// its standalone entry point reports, over a method mixing assignments,
    /// branches and conditions.
    #[test]
    fn shared_walk_matches_standalone_abc_size() {
        let src = "def method_name\n\
                   \x20 my_options = Hash.new if 1 == 1 || 2 == 2\n\
                   \x20 my_options.each do |key, value|\n\
                   \x20   p key\n\
                   \x20   p value\n\
                   \x20 end\n\
                   end\n";
        let (mut nums, lists) = default_packed();
        nums[45] = 0; // report every method
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::abc_size::check_abc_size(
            src.as_bytes(),
            cfg.abc_size_max_floor,
            cfg.abc_size_discount_repeated,
        );
        assert_eq!(bundle.abc_size.len(), alone.len());
        assert!(!alone.is_empty());
        for (a, b) in bundle.abc_size.iter().zip(&alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.head_end, &a.method_name),
                (b.start_offset, b.end_offset, b.head_end, &b.method_name)
            );
            assert_eq!(
                (a.assignments, a.branches, a.conditions),
                (b.assignments, b.branches, b.conditions)
            );
        }
        // Sanity: the canonical <3, 4, 5> vector from the vendor spec.
        let m = &bundle.abc_size[0];
        assert_eq!((m.assignments, m.branches, m.conditions), (3, 4, 5));
    }

    /// `Metrics/MethodLength` merged into the shared walk must report exactly
    /// what its standalone entry point reports, over a source exercising a `def`,
    /// a `def self.`, a `define_method` block and a dynamically-named
    /// `define_method` (the unfilterable case).
    #[test]
    fn shared_walk_matches_standalone_method_length() {
        let src = "def a\n  x = 1\n  x = 2\n  x = 3\nend\n\
                   def self.b\n  y = 1\n  y = 2\n  y = 3\nend\n\
                   define_method(:c) do\n  z = 1\n  z = 2\n  z = 3\nend\n\
                   define_method(name) do\n  w = 1\n  w = 2\n  w = 3\nend\n";
        let (mut nums, lists) = default_packed();
        nums[78] = 2; // method_length max
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::method_length::check_method_length(
            src.as_bytes(),
            cfg.method_length_max,
            cfg.method_length_count_comments,
            &cfg.method_length_count_as_one,
        );
        assert_eq!(alone.len(), 4);
        assert!(alone.iter().any(|c| !c.filterable));
        assert_eq!(bundle.method_length.len(), alone.len());
        for (a, b) in bundle.method_length.iter().zip(&alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.head_end, a.length, &a.name, a.filterable),
                (b.start_offset, b.end_offset, b.head_end, b.length, &b.name, b.filterable)
            );
        }
    }

    /// `Metrics/ClassLength` merged into the shared walk must report exactly
    /// what its standalone entry point reports, over a source exercising a
    /// plain class, a suppressed-inside-class `class << self`, a toplevel
    /// sclass and the `Foo = Struct.new(...) do` constructor form.
    #[test]
    fn shared_walk_matches_standalone_class_length() {
        let src = "class A\n  x = 1\n  x = 2\n  x = 3\nend\n\
                   class << self\n  y = 1\n  y = 2\n  y = 3\nend\n\
                   Foo = Struct.new(:a) do\n  z = 1\n  z = 2\n  z = 3\nend\n";
        let (mut nums, lists) = default_packed();
        nums[88] = 2; // class_length max
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::class_length::check_class_length(
            src.as_bytes(),
            cfg.class_length_max,
            cfg.class_length_count_comments,
            &cfg.class_length_count_as_one,
        );
        assert_eq!(alone.len(), 3);
        assert!(alone.iter().any(|c| c.sclass));
        assert_eq!(bundle.class_length.len(), alone.len());
        for (a, b) in bundle.class_length.iter().zip(&alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.head_end, a.length, a.sclass),
                (b.start_offset, b.end_offset, b.head_end, b.length, b.sclass)
            );
        }
    }

    /// `Metrics/ModuleLength` merged into the shared walk must report exactly
    /// what its standalone entry point reports, over a plain module and the
    /// `Foo = Module.new do` constructor form (whose offense is the constant
    /// name).
    #[test]
    fn shared_walk_matches_standalone_module_length() {
        let src = "module A\n  x = 1\n  x = 2\n  x = 3\nend\n\
                   Foo = Module.new do\n  z = 1\n  z = 2\n  z = 3\nend\n";
        let (mut nums, lists) = default_packed();
        nums[90] = 2; // module_length max
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::module_length::check_module_length(
            src.as_bytes(),
            cfg.module_length_max,
            cfg.module_length_count_comments,
            &cfg.module_length_count_as_one,
        );
        assert_eq!(alone.len(), 2);
        assert_eq!(bundle.module_length.len(), alone.len());
        for (a, b) in bundle.module_length.iter().zip(&alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.head_end, a.length),
                (b.start_offset, b.end_offset, b.head_end, b.length)
            );
        }
    }

    /// `Lint/RequireParentheses` merged into the shared walk must report
    /// exactly what its standalone entry point reports, over a source that
    /// covers the two stock branches: a predicate send with an `&&` last
    /// argument, and a non-predicate ternary first-argument whose condition is
    /// `&&` (the latter must NOT trigger the ternary branch — first argument is
    /// an `IfNode` only when the method's first argument actually IS a
    /// ternary).
    #[test]
    fn shared_walk_matches_standalone_require_parentheses() {
        let src = "day.is? 'monday' && month == :jan\n\
                   wd.include? 'tuesday' && true == true ? a : b\n\
                   weekdays.foo 'tuesday' && true == true\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::require_parentheses::check_require_parentheses(src.as_bytes());
        assert_eq!(bundle.require_parentheses.len(), alone.len());
        assert_eq!(bundle.require_parentheses.len(), 2);
        for (a, b) in bundle.require_parentheses.iter().zip(&alone) {
            assert_eq!(
                (a.start_offset, a.end_offset),
                (b.start_offset, b.end_offset)
            );
        }
    }

    /// `Layout/IndentationConsistency` merged into the shared walk must report
    /// exactly what its standalone entry point reports, over a source mixing a
    /// misindented class member, a nested offense-within-offense (reported but
    /// not corrected) and an access-modifier-based base column.
    #[test]
    fn shared_walk_matches_standalone_indentation_consistency() {
        let src = "describe A do\n\
                   \x20 render_views\n\
                   \x20   describe B do\n\
                   \x20           it C do\n\
                   \x20           end\n\
                   \x20       describe D do\n\
                   \x20            before do\n\
                   \x20           end\n\
                   \x20       end\n\
                   \x20   end\n\
                   end\n\
                   public\n\
                   \n\
                   \x20 def foo\n\
                   \x20 end\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::indentation_consistency::check_indentation_consistency(
            src.as_bytes(),
            cfg.indentation_consistency,
        );
        assert!(alone.len() >= 3);
        assert!(alone.iter().any(|o| !o.autocorrect));
        assert_eq!(bundle.indentation_consistency.len(), alone.len());
        for (a, b) in bundle.indentation_consistency.iter().zip(&alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.column_delta, a.autocorrect),
                (b.start_offset, b.end_offset, b.column_delta, b.autocorrect)
            );
        }
    }

    /// `indented_internal_methods` style merged into the shared walk must match
    /// the standalone entry (sections delimited by `private` / `protected`).
    #[test]
    fn shared_walk_matches_standalone_indentation_consistency_internal() {
        let src = "class A\n\
                   \x20 def pub\n\
                   \x20 end\n\
                   \x20 private\n\
                   \x20   def priv\n\
                   \x20   end\n\
                   \x20  def priv2\n\
                   \x20  end\n\
                   end\n";
        let (mut nums, lists) = default_packed();
        nums[47] = 1; // indented_internal_methods
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::indentation_consistency::check_indentation_consistency(
            src.as_bytes(),
            cfg.indentation_consistency,
        );
        assert!(!alone.is_empty());
        assert_eq!(bundle.indentation_consistency.len(), alone.len());
        for (a, b) in bundle.indentation_consistency.iter().zip(&alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.column_delta, a.autocorrect),
                (b.start_offset, b.end_offset, b.column_delta, b.autocorrect)
            );
        }
    }

    /// `Layout/EmptyLineBetweenDefs` merged into the shared walk must report
    /// exactly what its standalone entry point reports, over a source exercising
    /// adjacent method defs (insert), too-many-lines (remove), a class/module
    /// pair and a nested-begin def pair.
    #[test]
    fn shared_walk_matches_standalone_empty_line_between_defs() {
        let src = "def a\nend\ndef b\nend\n\n\n\ndef c; end\nclass Foo\nend\nmodule Baz\nend\nif x\n  def d\n  end\n  def e\n  end\nend\n";
        let (mut nums, lists) = default_packed();
        nums[51] = 0; // allow_adjacent_one_line_defs = false
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::empty_line_between_defs::check_empty_line_between_defs(
            src.as_bytes(),
            cfg.empty_line_between_defs.clone(),
        );
        assert!(alone.len() >= 4);
        assert!(alone.iter().any(|o| o.insert));
        assert!(alone.iter().any(|o| !o.insert));
        assert_eq!(bundle.empty_line_between_defs.len(), alone.len());
        for (a, b) in bundle.empty_line_between_defs.iter().zip(&alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, &a.message, a.insert, a.pos, a.n),
                (b.start_offset, b.end_offset, &b.message, b.insert, b.pos, b.n)
            );
        }
    }

    /// `Layout/EmptyLines` merged into the shared walk must report exactly
    /// what its standalone entry point reports, over a source that exercises
    /// a basic blank gap between statements, a string literal whose embedded
    /// blank lines must NOT trigger (per-line `tSTRING_CONTENT`), a percent
    /// array where the gap inside DOES trigger (no per-line tokens), and a
    /// gap inside a multi-line def.
    #[test]
    fn shared_walk_matches_standalone_empty_lines() {
        let src = "a = 1\n\n\nb = 2\n\
                   x = \"line\n\n\nstring\"\n\
                   y = %w[a\n\n\nb]\n\
                   def foo\n  bar\n\n\n  baz\nend\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::empty_lines::check_empty_lines(src.as_bytes());
        assert!(!alone.is_empty());
        assert_eq!(bundle.empty_lines.len(), alone.len());
        for (a, b) in bundle.empty_lines.iter().zip(&alone) {
            assert_eq!((a.start, a.end), (b.start, b.end));
        }
    }

    /// `Layout/EmptyLinesAroundArguments` merged into the shared walk must
    /// report exactly what its standalone entry point reports, over a source
    /// exercising an empty line before an argument, between arguments and before
    /// the closing parenthesis, plus a clean call that must stay silent.
    #[test]
    fn shared_walk_matches_standalone_empty_lines_around_arguments() {
        let src = "foo(\n\n  bar\n)\nbaz(\n  a,\n\n  b\n\n)\nclean(\n  x,\n  y\n)\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let alone = super::empty_lines_around_arguments::check_empty_lines_around_arguments(
            src.as_bytes(),
        );
        assert!(alone.len() >= 3);
        assert_eq!(bundle.empty_lines_around_arguments.len(), alone.len());
        for (a, b) in bundle.empty_lines_around_arguments.iter().zip(&alone) {
            assert_eq!(
                (a.start_offset, a.end_offset),
                (b.start_offset, b.end_offset)
            );
        }
    }

    #[test]
    fn shared_walk_matches_standalone_end_alignment() {
        let src = "var = if test\n      end\nclass Foo\n  end\nformat(\n  case c\n  when f\n    b\nend, qux\n)\n";
        for style in 0..=2u8 {
            let (mut nums, lists) = default_packed();
            nums[54] = style as i64;
            let cfg = BundleConfig::from_packed(&nums, lists.clone()).unwrap();
            let bundle = check_all_bundle(src.as_bytes(), &cfg);

            let alone = super::end_alignment::check_end_alignment(
                src.as_bytes(),
                cfg.end_alignment,
            );
            assert!(!alone.is_empty());
            assert_eq!(bundle.end_alignment.len(), alone.len());
            for (a, b) in bundle.end_alignment.iter().zip(&alone) {
                let ao = a.offense.as_ref().map(|o| (&o.message, o.align_column));
                let bo = b.offense.as_ref().map(|o| (&o.message, o.align_column));
                assert_eq!(
                    (a.end_start, a.end_end, &a.matching, ao),
                    (b.end_start, b.end_end, &b.matching, bo)
                );
            }
        }
    }

    #[test]
    fn shared_walk_matches_standalone_block_alignment() {
        let src = "variable = test do |a|\n  end\nrb += files.select do |f|\n  x\n  end\ntest {\n  }\n";
        for style in 0..=2u8 {
            let (mut nums, lists) = default_packed();
            nums[55] = style as i64;
            let cfg = BundleConfig::from_packed(&nums, lists.clone()).unwrap();
            let bundle = check_all_bundle(src.as_bytes(), &cfg);

            let alone = super::block_alignment::check_block_alignment(
                src.as_bytes(),
                cfg.block_alignment,
            );
            assert!(!alone.is_empty());
            assert_eq!(bundle.block_alignment.len(), alone.len());
            for (a, b) in bundle.block_alignment.iter().zip(&alone) {
                assert_eq!(
                    (a.end_start, a.end_end, &a.message, a.align_column),
                    (b.end_start, b.end_end, &b.message, b.align_column)
                );
            }
        }
    }

    #[test]
    fn shared_walk_matches_standalone_else_alignment() {
        let src = "if cond\n  x\n else\n  y\nend\nvar = if a\n        0\nelse\n  1\n    end\ncase a\nwhen b\n  c\n else\n  d\nend\n";
        for style in 0..=2u8 {
            let (mut nums, lists) = default_packed();
            nums[56] = style as i64;
            let cfg = BundleConfig::from_packed(&nums, lists.clone()).unwrap();
            let bundle = check_all_bundle(src.as_bytes(), &cfg);

            let alone =
                super::else_alignment::check_else_alignment(src.as_bytes(), cfg.else_alignment);
            assert!(!alone.is_empty());
            assert_eq!(bundle.else_alignment.len(), alone.len());
            for (a, b) in bundle.else_alignment.iter().zip(&alone) {
                assert_eq!(
                    (a.else_start, a.else_end, &a.message, a.column_delta),
                    (b.else_start, b.else_end, &b.message, b.column_delta)
                );
            }
        }
    }

    /// `Layout/FirstHashElementIndentation` merged into the shared walk must
    /// report exactly what its standalone entry point reports, over a source
    /// exercising the ancestor-sensitive paths: a paren-claimed hash, a
    /// parent-hash-key base, a start-of-line operand hash and a hanging right
    /// brace, across every style.
    #[test]
    fn shared_walk_matches_standalone_first_hash_element_indentation() {
        let src = "func({\n  a: 1\n})\n\
                   func(x: {\n  a: 1,\n       b: 2\n},\n     y: {\n  c: 1\n     })\n\
                   a << {\n a: 1\n }\n";
        for style in 0..=2u8 {
            let (mut nums, lists) = default_packed();
            nums[57] = style as i64;
            let cfg = BundleConfig::from_packed(&nums, lists.clone()).unwrap();
            let bundle = check_all_bundle(src.as_bytes(), &cfg);

            let alone =
                super::first_hash_element_indentation::check_first_hash_element_indentation(
                    src.as_bytes(),
                    cfg.first_hash_element_style,
                    cfg.first_hash_element_indent,
                    cfg.first_hash_element_enforce_fixed,
                    cfg.first_hash_element_separators,
                );
            assert!(!alone.is_empty());
            assert_eq!(bundle.first_hash_element_indentation.len(), alone.len());
            for (a, b) in bundle.first_hash_element_indentation.iter().zip(&alone) {
                assert_eq!(
                    (
                        a.start_offset,
                        a.end_offset,
                        a.column_delta,
                        &a.message,
                        a.correct_whole_pair
                    ),
                    (
                        b.start_offset,
                        b.end_offset,
                        b.column_delta,
                        &b.message,
                        b.correct_whole_pair
                    )
                );
            }
        }
    }

    /// `Layout/HashAlignment` merged into the shared walk must report exactly
    /// what its standalone entry point reports, across the key / separator /
    /// table styles, over a source exercising misaligned keys, separators,
    /// values and a kwsplat.
    #[test]
    fn shared_walk_matches_standalone_hash_alignment() {
        let src = "hash = {\n  a: 0,\n   bb: 1\n}\n\
                   h2 = {\n    'a'  => 0,\n  'bbb' =>  1\n}\n\
                   h3 = {foo: 'bar',\n       **extra\n}\n\
                   S = {\n  t: {\n   '@1x': {\n      f: 'png',\n      g: 'x',\n   },\n  }.freeze,\n  m: {},\n}\n";
        for style in 0..=2u8 {
            let (nums, mut lists) = default_packed();
            let name = match style {
                1 => "separator",
                2 => "table",
                _ => "key",
            };
            lists[17] = vec![name.to_string()];
            lists[18] = vec![name.to_string()];
            let cfg = BundleConfig::from_packed(&nums, lists.clone()).unwrap();
            let bundle = check_all_bundle(src.as_bytes(), &cfg);

            let alone =
                super::hash_alignment::check_hash_alignment(src.as_bytes(), &cfg.hash_alignment);
            assert_eq!(bundle.hash_alignment.len(), alone.len());
            for (a, b) in bundle.hash_alignment.iter().zip(&alone) {
                assert_eq!(
                    (
                        a.start_offset,
                        a.end_offset,
                        a.message,
                        a.key_delta,
                        a.separator_delta,
                        a.value_delta,
                        a.has_value,
                    ),
                    (
                        b.start_offset,
                        b.end_offset,
                        b.message,
                        b.key_delta,
                        b.separator_delta,
                        b.value_delta,
                        b.has_value,
                    )
                );
            }
        }
    }

    #[test]
    fn shared_walk_matches_standalone_hash_syntax() {
        let src = "a = { :x => 0, :y => 1 }\n\
                   b = { c: 1, d: :e }\n\
                   f = { :\"s t\" => 0, g: 2 }\n\
                   foo(value: value)\n\
                   {foo: foo, bar: bar}\n\
                   {foo:, bar: baz}\n";
        // style x shorthand grid.
        for style in 0..=3u8 {
            for short in 0..=4u8 {
                let (mut nums, lists) = default_packed();
                nums[64] = style as i64;
                nums[65] = short as i64;
                let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
                let bundle = check_all_bundle(src.as_bytes(), &cfg);
                let alone =
                    super::hash_syntax::check_hash_syntax(src.as_bytes(), &cfg.hash_syntax);
                assert_eq!(
                    bundle.hash_syntax.len(),
                    alone.len(),
                    "len mismatch style={style} short={short}"
                );
                for (a, b) in bundle.hash_syntax.iter().zip(&alone) {
                    assert_eq!(
                        (a.is_offense, a.start_offset, a.end_offset, a.message),
                        (b.is_offense, b.start_offset, b.end_offset, b.message),
                        "style={style} short={short}"
                    );
                }
            }
        }
    }

    #[test]
    fn shared_walk_matches_standalone_string_literals() {
        let src = "a = \"plain\"\n\
                   b = 'ok'\n\
                   c = \"with 'apostrophe'\"\n\
                   d = \"interp #{x}\"\n\
                   e = \"x #{ 'inner' } y\"\n\
                   f = 'a' \\\n'b'\n\
                   g = \"a\" \\\n\"b\"\n\
                   h = 'a' \\\n\"b\"\n\
                   i = %(percent)\n\
                   j = \"newline \\n here\"\n";
        for style in 0..=1u8 {
            for consistent in 0..=1i64 {
                let (mut nums, lists) = default_packed();
                nums[70] = style as i64;
                nums[71] = consistent;
                let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
                let bundle = check_all_bundle(src.as_bytes(), &cfg);
                let alone = super::string_literals::check_string_literals(
                    src.as_bytes(),
                    &cfg.string_literals,
                );
                assert_eq!(
                    bundle.string_literals.len(),
                    alone.len(),
                    "len mismatch style={style} consistent={consistent}"
                );
                for (a, b) in bundle.string_literals.iter().zip(&alone) {
                    assert_eq!(
                        (a.is_offense, a.start_offset, a.end_offset, a.message, a.fix, &a.content),
                        (b.is_offense, b.start_offset, b.end_offset, b.message, b.fix, &b.content),
                        "style={style} consistent={consistent}"
                    );
                }
            }
        }
    }

    #[test]
    fn shared_walk_matches_standalone_string_literals_in_interpolation() {
        let src = "a = \"plain\"\n\
                   b = \"interp #{\"inner\"}\"\n\
                   c = :\"sym #{\"x\"}\"\n\
                   d = /re #{\"y\".sub(\"z\", 'b')}/\n\
                   e = \"#{\"multi\nline\"}\"\n\
                   f = \"#{:\"deep #{\"q\"}\"}\"\n\
                   g = \"ok #{'single'}\"\n\
                   h = \"#{\"a\" \\\n\"b\"}\"\n";
        for style in 0..=1u8 {
            let (mut nums, lists) = default_packed();
            nums[73] = style as i64;
            let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
            let bundle = check_all_bundle(src.as_bytes(), &cfg);
            let alone =
                super::string_literals_in_interpolation::check_string_literals_in_interpolation(
                    src.as_bytes(),
                    &cfg.string_literals_in_interpolation,
                );
            assert_eq!(
                bundle.string_literals_in_interpolation.len(),
                alone.len(),
                "len mismatch style={style}"
            );
            for (a, b) in bundle.string_literals_in_interpolation.iter().zip(&alone) {
                assert_eq!(
                    (a.is_offense, a.start_offset, a.end_offset, a.detect, a.fix, &a.content),
                    (b.is_offense, b.start_offset, b.end_offset, b.detect, b.fix, &b.content),
                    "style={style}"
                );
            }
        }
    }

    #[test]
    fn check_all_bundle_matches_standalone_trailing_empty_lines() {
        // A file with two trailing blank lines, exercised under both styles.
        let src = "x = 0\n\n\n";
        for style in 0..=1u8 {
            let (mut nums, lists) = default_packed();
            nums[74] = style as i64;
            let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
            let bundle = check_all_bundle(src.as_bytes(), &cfg);
            let alone = super::trailing_empty_lines::check_trailing_empty_lines(
                src.as_bytes(),
                &cfg.trailing_empty_lines,
            );
            assert_eq!(
                bundle.trailing_empty_lines.is_some(),
                alone.is_some(),
                "presence mismatch style={style}"
            );
            if let (Some(a), Some(b)) = (&bundle.trailing_empty_lines, &alone) {
                assert_eq!(
                    (a.report_start, a.report_end, a.ac_start, a.ac_end, a.replacement, a.blank_lines),
                    (b.report_start, b.report_end, b.ac_start, b.ac_end, b.replacement, b.blank_lines),
                    "style={style}"
                );
            }
        }
    }

    /// `Layout/SpaceAroundMethodCallOperator` merged into the shared walk must
    /// report exactly what its standalone entry point reports, over a source
    /// exercising space before/after a dot, `&.`, after `::` (including a
    /// const chain and a const assignment whose target `::` must stay silent),
    /// plus a clean call and a clean multi-line chain.
    #[test]
    fn shared_walk_matches_standalone_space_around_method_call_operator() {
        let src = "foo . bar\nfoo &. bar\nRuboCop:: Cop:: Base\nA:: B = 1\n\
                   foo.bar\nfoo\n  .bar\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);
        let alone =
            super::space_around_method_call_operator::check_space_around_method_call_operator(
                src.as_bytes(),
            );
        assert!(alone.len() >= 5);
        assert_eq!(bundle.space_around_method_call_operator.len(), alone.len());
        for (a, b) in bundle
            .space_around_method_call_operator
            .iter()
            .zip(&alone)
        {
            assert_eq!(
                (a.start_offset, a.end_offset),
                (b.start_offset, b.end_offset)
            );
        }
    }

    /// `Layout/SpaceAroundKeyword` merged into the shared walk must report
    /// exactly what its standalone entry point reports, over a source covering
    /// the before/after keyword checks (`if`/`else`/`end`/`then`, a `do` block,
    /// a modifier `while`, `case`/`when`, `begin`/`rescue`/`ensure`, `super`,
    /// the `and` keyword, and a one-line `in` pattern) plus clean lines that
    /// exercise the `preceded_by_operator?` and accept-delimiter suppressions.
    #[test]
    fn shared_walk_matches_standalone_space_around_keyword() {
        let src = "if\"\"then a end\n\
                   if a; \"\"else end\n\
                   a do \"a\"end\n\
                   1while x\n\
                   case\"\" when 1; end\n\
                   begin \"\"ensure end\n\
                   begin rescue; else\"\" end\n\
                   super\"\"\n\
                   1and 2\n\
                   a in\"\"\n\
                   1..super.size\n\
                   super(1)\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);
        let alone = super::space_around_keyword::check_space_around_keyword(src.as_bytes());
        assert!(alone.len() >= 8);
        assert_eq!(bundle.space_around_keyword.len(), alone.len());
        for (a, b) in bundle.space_around_keyword.iter().zip(&alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.before),
                (b.start_offset, b.end_offset, b.before)
            );
        }
    }

    /// `Layout/SpaceInsideBlockBraces` merged into the shared walk must report
    /// exactly what its standalone entry point reports, across the style /
    /// empty-style / space-before-params axes, over a source exercising a
    /// space-missing block, a no-space inner, an empty `{}`, an empty `{ }`, a
    /// `{|` pipe, a multi-line aligned block and a `do`/`end` block (ignored).
    #[test]
    fn shared_walk_matches_standalone_space_inside_block_braces() {
        let src = "a.each {puts x}\n\
                   b.each { puts x }\n\
                   c.each {}\n\
                   d.each { }\n\
                   e.each {|n| n }\n\
                   f.each { |a|\n  b\n}\n\
                   g.each do |n| n end\n\
                   h.each { [1] }\n";
        for style in 0..=1u8 {
            for empty in 0..=1u8 {
                for sbbp in 0..=1u8 {
                    let (mut nums, lists) = default_packed();
                    nums[75] = style as i64;
                    nums[76] = empty as i64;
                    nums[77] = sbbp as i64;
                    let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
                    let bundle = check_all_bundle(src.as_bytes(), &cfg);
                    let alone = super::space_inside_block_braces::check_space_inside_block_braces(
                        src.as_bytes(),
                        cfg.space_inside_block_braces,
                    );
                    assert_eq!(
                        bundle.space_inside_block_braces.len(),
                        alone.len(),
                        "len mismatch style={style} empty={empty} sbbp={sbbp}"
                    );
                    for (a, b) in bundle.space_inside_block_braces.iter().zip(&alone) {
                        assert_eq!(
                            (a.start_offset, a.end_offset, a.message.code()),
                            (b.start_offset, b.end_offset, b.message.code()),
                            "style={style} empty={empty} sbbp={sbbp}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn shared_walk_matches_standalone_space_inside_hash_literal_braces() {
        let src = "h = {a: 1}\n\
                   g = {  b: 2  }\n\
                   e1 = {}\n\
                   e2 = { }\n\
                   e3 = {\n}\n\
                   c = { a: { b: 1 } }\n\
                   d = { k => %w{a} }\n\
                   f = { a: proc {} }\n\
                   cm = { # comment\n  a: 1 }\n\
                   case x\nin {k1: 0}\n  1\nend\n";
        for style in 0..=2i64 {
            for empty in 0..=1i64 {
                let (mut nums, lists) = default_packed();
                nums[94] = style;
                nums[95] = empty;
                let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
                let bundle = check_all_bundle(src.as_bytes(), &cfg);
                let alone = super::space_inside_hash_literal_braces::
                    check_space_inside_hash_literal_braces(
                        src.as_bytes(),
                        cfg.space_inside_hash_literal_braces,
                    );
                assert_eq!(
                    bundle.space_inside_hash_literal_braces.len(),
                    alone.len(),
                    "len mismatch style={style} empty={empty}"
                );
                for (a, b) in bundle.space_inside_hash_literal_braces.iter().zip(&alone) {
                    assert_eq!(
                        (a.start_offset, a.end_offset, a.message.code()),
                        (b.start_offset, b.end_offset, b.message.code()),
                        "style={style} empty={empty}"
                    );
                }
            }
        }
    }

    #[test]
    fn shared_walk_matches_standalone_space_inside_array_literal_brackets() {
        let src = "a = [ 2, 3 ]\n\
                   b = [1, [2], %w[a]]\n\
                   e1 = []\n\
                   e2 = [ ]\n\
                   e3 = [\n]\n\
                   d = [ [1] ]\n\
                   f = [\n  [1], [2]]\n\
                   cm = [ # comment\n  1]\n\
                   case v\nin [ x, y ]\n  1\nin ADT[ i, [j ]]\n  2\nin ADT([g ], [h ])\n  3\nend\n";
        for style in 0..=2i64 {
            for empty in 0..=1i64 {
                let (mut nums, lists) = default_packed();
                nums[96] = style;
                nums[97] = empty;
                let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
                let bundle = check_all_bundle(src.as_bytes(), &cfg);
                let alone = super::space_inside_array_literal_brackets::
                    check_space_inside_array_literal_brackets(
                        src.as_bytes(),
                        cfg.space_inside_array_literal_brackets,
                    );
                assert_eq!(
                    bundle.space_inside_array_literal_brackets.offenses.len(),
                    alone.offenses.len(),
                    "offense len mismatch style={style} empty={empty}"
                );
                for (a, b) in bundle
                    .space_inside_array_literal_brackets
                    .offenses
                    .iter()
                    .zip(&alone.offenses)
                {
                    assert_eq!(
                        (
                            a.start_offset,
                            a.end_offset,
                            a.message.code(),
                            a.node,
                            a.suppress_when_disable_uncorrectable
                        ),
                        (
                            b.start_offset,
                            b.end_offset,
                            b.message.code(),
                            b.node,
                            b.suppress_when_disable_uncorrectable
                        ),
                        "style={style} empty={empty}"
                    );
                }
                let pack = |r: &super::space_inside_array_literal_brackets::ArrayBracketsResult| {
                    r.node_ops
                        .iter()
                        .map(|ops| ops.iter().map(|o| o.packed()).collect::<Vec<_>>())
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    pack(&bundle.space_inside_array_literal_brackets),
                    pack(&alone),
                    "ops mismatch style={style} empty={empty}"
                );
            }
        }
    }

    #[test]
    fn shared_walk_matches_standalone_space_before_block_braces() {
        let src = "each { puts }\n\
                   each{ puts }\n\
                   7.times {}\n\
                   7.times {}\n\
                   ->(){ }\n\
                   foo.map(a,\n  b) { |x| x }\n\
                   foo.bar { |x|\n  x\n}\n\
                   x.each do |n| n end\n";
        for style in 0..=1i64 {
            for empty in 0..=2i64 {
                for bd in 0..=1i64 {
                    let (mut nums, lists) = default_packed();
                    nums[100] = style;
                    nums[101] = empty;
                    nums[102] = bd;
                    let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
                    let bundle = check_all_bundle(src.as_bytes(), &cfg);
                    let alone = super::space_before_block_braces::check_space_before_block_braces(
                        src.as_bytes(),
                        cfg.space_before_block_braces,
                    );
                    let tag = format!("style={style} empty={empty} bd={bd}");
                    let (b, a) = (&bundle.space_before_block_braces, &alone);
                    assert_eq!(b.offenses.len(), a.offenses.len(), "len mismatch {tag}");
                    for (x, y) in b.offenses.iter().zip(&a.offenses) {
                        assert_eq!(
                            (x.start_offset, x.end_offset, x.detected, x.from_empty),
                            (y.start_offset, y.end_offset, y.detected, y.from_empty),
                            "{tag}"
                        );
                    }
                    let pack = |s: &super::space_before_block_braces::Summary| {
                        (
                            s.a_correct,
                            s.b_match_first,
                            s.b_offense,
                            s.b_match_after,
                            s.saw_empty,
                        )
                    };
                    assert_eq!(pack(&b.summary), pack(&a.summary), "{tag}");
                }
            }
        }
    }

    #[test]
    fn shared_walk_matches_standalone_trailing_comma_in_arguments() {
        let src = "some_method(a, b, c,)\n\
                   object[1, 2,]\n\
                   func.(1, 2,)\n\
                   obj&.foo(a, b,)\n\
                   m(\n  a,\n  b,\n  c: 0,\n  d: 1,\n)\n\
                   n(\n  a,\n  b\n)\n\
                   p(a: 1,\n  c: 2,)\n\
                   q(\n  a,\n  &block\n)\n\
                   route(1, <<-HELP.chomp\n...\nHELP\n)\n\
                   single(arg)\n\
                   r(\n  a, b,\n  c,\n)\n";
        for style in 0..=3u8 {
            let (mut nums, lists) = default_packed();
            nums[72] = style as i64;
            let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
            let bundle = check_all_bundle(src.as_bytes(), &cfg);
            let alone = super::trailing_comma_in_arguments::check_trailing_comma_in_arguments(
                src.as_bytes(),
                &cfg.trailing_comma_in_arguments,
            );
            assert_eq!(
                bundle.trailing_comma_in_arguments.len(),
                alone.len(),
                "len mismatch style={style}"
            );
            for (a, b) in bundle.trailing_comma_in_arguments.iter().zip(&alone) {
                assert_eq!(
                    (a.start_offset, a.end_offset, a.message, a.fix),
                    (b.start_offset, b.end_offset, b.message, b.fix),
                    "style={style}"
                );
            }
        }
    }

    #[test]
    fn shared_walk_matches_standalone_trailing_comma_in_hash_literal() {
        let src = "h = { a: 1, b: 2, }\n\
                   g = {\n  a: 1,\n  b: 2,\n}\n\
                   f = {\n  a: 1,\n  b: 2\n}\n\
                   e = { a: 1,\n      b: 2 }\n\
                   d = {\n  **kw\n}\n\
                   m(a: 1, b: 2,)\n\
                   c = { x: { y: 1 }, }\n";
        for style in 0..=3u8 {
            let (mut nums, lists) = default_packed();
            nums[92] = style as i64;
            let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
            let bundle = check_all_bundle(src.as_bytes(), &cfg);
            let alone = super::trailing_comma_in_hash_literal::check_trailing_comma_in_hash_literal(
                src.as_bytes(),
                &cfg.trailing_comma_in_hash_literal,
            );
            assert_eq!(
                bundle.trailing_comma_in_hash_literal.len(),
                alone.len(),
                "len mismatch style={style}"
            );
            for (a, b) in bundle.trailing_comma_in_hash_literal.iter().zip(&alone) {
                assert_eq!(
                    (a.start_offset, a.end_offset, a.message, a.fix),
                    (b.start_offset, b.end_offset, b.message, b.fix),
                    "style={style}"
                );
            }
        }
    }

    #[test]
    fn shared_walk_matches_standalone_trailing_comma_in_array_literal() {
        let src = "x = [1, 2,]\n\
                   y = [\n  1,\n  2,\n]\n\
                   z = [\n  1,\n  2\n]\n\
                   w = [1,\n     2]\n\
                   v = %w[\n  a\n  b\n]\n\
                   u = [\n  *rest\n]\n\
                   t = [[1, 2,], [3],]\n";
        for style in 0..=3u8 {
            let (mut nums, lists) = default_packed();
            nums[93] = style as i64;
            let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
            let bundle = check_all_bundle(src.as_bytes(), &cfg);
            let alone =
                super::trailing_comma_in_array_literal::check_trailing_comma_in_array_literal(
                    src.as_bytes(),
                    &cfg.trailing_comma_in_array_literal,
                );
            assert_eq!(
                bundle.trailing_comma_in_array_literal.len(),
                alone.len(),
                "len mismatch style={style}"
            );
            for (a, b) in bundle.trailing_comma_in_array_literal.iter().zip(&alone) {
                assert_eq!(
                    (a.start_offset, a.end_offset, a.message, a.fix),
                    (b.start_offset, b.end_offset, b.message, b.fix),
                    "style={style}"
                );
            }
        }
    }

    /// A disabled-by-config dispatch-family cop must stay disabled in the
    /// bundle (its `build_rule` returns `None` and it joins no walk).
    #[test]
    fn shared_walk_respects_disabled_rules() {
        let src = "foo(bar,\n  baz)\nfoo([\n  1\n])\n";
        let (mut nums, lists) = default_packed();
        nums[23] = 1; // argument_alignment incompatible (with_first_argument)
        nums[26] = 1; // first_argument enforce_fixed_no_line_break
        nums[37] = 1; // first_array_element enforce_fixed_indentation
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);
        assert!(bundle.argument_alignment.is_empty());
        assert!(bundle.first_argument_indentation.is_empty());
        assert!(bundle.first_array_element_indentation.is_empty());
    }

    #[test]
    fn check_all_bundle_matches_standalone_duplicate_magic_comment() {
        let src = "# encoding: utf-8\n# encoding: ascii\n# frozen_string_literal: true\n# frozen_string_literal: true\nx = 1\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);
        let alone =
            super::duplicate_magic_comment::check_duplicate_magic_comment(src.as_bytes());
        assert!(!alone.is_empty());
        assert_eq!(bundle.duplicate_magic_comment, alone);
    }

    #[test]
    fn check_all_bundle_matches_standalone_duplicate_methods() {
        // Exercises defs, sclass, attr, alias and an anonymous class block.
        let src = "class A\n  def foo; end\n  def foo; end\n  attr_reader :bar\n  alias baz qux\n  class << self\n    def s; end\n  end\nend\nClass.new do\n  def anon; end\nend.tap { 1 }\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);
        let alone = super::duplicate_methods::check_duplicate_methods(
            src.as_bytes(),
            &cfg.duplicate_methods,
        );
        assert!(!alone.is_empty());
        assert_eq!(bundle.duplicate_methods.len(), alone.len());
        for (a, b) in bundle.duplicate_methods.iter().zip(&alone) {
            assert_eq!(a.key, b.key);
            assert_eq!(a.name, b.name);
            assert_eq!(
                (a.sexp_start, a.sexp_end, a.scope_line),
                (b.sexp_start, b.sexp_end, b.scope_line)
            );
            assert_eq!(
                (a.off_start, a.off_end, a.line, a.rescue_scope),
                (b.off_start, b.off_end, b.line, b.rescue_scope)
            );
        }
    }

    #[test]
    fn check_all_bundle_matches_standalone_checks() {
        // One file exercising several cops at once: a debugger call, a deep
        // nesting, an over-long line, a misindented body and a multiline chain.
        let src = "def f(x)\n      binding.pry\n  x&.a.b\nend\nFoo.a\n     .b\n# aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n";
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);

        let debugger_alone = super::debugger::check_debugger(
            src.as_bytes(),
            &cfg.debugger_methods,
            &cfg.debugger_requires,
        );
        assert_eq!(bundle.debugger.len(), debugger_alone.len());
        assert!(!bundle.debugger.is_empty());
        for (a, b) in bundle.debugger.iter().zip(&debugger_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset),
                (b.start_offset, b.end_offset)
            );
        }

        let snc_alone = super::safe_navigation_chain::check_safe_navigation_chain(
            src.as_bytes(),
            &cfg.safe_navigation_nil_methods,
        );
        assert_eq!(bundle.safe_navigation_chain.len(), snc_alone.len());
        assert!(!bundle.safe_navigation_chain.is_empty());

        let mc_alone = mc::check_multiline_method_call_indentation(src.as_bytes(), 0, 2, 2);
        assert_eq!(bundle.multiline_method_call.len(), mc_alone.len());
        assert!(!bundle.multiline_method_call.is_empty());

        let ll_alone = super::line_length::check_line_length(src.as_bytes(), 120, 2);
        assert_eq!(bundle.line_length.len(), ll_alone.len());
        assert!(!bundle.line_length.is_empty());
        let candidates: HashSet<usize> = ll_alone.iter().map(|c| c.line_index).collect();
        let breakables_alone = super::line_length_breakable::compute_breakables_filtered(
            src.as_bytes(),
            120,
            false,
            Some(&candidates),
        );
        assert_eq!(bundle.line_length_breakables.len(), breakables_alone.len());
        for (a, b) in bundle.line_length_breakables.iter().zip(&breakables_alone) {
            assert_eq!(
                (a.line_index, a.insert_offset, &a.delimiter),
                (b.line_index, b.insert_offset, &b.delimiter)
            );
        }

        let iw_alone = super::indentation_width::check_indentation_width(
            src.as_bytes(),
            cfg.indentation_width,
            &[],
            &[],
        );
        assert_eq!(bundle.indentation_width.len(), iw_alone.len());
        assert!(!bundle.indentation_width.is_empty());
        for (a, b) in bundle.indentation_width.iter().zip(&iw_alone) {
            assert_eq!(
                (a.start_offset, a.end_offset, a.column_delta),
                (b.start_offset, b.end_offset, b.column_delta)
            );
        }
    }

    #[test]
    fn if_unless_modifier_bundle_matches_standalone() {
        // One "use modifier" candidate (with a first-line comment and a
        // parenthesizing parent) and one "too long" candidate (with a
        // comment-move rewrite), so both record shapes are compared.
        let long = "a".repeat(110);
        let src = format!(
            "x = if a # note\n  b\nend\nfoo(arg) if {long}_cond\ny(z) if w # {long}\n"
        );
        let (nums, lists) = default_packed();
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);
        let alone = super::if_unless_modifier::check_if_unless_modifier(
            src.as_bytes(),
            cfg.if_unless_modifier,
        );
        assert_eq!(bundle.if_unless_modifier.len(), alone.len());
        assert_eq!(alone.len(), 3);
        for (a, b) in bundle.if_unless_modifier.iter().zip(&alone) {
            assert_eq!(a.kind, b.kind);
            assert_eq!(
                (a.keyword_start, a.keyword_end, a.node_start, a.node_end),
                (b.keyword_start, b.keyword_end, b.node_start, b.node_end)
            );
            assert_eq!(a.is_unless, b.is_unless);
            assert_eq!(a.another_modifier_same_line, b.another_modifier_same_line);
            assert_eq!(
                (a.has_comment, a.comment_start, a.comment_end, a.has_code_after_end),
                (b.has_comment, b.comment_start, b.comment_end, b.has_code_after_end)
            );
            assert_eq!(
                (a.fits_no_comment, a.fits_with_comment),
                (b.fits_no_comment, b.fits_with_comment)
            );
            assert_eq!(a.replacement_no_comment, b.replacement_no_comment);
            assert_eq!(a.replacement_with_comment, b.replacement_with_comment);
            assert_eq!(a.line_number, b.line_number);
            assert_eq!(a.ops.len(), b.ops.len());
            for (x, y) in a.ops.iter().zip(&b.ops) {
                assert_eq!((x.kind, x.start, x.end, &x.text), (y.kind, y.start, y.end, &y.text));
            }
        }
    }
}
