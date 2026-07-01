//! `Naming/VariableNumber`.

use ruby_prism::{Node, Visit};

/// An identifier whose numbering does not match the configured style.
pub struct VariableNumberOffense {
    pub start_offset: usize,
    pub end_offset: usize,
    /// 0 = variable, 1 = method name, 2 = symbol.
    pub identifier_type: u8,
    pub name: String,
    /// First alternative style (0=snake_case,1=normalcase,2=non_integer) that
    /// the name matches, or 255 when none (unrecognized).
    pub alternative: u8,
}

const SNAKE_CASE: u8 = 0;
const NORMALCASE: u8 = 1;
const NON_INTEGER: u8 = 2;

/// Port of `ConfigurableNumbering::FORMATS`. A name is valid for a style when
/// its trailing numbering matches; the leading sigil (`@`, `$`) never affects
/// the result since all patterns are anchored at the end.
fn valid_for(name: &str, style: u8) -> bool {
    let chars: Vec<char> = name.chars().collect();
    let n = chars.len();
    if n == 0 {
        return true;
    }
    let trailing_digits = chars
        .iter()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .count();
    let ends_non_digit = trailing_digits == 0; // `\D\z`
    let all_digits = trailing_digits == n; // `\A\d+\z`
    let before = (trailing_digits < n).then(|| chars[n - trailing_digits - 1]);
    // `_\d+\z`: trailing digits immediately preceded by `_`.
    let underscore_digits = trailing_digits > 0 && before == Some('_');
    // `[^_\d]\d+\z`: trailing digits preceded by a non-underscore non-digit.
    let normal_digits = trailing_digits > 0 && before.is_some_and(|c| c != '_');
    // `\A_\d+\z`: the whole name is `_` followed by digits (implicit param).
    let implicit_param = n >= 2 && chars[0] == '_' && trailing_digits == n - 1;

    match style {
        SNAKE_CASE => ends_non_digit || underscore_digits || all_digits,
        NORMALCASE => ends_non_digit || normal_digits || all_digits || implicit_param,
        NON_INTEGER => ends_non_digit || all_digits || implicit_param,
        _ => true,
    }
}

/// First alternative style (in `SupportedStyles` order) the name matches, or
/// 255 (unrecognized), mirroring `report_opposing_styles`.
fn first_alternative(name: &str, current: u8) -> u8 {
    for style in [SNAKE_CASE, NORMALCASE, NON_INTEGER] {
        if style != current && valid_for(name, style) {
            return style;
        }
    }
    255
}

fn strip_sigils(name: &str) -> String {
    name.chars().filter(|c| *c != '@' && *c != '$').collect()
}

pub fn check_variable_number(
    source: &[u8],
    style: u8,
    flags: u8,
    allowed_identifiers: &[String],
) -> (Vec<VariableNumberOffense>, bool) {
    let mut visitor = build_rule(source, style, flags, allowed_identifiers);
    super::parse_cache::with_parsed(source, |_source, node| visitor.visit(node));
    (visitor.offenses, visitor.had_correct)
}

/// Build the rule for use standalone or in a shared-walk bundle.
pub(crate) fn build_rule<'a>(
    source: &'a [u8],
    style: u8,
    flags: u8,
    allowed_identifiers: &'a [String],
) -> Visitor<'a> {
    Visitor {
        source,
        style,
        check_symbols: flags & 1 != 0,
        check_method_names: flags & 2 != 0,
        allowed_identifiers,
        offenses: Vec::new(),
        had_correct: false,
    }
}

pub(crate) struct Visitor<'a> {
    source: &'a [u8],
    style: u8,
    check_symbols: bool,
    check_method_names: bool,
    allowed_identifiers: &'a [String],
    pub(crate) offenses: Vec<VariableNumberOffense>,
    pub(crate) had_correct: bool,
}

impl Visitor<'_> {
    fn text(&self, start: usize, end: usize) -> String {
        String::from_utf8_lossy(&self.source[start..end]).into_owned()
    }

    /// Run the numbering check for one identifier. `name` is the identifier
    /// (with any sigil); `[start, end)` is the offense range.
    fn check(&mut self, name: String, start: usize, end: usize, identifier_type: u8) {
        let stripped = strip_sigils(&name);
        if self.allowed_identifiers.contains(&stripped) {
            return;
        }
        if valid_for(&name, self.style) {
            self.had_correct = true;
        } else {
            let alternative = first_alternative(&name, self.style);
            self.offenses.push(VariableNumberOffense {
                start_offset: start,
                end_offset: end,
                identifier_type,
                name,
                alternative,
            });
        }
    }

    fn check_loc(&mut self, loc: ruby_prism::Location<'_>, identifier_type: u8) {
        let (start, end) = (loc.start_offset(), loc.end_offset());
        self.check(self.text(start, end), start, end, identifier_type);
    }

    fn process(&mut self, node: &Node<'_>) {
        if let Some(n) = node.as_local_variable_write_node() {
            self.check_loc(n.name_loc(), 0);
        } else if let Some(n) = node.as_instance_variable_write_node() {
            self.check_loc(n.name_loc(), 0);
        } else if let Some(n) = node.as_class_variable_write_node() {
            self.check_loc(n.name_loc(), 0);
        } else if let Some(n) = node.as_global_variable_write_node() {
            self.check_loc(n.name_loc(), 0);
        } else if node.as_local_variable_target_node().is_some()
            || node.as_instance_variable_target_node().is_some()
            || node.as_class_variable_target_node().is_some()
            || node.as_global_variable_target_node().is_some()
            || node.as_required_parameter_node().is_some()
        {
            self.check_loc(node.location(), 0);
        } else if self.check_method_names
            && let Some(n) = node.as_def_node()
        {
            self.check_loc(n.name_loc(), 1);
        } else if self.check_symbols
            && let Some(n) = node.as_symbol_node()
        {
            let loc = node.location();
            let name = String::from_utf8_lossy(n.unescaped()).into_owned();
            self.check(name, loc.start_offset(), loc.end_offset(), 2);
        }
    }
}

