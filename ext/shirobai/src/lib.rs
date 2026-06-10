use magnus::{Error, Ruby, function};

/// Ruby entry point for `Lint/Debugger`. Takes the source, the flattened
/// `DebuggerMethods` list and the flattened `DebuggerRequires` list, and
/// returns `[[start_offset, end_offset], ...]`.
fn check_debugger(
    source: String,
    methods: Vec<String>,
    requires: Vec<String>,
) -> Vec<(usize, usize)> {
    shirobai_core::rules::debugger::check_debugger(source.as_bytes(), &methods, &requires)
        .into_iter()
        .map(|o| (o.start_offset, o.end_offset))
        .collect()
}

/// Ruby entry point for `Metrics/BlockLength`. Returns one entry per block
/// whose body exceeds `max`: `[[start, end, length, method_name, receiver], ...]`.
/// Config-driven allow filtering is applied on the Ruby side.
fn check_block_length(
    source: String,
    max: usize,
    count_comments: bool,
    count_as_one: Vec<String>,
) -> Vec<(usize, usize, usize, usize, String, String)> {
    shirobai_core::rules::block_length::check_block_length(
        source.as_bytes(),
        max,
        count_comments,
        &count_as_one,
    )
    .into_iter()
    .map(|c| {
        (
            c.start_offset,
            c.end_offset,
            c.head_end,
            c.length,
            c.method_name,
            c.receiver,
        )
    })
    .collect()
}

/// Ruby entry point for the complexity cops. Returns one entry per method:
/// `[[start, end, head_end, name, cyclomatic, perceived], ...]`.
#[allow(clippy::type_complexity)]
fn check_complexity(source: String) -> Vec<(usize, usize, usize, String, usize, usize)> {
    shirobai_core::rules::complexity::check_complexity(source.as_bytes())
        .into_iter()
        .map(|m| {
            (
                m.start_offset,
                m.end_offset,
                m.head_end,
                m.method_name,
                m.cyclomatic,
                m.perceived,
            )
        })
        .collect()
}

/// Ruby entry point for `Naming/VariableNumber`. Returns
/// `[[[start, end, id_type, name, alt], ...], had_correct]`.
#[allow(clippy::type_complexity)]
fn check_variable_number(
    source: String,
    style: u8,
    flags: u8,
    allowed_identifiers: Vec<String>,
) -> (Vec<(usize, usize, u8, String, u8)>, bool) {
    let (offenses, had_correct) = shirobai_core::rules::variable_number::check_variable_number(
        source.as_bytes(),
        style,
        flags,
        &allowed_identifiers,
    );
    let offenses = offenses
        .into_iter()
        .map(|o| {
            (
                o.start_offset,
                o.end_offset,
                o.identifier_type,
                o.name,
                o.alternative,
            )
        })
        .collect();
    (offenses, had_correct)
}

/// Ruby entry point for `Lint/SafeNavigationChain`. Returns
/// `[[start, end, replacement, wrap_start, wrap_end], ...]`.
fn check_safe_navigation_chain(
    source: String,
    nil_methods: Vec<String>,
) -> Vec<(usize, usize, String, usize, usize)> {
    shirobai_core::rules::safe_navigation_chain::check_safe_navigation_chain(
        source.as_bytes(),
        &nil_methods,
    )
    .into_iter()
    .map(|o| {
        (
            o.start_offset,
            o.end_offset,
            o.replacement,
            o.wrap_start,
            o.wrap_end,
        )
    })
    .collect()
}

/// Ruby entry point for `Layout/MultilineOperationIndentation`. Takes the
/// source, the enforced style (0=aligned, 1=indented), the configured
/// indentation width and the base `Layout/IndentationWidth` width. Returns
/// `[[start, end, column_delta, message], ...]`.
fn check_multiline_operation_indentation(
    source: String,
    style: u8,
    indent_width: usize,
    base_indent_width: usize,
) -> Vec<(usize, usize, isize, String)> {
    shirobai_core::rules::multiline_operation_indentation::check_multiline_operation_indentation(
        source.as_bytes(),
        style,
        indent_width,
        base_indent_width,
    )
    .into_iter()
    .map(|o| (o.start_offset, o.end_offset, o.column_delta, o.message))
    .collect()
}

/// Ruby entry point for `Layout/MultilineMethodCallIndentation`. Takes the
/// source, the enforced style (0=aligned, 1=indented,
/// 2=indented_relative_to_receiver), the configured indentation width and the
/// base `Layout/IndentationWidth` width. Returns `[[start, end, column_delta,
/// message, block_body_start, block_body_end, block_end_start, block_end_end],
/// ...]` (block fields are `0` when the call has no multiline block).
#[allow(clippy::type_complexity)]
fn check_multiline_method_call_indentation(
    source: String,
    style: u8,
    indent_width: usize,
    base_indent_width: usize,
) -> Vec<(usize, usize, isize, String, usize, usize, usize, usize)> {
    shirobai_core::rules::multiline_method_call_indentation::check_multiline_method_call_indentation(
        source.as_bytes(),
        style,
        indent_width,
        base_indent_width,
    )
    .into_iter()
    .map(|o| {
        (
            o.start_offset,
            o.end_offset,
            o.column_delta,
            o.message,
            o.block_body_start,
            o.block_body_end,
            o.block_end_start,
            o.block_end_end,
        )
    })
    .collect()
}

/// Ruby entry point for `Layout/DotPosition`. Takes the source and the enforced
/// style (0=leading, 1=trailing). Returns `[[dot_start, dot_end, remove_start,
/// remove_end, insert_pos], ...]`.
fn check_dot_position(source: String, style: u8) -> Vec<(usize, usize, usize, usize, usize)> {
    shirobai_core::rules::dot_position::check_dot_position(source.as_bytes(), style)
        .into_iter()
        .map(|o| {
            (
                o.start_offset,
                o.end_offset,
                o.remove_start,
                o.remove_end,
                o.insert_pos,
            )
        })
        .collect()
}

#[magnus::init(name = "shirobai")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Shirobai")?;
    module.define_module_function("check_debugger", function!(check_debugger, 3))?;
    module.define_module_function("check_block_length", function!(check_block_length, 4))?;
    module.define_module_function("check_complexity", function!(check_complexity, 1))?;
    module.define_module_function("check_variable_number", function!(check_variable_number, 4))?;
    module.define_module_function(
        "check_safe_navigation_chain",
        function!(check_safe_navigation_chain, 2),
    )?;
    module.define_module_function(
        "check_multiline_operation_indentation",
        function!(check_multiline_operation_indentation, 4),
    )?;
    module.define_module_function(
        "check_multiline_method_call_indentation",
        function!(check_multiline_method_call_indentation, 4),
    )?;
    module.define_module_function("check_dot_position", function!(check_dot_position, 2))?;
    Ok(())
}
