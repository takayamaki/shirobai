//! `Lint/DuplicateMagicComment`.
//!
//! Stock's `on_new_investigation`:
//!
//! 1. `return if processed_source.buffer.source.empty?`.
//! 2. `leading_magic_comments` (from the `FrozenStringLiteral` mixin) maps
//!    `MagicComment.parse` over `leading_comment_lines` — the line slice
//!    `processed_source.lines[0...(first_non_comment_token.line - 1)]`, or
//!    every line when the file has no non-comment token.
//! 3. Lines whose parse reports `encoding_specified?` go into one bucket,
//!    `elsif frozen_string_literal_specified?` into another (an Emacs
//!    comment carrying both counts only as encoding). Other magic kinds
//!    (`typed`, `shareable_constant_value`, `rbs_inline`) never count.
//! 4. Every bucket line after the first is an offense at
//!    `buffer.line_range(line)`; autocorrect removes the whole line
//!    including the final newline.
//!
//! The Rust side reproduces steps 2–3 and returns the 1-based lines of the
//! duplicates (encoding bucket first, then frozen-string-literal, each in
//! document order — stock's `each_value` iteration order). The wrapper
//! rebuilds the ranges with stock's own helpers, so offense and autocorrect
//! bytes match by construction.
//!
//! Why this cop is worth replacing: stock's `leading_comment_lines` asks the
//! first token for its `line`, which materializes parser-gem's
//! `Buffer#line_begins` on every file. On large non-ASCII files that
//! materialization is quadratic (`String#index` re-scans the string from the
//! start on every call to convert char offsets), e.g. 1.5s on Discourse's
//! `test_data.rb`. The Rust scan never touches the parser-gem buffer;
//! offense files (the only ones that still build `line_range`) are rare.
//!
//! # `leading_comment_lines` semantics (probed against stock)
//!
//! "First non-comment token" boils down to a byte scan from offset 0 that
//! skips, in any order:
//!
//! - whitespace bytes (parser's lexer): space, `\t`, `\r`, `\n`, `\x0C`, `\x0B`
//! - comments (prism reports `#` comments, `=begin/=end` blocks, and the
//!   shebang line as comments)
//! - `\` line continuations (`\` directly followed by a newline produces no
//!   token)
//!
//! The scan ends in one of three ways:
//!
//! - EOF → no token → every line of `processed_source.lines` is a leading
//!   line (this covers comment-only files).
//! - `__END__` at column 0 followed by `\n` / `\r\n` / EOF → the lexer
//!   stops, same as EOF. NUL / `\x04` / `\x1A` also stop parser-gem's
//!   lexer like EOF. (Parsers before 3.4 stopped at INDENTED `__END__`
//!   too; prism's Latest grammar tokenizes that as an identifier, and we
//!   follow prism — the documented TargetRubyVersion limitation.)
//! - anything else is the first token: leading lines are the lines strictly
//!   before its line (a `;` counts — probed: `;` on line 2 stops the prefix
//!   even though the AST is empty).
//!
//! When the scan ends with no token, `processed_source.lines` is NOT simply
//! every buffer line: rubocop-ast's `ProcessedSource#lines` cuts the array
//! at the first line equal to exactly `"__END__"` strictly after the last
//! token's line (comments are tokens; with no tokens at all there is no
//! cut). That is what hides a data section behind a mid-file `__END__`
//! while still exposing it when `__END__` opens the file.
//!
//! Line texts mirror `Buffer#source_lines`: split on `\n`, `chomp("\n")`
//! only — a stray `\r` stays in the line and is treated as `\s` by the
//! regexes, exactly like stock. When the source ends with `\n`, stock's
//! `lines` gains a phantom trailing `""` entry; it can never match a magic
//! comment, so the scan iterates real lines only but counts the phantom for
//! the all-lines case (`line_starts().len()` matches stock's `lines.size`).
//!
//! # `MagicComment` classification (stock `rubocop/magic_comment.rb`)
//!
//! `MagicComment.parse` picks the FIRST matching format:
//!
//! 1. Emacs: line matches `-\*-(?<token>.+)-\*-` (unanchored, greedy — the
//!    token spans the first `-*-` to the last `-*-`). Tokens split on `;`,
//!    stripped; encoding = a token matching `\A(?:en)?coding\s*:\s*TOKEN\z`
//!    (case-SENSITIVE), fsl = `\Afrozen[_-]string[_-]literal\s*:\s*TOKEN\z`.
//! 2. Vim: line matches `#\s*vim:\s*.+` (unanchored). Tokens split on the
//!    two-character `", "`; encoding = a token matching
//!    `\Afileencoding\s*=\s*TOKEN\z` and only when there are at least two
//!    tokens; fsl is never set by a Vim comment.
//! 3. Simple: encoding = `\A\s*#\s*(frozen_string_literal:\s*(true|false))?
//!    \s*(?:en)?coding: TOKEN` (case-insensitive, unanchored tail, exactly
//!    ONE space after the colon); fsl = `\A\s*#\s*frozen[_-]string[_-]
//!    literal:\s*TOKEN\s*\z` (case-insensitive, fully anchored).
//!
//! `TOKEN` is `[[:alnum:]\-_]+` — Unicode-aware alnum. `\s` is Ruby's
//! ASCII class `[ \t\r\n\f\v]`.

use super::line_index;
use super::parse_cache;

/// 1-based line of one duplicate magic comment (offense + whole-line
/// removal are rebuilt by the Ruby wrapper from the line number alone).
pub type DuplicateLine = usize;