impl super::dispatch::Rule for Visitor<'_> {
    fn interest(&self) -> super::dispatch::Interest {
        use super::dispatch::Interest;
        Interest(
            Interest::ENTER_ALL
                    | Interest::LEAF,
        )
    }
    
    fn enter(&mut self, node: &Node<'_>) {
        self.process(node);
    }

    fn leave(&mut self) {}

    fn enter_leaf(&mut self, node: &Node<'_>) {
        self.process(node);
    }
}

impl<'pr> Visit<'pr> for Visitor<'_> {
    fn visit_branch_node_enter(&mut self, node: Node<'pr>) {
        super::dispatch::Rule::enter(self, &node);
    }

    fn visit_leaf_node_enter(&mut self, node: Node<'pr>) {
        super::dispatch::Rule::enter_leaf(self, &node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(source: &str, style: u8) -> Vec<(String, u8)> {
        let (offenses, _) = check_variable_number(source.as_bytes(), style, 0b11, &[]);
        offenses
            .into_iter()
            .map(|o| (o.name, o.alternative))
            .collect()
    }

    #[test]
    fn valid_for_snake_case() {
        for ok in [
            "local_1", "local_12", "local_", "aB_1", "a_1_b", "_", "_foo", "@foo", "_1", "42",
        ] {
            assert!(valid_for(ok, SNAKE_CASE), "{ok} should be valid snake_case");
        }
        for bad in ["local1", "@local1", "camelCase1", "aB1", "a1"] {
            assert!(
                !valid_for(bad, SNAKE_CASE),
                "{bad} should be invalid snake_case"
            );
        }
    }

    #[test]
    fn valid_for_normalcase() {
        for ok in ["local1", "_1", "x42", "localFoo"] {
            assert!(valid_for(ok, NORMALCASE), "{ok} should be valid normalcase");
        }
        for bad in ["local_1", "sha_256", "myAttribute_1"] {
            assert!(
                !valid_for(bad, NORMALCASE),
                "{bad} should be invalid normalcase"
            );
        }
    }

    #[test]
    fn valid_for_non_integer() {
        for ok in ["localone", "_1", "42"] {
            assert!(
                valid_for(ok, NON_INTEGER),
                "{ok} should be valid non_integer"
            );
        }
        for bad in ["local1", "local_1"] {
            assert!(
                !valid_for(bad, NON_INTEGER),
                "{bad} should be invalid non_integer"
            );
        }
    }

    #[test]
    fn detects_local_variable() {
        assert_eq!(
            names("local1 = 1", SNAKE_CASE),
            vec![("local1".to_string(), NORMALCASE)]
        );
    }

    #[test]
    fn detects_instance_and_global_with_sigil() {
        let got = names("@local1 = 1\n$g1 = 1", SNAKE_CASE);
        assert_eq!(
            got,
            vec![
                ("@local1".to_string(), NORMALCASE),
                ("$g1".to_string(), NORMALCASE)
            ]
        );
    }

    #[test]
    fn detects_method_param_and_name() {
        let got = names("def method1(arg1); end", SNAKE_CASE);
        assert!(got.iter().any(|(n, _)| n == "method1"));
        assert!(got.iter().any(|(n, _)| n == "arg1"));
    }

    #[test]
    fn detects_symbol() {
        assert_eq!(
            names(":sym1", SNAKE_CASE),
            vec![("sym1".to_string(), NORMALCASE)]
        );
    }

    #[test]
    fn integer_symbols_accepted() {
        assert!(names(":\"42\"", SNAKE_CASE).is_empty());
        assert!(names("%i[1 2 3]", SNAKE_CASE).is_empty());
    }

    #[test]
    fn allowed_identifier_skipped() {
        let (offenses, _) = check_variable_number(
            "@capture3 = 1".as_bytes(),
            SNAKE_CASE,
            0b11,
            &["capture3".to_string()],
        );
        assert!(offenses.is_empty());
    }

    #[test]
    fn had_correct_flag() {
        let (offenses, had_correct) =
            check_variable_number("a1 = 1\na_2 = 1".as_bytes(), SNAKE_CASE, 0b11, &[]);
        assert_eq!(offenses.len(), 1);
        assert!(had_correct);
    }

    #[test]
    fn masgn_targets_checked() {
        assert_eq!(names("a1, b2 = 1, 2", SNAKE_CASE).len(), 2);
    }
}
