use magnus::{Error, Ruby, function};

/// Ruby entry point for `Lint/Debugger`. Takes the source, the flattened
/// `DebuggerMethods` list and the flattened `DebuggerRequires` list, and
/// returns `[[start_offset, end_offset], ...]`.
fn check_debugger(source: String, methods: Vec<String>, requires: Vec<String>) -> Vec<(usize, usize)> {
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
    .map(|c| (c.start_offset, c.end_offset, c.head_end, c.length, c.method_name, c.receiver))
    .collect()
}

#[magnus::init(name = "shirobai")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Shirobai")?;
    module.define_module_function("check_debugger", function!(check_debugger, 3))?;
    module.define_module_function("check_block_length", function!(check_block_length, 4))?;
    Ok(())
}
