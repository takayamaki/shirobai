//! Permanent CI gate for the `pm_lex` transmute.
//!
//! `pm_lex::parse_with_lex` transmutes a hand-written look-alike into
//! `ruby_prism::ParseResult`, betting on that struct's private field order.
//! The static assert in `pm_lex` catches a size/alignment drift at compile
//! time, but a pure field *reorder* by a new rustc would slip past it. This
//! test is the field-order guard: it parses the whole vendored RuboCop source
//! both ways and asserts the ASTs (and comment counts) are identical. It runs
//! in the normal `cargo test` build the CI already does with submodules
//! checked out, so it gates every toolchain bump without touching any workflow
//! file.
//!
//! The whole `vendor/rubocop` tree is in scope; it runs well under the debug
//! time budget. If it ever grows too slow, narrow `CORPUS_SUBDIR` to `lib`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ruby_prism::{Node, Visit};
use shirobai_core::pm_lex::parse_with_lex;

/// Subdirectory of `vendor/rubocop` to scan. Empty means the whole tree.
const CORPUS_SUBDIR: &str = "";

// Pre-order (start,end) offset fingerprint of an AST via the Visit trait.
// Same shape as the `pm_lex_probe` canary subcommand.
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

fn corpus_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.push("../../vendor/rubocop");
    if !CORPUS_SUBDIR.is_empty() {
        root.push(CORPUS_SUBDIR);
    }
    root
}

#[test]
fn transmute_path_ast_matches_normal_parse_on_vendored_rubocop() {
    let root = corpus_root();
    let mut files = Vec::new();
    collect_rb_files(&root, &mut files);
    files.sort();

    assert!(
        !files.is_empty(),
        "no .rb files under {}. The vendor/rubocop submodule is not checked out; \
         run `git submodule update --init vendor/rubocop` before `cargo test`.",
        root.display()
    );

    let started = Instant::now();
    let mut mismatches: Vec<String> = Vec::new();
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

        if fp_normal != fp_spiked || n_comments_normal != n_comments_spiked {
            mismatches.push(format!(
                "{}: nodes {}/{} comments {}/{}",
                path.display(),
                fp_normal.len(),
                fp_spiked.len(),
                n_comments_normal,
                n_comments_spiked,
            ));
        }
    }
    let elapsed = started.elapsed();

    assert!(
        mismatches.is_empty(),
        "pm_lex transmute path diverged from ruby_prism::parse on {} of {} files \
         (the ParseResult field-order bet may have broken). First few:\n{}",
        mismatches.len(),
        files.len(),
        mismatches
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
    );

    eprintln!(
        "pm_lex canary: {} files, {:.2}s (debug build)",
        files.len(),
        elapsed.as_secs_f64(),
    );
}