/// Ruby regex `\s` (no `/m`): ASCII whitespace.
pub(crate) fn is_rb_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x0C | 0x0B)
}

pub fn check_duplicate_magic_comment(source: &[u8]) -> Vec<DuplicateLine> {
    if source.is_empty() {
        return Vec::new();
    }

    let (scan, last_comment_start) = parse_cache::with_parsed_and_comments(
        source,
        |_owner, _root, comments| scan_front(source, &comments),
    );

    line_index::with_line_index(source, |li| {
        // Leading line count (`leading_comment_lines.size`): the lines
        // strictly before the first non-comment token's line, or — when
        // lexing ends without one — every line of `processed_source.lines`.
        // That array is `buffer.source_lines` CUT at the first line equal
        // to exactly `__END__` strictly after the last token's line
        // (`ProcessedSource#lines`; comments are tokens, and with no tokens
        // at all there is no cut), phantom trailing entry included.
        let leading = leading_line_count(source, li, &scan, last_comment_start);

        // `lines[i]` = bytes from line_start to the next `\n` (exclusive),
        // mirroring `Buffer#source_lines` (chomp("\n") only — a stray `\r`
        // stays). The phantom trailing entry (start == source.len()) yields
        // an empty line that matches nothing, same as stock's "" line.
        let mut encoding_lines: Vec<usize> = Vec::new();
        let mut fsl_lines: Vec<usize> = Vec::new();
        for (i, &start) in li.line_starts().iter().take(leading).enumerate() {
            let rest = &source[start.min(source.len())..];
            let end = rest
                .iter()
                .position(|&b| b == b'\n')
                .unwrap_or(rest.len());
            let line = &rest[..end];
            match classify(line) {
                Magic::Encoding => encoding_lines.push(i + 1),
                Magic::FrozenStringLiteral => fsl_lines.push(i + 1),
                Magic::None => {}
            }
        }

        let mut out = Vec::new();
        if encoding_lines.len() > 1 {
            out.extend_from_slice(&encoding_lines[1..]);
        }
        if fsl_lines.len() > 1 {
            out.extend_from_slice(&fsl_lines[1..]);
        }
        out
    })
}

/// How the token scan ended.
#[derive(Clone, Copy)]
pub(crate) enum ScanEnd {
    /// First non-comment token found at this byte offset.
    Token(usize),
    /// Lexing stopped (EOF, `__END__` data marker, NUL-family byte) with no
    /// non-comment token; `stop` is where it ended.
    NoToken { stop: usize },
}

/// Find the first non-comment token, mirroring parser-gem's lexer front
/// (with prism's Latest-grammar `__END__` treatment: the marker must sit at
/// column 0; parsers before 3.4 also stopped at indented `__END__`).
fn first_token_pos(source: &[u8], comments: &[(usize, usize)]) -> ScanEnd {
    let len = source.len();
    let mut pos = 0usize;
    let mut ci = 0usize; // cursor into the ordered, disjoint comment ranges
    loop {
        if pos >= len {
            return ScanEnd::NoToken { stop: pos };
        }
        while ci < comments.len() && comments[ci].0 < pos {
            ci += 1;
        }
        if ci < comments.len() && comments[ci].0 == pos {
            pos = comments[ci].1;
            ci += 1;
            continue;
        }
        let b = source[pos];
        if is_rb_space(b) {
            pos += 1;
            continue;
        }
        // `\` + newline is a line continuation: no token.
        if b == b'\\' {
            if source.get(pos + 1) == Some(&b'\n') {
                pos += 2;
                continue;
            }
            if source.get(pos + 1) == Some(&b'\r') && source.get(pos + 2) == Some(&b'\n') {
                pos += 3;
                continue;
            }
        }
        // NUL / ^D / ^Z stop the lexer like EOF.
        if matches!(b, 0x00 | 0x04 | 0x1A) {
            return ScanEnd::NoToken { stop: pos };
        }
        // `__END__` at column 0 followed by a newline or EOF stops the lexer.
        if b == b'_' && (pos == 0 || source[pos - 1] == b'\n') && is_end_marker(&source[pos..]) {
            return ScanEnd::NoToken { stop: pos };
        }
        return ScanEnd::Token(pos);
    }
}

fn is_end_marker(rest: &[u8]) -> bool {
    if !rest.starts_with(b"__END__") {
        return false;
    }
    match rest.get(7) {
        None => true,
        Some(&b'\n') => true,
        Some(&b'\r') => rest.get(8) == Some(&b'\n'),
        _ => false,
    }
}

enum Magic {
    Encoding,
    FrozenStringLiteral,
    None,
}

/// Stock's `if encoding_specified? / elsif frozen_string_literal_specified?`
/// over the `MagicComment.parse` result for one line.
fn classify(line: &[u8]) -> Magic {
    if let Some(token) = emacs_token(line) {
        // EmacsComment: `;`-separated tokens, each stripped.
        let mut encoding = false;
        let mut fsl = false;
        for tok in split_strip(token, b";") {
            if editor_kv_matches(tok, &[b"encoding", b"coding"], b':') {
                encoding = true;
            } else if fsl_keyword_kv_matches(tok) {
                fsl = true;
            }
        }
        if encoding {
            return Magic::Encoding;
        }
        if fsl {
            return Magic::FrozenStringLiteral;
        }
        return Magic::None;
    }
    if let Some(token) = vim_token(line) {
        // VimComment: `", "`-separated tokens; `fileencoding` only counts
        // when there are at least two tokens; fsl never.
        let toks = split_strip_str(token, ", ");
        if toks.len() > 1
            && toks
                .iter()
                .any(|t| editor_kv_matches(t, &[b"fileencoding"], b'='))
        {
            return Magic::Encoding;
        }
        return Magic::None;
    }
    if simple_encoding(line) {
        return Magic::Encoding;
    }
    if simple_fsl(line) {
        return Magic::FrozenStringLiteral;
    }
    Magic::None
}

