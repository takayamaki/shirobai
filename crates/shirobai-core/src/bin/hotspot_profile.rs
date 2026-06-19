//! Hotspot decomposition profiler (MEASUREMENT ONLY — does not change real cops).
//!
//! Breaks the heavy cops (first_argument_indentation, indentation_width,
//! multiline_bundle, dot_position) into internal phases and measures each in
//! CPU time over the corpus, so we can attribute the pure-analysis cost to
//! specific helpers/structures rather than treating it as opaque.
//!
//! Phases measured per relevant cop:
//! * `parse` — parse-only baseline (P)
//! * `walk_noop` — dispatch::run with an empty rule (shared traversal cost:
//!   parse-cache hit + tree walk + per-node virtual dispatch).
//!   This is the *shared* walk cost 16 cops duplicate.
//! * `frame_<cop>` — walk + replicate that cop's per-node frame extraction
//!   (prism accessors + the same Vec allocations) but NO
//!   offense analysis. (full − frame) ≈ analysis body;
//!   (frame − walk_noop) ≈ frame-construction overhead.
//! * `full_<cop>` — the real check_* (delegates to rules; unchanged logic).
//!
//! Microbenchmarks (`micro_*`) isolate the cost of the O(n)-prefix line/column
//! helpers as currently written vs. an O(1) precomputed line-index, by calling
//! them on the *real* offsets every CallNode in the corpus produces.
//!
//! Usage: hotspot_profile <mode> <corpus_dir> [iterations]

use std::path::PathBuf;

use ruby_prism::{Node, Visit, parse};
use shirobai_core::rules;
use shirobai_core::rules::dispatch::{self, Rule};

// ----------------- timing -----------------

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
#[cfg(not(unix))]
fn cpu_time_secs() -> f64 {
    0.0
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

// ----------------- a line index (the proposed O(1) replacement) -----------------

/// Precomputed sorted byte offsets of each line start. Built once per source
/// in O(n); answers line_of / line_start in O(log n) (or O(1) amortised).
struct LineIndex {
    starts: Vec<usize>,
}
impl LineIndex {
    fn build(src: &[u8]) -> Self {
        let mut starts = Vec::with_capacity(src.len() / 24 + 1);
        starts.push(0);
        for (i, &b) in src.iter().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        LineIndex { starts }
    }
    /// 1-based line number of `off`.
    fn line_of(&self, off: usize) -> usize {
        // partition_point: index of first start > off  == line number (1-based).
        self.starts.partition_point(|&s| s <= off)
    }
}

// ----------------- current O(n)-prefix helpers (copied verbatim) -----------------

fn cur_line_of(src: &[u8], off: usize) -> usize {
    src[..off].iter().filter(|&&b| b == b'\n').count() + 1
}
// ----------------- no-op rule (shared walk cost) -----------------

struct NoopRule;
impl Rule for NoopRule {
    fn enter(&mut self, _node: &Node<'_>) {}
    fn leave(&mut self) {}
}

// ----------------- frame-only rules (walk + frame extraction, no analysis) -----

fn loc(l: &ruby_prism::Location<'_>) -> (usize, usize) {
    (l.start_offset(), l.end_offset())
}

/// Replicates first_argument_indentation::make_frame work (incl. name.to_vec()).
struct FrameFirstArg {
    sink: usize,
    stack: Vec<(usize, usize, Vec<u8>)>,
}
impl Rule for FrameFirstArg {
    fn enter(&mut self, node: &Node<'_>) {
        let l = node.location();
        let name = if let Some(c) = node.as_call_node() {
            let _paren = c
                .opening_loc()
                .map(|o| o.as_slice() == b"(")
                .unwrap_or(false);
            let _recv = c.receiver().map(|r| loc(&r.location()));
            let _end = c.closing_loc().map(|cl| cl.start_offset());
            c.name().as_slice().to_vec() // <-- per-call heap alloc in real code
        } else if let Some(s) = node.as_super_node() {
            let _ = s.lparen_loc().is_some();
            let _ = s.rparen_loc().map(|cl| cl.start_offset());
            Vec::new()
        } else if node.as_splat_node().is_some() || node.as_assoc_splat_node().is_some() {
            Vec::new()
        } else {
            let _ = node.as_arguments_node().is_some()
                || node.as_statements_node().is_some()
                || node.as_keyword_hash_node().is_some();
            Vec::new()
        };
        self.sink = self.sink.wrapping_add(name.len());
        self.stack.push((l.start_offset(), l.end_offset(), name));
    }
    fn leave(&mut self) {
        if let Some((s, _, _)) = self.stack.pop() {
            self.sink = self.sink.wrapping_add(s);
        }
    }
}

/// Replicates indentation_width::enter frame work, incl. the per-call
/// `line_of(receiver_end)` O(n) scan done DURING frame build.
struct FrameIndent<'s> {
    src: &'s [u8],
    sink: usize,
    depth: usize,
}
impl<'s> Rule for FrameIndent<'s> {
    fn enter(&mut self, node: &Node<'_>) {
        if node.as_arguments_node().is_some() {
            self.sink = self.sink.wrapping_add(1);
        } else if let Some(c) = node.as_call_node() {
            if c.receiver().is_none() {
                self.sink = self.sink.wrapping_add(node.location().start_offset());
            } else {
                let _dot = c.call_operator_loc().map(|l| l.start_offset());
                let _sel = c.message_loc().map(|l| l.start_offset());
                // The real code does exactly this O(n) scan per call frame:
                let rll = c
                    .receiver()
                    .map(|r| cur_line_of(self.src, r.location().end_offset().saturating_sub(1)));
                self.sink = self.sink.wrapping_add(rll.unwrap_or(0));
            }
        }
        self.depth += 1;
    }
    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}

/// Same as FrameIndent but uses a precomputed LineIndex for receiver_last_line.
struct FrameIndentIdx<'s> {
    idx: &'s LineIndex,
    sink: usize,
    depth: usize,
}
impl<'s> Rule for FrameIndentIdx<'s> {
    fn enter(&mut self, node: &Node<'_>) {
        if node.as_arguments_node().is_some() {
            self.sink = self.sink.wrapping_add(1);
        } else if let Some(c) = node.as_call_node() {
            if c.receiver().is_none() {
                self.sink = self.sink.wrapping_add(node.location().start_offset());
            } else {
                let _dot = c.call_operator_loc().map(|l| l.start_offset());
                let _sel = c.message_loc().map(|l| l.start_offset());
                let rll = c.receiver().map(|r| {
                    self.idx
                        .line_of(r.location().end_offset().saturating_sub(1))
                });
                self.sink = self.sink.wrapping_add(rll.unwrap_or(0));
            }
        }
        self.depth += 1;
    }
    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}

