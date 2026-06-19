//! Standalone CPU-time profiler for the shared-walk investigation.
//!
//! Measures, over the whole Mastodon `.rb` corpus, the cost of three modes:
//!
//! * `parse` — parse each file with ruby-prism, discard the result. → P
//! * `walk1` — parse + one full recursive traversal that pushes a generic
//!   ancestor frame `(node_kind, start, end)` on enter and pops on
//!   leave (no analysis). → time(walk1); W = walk1 − P
//! * `walk17` — parse + the same full traversal repeated 17× per file
//!   (models 17 cops each walking independently). → time(walk17);
//!   Z = walk17 − P
//!
//! The traversal uses the same `Visit` / `visit_branch_node_enter` /
//! `visit_branch_node_leave` push/pop pattern the real cops use (see
//! `rules/redundant_self.rs`): a frame is pushed for every *branch* node on
//! enter and popped on leave; leaf nodes are not pushed (identical to the real
//! cops, whose generic hook only fires for branch nodes).
//!
//! Usage: walk_profile <parse|walk1|walk17> <corpus_dir> [iterations]
//! Prints the median CPU time (seconds) over `iterations` repeats to stdout,
//! plus min/max to stderr. CPU time is `CLOCK_PROCESS_CPUTIME_ID`.

use std::mem::discriminant;
use std::path::PathBuf;

use ruby_prism::{Node, Visit, parse};

/// A generic ancestor frame entry: an opaque node-kind discriminant plus the
/// node's byte range. Mirrors the `(node_kind, start_offset, end_offset)` frame
/// the real cops build.
struct FrameEntry {
    kind: u64,
    start: usize,
    end: usize,
}

/// Push/pop-only visitor: builds a generic ancestor stack with no analysis.
struct WalkVisitor {
    frame: Vec<FrameEntry>,
    /// Touched so the optimizer cannot eliminate the push/pop work.
    sink: usize,
}

impl<'pr> Visit<'pr> for WalkVisitor {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        // `discriminant` is the cheapest available node-kind tag on the public
        // enum; transmute to a comparable integer purely for the frame tuple.
        let kind = {
            let d = discriminant(&node);
            // discriminant is Copy + Eq but not numeric; hash its bit pattern.
            // SAFETY: Discriminant<Node> is a thin tag; read its bytes.
            let bytes = unsafe {
                std::slice::from_raw_parts((&d as *const _) as *const u8, std::mem::size_of_val(&d))
            };
            bytes.iter().fold(0u64, |a, &b| (a << 8) | b as u64)
        };
        let loc = node.location();
        self.frame.push(FrameEntry {
            kind,
            start: loc.start_offset(),
            end: loc.end_offset(),
        });
    }

    fn visit_branch_node_leave(&mut self) {
        if let Some(e) = self.frame.pop() {
            // Cheap touch to defeat dead-code elimination.
            self.sink = self.sink.wrapping_add(e.kind as usize ^ e.start ^ e.end);
        }
    }
}

fn walk_once(node: &Node<'_>) -> usize {
    let mut v = WalkVisitor {
        frame: Vec::new(),
        sink: 0,
    };
    v.visit(node);
    v.sink
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
            } else if path.extension().is_some_and(|e| e == "rb")
                && let Ok(bytes) = std::fs::read(&path)
            {
                out.push(bytes);
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
    // Fallback: wall clock. (Reported in the harness when libc/unix unavailable.)
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

fn run_mode(mode: &str, sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    match mode {
        "parse" => {
            for src in sources {
                let result = parse(src);
                // Touch the root so parse is not optimized away.
                sink = sink.wrapping_add(result.node().location().end_offset());
            }
        }
        "walk1" => {
            for src in sources {
                let result = parse(src);
                sink = sink.wrapping_add(walk_once(&result.node()));
            }
        }
        "walk17" => {
            for src in sources {
                let result = parse(src);
                let root = result.node();
                for _ in 0..17 {
                    sink = sink.wrapping_add(walk_once(&root));
                }
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
    if args.len() < 3 {
        eprintln!("usage: walk_profile <parse|walk1|walk17> <corpus_dir> [iterations]");
        std::process::exit(2);
    }
    let mode = &args[1];
    let dir = &args[2];
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);

    let sources = collect_rb_files(dir);
    if sources.is_empty() {
        eprintln!("no .rb files found under {dir}");
        std::process::exit(1);
    }
    eprintln!("mode={mode} files={} iterations={iters}", sources.len());

    let mut times = Vec::with_capacity(iters);
    let mut sink_acc = 0usize;
    for _ in 0..iters {
        let t0 = cpu_time_secs();
        sink_acc = sink_acc.wrapping_add(run_mode(mode, &sources));
        let t1 = cpu_time_secs();
        times.push(t1 - t0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = times[times.len() / 2];
    let min = times[0];
    let max = times[times.len() - 1];
    // Median to stdout (script-consumable), min/max + sink to stderr.
    println!("{median:.6}");
    eprintln!("median={median:.6} min={min:.6} max={max:.6} sink={sink_acc}");
}
