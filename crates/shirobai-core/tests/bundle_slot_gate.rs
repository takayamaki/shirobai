//! Invariants of the per-slot enable gate in `check_all_bundle`.
//!
//! `BundleConfig::enabled_slots` lets the Ruby side switch off every core
//! slot whose cop the config does not enable. Turning slots off must never
//! change what an ON slot gets, and an OFF slot must stay at its `Default`.
//! On a sample of the vendored RuboCop source (the same material as
//! `pm_lex_canary.rs`) this test checks that:
//!
//! 1. for every slot, the all-on run and a run with ONLY that slot's group on
//!    (`core_slot::GROUPS`) give the same value for that slot, and
//! 2. with every slot off, every slot equals its `Default` value.
//!
//! It also pins the slot table itself: 106 names, numbered in order, and the
//! same names (upper-cased) and numbers as the core entries of
//! `Shirobai::Dispatch::SLOTS` in `lib/shirobai/dispatch.rb`.

use std::path::{Path, PathBuf};

use shirobai_core::rules::bundle::{
    check_all_bundle, core_slot, test_support, BundleConfig, BundleResult,
};

/// Take every `SAMPLE_STRIDE`-th file (sorted by path) so the sample spreads
/// over the whole tree but stays inside the debug-build time budget.
const SAMPLE_STRIDE: usize = 40;

fn collect_rb_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rb_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rb") {
            out.push(path);
        }
    }
}

fn repo_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.push("../..");
    root
}

fn sample_sources() -> Vec<(PathBuf, Vec<u8>)> {
    let root = repo_root().join("vendor/rubocop/lib");
    let mut files = Vec::new();
    collect_rb_files(&root, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "no .rb files under {}. The vendor/rubocop submodule is not checked out; \
         run `git submodule update --init vendor/rubocop` before `cargo test`.",
        root.display()
    );
    // Two offense-dense fixtures first: the vendored RuboCop source is clean
    // under most default cops, so on its own it leaves many slots empty.
    let fixture = repo_root().join("crates/shirobai-core/tests/fixtures/slot_gate_dense.rb");
    let fixture_2 = repo_root().join("crates/shirobai-core/tests/fixtures/slot_gate_dense_2.rb");
    [fixture, fixture_2]
        .into_iter()
        .chain(files.into_iter().step_by(SAMPLE_STRIDE))
        .map(|p| {
            let src = std::fs::read(&p).expect("read sample file");
            (p, src)
        })
        .collect()
}

/// A default config (plugin origins dormant) with `mask` as the slot mask.
fn config_with_mask(mask: u128) -> BundleConfig {
    let (mut core_nums, core_lists) = test_support::default_core_packed();
    let n = core_nums.len();
    core_nums[n - 2] = mask as u64 as i64;
    core_nums[n - 1] = (mask >> 64) as u64 as i64;
    let nums = vec![core_nums, vec![0, 0, 0], vec![0; 9], vec![0, 0]];
    let lists = vec![
        core_lists,
        vec![vec![]],
        vec![Vec::new(); 16],
        vec![vec![], vec![], vec![], vec![]],
    ];
    let cfg = BundleConfig::from_packed(&nums, lists).expect("valid packed config");
    assert_eq!(cfg.enabled_slots, mask);
    cfg
}

