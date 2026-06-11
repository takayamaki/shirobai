//! `Naming/MethodName`.

use ruby_prism::{Node, Visit};

/// A method-name site whose name may violate the configured style.
pub struct MethodNameCandidate {
    /// Offense range used for the style-violation message.
    pub start_offset: usize,
    pub end_offset: usize,
    /// The bare method name (no sigil, no colon/quotes).
    pub name: String,
    /// Whether the name matches the configured style (or is a class emitter).
    pub valid: bool,
    /// Alternative style index the name matches (0/1), or 255 (unrecognized).
    pub alternative: u8,
    /// Offense range / name used for the `ForbiddenIdentifiers` message.
    pub forbidden_start: usize,
    pub forbidden_end: usize,
    pub forbidden_name: String,
}

const SNAKE_CASE: u8 = 0;
const CAMEL_CASE: u8 = 1;

/// Operator method names that are always accepted (`OPERATOR_METHODS`).
const OPERATOR_METHODS: &[&[u8]] = &[
    b"|", b"^", b"&", b"<=>", b"==", b"===", b"=~", b">", b">=", b"<", b"<=", b"<<", b">>", b"+",
    b"-", b"*", b"/", b"%", b"**", b"~", b"+@", b"-@", b"!@", b"~@", b"[]", b"[]=", b"!", b"!=",
    b"!~", b"`",
];

fn is_operator(name: &[u8]) -> bool {
    OPERATOR_METHODS.contains(&name)
}

/// Port of `ConfigurableNaming::FORMATS`.
///
/// `snake_case`: `/^@{0,2}[\d[[:lower:]]_]+[!?=]?$/`
/// `camelCase`:  `/^@{0,2}(?:_|_?[[:lower:]][\d[[:lower:]][[:upper:]]]*)[!?=]?$/`
///
/// Method names carry no sigil, so the optional leading `@{0,2}` never matters.
/// `[[:lower:]]` / `[[:upper:]]` are Unicode-aware (matching Onigmo on a UTF-8
/// source), so `char::is_lowercase` / `is_uppercase` are used.
fn valid_for(name: &str, style: u8) -> bool {
    let mut chars: Vec<char> = name.chars().collect();
    if chars.is_empty() {
        return false;
    }
    // Optional trailing `[!?=]`.
    if matches!(chars.last(), Some('!' | '?' | '=')) {
        chars.pop();
    }
    if chars.is_empty() {
        return false;
    }
    match style {
        SNAKE_CASE => chars
            .iter()
            .all(|c| c.is_ascii_digit() || *c == '_' || c.is_lowercase()),
        CAMEL_CASE => {
            // `_` alone, or `_?[[:lower:]][\d[[:lower:]][[:upper:]]]*`.
            if chars == ['_'] {
                return true;
            }
            let mut i = 0;
            if chars[0] == '_' {
                i = 1;
            }
            if i >= chars.len() || !chars[i].is_lowercase() {
                return false;
            }
            i += 1;
            chars[i..]
                .iter()
                .all(|c| c.is_ascii_digit() || c.is_lowercase() || c.is_uppercase())
        }
        _ => true,
    }
}

/// First alternative style (in `SupportedStyles` order) the name matches, or
/// 255 (unrecognized), mirroring `report_opposing_styles`.
fn first_alternative(name: &str, current: u8) -> u8 {
    for style in [SNAKE_CASE, CAMEL_CASE] {
        if style != current && valid_for(name, style) {
            return style;
        }
    }
    255
}

pub fn check_method_name(source: &[u8], style: u8) -> Vec<MethodNameCandidate> {
    check_method_name_filtered(source, style, false).0
}

/// Like [`check_method_name`], but with an optional invalid-only filter: when
/// `filtered` is set, the style-compliant candidates (the vast majority) are
/// dropped before crossing back into Ruby. The returned flag reports whether
/// any valid candidate existed, so the Ruby side can still run its
/// `correct_style_detected` bookkeeping. The filter runs after the whole walk
/// because the class-emitter resolution can flip `valid` retroactively.
pub fn check_method_name_filtered(
    source: &[u8],
    style: u8,
    filtered: bool,
) -> (Vec<MethodNameCandidate>, bool) {
    super::parse_cache::with_parsed(source, |source, node| {
        let mut visitor = Visitor {
            source,
            style,
            candidates: Vec::new(),
            scopes: vec![Scope::default()],
        };
        visitor.visit(node);
        // Resolve any class-emitter candidates still pending at the top scope.
        visitor.resolve_scope();
        let mut candidates = visitor.candidates;
        let had_valid = candidates.iter().any(|c| c.valid);
        if filtered {
            candidates.retain(|c| !c.valid);
        }
        (candidates, had_valid)
    })
}

