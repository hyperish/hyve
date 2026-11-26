use hyve::prelude::*;

#[server(POST "/projects")]
pub fn create() -> HyveResult<()> {
    Ok(())
}

fn main() {}