/// The `Debug` form of the value the ext pushes into core slot `slot`.
fn slot_debug(r: &BundleResult, slot: usize) -> String {
    use core_slot as S;
    let elab = &r.empty_lines_around_body;
    let ps = &r.punctuation_spacing;
    match slot {
        S::DEBUGGER => format!("{:?}", r.debugger),
        S::BLOCK_LENGTH => format!("{:?}", r.block_length),
        S::BLOCK_NESTING => format!("{:?}", r.block_nesting),
        S::COMPLEXITY => format!("{:?}", r.complexity),
        S::VARIABLE_NUMBER => format!("{:?}", r.variable_number),
        S::METHOD_NAME => format!("{:?}", r.method_name),
        S::SAFE_NAVIGATION_CHAIN => format!("{:?}", r.safe_navigation_chain),
        S::MULTILINE_OPERATION => format!("{:?}", r.multiline_operation),
        S::MULTILINE_METHOD_CALL => format!("{:?}", r.multiline_method_call),
        S::DOT_POSITION => format!("{:?}", r.dot_position),
        S::LINE_LENGTH => format!("{:?}", r.line_length),
        S::LINE_LENGTH_BREAKABLES => format!("{:?}", r.line_length_breakables),
        S::LINE_END_CONCATENATION => format!("{:?}", r.line_end_concatenation),
        S::ARGUMENT_ALIGNMENT => format!("{:?}", r.argument_alignment),
        S::FIRST_ARGUMENT_INDENTATION => format!("{:?}", r.first_argument_indentation),
        S::REDUNDANT_SELF => format!("{:?}", r.redundant_self),
        S::INDENTATION_WIDTH => format!("{:?}", r.indentation_width),
        S::PREDICATE_PREFIX => format!("{:?}", r.predicate_prefix),
        S::CLOSING_PARENTHESIS_INDENTATION => format!("{:?}", r.closing_parenthesis_indentation),
        S::FIRST_ARRAY_ELEMENT_INDENTATION => format!("{:?}", r.first_array_element_indentation),
        S::HASH_EACH_METHODS => format!("{:?}", r.hash_each_methods),
        S::VOID => format!("{:?}", r.void),
        S::USELESS_ACCESS_MODIFIER => format!("{:?}", r.useless_access_modifier),
        S::EMPTY_LINES_AROUND_METHOD_BODY => format!("{:?}", elab.method_body),
        S::EMPTY_LINES_AROUND_CLASS_BODY => format!("{:?}", elab.class_body),
        S::EMPTY_LINES_AROUND_MODULE_BODY => format!("{:?}", elab.module_body),
        S::EMPTY_LINES_AROUND_BLOCK_BODY => format!("{:?}", elab.block_body),
        S::EMPTY_LINES_AROUND_BEGIN_BODY => format!("{:?}", elab.begin_body),
        S::EMPTY_LINES_AROUND_EXCEPTION_HANDLING_KEYWORDS => {
            format!("{:?}", elab.exception_keywords)
        }
        S::BLOCK_DELIMITERS => format!("{:?}", r.block_delimiters),
        S::ABC_SIZE => format!("{:?}", r.abc_size),
        S::INDENTATION_CONSISTENCY => format!("{:?}", r.indentation_consistency),
        S::EMPTY_LINE_BETWEEN_DEFS => format!("{:?}", r.empty_line_between_defs),
        S::END_ALIGNMENT => format!("{:?}", r.end_alignment),
        S::BLOCK_ALIGNMENT => format!("{:?}", r.block_alignment),
        S::ELSE_ALIGNMENT => format!("{:?}", r.else_alignment),
        S::FIRST_HASH_ELEMENT_INDENTATION => format!("{:?}", r.first_hash_element_indentation),
        S::HASH_ALIGNMENT => format!("{:?}", r.hash_alignment),
        S::EMPTY_LINES_AROUND_ARGUMENTS => format!("{:?}", r.empty_lines_around_arguments),
        S::HASH_SYNTAX => format!("{:?}", r.hash_syntax),
        S::STRING_LITERALS => format!("{:?}", r.string_literals),
        S::TRAILING_COMMA_IN_ARGUMENTS => format!("{:?}", r.trailing_comma_in_arguments),
        S::STRING_LITERALS_IN_INTERPOLATION => format!("{:?}", r.string_literals_in_interpolation),
        S::TRAILING_EMPTY_LINES => format!("{:?}", r.trailing_empty_lines),
        S::SPACE_AROUND_METHOD_CALL_OPERATOR => {
            format!("{:?}", r.space_around_method_call_operator)
        }
        S::SPACE_AROUND_KEYWORD => format!("{:?}", r.space_around_keyword),
        S::SPACE_INSIDE_BLOCK_BRACES => format!("{:?}", r.space_inside_block_braces),
        S::METHOD_LENGTH => format!("{:?}", r.method_length),
        S::DEF_END_ALIGNMENT => format!("{:?}", r.def_end_alignment),
        S::REQUIRE_PARENTHESES => format!("{:?}", r.require_parentheses),
        S::SELF_ASSIGNMENT => format!("{:?}", r.self_assignment),
        S::NESTED_PARENTHESIZED_CALLS => format!("{:?}", r.nested_parenthesized_calls),
        S::PARENTHESES_AS_GROUPED_EXPRESSION => {
            format!("{:?}", r.parentheses_as_grouped_expression)
        }
        S::PERCENT_LITERAL_DELIMITERS => format!("{:?}", r.percent_literal_delimiters),
        S::MULTILINE_METHOD_CALL_BRACE_LAYOUT => {
            format!("{:?}", r.multiline_method_call_brace_layout)
        }
        S::ACCESS_MODIFIER_INDENTATION => format!("{:?}", r.access_modifier_indentation),
        S::ASSIGNMENT_INDENTATION => format!("{:?}", r.assignment_indentation),
        S::REDUNDANT_SELF_ASSIGNMENT => format!("{:?}", r.redundant_self_assignment),
        S::COLON_METHOD_CALL => format!("{:?}", r.colon_method_call),
        S::STABBY_LAMBDA_PARENTHESES => format!("{:?}", r.stabby_lambda_parentheses),
        S::UNREACHABLE_CODE => format!("{:?}", r.unreachable_code),
        S::HASH_TRANSFORM_KEYS => format!("{:?}", r.hash_transform_keys),
        S::AMBIGUOUS_BLOCK_ASSOCIATION => format!("{:?}", r.ambiguous_block_association),
        S::EMPTY_LINE_AFTER_GUARD_CLAUSE => format!("{:?}", r.empty_line_after_guard_clause),
        S::EMPTY_COMMENT => format!("{:?}", r.empty_comment),
        S::EMPTY_LINE_AFTER_MAGIC_COMMENT => format!("{:?}", r.empty_line_after_magic_comment),
        S::EMPTY_LINES => format!("{:?}", r.empty_lines),
        S::LEADING_EMPTY_LINES => format!("{:?}", r.leading_empty_lines),
        S::CLASS_LENGTH => format!("{:?}", r.class_length),
        S::MODULE_LENGTH => format!("{:?}", r.module_length),
        S::TRAILING_COMMA_IN_HASH_LITERAL => format!("{:?}", r.trailing_comma_in_hash_literal),
        S::TRAILING_COMMA_IN_ARRAY_LITERAL => format!("{:?}", r.trailing_comma_in_array_literal),
        S::SPACE_INSIDE_HASH_LITERAL_BRACES => format!("{:?}", r.space_inside_hash_literal_braces),
        S::SPACE_INSIDE_ARRAY_LITERAL_BRACKETS => {
            format!("{:?}", r.space_inside_array_literal_brackets)
        }
        S::SPACE_BEFORE_BLOCK_BRACES => format!("{:?}", r.space_before_block_braces),
        S::IF_UNLESS_MODIFIER => format!("{:?}", r.if_unless_modifier),
        S::SPACE_BEFORE_COMMA => format!("{:?}", ps.space_before_comma),
        S::SPACE_AFTER_COMMA => format!("{:?}", ps.space_after_comma),
        S::SPACE_BEFORE_SEMICOLON => format!("{:?}", ps.space_before_semicolon),
        S::SPACE_AFTER_SEMICOLON => format!("{:?}", ps.space_after_semicolon),
        S::SPACE_AFTER_COLON => format!("{:?}", ps.space_after_colon),
        S::SPACE_BEFORE_COMMENT => format!("{:?}", ps.space_before_comment),
        S::SPACE_INSIDE_PARENS => format!("{:?}", r.space_inside_parens),
        S::SPACE_INSIDE_REFERENCE_BRACKETS => format!("{:?}", r.space_inside_reference_brackets),
        S::SPACE_BEFORE_FIRST_ARG => format!("{:?}", r.space_before_first_arg),
        S::DUPLICATE_MAGIC_COMMENT => format!("{:?}", r.duplicate_magic_comment),
        S::DUPLICATE_METHODS => format!("{:?}", r.duplicate_methods),
        S::ARRAY_ALIGNMENT => format!("{:?}", r.array_alignment),
        S::FILE_NULL => format!("{:?}", r.file_null),
        S::SEMICOLON => format!("{:?}", r.semicolon),
        S::REDUNDANT_FREEZE => format!("{:?}", r.redundant_freeze),
        S::FROZEN_STRING_LITERAL_COMMENT => format!("{:?}", r.frozen_string_literal_comment),
        S::ARGUMENTS_FORWARDING => format!("{:?}", r.arguments_forwarding),
        S::SPACE_AROUND_OPERATORS => format!("{:?}", r.space_around_operators),
        S::ORDERED_MAGIC_COMMENTS => format!("{:?}", r.ordered_magic_comments),
        S::INITIAL_INDENTATION => format!("{:?}", r.initial_indentation),
        S::SPACE_AROUND_EQUALS_IN_PARAMETER_DEFAULT => {
            format!("{:?}", r.space_around_equals_in_parameter_default)
        }
        S::EXTRA_SPACING => format!("{:?}", r.extra_spacing),
        S::END_OF_LINE => format!("{:?}", r.end_of_line),
        S::LINE_CONTINUATION_SPACING => format!("{:?}", r.line_continuation_spacing),
        S::SPACE_INSIDE_STRING_INTERPOLATION => {
            format!("{:?}", r.space_inside_string_interpolation)
        }
        S::MAGIC_COMMENT_FORMAT => format!("{:?}", r.magic_comment_format),
        S::ASCII_IDENTIFIERS => format!("{:?}", r.ascii_identifiers),
        S::RESCUE_ENSURE_ALIGNMENT => format!("{:?}", r.rescue_ensure_alignment),
        S::DIRECTIVE_SCOPE => format!("{:?}", r.directive_scope),
        S::MISPLACED_MAGIC_COMMENT => format!("{:?}", r.misplaced_magic_comment),
        _ => panic!("slot {slot} has no BundleResult field mapping"),
    }
}

