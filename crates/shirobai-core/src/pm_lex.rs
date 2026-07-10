//! Single-pass parse + lex: raw prism parse that also collects the token stream.
//!
//! Goal: get the same in-memory AST that `ruby_prism::parse()` produces AND the
//! token stream from the *same single parse*, without prism's `pm_serialize_lex`
//! tax (a second full parse just to get tokens).
//!
//! How: `ruby_prism::parse()` hard-wires `pm_parser_init` -> `pm_parse` with no
//! seam to install a `lex_callback`. We replicate that body ourselves with the
//! raw `ruby_prism_sys` symbols, set `parser.lex_callback` to a token-collecting
//! C callback in between, then hand the result back to safe `ruby_prism` code by
//! `transmute`-ing a layout-identical look-alike into `ruby_prism::ParseResult`
//! (whose fields are private and whose only constructor, `parse()`, cannot take
//! a callback).
//!
//! The transmute is a load-bearing bet on `ParseResult`'s field layout. Two
//! things guard it: [`assert_parse_result_layout`] is a `const` static assert on
//! size + alignment (a compile error if either drifts), and the canary tests
//! (here and in `tests/pm_lex_canary.rs`) compare the transmute-path AST against
//! a normal `parse()` on a large corpus, catching any residual field-order
//! change on a rustc update. A constructor that takes a `lex_callback` is being
//! proposed upstream to prism; once it lands this module can drop the transmute.

use std::mem::MaybeUninit;
use std::os::raw::c_void;
use std::ptr::NonNull;

use ruby_prism::ParseResult;
use ruby_prism_sys::{
    pm_lex_callback_t, pm_node_t, pm_parse, pm_parser_init, pm_parser_t, pm_token_t,
};

pub use crate::pm_lex_token_names::PM_TOKEN_NAMES;

/// One lexer token, collected in-line during the parse.
/// Byte offsets are from the start of the source (prism is byte-oriented).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawToken {
    /// prism `pm_token_type` integer. Name via [`RawToken::type_name`].
    pub token_type: u32,
    pub start_offset: usize,
    pub length: usize,
    /// `parser.lex_state` sampled at callback time (matches `Prism.lex`).
    pub lex_state: u32,
}

impl RawToken {
    /// Ruby `Prism::Token#type` symbol name for this token type.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        PM_TOKEN_NAMES
            .get(self.token_type as usize)
            .copied()
            .unwrap_or("")
    }
}

/// A look-alike of `ruby_prism::ParseResult<'pr>`.
///
/// `ParseResult`'s fields are private, so we cannot name them; we mirror their
/// declared order and types here and `transmute`. `ParseResult` is `repr(Rust)`
/// (no `repr(C)`), so field order is not guaranteed by the language — this only
/// works because two structs with identical field types get the same layout
/// under the current rustc, and we pin size+alignment with a static assert. If a
/// future rustc reorders fields, the canary tests break loudly.
///
/// Fields are only ever read through the `transmute` into `ParseResult`, so the
/// compiler cannot see the reads — hence `dead_code` is allowed here.
#[repr(Rust)]
#[allow(dead_code)]
struct ParseResultLookAlike<'pr> {
    source: &'pr [u8],
    parser: NonNull<pm_parser_t>,
    node: NonNull<pm_node_t>,
}

/// Compile-time guard: our look-alike must match `ParseResult` byte-for-byte in
/// size and alignment. (Field *order* is the residual, canary-tested bet.)
pub const fn assert_parse_result_layout() {
    use std::mem::{align_of, size_of};
    assert!(
        size_of::<ParseResultLookAlike<'_>>() == size_of::<ParseResult<'_>>(),
        "ParseResult size drifted from look-alike; transmute is unsafe"
    );
    assert!(
        align_of::<ParseResultLookAlike<'_>>() == align_of::<ParseResult<'_>>(),
        "ParseResult alignment drifted from look-alike; transmute is unsafe"
    );
}
const _: () = assert_parse_result_layout();

/// The C callback prism invokes for every lexed token. `data` is a
/// `*mut Vec<RawToken>`. Reads `parser.lex_state` at call time, mirroring how
/// `Prism.lex` records lex state.
///
/// # Safety
/// `data` must point to a live `Vec<RawToken>`; `parser`/`token` are valid for
/// the duration of the call (prism guarantees this while lexing).
unsafe extern "C" fn collect_token(
    data: *mut c_void,
    parser: *mut pm_parser_t,
    token: *mut pm_token_t,
) {
    // SAFETY: contract documented above; all derefs are of live prism/caller
    // memory for the span of this call.
    unsafe {
        let tokens = &mut *(data.cast::<Vec<RawToken>>());
        let tok = &*token;
        let source_start = (*parser).start;
        let start_offset = tok.start.offset_from(source_start) as usize;
        let length = tok.end.offset_from(tok.start) as usize;
        let lex_state = (*parser).lex_state;
        tokens.push(RawToken {
            token_type: tok.type_,
            start_offset,
            length,
            lex_state,
        });
    }
}

