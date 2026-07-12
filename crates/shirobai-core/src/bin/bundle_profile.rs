//! Phase-decomposition profiler for the 68-cop bundle (MEASUREMENT ONLY).
//!
//! Splits the pure-Rust cost of `check_all_bundle` into phases over a corpus:
//!
//! * `parse`   — prism parse only (P).
//! * `walk0`   — parse + shared walk driving 1 noop rule (W ≈ walk mechanics).
//! * `walkN:<n>` — parse + shared walk driving n noop rules (dyn-dispatch
//!   scaling; per-rule fan-out cost = slope over n).
//! * `bundle`  — the real `check_all_bundle` with a packed config read from a
//!   file dumped by the Ruby side (same values e2e uses).
//! * `guard`   — `check_empty_line_after_guard_clause` standalone (its own
//!   AST walk today; candidate to fold into the shared walk).
//! * `mname`   — `check_method_name_filtered` standalone (own pruned walk).
//! * `linelen` — the `LineLength` source scan (no heredocs).
//!
//! Usage: bundle_profile <mode> <corpus_dir> <packed_config_file> [iterations]
//! Prints median CPU seconds over iterations to stdout; min/max to stderr.

use std::path::PathBuf;

use ruby_prism::{Node, parse};
use shirobai_core::rules;
use shirobai_core::rules::bundle::BundleConfig;
use shirobai_core::rules::dispatch::{self, Rule};

#[cfg(unix)]
fn cpu_time_secs() -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: ts is a valid timespec.
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