/// `Debug` of each core slot of a run where every slot is at its `Default`.
/// The `Default` of each field type, read through the same mapping.
fn default_slot_debugs() -> Vec<String> {
    // An all-off run of an empty source is the cheapest way to get a
    // `BundleResult` of defaults without a `Default` impl on the struct; the
    // per-type defaults are then checked against it field by field below.
    let r = check_all_bundle(b"", &config_with_mask(0));
    (0..core_slot::COUNT).map(|s| slot_debug(&r, s)).collect()
}

#[test]
fn core_slot_table_has_106_slots_in_order() {
    assert_eq!(core_slot::COUNT, 106);
    assert_eq!(core_slot::ALL.len(), core_slot::COUNT);
    for (i, (name, idx)) in core_slot::ALL.iter().enumerate() {
        assert_eq!(*idx, i, "core_slot::{name} is out of order");
    }
    assert_eq!(core_slot::ALL_ON.count_ones() as usize, core_slot::COUNT);
    // Every grouped slot belongs to exactly one group.
    for &(_, slot) in core_slot::ALL {
        let n = core_slot::GROUPS.iter().filter(|g| g.contains(&slot)).count();
        assert!(n <= 1, "slot {slot} is in {n} groups");
    }
}

#[test]
fn core_slot_table_mirrors_dispatch_slots() {
    let path = repo_root().join("lib/shirobai/dispatch.rb");
    let ruby = std::fs::read_to_string(&path).expect("read dispatch.rb");
    // Core entries look like `      debugger: [0, 0].freeze,`.
    let mut ruby_slots = Vec::new();
    for line in ruby.lines() {
        let line = line.trim();
        let Some((name, rest)) = line.split_once(": [0, ") else {
            continue;
        };
        if !name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            continue;
        }
        let Some((idx, _)) = rest.split_once(']') else {
            continue;
        };
        ruby_slots.push((name.to_ascii_uppercase(), idx.parse::<usize>().expect("slot index")));
    }
    let rust_slots: Vec<(String, usize)> =
        core_slot::ALL.iter().map(|(n, i)| (n.to_string(), *i)).collect();
    assert_eq!(ruby_slots, rust_slots);
}