/// `EmacsComment::REGEXP = /-\*-(?<token>.+)-\*-/`: token spans the first
/// `-*-` (with at least one byte after it) to the LAST closing `-*-`.
pub(crate) fn emacs_token(line: &[u8]) -> Option<&[u8]> {
    let open = find(line, b"-*-", 0)?;
    // Greedy `.+`: the closer is the last `-*-` starting at least one byte
    // past the opener's end. (`.` does not match `\n`; lines contain none.)
    let mut close = None;
    let mut from = open + 4;
    while let Some(at) = find(line, b"-*-", from) {
        close = Some(at);
        from = at + 1;
    }
    close.map(|c| &line[open + 3..c])
}

/// `VimComment::REGEXP = /#\s*vim:\s*(?<token>.+)/`: unanchored; token is
/// the rest of the line (greedy `\s*` backtracks so `.+` keeps at least one
/// byte; the stripped-split consumer makes the exact split irrelevant, so we
/// return everything after `vim:`).
fn vim_token(line: &[u8]) -> Option<&[u8]> {
    let mut from = 0;
    while let Some(hash) = find(line, b"#", from) {
        let mut p = hash + 1;
        while p < line.len() && is_rb_space(line[p]) {
            p += 1;
        }
        if line[p..].starts_with(b"vim:") {
            let rest = &line[p + 4..];
            if !rest.is_empty() {
                return Some(rest);
            }
        }
        from = hash + 1;
    }
    None
}

/// Editor token match `\A<keyword>\s*<op>\s*TOKEN\z` (case-sensitive; for
/// Emacs `keywords` carries both spellings of `(?:en)?coding`).
fn editor_kv_matches(tok: &[u8], keywords: &[&[u8]], op: u8) -> bool {
    for kw in keywords {
        if let Some(rest) = tok.strip_prefix(*kw) {
            let mut p = 0;
            while p < rest.len() && is_rb_space(rest[p]) {
                p += 1;
            }
            if rest.get(p) != Some(&op) {
                continue;
            }
            p += 1;
            while p < rest.len() && is_rb_space(rest[p]) {
                p += 1;
            }
            if token_len(&rest[p..]) == rest.len() - p && p < rest.len() {
                return true;
            }
        }
    }
    false
}

/// Emacs fsl token: `\Afrozen[_-]string[_-]literal\s*:\s*TOKEN\z`.
fn fsl_keyword_kv_matches(tok: &[u8]) -> bool {
    match fsl_keyword_len(tok, false) {
        Some(klen) => editor_kv_matches(&tok[klen..], &[b""], b':'),
        None => false,
    }
}

/// Length of `frozen[_-]string[_-]literal` at the start of `s`, or None.
/// `ci` switches ASCII case-insensitive matching (SimpleComment is `/i`).
fn fsl_keyword_len(s: &[u8], ci: bool) -> Option<usize> {
    let mut p = eat(s, 0, b"frozen", ci)?;
    p = eat_one_of(s, p, b"_-")?;
    p = eat(s, p, b"string", ci)?;
    p = eat_one_of(s, p, b"_-")?;
    p = eat(s, p, b"literal", ci)?;
    Some(p)
}

fn eat(s: &[u8], at: usize, word: &[u8], ci: bool) -> Option<usize> {
    let end = at + word.len();
    if end > s.len() {
        return None;
    }
    let got = &s[at..end];
    let ok = if ci {
        got.eq_ignore_ascii_case(word)
    } else {
        got == word
    };
    ok.then_some(end)
}

fn eat_one_of(s: &[u8], at: usize, set: &[u8]) -> Option<usize> {
    s.get(at)
        .filter(|b| set.contains(b))
        .map(|_| at + 1)
}

/// SimpleComment#encoding:
/// `\A\s*#\s*(frozen_string_literal:\s*(true|false))?\s*(?:en)?coding: TOKEN`
/// — case-insensitive, tail unanchored, exactly one literal space after the
/// colon.
fn simple_encoding(line: &[u8]) -> bool {
    let mut p = skip_space(line, 0);
    if line.get(p) != Some(&b'#') {
        return false;
    }
    p = skip_space(line, p + 1);
    // Optional `frozen_string_literal:\s*(true|false)` group (underscores
    // only — `FSTRING_LITERAL_COMMENT` has no dash variant). No ambiguity
    // with the coding keyword ('f' vs 'e'/'c'), so plain sequencing is
    // exactly the regex's backtracking outcome.
    if let Some(after_kw) = eat(line, p, b"frozen_string_literal:", true) {
        let v = skip_space(line, after_kw);
        let after_val = eat(line, v, b"true", true).or_else(|| eat(line, v, b"false", true));
        if let Some(after_val) = after_val {
            p = skip_space(line, after_val);
        }
    }
    let p = eat(line, p, b"encoding", true)
        .or_else(|| eat(line, p, b"coding", true));
    let Some(p) = p else { return false };
    if !line[p..].starts_with(b": ") {
        return false;
    }
    token_len(&line[p + 2..]) > 0
}

