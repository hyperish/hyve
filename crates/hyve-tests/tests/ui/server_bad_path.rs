use hyve::prelude::*;

#[server(GET "projects")]
pub async fn no_slash() -> HyveResult<()> {
    Ok(())
}

#[server(GET "/projects/{id")]
pub async fn open_brace() -> HyveResult<()> {
    Ok(())
}

fn main() {}