/// A lexical scope that can contain method definitions and nested classes.
#[derive(Default)]
struct Scope {
    is_def: bool,
    /// Source text of each directly nested class' constant path.
    class_names: Vec<String>,
    /// Indices into `candidates` for `defs` whose validity depends on a sibling
    /// class definition (resolved when this scope is left).
    pending: Vec<(usize, String)>,
}

struct Visitor<'a> {
    source: &'a [u8],
    style: u8,
    candidates: Vec<MethodNameCandidate>,
    scopes: Vec<Scope>,
}

impl Visitor<'_> {
    fn text(&self, start: usize, end: usize) -> String {
        String::from_utf8_lossy(&self.source[start..end]).into_owned()
    }

    /// Resolve and pop the innermost scope: any pending `defs` becomes valid
    /// when a sibling class shares its name (the class-emitter exception).
    fn resolve_scope(&mut self) {
        let scope = self.scopes.pop().unwrap();
        for (idx, name) in scope.pending {
            if scope.class_names.contains(&name) {
                self.candidates[idx].valid = true;
            }
        }
    }

    /// Push a non-style candidate, checking the configured style; `name` is the
    /// bare method name and `[start, end)` the offense range.
    fn push_named(&mut self, name: String, start: usize, end: usize) {
        let valid = valid_for(&name, self.style);
        let alternative = if valid {
            0
        } else {
            first_alternative(&name, self.style)
        };
        self.candidates.push(MethodNameCandidate {
            start_offset: start,
            end_offset: end,
            forbidden_start: start,
            forbidden_end: end,
            forbidden_name: name.clone(),
            name,
            valid,
            alternative,
        });
    }

    /// A `def`/`defs` definition. When the name fails the style check and the
    /// definition is a singleton method (`is_defs`), defer it for the
    /// class-emitter exception against the enclosing non-`def` scope.
    fn push_def(&mut self, name: String, start: usize, end: usize, is_defs: bool) {
        let idx = self.candidates.len();
        let valid = valid_for(&name, self.style);
        self.push_named(name.clone(), start, end);
        if is_defs && !valid {
            // Climb out of any enclosing `def` scopes to the container.
            for scope in self.scopes.iter_mut().rev() {
                if !scope.is_def {
                    scope.pending.push((idx, name));
                    break;
                }
            }
        }
    }

    /// Handle a method-name literal argument (`define_method`, `Struct`/`Data`
    /// members, `alias`/`alias_method`). Returns whether a candidate was added.
    fn handle_literal(&mut self, arg: &Node<'_>) {
        let (name, loc) = if let Some(s) = arg.as_symbol_node() {
            // Skip interpolated / dynamic symbols (no static value).
            if s.value_loc().is_none() {
                return;
            }
            (
                String::from_utf8_lossy(s.unescaped()).into_owned(),
                arg.location(),
            )
        } else if let Some(s) = arg.as_string_node() {
            (
                String::from_utf8_lossy(s.unescaped()).into_owned(),
                arg.location(),
            )
        } else {
            return;
        };
        if is_operator(name.as_bytes()) {
            return;
        }
        self.push_named(name, loc.start_offset(), loc.end_offset());
    }

    /// `(const {nil? cbase} :NAME)` matcher used by `Struct.new` / `Data.define`.
    fn is_toplevel_const(&self, node: &Node<'_>, expected: &[u8]) -> bool {
        if let Some(c) = node.as_constant_read_node() {
            c.name().as_slice() == expected
        } else if let Some(c) = node.as_constant_path_node() {
            c.parent().is_none() && c.name().is_some_and(|n| n.as_slice() == expected)
        } else {
            false
        }
    }

    fn handle_send(&mut self, call: &ruby_prism::CallNode<'_>) {
        let method = call.name().as_slice().to_vec();
        let receiver = call.receiver();
        let args: Vec<Node<'_>> = call
            .arguments()
            .map(|a| a.arguments().iter().collect())
            .unwrap_or_default();

        if (method == b"define_method" || method == b"define_singleton_method")
            && receiver.is_none()
        {
            if let Some(first) = args.first() {
                self.handle_literal(first);
            }
        } else if receiver
            .as_ref()
            .is_some_and(|r| self.is_toplevel_const(r, b"Struct") && method == b"new")
        {
            // Drop a leading string argument (the struct's class name).
            let skip = args.first().is_some_and(|a| a.as_string_node().is_some());
            for arg in args.iter().skip(usize::from(skip)) {
                self.handle_literal(arg);
            }
        } else if receiver
            .as_ref()
            .is_some_and(|r| self.is_toplevel_const(r, b"Data") && method == b"define")
        {
            for arg in &args {
                self.handle_literal(arg);
            }
        } else if method == b"alias_method" && receiver.is_none() {
            if args.len() == 2 {
                self.handle_literal(&args[0]);
            }
        } else if receiver.is_none()
            && matches!(
                method.as_slice(),
                b"attr_reader" | b"attr_writer" | b"attr_accessor" | b"attr"
            )
            && !args.is_empty()
        {
            self.handle_attr(call, &args);
        }
    }

    /// `attr_*` accessor. The style-violation range spans the argument list
    /// (`selector end + 1 .. send end`); the forbidden range/name use the last
    /// argument literal, mirroring `range_position` / `register_forbidden_name`.
    fn handle_attr(&mut self, call: &ruby_prism::CallNode<'_>, args: &[Node<'_>]) {
        let Some(selector) = call.message_loc() else {
            return;
        };
        let style_start = selector.end_offset() + 1;
        let style_end = call.location().end_offset();
        let last = args.last().unwrap();
        let fb_loc = last.location();
        let fb_name = literal_name(last);

        for arg in args {
            let name = match literal_name(arg) {
                Some(n) => n,
                None => continue,
            };
            let valid = valid_for(&name, self.style);
            let alternative = if valid {
                0
            } else {
                first_alternative(&name, self.style)
            };
            self.candidates.push(MethodNameCandidate {
                start_offset: style_start,
                end_offset: style_end,
                name,
                valid,
                alternative,
                forbidden_start: fb_loc.start_offset(),
                forbidden_end: fb_loc.end_offset(),
                forbidden_name: fb_name.clone().unwrap_or_default(),
            });
        }
    }
}