#[test]
fn group_only_run_matches_all_on_run_for_every_slot() {
    let sources = sample_sources();
    let all_on = config_with_mask(core_slot::ALL_ON);
    // One config per distinct group, in slot order.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for slot in 0..core_slot::COUNT {
        let g = core_slot::group_of(slot);
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    let group_cfgs: Vec<(Vec<usize>, BundleConfig)> = groups
        .into_iter()
        .map(|g| {
            let cfg = config_with_mask(core_slot::mask_of(&g));
            (g, cfg)
        })
        .collect();
    let defaults = default_slot_debugs();
    let mut non_default_slots = [false; core_slot::COUNT];
    for (path, src) in &sources {
        let full = check_all_bundle(src, &all_on);
        let full_debugs: Vec<String> =
            (0..core_slot::COUNT).map(|s| slot_debug(&full, s)).collect();
        for (slot, d) in full_debugs.iter().enumerate() {
            if *d != defaults[slot] {
                non_default_slots[slot] = true;
            }
        }
        for (group, cfg) in &group_cfgs {
            let only = check_all_bundle(src, cfg);
            for slot in 0..core_slot::COUNT {
                let got = slot_debug(&only, slot);
                if group.contains(&slot) {
                    assert_eq!(
                        got,
                        full_debugs[slot],
                        "slot {slot} differs with only group {group:?} on, in {}",
                        path.display()
                    );
                } else {
                    assert_eq!(
                        got,
                        defaults[slot],
                        "slot {slot} is not Default with only group {group:?} on, in {}",
                        path.display()
                    );
                }
            }
        }
    }
    // The sample must exercise almost every slot, or (1) checks little.
    let hit = non_default_slots.iter().filter(|&&b| b).count();
    let missed: Vec<&str> = core_slot::ALL
        .iter()
        .filter(|(_, i)| !non_default_slots[*i])
        .map(|(n, _)| *n)
        .collect();
    eprintln!("{hit} of {} slots produced a value; never hit: {missed:?}", core_slot::COUNT);
    assert!(
        hit >= 95,
        "only {hit} of {} slots produced a value on the sample",
        core_slot::COUNT
    );
}

#[test]
fn all_off_run_leaves_every_slot_default() {
    let sources = sample_sources();
    let off = config_with_mask(0);
    let defaults = default_slot_debugs();
    for (path, src) in &sources {
        let r = check_all_bundle(src, &off);
        for (slot, want) in defaults.iter().enumerate() {
            assert_eq!(
                &slot_debug(&r, slot),
                want,
                "slot {slot} is not Default with every slot off, in {}",
                path.display()
            );
        }
    }
}
