//! Bundled single-walk runs: drive several cops over one parse + one AST walk.
//!
//! When multiple shirobai cops run on the same file, routing them through one
//! [`dispatch::run`](super::dispatch::run) collapses their N traversals into one
//! shared walk (the parse is already shared by `parse_cache`).

use std::collections::HashSet;

use super::multiline_method_call_indentation::{self as mc, MethodCallIndentOffense};
use super::multiline_operation_indentation::{self as op, OperationIndentOffense};
use super::{
    argument_alignment, block_length, block_nesting, complexity, debugger, dot_position,
    first_argument_indentation, indentation_width, line_end_concatenation, line_length,
    line_length_breakable, method_name, redundant_self, safe_navigation_chain, variable_number,
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
/// `nums` (`i64`, booleans are `0`/`1`), 34 entries:
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
///
/// `lists` (`Vec<String>`), 7 entries:
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
///
/// `Layout/IndentationWidth`'s `allowed_lines` and `prior_ranges` are fixed to
/// empty in the bundle: the non-empty cases (configured `AllowedPatterns`,
/// autocorrect re-passes) take the per-cop fallback path on the Ruby side.
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
    pub first_argument_style: u8,
    pub first_argument_indent: usize,
    pub first_argument_enforce_fixed_no_line_break: bool,
    pub indentation_width: indentation_width::Config,
    pub redundant_self_kernel_methods: Vec<String>,
}

const NUMS_LEN: usize = 34;
const LISTS_LEN: usize = 7;

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
            redundant_self_kernel_methods: next_list(),
        })
    }
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
    pub first_argument_indentation: Vec<first_argument_indentation::FirstArgIndentOffense>,
    pub redundant_self: Vec<redundant_self::RedundantSelfOffense>,
    pub indentation_width: Vec<indentation_width::IndentationOffense>,
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
    ];
    if let Some(rule) = aa_rule.as_mut() {
        rules.push(rule);
    }
    if let Some(rule) = fa_rule.as_mut() {
        rules.push(rule);
    }
    super::dispatch::run(source, &mut rules);

    let multiline_operation = op_rule.offenses;
    let multiline_method_call = mc_rule.offenses;
    let argument_alignment = aa_rule.map(|r| r.offenses).unwrap_or_default();
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
        first_argument_indentation,
        redundant_self,
        indentation_width,
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
        ];
        let lists = vec![
            vec!["binding.pry".to_string(), "debugger".to_string()],
            vec![],
            vec![],
            vec![],
            vec![],
            vec!["to_s".to_string()],
            vec!["puts".to_string()],
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
        assert_eq!(cfg.debugger_methods, vec!["binding.pry", "debugger"]);
        assert_eq!(cfg.safe_navigation_nil_methods, vec!["to_s"]);
        assert_eq!(cfg.redundant_self_kernel_methods, vec!["puts"]);
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

    /// A disabled-by-config dispatch-family cop must stay disabled in the
    /// bundle (its `build_rule` returns `None` and it joins no walk).
    #[test]
    fn shared_walk_respects_disabled_rules() {
        let src = "foo(bar,\n  baz)\n";
        let (mut nums, lists) = default_packed();
        nums[23] = 1; // argument_alignment incompatible (with_first_argument)
        nums[26] = 1; // first_argument enforce_fixed_no_line_break
        let cfg = BundleConfig::from_packed(&nums, lists).unwrap();
        let bundle = check_all_bundle(src.as_bytes(), &cfg);
        assert!(bundle.argument_alignment.is_empty());
        assert!(bundle.first_argument_indentation.is_empty());
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
}
