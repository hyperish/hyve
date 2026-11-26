use hyve::prelude::*;

#[server]
pub async fn ping() -> HyveResult<()> {
    Ok(())
}

fn main() {}