fn collect_rb_files(dir: &PathBuf, out: &mut Vec<PathBuf>) {
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

struct NoopRule {
    sink: usize,
}

impl Rule for NoopRule {
    fn enter(&mut self, node: &Node<'_>) {
        // Touch the node so the call cannot be optimized out entirely.
        self.sink = self.sink.wrapping_add(node.location().start_offset());
    }

    fn leave(&mut self) {
        self.sink = self.sink.wrapping_add(1);
    }
}

/// Reads one or more packed segments (nums line, list count, lists), one per
/// origin, matching the per-origin wire format. A dump with fewer segments
/// than `N_ORIGINS` (e.g. a core-only config) gets the dormant plugin
/// segments appended so old dump scripts keep working after regeneration.
fn read_packed(path: &str) -> (Vec<Vec<i64>>, Vec<Vec<Vec<String>>>) {
    let text = std::fs::read_to_string(path).expect("packed config file");
    let mut lines = text.lines().peekable();
    let mut packed_nums = Vec::new();
    let mut packed_lists = Vec::new();
    while lines.peek().is_some_and(|l| !l.trim().is_empty()) {
        let nums: Vec<i64> = lines
            .next()
            .expect("nums line")
            .split_whitespace()
            .map(|t| t.parse().expect("num"))
            .collect();
        let list_count: usize = lines.next().expect("list count").parse().expect("count");
        let mut lists = Vec::with_capacity(list_count);
        for _ in 0..list_count {
            let k: usize = lines.next().expect("list len").parse().expect("len");
            let mut items = Vec::with_capacity(k);
            for _ in 0..k {
                items.push(lines.next().expect("list item").to_string());
            }
            lists.push(items);
        }
        packed_nums.push(nums);
        packed_lists.push(lists);
    }
    if packed_nums.len() < 2 {
        // Dormant performance segment.
        packed_nums.push(vec![0, 0, 0]);
        packed_lists.push(vec![vec![]]);
    }
    if packed_nums.len() < 3 {
        // Dormant rspec segment: enable flag off + 16 empty role lists.
        packed_nums.push(vec![0]);
        packed_lists.push(vec![vec![]; 16]);
    }
    if packed_nums.len() < 4 {
        // Dormant rails segment: enable flag off + no lists.
        packed_nums.push(vec![0]);
        packed_lists.push(vec![]);
    }
    (packed_nums, packed_lists)
}

/// Per-rule attribution: each standalone `check_*` entry drives its own walk
/// over the (cache-hot) parse. Reported per-rule seconds therefore include one
/// walk's traversal mechanics each; subtract the `walk0` phase median (whole
/// corpus) for the pure rule body. Ordering matches `Dispatch::SLOTS` roughly.
fn run_perrule(sources: &[Vec<u8>], cfg: &BundleConfig, iterations: usize) {
    use shirobai_core::rules as r;
    type Entry<'c> = (&'static str, Box<dyn Fn(&[u8]) + 'c>);
    macro_rules! e {
        ($name:literal, $body:expr) => {
            ($name, Box::new($body) as Box<dyn Fn(&[u8])>)
        };
    }
    let entries: Vec<Entry> = vec![
        e!("debugger", |s: &[u8]| {
            std::hint::black_box(r::debugger::check_debugger(
                s,
                &cfg.debugger_methods,
                &cfg.debugger_requires,
            ));
        }),
        e!("block_length", |s: &[u8]| {
            std::hint::black_box(r::block_length::check_block_length_filtered(
                s,
                cfg.block_length_max,
                cfg.block_length_count_comments,
                &cfg.block_length_count_as_one,
                &cfg.block_length_allowed_methods,
                cfg.block_length_filtered,
            ));
        }),
        e!("block_nesting", |s: &[u8]| {
            std::hint::black_box(r::block_nesting::check_block_nesting(
                s,
                cfg.block_nesting_max,
                cfg.block_nesting_count_blocks,
                cfg.block_nesting_count_modifier_forms,
            ));
        }),
        e!("complexity", |s: &[u8]| {
            std::hint::black_box(r::complexity::check_complexity_exceeding(
                s,
                cfg.max_cyclomatic,
                cfg.max_perceived,
            ));
        }),
        e!("variable_number", |s: &[u8]| {
            std::hint::black_box(r::variable_number::check_variable_number(
                s,
                cfg.variable_number_style,
                cfg.variable_number_flags,
                &cfg.variable_number_allowed_identifiers,
            ));
        }),
        e!("method_name", |s: &[u8]| {
            std::hint::black_box(r::method_name::check_method_name_filtered(
                s,
                cfg.method_name_style,
                true,
            ));
        }),
        e!("safe_navigation_chain", |s: &[u8]| {
            std::hint::black_box(r::safe_navigation_chain::check_safe_navigation_chain(
                s,
                &cfg.safe_navigation_nil_methods,
            ));
        }),
        e!("multiline_operation", |s: &[u8]| {
            let c = cfg.multiline_operation;
            std::hint::black_box(
                r::multiline_operation_indentation::check_multiline_operation_indentation(
                    s, c.0, c.1, c.2,
                ),
            );
        }),
        e!("multiline_method_call", |s: &[u8]| {
            let c = cfg.multiline_method_call;
            std::hint::black_box(
                r::multiline_method_call_indentation::check_multiline_method_call_indentation(
                    s, c.0, c.1, c.2,
                ),
            );
        }),
        e!("dot_position", |s: &[u8]| {
            std::hint::black_box(r::dot_position::check_dot_position(
                s,
                cfg.dot_position_style,
            ));
        }),
        e!("line_length", |s: &[u8]| {
            std::hint::black_box(r::line_length::check_line_length(
                s,
                cfg.line_length_max,
                cfg.line_length_tab_width,
            ));
        }),
        e!("line_end_concatenation", |s: &[u8]| {
            std::hint::black_box(r::line_end_concatenation::check_line_end_concatenation(s));
        }),
        e!("argument_alignment", |s: &[u8]| {
            std::hint::black_box(r::argument_alignment::check_argument_alignment(
                s,
                cfg.argument_alignment_style,
                cfg.argument_alignment_indent,
                cfg.argument_alignment_incompatible,
            ));
        }),
        e!("first_argument_indentation", |s: &[u8]| {
            std::hint::black_box(r::first_argument_indentation::check_first_argument_indentation(
                s,
                cfg.first_argument_style,
                cfg.first_argument_indent,
                cfg.first_argument_enforce_fixed_no_line_break,
            ));
        }),
        e!("redundant_self", |s: &[u8]| {
            std::hint::black_box(r::redundant_self::check_redundant_self(
                s,
                &cfg.redundant_self_kernel_methods,
            ));
        }),
        e!("indentation_width", |s: &[u8]| {
            std::hint::black_box(r::indentation_width::check_indentation_width(
                s,
                cfg.indentation_width,
                &[],
                &[],
            ));
        }),
        e!("predicate_prefix", |s: &[u8]| {
            std::hint::black_box(r::predicate_prefix::check_predicate_prefix(
                s,
                &cfg.predicate_prefix_name_prefixes,
                &cfg.predicate_prefix_macros,
            ));
        }),
        e!("closing_parenthesis_indentation", |s: &[u8]| {
            std::hint::black_box(
                r::closing_parenthesis_indentation::check_closing_parenthesis_indentation(
                    s,
                    cfg.closing_paren_indent,
                ),
            );
        }),
        e!("first_array_element_indentation", |s: &[u8]| {
            std::hint::black_box(
                r::first_array_element_indentation::check_first_array_element_indentation(
                    s,
                    cfg.first_array_element_style,
                    cfg.first_array_element_indent,
                    cfg.first_array_element_enforce_fixed,
                ),
            );
        }),
        e!("hash_each_methods", |s: &[u8]| {
            std::hint::black_box(r::hash_each_methods::check_hash_each_methods(
                s,
                &cfg.hash_each_allowed_receivers,
            ));
        }),
        e!("void", |s: &[u8]| {
            std::hint::black_box(r::void::check_void(s, cfg.void_check_nonmutating));
        }),
        e!("useless_access_modifier", |s: &[u8]| {
            std::hint::black_box(r::useless_access_modifier::check_useless_access_modifier(
                s,
                &cfg.useless_access_modifier_context_creating,
                &cfg.useless_access_modifier_method_creating,
                cfg.useless_access_modifier_active_support,
            ));
        }),
        e!("empty_lines_around_body", |s: &[u8]| {
            std::hint::black_box(r::empty_lines_around_body::check_empty_lines_around_body(
                s,
                cfg.empty_lines_around_body,
            ));
        }),
        e!("block_delimiters", |s: &[u8]| {
            std::hint::black_box(r::block_delimiters::check_block_delimiters_events(
                s,
                &cfg.block_delimiters,
            ));
        }),
        e!("abc_size", |s: &[u8]| {
            std::hint::black_box(r::abc_size::check_abc_size(
                s,
                cfg.abc_size_max_floor,
                cfg.abc_size_discount_repeated,
                cfg.abc_size_it_is_send,
            ));
        }),
        e!("indentation_consistency", |s: &[u8]| {
            std::hint::black_box(r::indentation_consistency::check_indentation_consistency(
                s,
                cfg.indentation_consistency,
            ));
        }),
        e!("empty_line_between_defs", |s: &[u8]| {
            std::hint::black_box(r::empty_line_between_defs::check_empty_line_between_defs(
                s,
                cfg.empty_line_between_defs.clone(),
            ));
        }),
        e!("end_alignment", |s: &[u8]| {
            std::hint::black_box(r::end_alignment::check_end_alignment(s, cfg.end_alignment));
        }),
        e!("block_alignment", |s: &[u8]| {
            std::hint::black_box(r::block_alignment::check_block_alignment(
                s,
                cfg.block_alignment,
            ));
        }),
        e!("else_alignment", |s: &[u8]| {
            std::hint::black_box(r::else_alignment::check_else_alignment(
                s,
                cfg.else_alignment,
            ));
        }),
        e!("first_hash_element_indentation", |s: &[u8]| {
            std::hint::black_box(
                r::first_hash_element_indentation::check_first_hash_element_indentation(
                    s,
                    cfg.first_hash_element_style,
                    cfg.first_hash_element_indent,
                    cfg.first_hash_element_enforce_fixed,
                    cfg.first_hash_element_separators,
                ),
            );
        }),
        e!("hash_alignment", |s: &[u8]| {
            std::hint::black_box(r::hash_alignment::check_hash_alignment(
                s,
                &cfg.hash_alignment,
            ));
        }),
        e!("empty_lines_around_arguments", |s: &[u8]| {
            std::hint::black_box(
                r::empty_lines_around_arguments::check_empty_lines_around_arguments(s),
            );
        }),
        e!("hash_syntax", |s: &[u8]| {
            std::hint::black_box(r::hash_syntax::check_hash_syntax(s, &cfg.hash_syntax));
        }),
        e!("string_literals", |s: &[u8]| {
            std::hint::black_box(r::string_literals::check_string_literals(
                s,
                &cfg.string_literals,
            ));
        }),
        e!("trailing_comma_in_arguments", |s: &[u8]| {
            std::hint::black_box(
                r::trailing_comma_in_arguments::check_trailing_comma_in_arguments(
                    s,
                    &cfg.trailing_comma_in_arguments,
                ),
            );
        }),
        e!("string_literals_in_interpolation", |s: &[u8]| {
            std::hint::black_box(
                r::string_literals_in_interpolation::check_string_literals_in_interpolation(
                    s,
                    &cfg.string_literals_in_interpolation,
                ),
            );
        }),
        e!("trailing_empty_lines", |s: &[u8]| {
            std::hint::black_box(r::trailing_empty_lines::check_trailing_empty_lines(
                s,
                &cfg.trailing_empty_lines,
            ));
        }),
        e!("space_around_method_call_operator", |s: &[u8]| {
            std::hint::black_box(
                r::space_around_method_call_operator::check_space_around_method_call_operator(s),
            );
        }),
        e!("space_around_keyword", |s: &[u8]| {
            std::hint::black_box(r::space_around_keyword::check_space_around_keyword(s));
        }),
        e!("space_inside_block_braces", |s: &[u8]| {
            std::hint::black_box(r::space_inside_block_braces::check_space_inside_block_braces(
                s,
                cfg.space_inside_block_braces,
            ));
        }),
        e!("space_inside_hash_literal_braces", |s: &[u8]| {
            std::hint::black_box(
                r::space_inside_hash_literal_braces::check_space_inside_hash_literal_braces(
                    s,
                    cfg.space_inside_hash_literal_braces,
                ),
            );
        }),
        e!("space_inside_array_literal_brackets", |s: &[u8]| {
            std::hint::black_box(
                r::space_inside_array_literal_brackets::check_space_inside_array_literal_brackets(
                    s,
                    cfg.space_inside_array_literal_brackets,
                ),
            );
        }),
        e!("space_before_block_braces", |s: &[u8]| {
            std::hint::black_box(r::space_before_block_braces::check_space_before_block_braces(
                s,
                cfg.space_before_block_braces,
            ));
        }),
        e!("method_length", |s: &[u8]| {
            std::hint::black_box(r::method_length::check_method_length(
                s,
                cfg.method_length_max,
                cfg.method_length_count_comments,
                &cfg.method_length_count_as_one,
            ));
        }),
        e!("def_end_alignment", |s: &[u8]| {
            std::hint::black_box(r::def_end_alignment::check_def_end_alignment(
                s,
                cfg.def_end_alignment,
            ));
        }),
        e!("require_parentheses", |s: &[u8]| {
            std::hint::black_box(r::require_parentheses::check_require_parentheses(s));
        }),
        e!("self_assignment", |s: &[u8]| {
            std::hint::black_box(r::self_assignment::check_self_assignment(s));
        }),
        e!("nested_parenthesized_calls", |s: &[u8]| {
            std::hint::black_box(r::nested_parenthesized_calls::check_nested_parenthesized_calls(
                s,
                &cfg.nested_parenthesized_calls_allowed_methods,
            ));
        }),
        e!("parentheses_as_grouped_expression", |s: &[u8]| {
            std::hint::black_box(
                r::parentheses_as_grouped_expression::check_parentheses_as_grouped_expression(s),
            );
        }),
        e!("percent_literal_delimiters", |s: &[u8]| {
            std::hint::black_box(r::percent_literal_delimiters::check_percent_literal_delimiters(
                s,
                &cfg.percent_literal_delimiters,
            ));
        }),
        e!("multiline_method_call_brace_layout", |s: &[u8]| {
            std::hint::black_box(
                r::multiline_method_call_brace_layout::check_multiline_method_call_brace_layout(
                    s,
                    cfg.multiline_method_call_brace_style,
                ),
            );
        }),
        e!("access_modifier_indentation", |s: &[u8]| {
            std::hint::black_box(
                r::access_modifier_indentation::check_access_modifier_indentation(
                    s,
                    cfg.access_modifier_indentation,
                ),
            );
        }),
        e!("assignment_indentation", |s: &[u8]| {
            std::hint::black_box(r::assignment_indentation::check_assignment_indentation(
                s,
                cfg.assignment_indentation,
            ));
        }),
        e!("redundant_self_assignment", |s: &[u8]| {
            std::hint::black_box(r::redundant_self_assignment::check_redundant_self_assignment(s));
        }),
        e!("colon_method_call", |s: &[u8]| {
            std::hint::black_box(r::colon_method_call::check_colon_method_call(s));
        }),
        e!("stabby_lambda_parentheses", |s: &[u8]| {
            std::hint::black_box(r::stabby_lambda_parentheses::check_stabby_lambda_parentheses(
                s,
                cfg.stabby_lambda_parentheses,
            ));
        }),
        e!("unreachable_code", |s: &[u8]| {
            std::hint::black_box(r::unreachable_code::check_unreachable_code(s));
        }),
        e!("hash_transform_keys", |s: &[u8]| {
            std::hint::black_box(r::hash_transform_keys::check_hash_transform_keys(s));
        }),
        e!("ambiguous_block_association", |s: &[u8]| {
            let c = &cfg.ambiguous_block_association;
            std::hint::black_box(r::ambiguous_block_association::check_ambiguous_block_association(
                s,
                &c.allowed_methods,
                &[],
            ));
        }),
        e!("empty_line_after_guard_clause", |s: &[u8]| {
            std::hint::black_box(
                r::empty_line_after_guard_clause::check_empty_line_after_guard_clause(s),
            );
        }),
        e!("empty_comment", |s: &[u8]| {
            std::hint::black_box(r::empty_comment::check_empty_comment(s, cfg.empty_comment));
        }),
        e!("empty_line_after_magic_comment", |s: &[u8]| {
            std::hint::black_box(
                r::empty_line_after_magic_comment::check_empty_line_after_magic_comment(s),
            );
        }),
        e!("empty_lines", |s: &[u8]| {
            std::hint::black_box(r::empty_lines::check_empty_lines(s));
        }),
        e!("leading_empty_lines", |s: &[u8]| {
            std::hint::black_box(r::leading_empty_lines::check_leading_empty_lines(s));
        }),
    ];

    let mut totals = vec![f64::MAX; entries.len()];
    for _ in 0..iterations {
        let mut iter_totals = vec![0.0f64; entries.len()];
        for s in sources {
            // Warm the parse cache so every entry below hits it.
            shirobai_core::rules::parse_cache::with_parsed(s, |_, _| ());
            for (i, (_, f)) in entries.iter().enumerate() {
                let t0 = cpu_time_secs();
                f(s);
                iter_totals[i] += cpu_time_secs() - t0;
            }
        }
        for (t, it) in totals.iter_mut().zip(&iter_totals) {
            *t = t.min(*it);
        }
    }
    let mut rows: Vec<(&str, f64)> = entries
        .iter()
        .map(|(n, _)| *n)
        .zip(totals.iter().copied())
        .collect();
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let sum: f64 = totals.iter().sum();
    println!("perrule total (min over {iterations} iters): {sum:.3}s");
    for (name, t) in rows {
        println!("{t:8.3}  {name}");
    }
}

fn run_pass(mode: &str, n_rules: usize, sources: &[Vec<u8>], cfg: &BundleConfig) -> usize {
    let mut sink = 0usize;
    match mode {
        "parse" => {
            for s in sources {
                let result = parse(s);
                sink = sink.wrapping_add(result.node().location().end_offset());
            }
        }
        "walk" => {
            for s in sources {
                let mut noops: Vec<NoopRule> = (0..n_rules).map(|_| NoopRule { sink: 0 }).collect();
                let mut refs: Vec<&mut dyn Rule> =
                    noops.iter_mut().map(|r| r as &mut dyn Rule).collect();
                dispatch::run(s, &mut refs);
                sink = sink.wrapping_add(noops.iter().map(|r| r.sink).sum::<usize>());
            }
        }
        "bundle" => {
            for s in sources {
                let result = rules::bundle::check_all_bundle(s, cfg);
                sink = sink.wrapping_add(result.line_length.len());
                sink = sink.wrapping_add(result.indentation_width.len());
            }
        }
        "guard" => {
            for s in sources {
                let out = rules::empty_line_after_guard_clause::check_empty_line_after_guard_clause(s);
                sink = sink.wrapping_add(out.len());
            }
        }
        "mname" => {
            for s in sources {
                let out = rules::method_name::check_method_name_filtered(s, 0, true);
                sink = sink.wrapping_add(out.0.len());
            }
        }
        "linelen" => {
            for s in sources {
                let out = rules::line_length::check_line_length(s, 120, 2);
                sink = sink.wrapping_add(out.len());
            }
        }
        other => {
            eprintln!("unknown mode: {other}");
            std::process::exit(2);
        }
    }
    sink
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: bundle_profile <mode> <corpus_dir> <packed_config_file> [iterations]");
        std::process::exit(2);
    }
    let mode_arg = args[1].as_str();
    let corpus = PathBuf::from(&args[2]);
    let (nums, lists) = read_packed(&args[3]);
    let iterations: usize = args.get(4).map_or(3, |v| v.parse().expect("iterations"));
    let cfg = BundleConfig::from_packed(&nums, lists).expect("valid packed config");

    let (mode, n_rules) = match mode_arg.strip_prefix("walkN:") {
        Some(n) => ("walk", n.parse().expect("walkN count")),
        None if mode_arg == "walk0" => ("walk", 1),
        None => (mode_arg, 0),
    };

    let mut files = Vec::new();
    collect_rb_files(&corpus, &mut files);
    files.sort();
    let sources: Vec<Vec<u8>> = files
        .iter()
        .filter_map(|p| std::fs::read(p).ok())
        .collect();
    eprintln!("files: {}", sources.len());

    if mode == "perrule" {
        run_perrule(&sources, &cfg, iterations);
        return;
    }

    // Warm-up pass (untimed): touch every source once.
    let warm = run_pass(mode, n_rules, &sources, &cfg);

    let mut times: Vec<f64> = (0..iterations)
        .map(|_| {
            let t0 = cpu_time_secs();
            let sink = run_pass(mode, n_rules, &sources, &cfg);
            let dt = cpu_time_secs() - t0;
            eprintln!("  iter: {dt:.3}s (sink {sink})");
            dt
        })
        .collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = times[times.len() / 2];
    eprintln!(
        "min {:.3}s / max {:.3}s / warm sink {warm}",
        times[0],
        times[times.len() - 1]
    );
    println!("{mode_arg}\t{median:.3}");
}