/// SimpleComment fsl:
/// `\A\s*#\s*frozen[_-]string[_-]literal:\s*(?<token>TOKEN)\s*\z` (/i).
fn simple_fsl(line: &[u8]) -> bool {
    let mut p = skip_space(line, 0);
    if line.get(p) != Some(&b'#') {
        return false;
    }
    p = skip_space(line, p + 1);
    let Some(kw_end) = fsl_keyword_len(&line[p..], true) else {
        return false;
    };
    p += kw_end;
    if line.get(p) != Some(&b':') {
        return false;
    }
    p = skip_space(line, p + 1);
    let tlen = token_len(&line[p..]);
    if tlen == 0 {
        return false;
    }
    p = skip_space(line, p + tlen);
    p == line.len()
}

fn skip_space(s: &[u8], mut p: usize) -> usize {
    while p < s.len() && is_rb_space(s[p]) {
        p += 1;
    }
    p
}

/// Length of the longest `TOKEN` (`[[:alnum:]\-_]+`, Unicode alnum) prefix.
fn token_len(s: &[u8]) -> usize {
    // ASCII fast path; fall back to char decoding for multibyte (Ruby's
    // [[:alnum:]] is Unicode-aware on UTF-8 strings).
    let mut p = 0;
    while p < s.len() {
        let b = s[p];
        if b.is_ascii() {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
                p += 1;
                continue;
            }
            break;
        }
        match std::str::from_utf8(&s[p..]) {
            Ok(rest) => {
                let ch = rest.chars().next().unwrap();
                if ch.is_alphanumeric() {
                    p += ch.len_utf8();
                    continue;
                }
                break;
            }
            Err(e) => {
                // Try to decode just the next char.
                let valid = e.valid_up_to();
                if valid == 0 {
                    break;
                }
                let rest = std::str::from_utf8(&s[p..p + valid]).unwrap();
                let ch = rest.chars().next().unwrap();
                if ch.is_alphanumeric() {
                    p += ch.len_utf8();
                    continue;
                }
                break;
            }
        }
    }
    p
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from > hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

/// Ruby `String#split(sep)` + `map(&:strip)`. Trailing empty fields are
/// dropped by split; strip removes `\0` and ASCII whitespace.
fn split_strip<'a>(s: &'a [u8], sep: &[u8]) -> Vec<&'a [u8]> {
    split_strip_impl(s, sep)
}

fn split_strip_str<'a>(s: &'a [u8], sep: &str) -> Vec<&'a [u8]> {
    split_strip_impl(s, sep.as_bytes())
}

fn split_strip_impl<'a>(s: &'a [u8], sep: &[u8]) -> Vec<&'a [u8]> {
    let mut parts: Vec<&[u8]> = Vec::new();
    let mut from = 0;
    while let Some(at) = find(s, sep, from) {
        parts.push(&s[from..at]);
        from = at + sep.len();
    }
    parts.push(&s[from..]);
    // Ruby split drops trailing empty fields.
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts.iter().map(|p| strip_rb(p)).collect()
}

/// Ruby `String#strip`: removes leading/trailing `[\0\t\n\v\f\r ]`.
fn strip_rb(s: &[u8]) -> &[u8] {
    let is_strip = |b: &u8| is_rb_space(*b) || *b == 0;
    let start = s.iter().position(|b| !is_strip(b)).unwrap_or(s.len());
    let end = s.iter().rposition(|b| !is_strip(b)).map_or(start, |e| e + 1);
    &s[start..end]
}

/// The token-scan front shared by every leading-comment consumer: how the
/// scan for the first non-comment token ended, plus the start offset of the
/// last comment token before that point (which drives the
/// `ProcessedSource#lines` `__END__` cut in [`leading_line_count`]).
pub(crate) fn scan_front(
    source: &[u8],
    comments: &[(usize, usize)],
) -> (ScanEnd, Option<usize>) {
    let scan = first_token_pos(source, comments);
    let stop = match scan {
        ScanEnd::Token(pos) => pos,
        ScanEnd::NoToken { stop } => stop,
    };
    let last_comment_start = comments
        .iter()
        .take_while(|c| c.0 < stop)
        .last()
        .map(|c| c.0);
    (scan, last_comment_start)
}

/// The line's bytes from `start` up to (excluding) the next `\n`, mirroring
/// `Buffer#source_lines` (chomp(`"\n"`) only — a stray `\r` stays).
pub(crate) fn line_slice(source: &[u8], start: usize) -> &[u8] {
    let rest = &source[start.min(source.len())..];
    let end = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
    &rest[..end]
}

/// `leading_comment_lines.size` — shared by `Lint/DuplicateMagicComment` and
/// `RuboCop::Cop::FrozenStringLiteral#frozen_string_literals_enabled?` (see
/// [`frozen_string_literals_enabled`]). See the call site above for the exact
/// `ProcessedSource#lines` / `__END__` semantics.
pub(crate) fn leading_line_count(
    source: &[u8],
    li: &line_index::LineIndex,
    scan: &ScanEnd,
    last_comment_start: Option<usize>,
) -> usize {
    match scan {
        ScanEnd::Token(pos) => li.line_of(*pos) - 1,
        ScanEnd::NoToken { .. } => {
            let all = li.line_starts().len();
            match last_comment_start {
                None => all,
                Some(cstart) => {
                    let last_token_line = li.line_of(cstart);
                    (last_token_line..all)
                        .find(|&ix| line_slice(source, li.line_starts()[ix]) == b"__END__")
                        .unwrap_or(all)
                }
            }
        }
    }
}