/// Replicates multiline_method_call_indentation::frame_for, incl. per-call
/// args Vec allocation.
struct FrameMc {
    sink: usize,
    stack: Vec<usize>,
}
impl Rule for FrameMc {
    fn enter(&mut self, node: &Node<'_>) {
        if let Some(c) = node.as_call_node() {
            let _paren = match c.closing_loc() {
                Some(close) if close.as_slice() == b")" => c
                    .opening_loc()
                    .map(|o| (o.start_offset(), close.end_offset())),
                _ => None,
            };
            let args: Vec<(usize, usize)> = c
                .arguments()
                .map(|a| {
                    a.arguments()
                        .iter()
                        .map(|arg| loc(&arg.location()))
                        .collect()
                })
                .unwrap_or_default();
            let _dot = c.call_operator_loc().map(|d| d.start_offset());
            let _sel = c.message_loc().as_ref().map(loc);
            self.sink = self.sink.wrapping_add(args.len());
        }
        self.stack.push(node.location().start_offset());
    }
    fn leave(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.sink = self.sink.wrapping_add(s);
        }
    }
}

// dot_position has its own (non-dispatch) walk; replicate the bare traversal +
// the per-dotted-call work UP TO the early-bail, both with current O(n) helpers
// and with the line-index, to show the saving.

struct DotWalkCur<'s> {
    src: &'s [u8],
    sink: usize,
}
impl<'pr, 's> Visit<'pr> for DotWalkCur<'s> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if let Some(dot) = node.call_operator_loc() {
            let dt = &self.src[dot.start_offset()..dot.end_offset()];
            if (dt == b"." || dt == b"&.")
                && node.receiver().is_some()
                && let Some(sel) = node
                    .message_loc()
                    .map(|m| m.start_offset())
                    .or_else(|| node.opening_loc().map(|o| o.start_offset()))
            {
                let recv_end = node.receiver().unwrap().location().end_offset();
                // The real is_offense() O(n) prefix scans (two of them) before bail:
                let a = cur_line_of(self.src, sel);
                let b = cur_line_of(self.src, recv_end);
                self.sink = self.sink.wrapping_add(a ^ b);
            }
        }
        ruby_prism::visit_call_node(self, node);
    }
}

