use hyve::prelude::*;

#[server(PUT "/projects")]
pub async fn put() -> HyveResult<()> {
    Ok(())
}

#[server(post "/projects")]
pub async fn lower() -> HyveResult<()> {
    Ok(())
}

fn main() {}