/// `RuboCop::Cop::FrozenStringLiteral#frozen_string_literals_enabled?`.
///
/// Scans the same leading comment lines as `Lint/DuplicateMagicComment` and
/// returns the first line whose `MagicComment.parse` specifies a
/// `frozen_string_literal` value: `true` iff that value is `true`. With no such
/// line the result is `sfbd_default` (`AllCops/StringLiteralsFrozenByDefault`
/// coerced to a plain boolean; `nil`/`false` both map to `false` on the Ruby
/// side, so the wrapper passes `false` for either).
///
/// Pure bytes plus the shared prism parse (comments only) — no parser tokens.
pub fn frozen_string_literals_enabled(source: &[u8], sfbd_default: bool) -> bool {
    if source.is_empty() {
        return sfbd_default;
    }

    let (scan, last_comment_start) = parse_cache::with_parsed_and_comments(
        source,
        |_owner, _root, comments| scan_front(source, &comments),
    );

    line_index::with_line_index(source, |li| {
        let leading = leading_line_count(source, li, &scan, last_comment_start);
        for &start in li.line_starts().iter().take(leading) {
            if let Some(value) = line_fsl(line_slice(source, start)) {
                return value == FslValue::True;
            }
        }
        sfbd_default
    })
}

/// `RuboCop::Cop::FrozenStringLiteral#frozen_string_literals_disabled?`: true
/// iff ANY leading comment line's `MagicComment.parse.frozen_string_literal` is
/// `false` (stock uses `any?`, not the first-match `find` of the enabled path).
/// Same leading-line scan as [`frozen_string_literals_enabled`], no parser
/// tokens.
pub fn frozen_string_literals_disabled(source: &[u8]) -> bool {
    if source.is_empty() {
        return false;
    }

    let (scan, last_comment_start) = parse_cache::with_parsed_and_comments(
        source,
        |_owner, _root, comments| scan_front(source, &comments),
    );

    line_index::with_line_index(source, |li| {
        let leading = leading_line_count(source, li, &scan, last_comment_start);
        li.line_starts()
            .iter()
            .take(leading)
            .any(|&start| line_fsl(line_slice(source, start)) == Some(FslValue::False))
    })
}

/// Frozen-string-literal value of one line's `MagicComment.parse` result:
/// stock downcases the token and coerces `"true"`/`"false"` to booleans; any
/// other token stays a plain string (`Other`). `valid_literal_value?` is
/// `True | False`; `frozen_string_literal?` is `True`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FslValue {
    True,
    False,
    /// Specified, but neither `true` nor `false`.
    Other,
}

fn fsl_value_of(tok: &[u8]) -> FslValue {
    if tok.eq_ignore_ascii_case(b"true") {
        FslValue::True
    } else if tok.eq_ignore_ascii_case(b"false") {
        FslValue::False
    } else {
        FslValue::Other
    }
}

/// `MagicComment.parse(line).frozen_string_literal` collapsed to a value
/// kind: `None` when not specified, otherwise the coerced value. Follows
/// `MagicComment.parse`'s Emacs-then-Vim-then-Simple format dispatch.
pub(crate) fn line_fsl(line: &[u8]) -> Option<FslValue> {
    if let Some(token) = emacs_token(line) {
        // EditorComment#match returns the first token matching the fsl keyword.
        for tok in split_strip(token, b";") {
            if let Some(v) = emacs_fsl_token_value(tok) {
                return Some(v);
            }
        }
        return None;
    }
    if vim_token(line).is_some() {
        // Vim comments never specify frozen_string_literal.
        return None;
    }
    simple_fsl_value(line)
}

/// Emacs editor token value for `\Afrozen[_-]string[_-]literal\s*:\s*TOKEN\z`
/// (case-SENSITIVE keyword, like `EditorComment#match`; value downcased).
fn emacs_fsl_token_value(tok: &[u8]) -> Option<FslValue> {
    let klen = fsl_keyword_len(tok, false)?;
    let rest = &tok[klen..];
    let mut p = skip_space(rest, 0);
    if rest.get(p) != Some(&b':') {
        return None;
    }
    p = skip_space(rest, p + 1);
    let tlen = token_len(&rest[p..]);
    if tlen == 0 || p + tlen != rest.len() {
        return None;
    }
    Some(fsl_value_of(&rest[p..p + tlen]))
}

/// SimpleComment fsl value:
/// `\A\s*#\s*frozen[_-]string[_-]literal:\s*(?<token>TOKEN)\s*\z` (/i,
/// value downcased).
fn simple_fsl_value(line: &[u8]) -> Option<FslValue> {
    let mut p = skip_space(line, 0);
    if line.get(p) != Some(&b'#') {
        return None;
    }
    p = skip_space(line, p + 1);
    let kw_end = fsl_keyword_len(&line[p..], true)?;
    p += kw_end;
    if line.get(p) != Some(&b':') {
        return None;
    }
    p = skip_space(line, p + 1);
    let tlen = token_len(&line[p..]);
    if tlen == 0 {
        return None;
    }
    let value = &line[p..p + tlen];
    p = skip_space(line, p + tlen);
    if p != line.len() {
        return None;
    }
    Some(fsl_value_of(value))
}