struct DotWalkIdx<'s> {
    idx: &'s LineIndex,
    src: &'s [u8],
    sink: usize,
}
impl<'pr, 's> Visit<'pr> for DotWalkIdx<'s> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if let Some(dot) = node.call_operator_loc() {
            let dt = &self.src[dot.start_offset()..dot.end_offset()];
            if (dt == b"." || dt == b"&.")
                && node.receiver().is_some()
                && let Some(sel) = node
                    .message_loc()
                    .map(|m| m.start_offset())
                    .or_else(|| node.opening_loc().map(|o| o.start_offset()))
            {
                let recv_end = node.receiver().unwrap().location().end_offset();
                let a = self.idx.line_of(sel);
                let b = self.idx.line_of(recv_end);
                self.sink = self.sink.wrapping_add(a ^ b);
            }
        }
        ruby_prism::visit_call_node(self, node);
    }
}

// ----------------- phase runners -----------------

fn run_parse(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        let r = parse(s);
        sink = sink.wrapping_add(r.node().location().end_offset());
    }
    sink
}

fn run_walk_noop(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        let mut r = NoopRule;
        dispatch::run(s, &mut [&mut r]);
        sink = sink.wrapping_add(s.len());
    }
    sink
}

fn run_frame_firstarg(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        let mut r = FrameFirstArg {
            sink: 0,
            stack: Vec::new(),
        };
        dispatch::run(s, &mut [&mut r]);
        sink = sink.wrapping_add(r.sink);
    }
    sink
}

fn run_frame_indent(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        let mut r = FrameIndent {
            src: s,
            sink: 0,
            depth: 0,
        };
        dispatch::run(s, &mut [&mut r]);
        sink = sink.wrapping_add(r.sink);
    }
    sink
}

fn run_frame_indent_idx(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        let idx = LineIndex::build(s);
        let mut r = FrameIndentIdx {
            idx: &idx,
            sink: 0,
            depth: 0,
        };
        dispatch::run(s, &mut [&mut r]);
        sink = sink.wrapping_add(r.sink);
    }
    sink
}

fn run_frame_mc(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        let mut r = FrameMc {
            sink: 0,
            stack: Vec::new(),
        };
        dispatch::run(s, &mut [&mut r]);
        sink = sink.wrapping_add(r.sink);
    }
    sink
}

fn run_dot_walk_cur(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        let res = parse(s);
        let mut v = DotWalkCur { src: s, sink: 0 };
        v.visit(&res.node());
        sink = sink.wrapping_add(v.sink);
    }
    sink
}

fn run_dot_walk_idx(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        let res = parse(s);
        let idx = LineIndex::build(s);
        let mut v = DotWalkIdx {
            idx: &idx,
            src: s,
            sink: 0,
        };
        v.visit(&res.node());
        sink = sink.wrapping_add(v.sink);
    }
    sink
}

// --- full real cops (unchanged logic) ---

fn run_full_firstarg(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        sink = sink.wrapping_add(
            rules::first_argument_indentation::check_first_argument_indentation(s, 0, 2, false)
                .len(),
        );
    }
    sink
}

fn run_full_indent(sources: &[Vec<u8>]) -> usize {
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
    let mut sink = 0usize;
    for s in sources {
        sink = sink.wrapping_add(
            rules::indentation_width::check_indentation_width(s, cfg, &allowed, &prior).len(),
        );
    }
    sink
}

fn run_full_bundle(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        let (op, mc) = rules::bundle::check_multiline_bundle(s, 0, 2, 2, 0, 2, 2);
        sink = sink.wrapping_add(op.len()).wrapping_add(mc.len());
    }
    sink
}

fn run_full_dot(sources: &[Vec<u8>]) -> usize {
    let mut sink = 0usize;
    for s in sources {
        sink = sink.wrapping_add(rules::dot_position::check_dot_position(s, 0).len());
    }
    sink
}

