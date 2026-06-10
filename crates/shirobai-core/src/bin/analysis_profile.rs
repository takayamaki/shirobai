//! Standalone CPU-time profiler for the pure-Rust analysis cost.
//!
//! Companion to `walk_profile.rs`. Where `walk_profile` measures a *generic*
//! traversal, this binary calls the **real** `shirobai_core::rules::*::check_*`
//! functions — the same pure-Rust analysis bodies the magnus wrappers in
//! `ext/shirobai/src/lib.rs` invoke — directly, with **no magnus boundary**.
//! Comparing the sum of these against the e2e Rust cost isolates the magnus
//! marshaling / value-copy overhead.
//!
//! Each `check_*` re-parses its source via `parse_cache::with_parsed` (cache
//! miss per distinct source), so a per-cop timing includes parse. The
//! `parse` mode measures parse-only (P); per-cop pure analysis = timing − P.
//!
//! Config values are RuboCop defaults (see `vendor/rubocop/config/default.yml`
//! and the Ruby cop wrappers in `lib/shirobai/cop/**`). The goal is cost
//! measurement, not offense exactness; values are chosen to exercise the real
//! analysis path, not to short-circuit it.
//!
//! Usage: analysis_profile <mode> <corpus_dir> [iterations]
//!   mode = parse | <cop-name> | all
//! Prints median CPU time (s) to stdout; min/max/sink to stderr.
//! CPU time is `CLOCK_PROCESS_CPUTIME_ID`.

use std::path::PathBuf;

use ruby_prism::parse;
use shirobai_core::rules;

/// All cop modes this profiler understands, in a stable order.
const COPS: &[&str] = &[
    "debugger",
    "block_length",
    "block_nesting",
    "complexity",
    "variable_number",
    "method_name",
    "safe_navigation_chain",
    "dot_position",
    "line_length",
    "line_length_breakables",
    "argument_alignment",
    "first_argument_indentation",
    "redundant_self",
    "indentation_width",
    "multiline_bundle",
];

// ----- config values (RuboCop defaults) -----