/// Which `Lint/OrderedMagicComments` bucket a leading line falls into. Stock's
/// `magic_comment_lines` scan uses `MagicComment.parse(line)` and an
/// `if encoding_specified? / elsif valid?` split:
///
/// - [`OrderedBucket::Encoding`] — `encoding_specified?` (the `if` arm; no
///   `#`-prefix requirement, since `encoding` extraction allows leading space).
/// - [`OrderedBucket::OtherValid`] — reached only in the `elsif`, so encoding
///   is already false: `valid?` = `@comment.start_with?('#')` (no leading
///   space) AND `any?` = one of the remaining magic kinds is specified
///   (`frozen_string_literal`, `shareable_constant_value`, `rbs_inline`,
///   `typed`). `rbs_inline` is special: it counts only when the value is
///   exactly `enabled`/`disabled` (`rbs_inline_specified?` ==
///   `valid_rbs_inline_value?`).
/// - [`OrderedBucket::None`] — neither.
///
/// The format dispatch mirrors `MagicComment.parse` (Emacs, else Vim, else
/// Simple); each format only exposes the magic kinds it can carry (Vim can
/// only carry `fileencoding`; Emacs cannot carry `rbs_inline`/`typed`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OrderedBucket {
    Encoding,
    OtherValid,
    None,
}

pub(crate) fn ordered_bucket(line: &[u8]) -> OrderedBucket {
    // `@comment.start_with?('#')` — the raw line, no leading space allowed.
    let hash_prefixed = line.first() == Some(&b'#');

    if let Some(token) = emacs_token(line) {
        // EmacsComment: `;`-separated tokens, each stripped. `encoding` and
        // `frozen_string_literal`/`shareable_constant_value` are the only kinds
        // Emacs can carry (`rbs_inline`/`typed` extraction is nil).
        let mut encoding = false;
        let mut other = false;
        for tok in split_strip(token, b";") {
            if editor_kv_matches(tok, &[b"encoding", b"coding"], b':') {
                encoding = true;
            } else if fsl_keyword_kv_matches(tok) || shareable_keyword_kv_matches(tok) {
                other = true;
            }
        }
        if encoding {
            return OrderedBucket::Encoding;
        }
        if hash_prefixed && other {
            return OrderedBucket::OtherValid;
        }
        return OrderedBucket::None;
    }
    if let Some(token) = vim_token(line) {
        // VimComment: `", "`-separated tokens; `fileencoding` only counts with
        // at least two tokens. No other magic kind is carried by Vim.
        let toks = split_strip_str(token, ", ");
        if toks.len() > 1
            && toks
                .iter()
                .any(|t| editor_kv_matches(t, &[b"fileencoding"], b'='))
        {
            return OrderedBucket::Encoding;
        }
        return OrderedBucket::None;
    }
    if simple_encoding(line) {
        return OrderedBucket::Encoding;
    }
    if hash_prefixed
        && (simple_fsl(line)
            || simple_shareable_specified(line)
            || simple_rbs_inline_specified(line)
            || simple_typed_specified(line))
    {
        return OrderedBucket::OtherValid;
    }
    OrderedBucket::None
}

/// Emacs editor token match for `\Ashareable[_-]constant[_-]value\s*:\s*TOKEN\z`
/// (case-SENSITIVE keyword, like `EditorComment#match`).
fn shareable_keyword_kv_matches(tok: &[u8]) -> bool {
    match shareable_keyword_len(tok, false) {
        Some(klen) => editor_kv_matches(&tok[klen..], &[b""], b':'),
        None => false,
    }
}

/// Length of `shareable[_-]constant[_-]value` at the start of `s`, or None.
fn shareable_keyword_len(s: &[u8], ci: bool) -> Option<usize> {
    let mut p = eat(s, 0, b"shareable", ci)?;
    p = eat_one_of(s, p, b"_-")?;
    p = eat(s, p, b"constant", ci)?;
    p = eat_one_of(s, p, b"_-")?;
    p = eat(s, p, b"value", ci)?;
    Some(p)
}

/// SimpleComment anchored token match `\A\s*#\s*<keyword>:\s*TOKEN\s*\z` (/i),
/// returning the captured TOKEN. `kw_len` matches the keyword at a slice start.
fn simple_anchored_token(
    line: &[u8],
    kw_len: impl Fn(&[u8]) -> Option<usize>,
) -> Option<&[u8]> {
    let mut p = skip_space(line, 0);
    if line.get(p) != Some(&b'#') {
        return None;
    }
    p = skip_space(line, p + 1);
    let klen = kw_len(&line[p..])?;
    p += klen;
    if line.get(p) != Some(&b':') {
        return None;
    }
    p = skip_space(line, p + 1);
    let tlen = token_len(&line[p..]);
    if tlen == 0 {
        return None;
    }
    let tok = &line[p..p + tlen];
    p = skip_space(line, p + tlen);
    if p != line.len() {
        return None;
    }
    Some(tok)
}

/// `shareable_constant_value_specified?` for a SimpleComment line (any TOKEN).
fn simple_shareable_specified(line: &[u8]) -> bool {
    simple_anchored_token(line, |s| shareable_keyword_len(s, true)).is_some()
}

/// `typed_specified?` for a SimpleComment line (any TOKEN).
fn simple_typed_specified(line: &[u8]) -> bool {
    simple_anchored_token(line, |s| eat(s, 0, b"typed", true)).is_some()
}