// Microbench: count how many CallNode offsets a corpus produces and the
// total cost of cur_line_of vs index.line_of over ALL those offsets.

fn collect_call_offsets(sources: &[Vec<u8>]) -> Vec<(usize, Vec<usize>)> {
    struct Collect {
        offs: Vec<usize>,
    }
    impl<'pr> Visit<'pr> for Collect {
        fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
            self.offs.push(node.as_node().location().start_offset());
            if let Some(r) = node.receiver() {
                self.offs.push(r.location().end_offset());
            }
            ruby_prism::visit_call_node(self, node);
        }
    }
    sources
        .iter()
        .map(|s| {
            let res = parse(s);
            let mut c = Collect { offs: Vec::new() };
            c.visit(&res.node());
            (s.len(), c.offs)
        })
        .collect()
}

fn run_mode(mode: &str, sources: &[Vec<u8>]) -> usize {
    match mode {
        "parse" => run_parse(sources),
        "walk_noop" => run_walk_noop(sources),
        "frame_firstarg" => run_frame_firstarg(sources),
        "frame_indent" => run_frame_indent(sources),
        "frame_indent_idx" => run_frame_indent_idx(sources),
        "frame_mc" => run_frame_mc(sources),
        "full_firstarg" => run_full_firstarg(sources),
        "full_indent" => run_full_indent(sources),
        "full_bundle" => run_full_bundle(sources),
        "full_dot" => run_full_dot(sources),
        "dot_walk_cur" => run_dot_walk_cur(sources),
        "dot_walk_idx" => run_dot_walk_idx(sources),
        other => {
            eprintln!("unknown mode: {other}");
            std::process::exit(2);
        }
    }
}

fn time_mode(mode: &str, sources: &[Vec<u8>], iters: usize) -> f64 {
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
    eprintln!("mode={mode} median={median:.6} sink={sink}");
    median
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: hotspot_profile <mode|micro> <corpus_dir> [iterations]");
        std::process::exit(2);
    }
    let mode = args[1].clone();
    let dir = &args[2];
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
    let sources = collect_rb_files(dir);
    if sources.is_empty() {
        eprintln!("no .rb files under {dir}");
        std::process::exit(1);
    }
    eprintln!("files={} iters={iters}", sources.len());

    if mode == "micro" {
        // Cost of line_of over EVERY callnode offset: current O(n) vs index O(log n).
        let data = collect_call_offsets(&sources);
        let total_offs: usize = data.iter().map(|(_, o)| o.len()).sum();
        eprintln!("total callnode offsets={total_offs}");

        // current
        let mut sink = 0usize;
        let t0 = cpu_time_secs();
        for (_len, offs) in &data {
            // We need the source; recompute via index of data position.
            // (sources and data are parallel.)
            let _ = offs;
        }
        // Re-pair with sources for correctness.
        let t_cur = {
            let mut s = 0usize;
            let t0 = cpu_time_secs();
            for (src, (_l, offs)) in sources.iter().zip(&data) {
                for &o in offs {
                    s = s.wrapping_add(cur_line_of(src, o));
                }
            }
            let t1 = cpu_time_secs();
            sink = sink.wrapping_add(s);
            t1 - t0
        };
        let _ = t0;

        // index: build once per source then query
        let t_idx_build;
        let t_idx_query;
        {
            let mut s = 0usize;
            let t0 = cpu_time_secs();
            let indices: Vec<LineIndex> = sources.iter().map(|src| LineIndex::build(src)).collect();
            let t1 = cpu_time_secs();
            t_idx_build = t1 - t0;
            let t2 = cpu_time_secs();
            for (idx, (_l, offs)) in indices.iter().zip(&data) {
                for &o in offs {
                    s = s.wrapping_add(idx.line_of(o));
                }
            }
            let t3 = cpu_time_secs();
            t_idx_query = t3 - t2;
            sink = sink.wrapping_add(s);
        }
        eprintln!(
            "micro line_of: current_Onprefix={t_cur:.6}s  index_build={t_idx_build:.6}s  index_query={t_idx_query:.6}s  index_total={:.6}s  sink={sink}",
            t_idx_build + t_idx_query
        );
        println!("{t_cur:.6}\t{:.6}", t_idx_build + t_idx_query);
        return;
    }

    let m = time_mode(&mode, &sources, iters);
    println!("{m:.6}");
}
