//! SPIKE harness (measurement / verification only). Three subcommands:
//!
//!   dump-tokens <corpus> <out_file>
//!       Per .rb file: `# <path>` then one `name\tstart\tlen\tstate` line per
//!       token. Diff against the Ruby `Prism.lex` oracle (oracle_lex.rb).
//!
//!   canary <corpus>
//!       For every .rb file compare the AST reached via the transmute path
//!       (parse_with_lex) against a normal ruby_prism::parse(). Reports files,
//!       AST fingerprint mismatches, and comment-count mismatches.
//!
//!   profile <parse|lex|reparse> <corpus> [iters]
//!       CLOCK_PROCESS_CPUTIME_ID median over iters (default 5). Modes:
//!         parse   - ruby_prism::parse only (AST, no tokens)
//!         lex     - parse_with_lex (AST + in-line token collection)
//!         reparse - two full parses (reproduces the pm_serialize_lex tax:
//!                   AST from one parse, tokens from a second full parse)

use std::io::Write;
use std::path::{Path, PathBuf};

use ruby_prism::{Node, Visit};
use shirobai_core::pm_lex_spike::{RawToken, parse_with_lex, parse_with_lex_into};

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

fn sorted_rb_files(corpus: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rb_files(corpus, &mut files);
    files.sort();
    files
}

// Pre-order (start,end) offset fingerprint of an AST via the Visit trait.
struct Fingerprint(Vec<(usize, usize)>);
impl<'pr> Visit<'pr> for Fingerprint {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        let l = node.location();
        self.0.push((l.start_offset(), l.end_offset()));
    }
    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        let l = node.location();
        self.0.push((l.start_offset(), l.end_offset()));
    }
}
fn fingerprint(root: &Node<'_>) -> Vec<(usize, usize)> {
    let mut fp = Fingerprint(Vec::new());
    fp.visit(root);
    fp.0
}

fn dump_tokens(corpus: &Path, out_file: &Path) {
    let files = sorted_rb_files(corpus);
    let mut out = std::io::BufWriter::new(std::fs::File::create(out_file).expect("create out"));
    for path in &files {
        let Ok(src) = std::fs::read(path) else {
            continue;
        };
        let rel = path.strip_prefix(corpus).unwrap_or(path);
        writeln!(out, "# {}", rel.display()).unwrap();
        let (_result, tokens) = parse_with_lex(&src);
        for t in &tokens {
            writeln!(
                out,
                "{}\t{}\t{}\t{}",
                t.type_name(),
                t.start_offset,
                t.length,
                t.lex_state
            )
            .unwrap();
        }
    }
    eprintln!("dumped {} files -> {}", files.len(), out_file.display());
}

fn canary(corpus: &Path) {
    let files = sorted_rb_files(corpus);
    let mut n_ok = 0usize;
    let mut ast_mismatch = 0usize;
    let mut comment_mismatch = 0usize;
    let mut first_bad: Vec<String> = Vec::new();
    for path in &files {
        let Ok(src) = std::fs::read(path) else {
            continue;
        };
        let normal = ruby_prism::parse(&src);
        let fp_normal = fingerprint(&normal.node());
        let n_comments_normal = normal.comments().count();
        drop(normal);

        let (spiked, _tokens) = parse_with_lex(&src);
        let fp_spiked = fingerprint(&spiked.node());
        let n_comments_spiked = spiked.comments().count();
        drop(spiked); // exercise Drop of a transmuted ParseResult.

        let ast_ok = fp_normal == fp_spiked;
        let com_ok = n_comments_normal == n_comments_spiked;
        if ast_ok && com_ok {
            n_ok += 1;
        } else {
            if !ast_ok {
                ast_mismatch += 1;
            }
            if !com_ok {
                comment_mismatch += 1;
            }
            if first_bad.len() < 10 {
                first_bad.push(format!(
                    "{} ast_ok={} nodes {}/{} comments {}/{}",
                    path.display(),
                    ast_ok,
                    fp_normal.len(),
                    fp_spiked.len(),
                    n_comments_normal,
                    n_comments_spiked
                ));
            }
        }
    }
    println!("canary files={}", files.len());
    println!("  parity_ok           = {n_ok}");
    println!("  ast_fingerprint_bad = {ast_mismatch}");
    println!("  comment_count_bad   = {comment_mismatch}");
    for b in &first_bad {
        println!("  BAD {b}");
    }
    let parity = if files.is_empty() {
        0.0
    } else {
        100.0 * n_ok as f64 / files.len() as f64
    };
    println!("  parity = {parity:.2}%");
}

fn profile(mode: &str, corpus: &Path, iters: usize) {
    let files = sorted_rb_files(corpus);
    let sources: Vec<Vec<u8>> = files.iter().filter_map(|p| std::fs::read(p).ok()).collect();
    eprintln!("files: {}", sources.len());

    let mut buf: Vec<RawToken> = Vec::new();
    let run = |mode: &str, buf: &mut Vec<RawToken>| -> usize {
        let mut sink = 0usize;
        for s in &sources {
            match mode {
                "parse" => {
                    let r = ruby_prism::parse(s);
                    sink = sink.wrapping_add(r.node().location().end_offset());
                }
                "lex" => {
                    let (r, tokens) = parse_with_lex(s);
                    sink = sink.wrapping_add(r.node().location().end_offset());
                    sink = sink.wrapping_add(tokens.len());
                }
                "lex_reuse" => {
                    // Reuse one token buffer across files (retains capacity):
                    // isolates pure callback+push cost from per-file heap alloc.
                    let r = parse_with_lex_into(s, buf);
                    sink = sink.wrapping_add(r.node().location().end_offset());
                    sink = sink.wrapping_add(buf.len());
                }
                "reparse" => {
                    // AST from one parse; tokens would come from a *second*
                    // full parse (the pm_serialize_lex tax being reproduced).
                    let r = ruby_prism::parse(s);
                    sink = sink.wrapping_add(r.node().location().end_offset());
                    let r2 = ruby_prism::parse(s);
                    sink = sink.wrapping_add(r2.node().location().end_offset());
                }
                other => {
                    eprintln!("unknown profile mode: {other}");
                    std::process::exit(2);
                }
            }
        }
        sink
    };

    // Warm-up (untimed).
    let warm = run(mode, &mut buf);
    let mut times: Vec<f64> = (0..iters)
        .map(|_| {
            let t0 = cpu_time_secs();
            let sink = run(mode, &mut buf);
            let dt = cpu_time_secs() - t0;
            eprintln!("  iter {mode}: {dt:.4}s (sink {sink})");
            dt
        })
        .collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = times[times.len() / 2];
    eprintln!("warm sink {warm}");
    println!("{mode}\t{median:.4}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("dump-tokens") => {
            let corpus = PathBuf::from(args.get(2).expect("corpus dir"));
            let out = PathBuf::from(args.get(3).expect("out file"));
            dump_tokens(&corpus, &out);
        }
        Some("canary") => {
            let corpus = PathBuf::from(args.get(2).expect("corpus dir"));
            canary(&corpus);
        }
        Some("profile") => {
            let mode = args.get(2).expect("mode").clone();
            let corpus = PathBuf::from(args.get(3).expect("corpus dir"));
            let iters: usize = args.get(4).map_or(5, |v| v.parse().expect("iters"));
            profile(&mode, &corpus, iters);
        }
        _ => {
            eprintln!(
                "usage:\n  pm_lex_spike dump-tokens <corpus> <out_file>\n  pm_lex_spike canary <corpus>\n  pm_lex_spike profile <parse|lex|reparse> <corpus> [iters]"
            );
            std::process::exit(2);
        }
    }
}