fn debugger_methods() -> Vec<String> {
    // Flattened Lint/Debugger DebuggerMethods (all groups).
    [
        "binding.irb",
        "Kernel.binding.irb",
        "byebug",
        "remote_byebug",
        "Kernel.byebug",
        "Kernel.remote_byebug",
        "page.save_and_open_page",
        "page.save_and_open_screenshot",
        "page.save_page",
        "page.save_screenshot",
        "save_and_open_page",
        "save_and_open_screenshot",
        "save_page",
        "save_screenshot",
        "binding.b",
        "binding.break",
        "Kernel.binding.b",
        "Kernel.binding.break",
        "binding.pry",
        "binding.remote_pry",
        "binding.pry_remote",
        "Kernel.binding.pry",
        "Kernel.binding.remote_pry",
        "Kernel.binding.pry_remote",
        "Pry.rescue",
        "pry",
        "debugger",
        "Kernel.debugger",
        "jard",
        "binding.console",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn debugger_requires() -> Vec<String> {
    ["debug/open", "debug/start"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn variable_number_allowed() -> Vec<String> {
    [
        "TLS1_1",
        "TLS1_2",
        "capture3",
        "iso8601",
        "rfc1123_date",
        "rfc822",
        "rfc2822",
        "rfc3339",
        "x86_64",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn safe_nav_nil_methods() -> Vec<String> {
    ["present?", "blank?", "presence", "try", "try!", "in?"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn kernel_methods() -> Vec<String> {
    // Snapshot of `Kernel.methods(false)` (Ruby 4.0). RedundantSelf supplies
    // this allow-list; a representative list keeps the analysis path live.
    [
        "Array", "Complex", "Float", "Hash", "Integer", "Pathname", "Rational", "String",
        "__callee__", "__dir__", "__method__", "`", "abort", "at_exit", "autoload", "autoload?",
        "binding", "block_given?", "caller", "caller_locations", "catch", "eval", "exec", "exit",
        "exit!", "fail", "fork", "format", "gets", "global_variables", "iterator?", "lambda",
        "load", "local_variables", "loop", "open", "p", "print", "printf", "proc", "putc", "puts",
        "raise", "rand", "readline", "readlines", "require", "require_relative", "select",
        "set_trace_func", "sleep", "spawn", "sprintf", "srand", "syscall", "system", "test",
        "throw", "trace_var", "trap", "untrace_var", "warn",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Run one cop's `check_*` over every source, returning an accumulated sink so
/// the optimizer cannot drop the call.
fn run_cop(cop: &str, sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    match cop {
        "debugger" => {
            let m = debugger_methods();
            let r = debugger_requires();
            for s in sources {
                sink = sink.wrapping_add(rules::debugger::check_debugger(s, &m, &r).len());
            }
        }
        "block_length" => {
            let cao: Vec<String> = Vec::new();
            for s in sources {
                sink = sink
                    .wrapping_add(rules::block_length::check_block_length(s, 25, false, &cao).len());
            }
        }
        "block_nesting" => {
            for s in sources {
                let (off, deep) = rules::block_nesting::check_block_nesting(s, 3, false, false);
                sink = sink.wrapping_add(off.len()).wrapping_add(deep);
            }
        }
        "complexity" => {
            for s in sources {
                sink = sink.wrapping_add(rules::complexity::check_complexity(s).len());
            }
        }
        "variable_number" => {
            let allowed = variable_number_allowed();
            // style=1 normalcase, flags = CheckMethodNames(2) | CheckSymbols(1) = 3
            for s in sources {
                let (off, had) = rules::variable_number::check_variable_number(s, 1, 3, &allowed);
                sink = sink.wrapping_add(off.len()).wrapping_add(had as usize);
            }
        }
        "method_name" => {
            // style=0 snake_case
            for s in sources {
                sink = sink.wrapping_add(rules::method_name::check_method_name(s, 0).len());
            }
        }
        "safe_navigation_chain" => {
            let nm = safe_nav_nil_methods();
            for s in sources {
                sink = sink.wrapping_add(
                    rules::safe_navigation_chain::check_safe_navigation_chain(s, &nm).len(),
                );
            }
        }
        "dot_position" => {
            // style=0 leading
            for s in sources {
                sink = sink.wrapping_add(rules::dot_position::check_dot_position(s, 0).len());
            }
        }
        "line_length" => {
            // max=120, tab_width=0
            for s in sources {
                sink = sink.wrapping_add(rules::line_length::check_line_length(s, 120, 0).len());
            }
        }
        "line_length_breakables" => {
            // max=120, split_strings=false
            for s in sources {
                sink = sink.wrapping_add(
                    rules::line_length_breakable::compute_breakables(s, 120, false).len(),
                );
            }
        }
        "argument_alignment" => {
            // style=0 with_first_argument, indent=2, incompatible=false
            for s in sources {
                sink = sink.wrapping_add(
                    rules::argument_alignment::check_argument_alignment(s, 0, 2, false).len(),
                );
            }
        }
        "first_argument_indentation" => {
            // style=0 special_for_inner_method_call_in_parentheses, indent=2, enforce=false
            for s in sources {
                sink = sink.wrapping_add(
                    rules::first_argument_indentation::check_first_argument_indentation(
                        s, 0, 2, false,
                    )
                    .len(),
                );
            }
        }
        "redundant_self" => {
            let km = kernel_methods();
            for s in sources {
                sink = sink
                    .wrapping_add(rules::redundant_self::check_redundant_self(s, &km).len());
            }
        }
        "indentation_width" => {
            // Config: width=2, relative_to_receiver=false, access_modifier_outdent=false,
            // indented_internal_methods=false, end_align=2(start_of_line),
            // def_end_align_def=false, use_tabs=false
            let cfg = rules::indentation_width::Config {
                width: 2,
                relative_to_receiver: false,
                access_modifier_outdent: false,
                indented_internal_methods: false,
                end_align: 2,
                def_end_align_def: false,
                use_tabs: false,
            };
            let allowed: Vec<usize> = Vec::new();
            let prior: Vec<(usize, usize)> = Vec::new();
            for s in sources {
                sink = sink.wrapping_add(
                    rules::indentation_width::check_indentation_width(s, cfg, &allowed, &prior)
                        .len(),
                );
            }
        }
        "multiline_bundle" => {
            // op aligned(0)/indent2/base2, mc aligned(0)/indent2/base2
            for s in sources {
                let (op, mc) = rules::bundle::check_multiline_bundle(s, 0, 2, 2, 0, 2, 2);
                sink = sink.wrapping_add(op.len()).wrapping_add(mc.len());
            }
        }
        other => {
            eprintln!("unknown cop: {other}");
            std::process::exit(2);
        }
    }
    sink
}

fn run_parse(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        let r = parse(s);
        sink = sink.wrapping_add(r.node().location().end_offset());
    }
    sink
}

fn collect_rb_files(dir: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::from(dir)];
    while let Some(p) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&p) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rb") {
                if let Ok(bytes) = std::fs::read(&path) {
                    out.push(bytes);
                }
            }
        }
    }
    out
}

#[cfg(unix)]
fn cpu_time_secs() -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: ts is a valid timespec.
    unsafe {
        libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts);
    }
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

#[cfg(not(unix))]
fn cpu_time_secs() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

fn run_mode(mode: &str, sources: &[Vec<u8>]) -> usize {
    if mode == "parse" {
        run_parse(sources)
    } else {
        run_cop(mode, sources)
    }
}

fn time_mode(mode: &str, sources: &[Vec<u8>], iters: usize) -> (f64, f64, f64) {
    let mut times = Vec::with_capacity(iters);
    let mut sink = 0usize;
    for _ in 0..iters {
        let t0 = cpu_time_secs();
        sink = sink.wrapping_add(run_mode(mode, sources));
        let t1 = cpu_time_secs();
        times.push(t1 - t0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = times[times.len() / 2];
    let min = times[0];
    let max = times[times.len() - 1];
    eprintln!(
        "mode={mode} median={median:.6} min={min:.6} max={max:.6} sink={sink}"
    );
    (median, min, max)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: analysis_profile <parse|<cop>|all> <corpus_dir> [iterations]");
        eprintln!("cops: parse {}", COPS.join(" "));
        std::process::exit(2);
    }
    let mode = args[1].clone();
    let dir = &args[2];
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);

    let sources = collect_rb_files(dir);
    if sources.is_empty() {
        eprintln!("no .rb files found under {dir}");
        std::process::exit(1);
    }
    eprintln!("files={} iterations={iters}", sources.len());

    if mode == "all" {
        // Convenience: run parse + every cop in one process (one timing each).
        let (p, _, _) = time_mode("parse", &sources, iters);
        println!("parse\t{p:.6}");
        for cop in COPS {
            let (m, _, _) = time_mode(cop, &sources, iters);
            println!("{cop}\t{m:.6}\t{:.6}", m - p);
        }
    } else {
        let (median, _, _) = time_mode(&mode, &sources, iters);
        println!("{median:.6}");
    }
}
