use magnus::{Error, Ruby, function};

/// Ruby entry point for `Lint/Debugger`. Takes the source and the flattened
/// `DebuggerMethods` list, returns `[[start_offset, end_offset], ...]`.
fn check_debugger(source: String, methods: Vec<String>) -> Vec<(usize, usize)> {
    shirobai_core::rules::debugger::check_debugger(source.as_bytes(), &methods)
        .into_iter()
        .map(|o| (o.start_offset, o.end_offset))
        .collect()
}

#[magnus::init(name = "shirobai")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("Shirobai")?;
    module.define_module_function("check_debugger", function!(check_debugger, 2))?;
    Ok(())
}