/// Like [`parse_with_lex`] but appends tokens into a caller-owned buffer
/// (cleared first, capacity retained). Lets a caller amortize the token Vec
/// allocation across files, isolating the pure callback+push cost from
/// per-file heap allocation. Returns the `ParseResult`.
///
/// # Safety
/// Uses raw prism FFI and a `transmute` into `ParseResult`. Sound as long as
/// `ParseResultLookAlike` matches `ParseResult`'s layout (guarded by the static
/// assert + canary tests) and prism's C ABI is stable.
#[must_use]
pub fn parse_with_lex_into<'s>(
    source: &'s [u8],
    out: &mut Vec<RawToken>,
) -> ParseResult<'s> {
    out.clear();
    let mut callback = pm_lex_callback_t {
        data: (out as *mut Vec<RawToken>).cast::<c_void>(),
        callback: Some(collect_token),
    };

    // SAFETY: replicate ruby_prism::parse()'s body verbatim, plus lex_callback.
    let look_alike = unsafe {
        let uninit = Box::new(MaybeUninit::<pm_parser_t>::uninit());
        let uninit = Box::into_raw(uninit);

        pm_parser_init(
            (*uninit).as_mut_ptr(),
            source.as_ptr(),
            source.len(),
            std::ptr::null(),
        );

        let parser = (*uninit).assume_init_mut();
        let parser = NonNull::new_unchecked(parser);

        (*parser.as_ptr()).lex_callback = &mut callback as *mut pm_lex_callback_t;

        let node = pm_parse(parser.as_ptr());
        let node = NonNull::new_unchecked(node);

        (*parser.as_ptr()).lex_callback = std::ptr::null_mut();

        ParseResultLookAlike {
            source,
            parser,
            node,
        }
    };

    // SAFETY: identical layout (static assert on size/align; canary on fields).
    unsafe { std::mem::transmute(look_alike) }
}

/// Parse `source` and collect its token stream in the *same* parse.
///
/// Returns a real `ruby_prism::ParseResult` (safe to walk / drop like any other)
/// plus the collected tokens. Mirrors `ruby_prism::parse()` exactly, only adding
/// the `lex_callback` wiring in between init and parse. Allocates a fresh token
/// Vec per call; use [`parse_with_lex_into`] to reuse one buffer across files.
///
/// # Safety
/// Same contract as [`parse_with_lex_into`].
#[must_use]
pub fn parse_with_lex(source: &[u8]) -> (ParseResult<'_>, Vec<RawToken>) {
    // Tokens Vec lives on the heap in a stable slot that outlives `pm_parse`.
    let mut tokens: Box<Vec<RawToken>> = Box::default();
    let mut callback = pm_lex_callback_t {
        data: (&mut *tokens as *mut Vec<RawToken>).cast::<c_void>(),
        callback: Some(collect_token),
    };

    // SAFETY: replicate ruby_prism::parse()'s body verbatim, plus lex_callback.
    let look_alike = unsafe {
        let uninit = Box::new(MaybeUninit::<pm_parser_t>::uninit());
        let uninit = Box::into_raw(uninit);

        pm_parser_init(
            (*uninit).as_mut_ptr(),
            source.as_ptr(),
            source.len(),
            std::ptr::null(),
        );

        let parser = (*uninit).assume_init_mut();
        let parser = NonNull::new_unchecked(parser);

        // Install the collecting callback before parsing.
        (*parser.as_ptr()).lex_callback = &mut callback as *mut pm_lex_callback_t;

        let node = pm_parse(parser.as_ptr());
        let node = NonNull::new_unchecked(node);

        // Detach the callback: nothing reads it post-parse, and `callback`
        // (and `tokens`) get moved/dropped independently of the parser.
        (*parser.as_ptr()).lex_callback = std::ptr::null_mut();

        ParseResultLookAlike {
            source,
            parser,
            node,
        }
    };

    // The load-bearing transmute. Look-alike -> real ParseResult.
    // SAFETY: identical layout (static assert on size/align; canary on fields).
    let result: ParseResult<'_> = unsafe { std::mem::transmute(look_alike) };

    (result, *tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruby_prism::{Node, Visit};

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

    #[test]
    fn layout_static_assert_holds() {
        assert_parse_result_layout();
    }

    #[test]
    fn collects_tokens_matching_a_known_snippet() {
        let src = b"def foo(a)\n  a + 1\nend\n";
        let (_result, tokens) = parse_with_lex(src);
        let named: Vec<(&str, usize, usize, u32)> = tokens
            .iter()
            .map(|t| (t.type_name(), t.start_offset, t.length, t.lex_state))
            .collect();
        // From `Prism.lex` on the same source.
        assert_eq!(named[0], ("KEYWORD_DEF", 0, 3, 128));
        assert_eq!(named[1], ("IDENTIFIER", 4, 3, 8));
        assert_eq!(named[2], ("PARENTHESIS_LEFT", 7, 1, 1025));
        // Last token is always EOF.
        assert_eq!(tokens.last().unwrap().type_name(), "EOF");
    }

    #[test]
    fn transmute_path_ast_matches_normal_parse() {
        let srcs: &[&[u8]] = &[
            b"x = 1\n",
            b"def m(a, b)\n  a.foo { |c| c + b }\nend\n",
            b"class C < D\n  attr_reader :x\n  # comment\nend\n",
            b"[1, 2, 3].map { _1 * 2 }.select(&:even?)\n",
            "\u{3042} = 'multibyte'\n".as_bytes(), // non-ASCII
        ];
        for src in srcs {
            let normal = ruby_prism::parse(src);
            let (spiked, _tokens) = parse_with_lex(src);
            assert_eq!(
                fingerprint(&normal.node()),
                fingerprint(&spiked.node()),
                "AST fingerprint diverged for {:?}",
                String::from_utf8_lossy(src)
            );
            // Comments must also survive the transmute path.
            let n_comments_normal = normal.comments().count();
            let n_comments_spiked = spiked.comments().count();
            assert_eq!(n_comments_normal, n_comments_spiked);
            // Drop of `spiked` (a transmuted ParseResult) must not crash.
        }
    }
}
