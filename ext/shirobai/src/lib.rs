use magnus::{Error, Ruby};

#[magnus::init(name = "shirobai")]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let _module = ruby.define_module("Shirobai")?;
    Ok(())
}
