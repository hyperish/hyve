use hyve::prelude::*;

#[server(POST "/projects")]
pub async fn create(#[server_ctx] name: String) -> HyveResult<()> {
    let _ = name;
    Ok(())
}

fn main() {}