/// The static symbol/string value of an `attr_*` argument, if any.
fn literal_name(node: &Node<'_>) -> Option<String> {
    if let Some(s) = node.as_symbol_node() {
        s.value_loc()?;
        Some(String::from_utf8_lossy(s.unescaped()).into_owned())
    } else {
        node.as_string_node()
            .map(|s| String::from_utf8_lossy(s.unescaped()).into_owned())
    }
}

impl<'pr> Visit<'pr> for Visitor<'_> {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let name = String::from_utf8_lossy(node.name().as_slice()).into_owned();
        if !is_operator(name.as_bytes()) {
            let loc = node.name_loc();
            self.push_def(
                name,
                loc.start_offset(),
                loc.end_offset(),
                node.receiver().is_some(),
            );
        }
        // Recurse into the body within a `def` scope so nested defs are found.
        self.scopes.push(Scope {
            is_def: true,
            ..Scope::default()
        });
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.resolve_scope();
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        let path = node.constant_path();
        let cloc = path.location();
        let cname = self.text(cloc.start_offset(), cloc.end_offset());
        if let Some(scope) = self.scopes.last_mut() {
            scope.class_names.push(cname);
        }
        self.scopes.push(Scope::default());
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.resolve_scope();
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        self.scopes.push(Scope::default());
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.resolve_scope();
    }

    fn visit_singleton_class_node(&mut self, node: &ruby_prism::SingletonClassNode<'pr>) {
        self.scopes.push(Scope::default());
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.resolve_scope();
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.handle_send(node);
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_alias_method_node(&mut self, node: &ruby_prism::AliasMethodNode<'pr>) {
        let new_name = node.new_name();
        if let Some(s) = new_name.as_symbol_node()
            && s.value_loc().is_some()
        {
            let name = String::from_utf8_lossy(s.unescaped()).into_owned();
            if !is_operator(name.as_bytes()) {
                let loc = new_name.location();
                self.push_named(name, loc.start_offset(), loc.end_offset());
            }
        }
        ruby_prism::visit_alias_method_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: &str, style: u8) -> Vec<(String, bool, u8)> {
        check_method_name(source.as_bytes(), style)
            .into_iter()
            .map(|c| (c.name, c.valid, c.alternative))
            .collect()
    }

    #[test]
    fn valid_for_snake_case() {
        for ok in [
            "foo_bar",
            "foo",
            "foo?",
            "foo!",
            "foo=",
            "foo_1",
            "última_vista",
            "_",
        ] {
            assert!(valid_for(ok, SNAKE_CASE), "{ok} should be valid snake_case");
        }
        for bad in ["fooBar", "MyMethod", "fooBar?"] {
            assert!(
                !valid_for(bad, SNAKE_CASE),
                "{bad} should be invalid snake_case"
            );
        }
    }

    #[test]
    fn valid_for_camel_case() {
        for ok in ["fooBar", "foo", "fooBar?", "_", "foo1"] {
            assert!(valid_for(ok, CAMEL_CASE), "{ok} should be valid camelCase");
        }
        for bad in ["foo_bar", "MyMethod", "Foo"] {
            assert!(
                !valid_for(bad, CAMEL_CASE),
                "{bad} should be invalid camelCase"
            );
        }
    }

    #[test]
    fn def_offense() {
        assert_eq!(
            names("def myMethod; end", SNAKE_CASE),
            vec![("myMethod".to_string(), false, CAMEL_CASE)]
        );
    }

    #[test]
    fn def_accepted() {
        assert_eq!(
            names("def my_method; end", SNAKE_CASE),
            vec![("my_method".to_string(), true, 0)]
        );
    }

    #[test]
    fn operator_def_skipped() {
        assert!(names("def +(o); end", SNAKE_CASE).is_empty());
        assert!(names("def ~@; end", SNAKE_CASE).is_empty());
    }

    #[test]
    fn defs_offense() {
        let got = names("def self.myMethod; end", SNAKE_CASE);
        assert_eq!(got, vec![("myMethod".to_string(), false, CAMEL_CASE)]);
    }

    #[test]
    fn class_emitter_accepted() {
        let src = "class Sequel\n  def self.Model(s); end\n  class Model; end\nend";
        let got = names(src, SNAKE_CASE);
        // `valid` flips true via the class-emitter exception; `alternative` is
        // unused once valid, so it keeps its pre-resolution value.
        assert_eq!(got, vec![("Model".to_string(), true, 255)]);
    }

    #[test]
    fn class_emitter_without_class_is_offense() {
        let src = "module Sequel\n  def self.Model(s); end\nend";
        let got = names(src, SNAKE_CASE);
        assert_eq!(got, vec![("Model".to_string(), false, 255)]);
    }

    #[test]
    fn define_method_symbol() {
        let got = names("define_method :fooBar do\nend", SNAKE_CASE);
        assert_eq!(got, vec![("fooBar".to_string(), false, CAMEL_CASE)]);
    }

    #[test]
    fn struct_members() {
        let got = names(
            r#"Struct.new("camelCase", :snake_case, :camelCase)"#,
            SNAKE_CASE,
        );
        // Leading string is the class name and skipped; `snake_case` is a valid
        // member, `camelCase` is the offender.
        assert_eq!(
            got,
            vec![
                ("snake_case".to_string(), true, 0),
                ("camelCase".to_string(), false, CAMEL_CASE)
            ]
        );
    }

    #[test]
    fn alias_method_first_arg() {
        let got = names("alias_method :fooBar, :foo", SNAKE_CASE);
        assert_eq!(got, vec![("fooBar".to_string(), false, CAMEL_CASE)]);
    }

    #[test]
    fn alias_bareword() {
        let got = names("alias fooBar foo", SNAKE_CASE);
        assert_eq!(got, vec![("fooBar".to_string(), false, CAMEL_CASE)]);
    }

    #[test]
    fn attr_offense_range_spans_args() {
        let cands = check_method_name("attr_reader :myMethod".as_bytes(), SNAKE_CASE);
        assert_eq!(cands.len(), 1);
        // `selector end + 1` = after `attr_reader ` (12) .. end (21).
        assert_eq!((cands[0].start_offset, cands[0].end_offset), (12, 21));
    }

    #[test]
    fn filtered_drops_valid_and_reports_had_valid() {
        let src = "def good_one; end\ndef badOne; end\ndef good_two; end";
        let (cands, had_valid) = check_method_name_filtered(src.as_bytes(), SNAKE_CASE, true);
        assert!(had_valid);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].name, "badOne");
        // Unfiltered keeps everything and still reports the flag.
        let (all, had_valid) = check_method_name_filtered(src.as_bytes(), SNAKE_CASE, false);
        assert!(had_valid);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn filtered_without_valid_candidates() {
        let (cands, had_valid) = check_method_name_filtered(b"def badOne; end", SNAKE_CASE, true);
        assert!(!had_valid);
        assert_eq!(cands.len(), 1);
    }

    // The class-emitter exception flips `valid` after the walk; the filter must
    // honour the post-resolution value.
    #[test]
    fn filtered_respects_class_emitter_resolution() {
        let src = "class Sequel\n  def self.Model(s); end\n  class Model; end\nend";
        let (cands, had_valid) = check_method_name_filtered(src.as_bytes(), SNAKE_CASE, true);
        assert!(had_valid);
        assert!(cands.is_empty());
    }
}