/// `rbs_inline_specified?` for a SimpleComment line: the value must be exactly
/// `enabled` or `disabled` (`valid_rbs_inline_value?`), not merely present.
fn simple_rbs_inline_specified(line: &[u8]) -> bool {
    match simple_anchored_token(line, |s| eat(s, 0, b"rbs_inline", true)) {
        Some(tok) => tok.eq_ignore_ascii_case(b"enabled") || tok.eq_ignore_ascii_case(b"disabled"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<usize> {
        check_duplicate_magic_comment(src.as_bytes())
    }

    fn fsl_enabled(src: &str, sfbd: bool) -> bool {
        frozen_string_literals_enabled(src.as_bytes(), sfbd)
    }

    #[test]
    fn fsl_enabled_simple_comment() {
        assert!(fsl_enabled("# frozen_string_literal: true\nx = 1\n", false));
        assert!(!fsl_enabled("# frozen_string_literal: false\nx = 1\n", false));
        // Non-true/false value is "specified" but not enabled.
        assert!(!fsl_enabled("# frozen_string_literal: yes\nx = 1\n", true));
    }

    #[test]
    fn fsl_enabled_falls_back_to_sfbd() {
        assert!(fsl_enabled("x = 1\n", true));
        assert!(!fsl_enabled("x = 1\n", false));
        assert!(!fsl_enabled("# encoding: utf-8\nx = 1\n", false));
    }

    #[test]
    fn fsl_enabled_first_specified_wins() {
        // First fsl-specified leading comment decides.
        assert!(!fsl_enabled(
            "# frozen_string_literal: false\n# frozen_string_literal: true\nx = 1\n",
            true
        ));
    }

    #[test]
    fn fsl_enabled_only_leading_lines() {
        // A comment after the first non-comment token is not leading.
        assert!(!fsl_enabled("x = 1\n# frozen_string_literal: true\n", false));
    }

    #[test]
    fn fsl_enabled_emacs_and_dash_forms() {
        assert!(fsl_enabled("# -*- frozen_string_literal: true -*-\nx = 1\n", false));
        assert!(fsl_enabled("# frozen-string-literal: true\nx = 1\n", false));
    }

    // Typical: two identical fsl comments -> second line flagged.
    #[test]
    fn fsl_duplicate() {
        assert_eq!(
            run("# frozen_string_literal: true\n# frozen_string_literal: true\nx = 1\n"),
            vec![2]
        );
    }

    // Typical: two encoding comments (values differ) -> still duplicates.
    #[test]
    fn encoding_duplicate_different_values() {
        assert_eq!(run("# encoding: ascii\n# encoding: utf-8\nx = 1\n"), vec![2]);
    }

    // Case-insensitive simple comments.
    #[test]
    fn case_insensitive_simple() {
        assert_eq!(run("# ENCODING: UTF-8\n# Encoding: utf-8\nx = 1\n"), vec![2]);
        assert_eq!(
            run("# frozen_string_literal: true\n# FROZEN_STRING_LITERAL: TRUE\nx = 1\n"),
            vec![2]
        );
    }

    // `coding` prefix variant counts as encoding.
    #[test]
    fn coding_prefix() {
        assert_eq!(run("# coding: utf-8\n# encoding: utf-8\nx = 1\n"), vec![2]);
    }

    // Both buckets: encoding duplicates first, then fsl duplicates.
    #[test]
    fn both_buckets_order() {
        let src = "# encoding: ascii\n# frozen_string_literal: true\n# encoding: ascii\n# frozen_string_literal: true\nx = 1\n";
        assert_eq!(run(src), vec![3, 4]);
    }

    // No duplicate -> empty.
    #[test]
    fn no_duplicates() {
        assert_eq!(
            run("# encoding: ascii\n# frozen_string_literal: true\nx = 1\n"),
            Vec::<usize>::new()
        );
    }

    // Empty source -> nothing (stock returns early).
    #[test]
    fn empty_source() {
        assert_eq!(run(""), Vec::<usize>::new());
    }

    // Exactly one space required after the colon in simple encoding.
    #[test]
    fn simple_encoding_space_rules() {
        assert_eq!(run("# encoding:utf-8\n# encoding: utf-8\nx = 1\n"), Vec::<usize>::new());
        assert_eq!(run("# encoding:  utf-8\n# encoding: utf-8\nx = 1\n"), Vec::<usize>::new());
    }

    // Unanchored tail: trailing text after the encoding value still counts.
    #[test]
    fn simple_encoding_trailing_text() {
        assert_eq!(run("# encoding: utf-8 extra\n# encoding: utf-8\nx = 1\n"), vec![2]);
    }

    // fsl is fully anchored: trailing text disqualifies.
    #[test]
    fn simple_fsl_anchored() {
        assert_eq!(
            run("# frozen_string_literal: true extra\n# frozen_string_literal: true\nx = 1\n"),
            Vec::<usize>::new()
        );
    }

    // fsl keyword accepts dashes; any TOKEN value counts as "specified".
    #[test]
    fn fsl_dashes_and_any_value() {
        assert_eq!(
            run("# frozen-string-literal: true\n# frozen_string_literal: yes\nx = 1\n"),
            vec![2]
        );
    }

    // The combined fsl+coding simple form counts as ENCODING (if/elsif).
    #[test]
    fn combined_fsl_coding_is_encoding() {
        let src = "# frozen_string_literal: true coding: utf-8\n# encoding: ascii\n# frozen_string_literal: true\nx = 1\n";
        assert_eq!(run(src), vec![2]);
    }

    // Emacs form: `-*- coding: utf-8 -*-` counts as encoding; keyword match
    // is case-SENSITIVE.
    #[test]
    fn emacs_form() {
        assert_eq!(run("# -*- coding: utf-8 -*-\n# encoding: ascii\nx = 1\n"), vec![2]);
        assert_eq!(
            run("# -*- CODING: utf-8 -*-\n# encoding: ascii\nx = 1\n"),
            Vec::<usize>::new()
        );
    }

    // Emacs multi-token: fsl+coding counts as encoding (bucket priority).
    #[test]
    fn emacs_multi_token() {
        let src = "# -*- frozen_string_literal: true; coding: utf-8 -*-\n# encoding: ascii\n# frozen_string_literal: true\nx = 1\n";
        assert_eq!(run(src), vec![2]);
    }

    // Emacs fsl-only token bucket.
    #[test]
    fn emacs_fsl_only() {
        let src = "# -*- frozen_string_literal: true -*-\n# frozen_string_literal: true\nx = 1\n";
        assert_eq!(run(src), vec![2]);
    }

    // Vim form needs >= 2 tokens (split on comma-space) for fileencoding.
    #[test]
    fn vim_form() {
        assert_eq!(
            run("# vim: ft=ruby, fileencoding=utf-8\n# encoding: ascii\nx = 1\n"),
            vec![2]
        );
        assert_eq!(
            run("# vim: fileencoding=utf-8\n# encoding: ascii\nx = 1\n"),
            Vec::<usize>::new()
        );
        // Comma without space -> one token -> no encoding.
        assert_eq!(
            run("# vim: ft=ruby,fileencoding=utf-8\n# encoding: ascii\nx = 1\n"),
            Vec::<usize>::new()
        );
    }

    // Vim comment matched mid-line (unanchored REGEXP).
    #[test]
    fn vim_mid_line() {
        assert_eq!(
            run("# stuff # vim: ft=ruby, fileencoding=utf-8\n# encoding: ascii\nx = 1\n"),
            vec![2]
        );
    }

    // Leading lines: shebang and blank lines do not stop the prefix.
    #[test]
    fn shebang_and_blanks() {
        assert_eq!(
            run("#!/usr/bin/env ruby\n# encoding: utf-8\n\n# encoding: utf-8\nx = 1\n"),
            vec![4]
        );
    }

    // A `;` is a non-comment token: it ends the leading prefix.
    #[test]
    fn semicolon_stops_prefix() {
        assert_eq!(
            run("# encoding: utf-8\n;\n# encoding: utf-8\nx = 1\n"),
            Vec::<usize>::new()
        );
    }

    // Magic-shaped lines inside a `=begin/=end` block count: the leading
    // lines are a raw line slice, not comment texts.
    #[test]
    fn magic_inside_block_comment() {
        assert_eq!(
            run("=begin\n# encoding: utf-8\n=end\n# encoding: utf-8\nx = 1\n"),
            vec![4]
        );
    }

    // Comment-only file: every line is leading.
    #[test]
    fn comments_only() {
        assert_eq!(run("# encoding: utf-8\n# encoding: utf-8\n"), vec![2]);
    }

    // No trailing newline on the duplicate line.
    #[test]
    fn no_trailing_newline() {
        assert_eq!(run("# encoding: utf-8\n# encoding: utf-8"), vec![2]);
    }

    // Backslash-newline produces no token; prefix continues past it.
    #[test]
    fn backslash_continuation() {
        assert_eq!(run("# encoding: utf-8\n\\\n# encoding: utf-8\nx = 1\n"), vec![3]);
    }

    // `__END__` on line 1 stops lexing with NO tokens at all: there is no
    // lines cut, so every line is leading, including magic-shaped lines in
    // the data section.
    #[test]
    fn end_marker_line_one() {
        assert_eq!(run("__END__\n# encoding: a\n# encoding: a\n"), vec![3]);
    }

    // Mid-file `__END__` after comment tokens: `ProcessedSource#lines` cuts
    // at the `__END__` line, hiding the data section.
    #[test]
    fn end_marker_mid_file_cuts_lines() {
        assert_eq!(
            run("# encoding: utf-8\n# encoding: utf-8\n__END__\n# encoding: a\n"),
            vec![2]
        );
        assert_eq!(
            run("# encoding: utf-8\n__END__\n# encoding: utf-8\n"),
            Vec::<usize>::new()
        );
    }

    // Indented `__END__` is an identifier token under prism's Latest
    // grammar (parsers before 3.4 stopped lexing there instead — the
    // documented TargetRubyVersion divergence).
    #[test]
    fn indented_end_marker_is_token() {
        assert_eq!(
            run("# encoding: utf-8\n  __END__\n# encoding: utf-8\n"),
            Vec::<usize>::new()
        );
    }

    // Three encoding comments -> two offenses (lines 2 and 3).
    #[test]
    fn three_encodings() {
        assert_eq!(
            run("# encoding: utf-8\n# encoding: utf-8\n# encoding: utf-8\nx = 1\n"),
            vec![2, 3]
        );
    }

    // typed / shareable_constant_value / rbs_inline never count.
    #[test]
    fn other_magic_kinds_ignored() {
        assert_eq!(run("# typed: true\n# typed: true\nx = 1\n"), Vec::<usize>::new());
        assert_eq!(
            run("# shareable_constant_value: literal\n# shareable_constant_value: literal\nx = 1\n"),
            Vec::<usize>::new()
        );
        assert_eq!(
            run("# rbs_inline: enabled\n# rbs_inline: enabled\nx = 1\n"),
            Vec::<usize>::new()
        );
    }

    // Unicode alnum in TOKEN (Ruby [[:alnum:]] is Unicode-aware).
    #[test]
    fn unicode_token_value() {
        assert_eq!(run("# encoding: utf-8\n# encoding: utf-é\nx = 1\n"), vec![2]);
    }

    // Indented magic comments still match (`\A\s*#`).
    #[test]
    fn indented_magic() {
        assert_eq!(run("  # encoding: utf-8\n\t# encoding: utf-8\nx = 1\n"), vec![2]);
    }

    // `## encoding:` does not match (second `#` is not `\s`).
    #[test]
    fn double_hash_no_match() {
        assert_eq!(run("## encoding: utf-8\n# encoding: utf-8\nx = 1\n"), Vec::<usize>::new());
    }
}
