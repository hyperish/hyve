use hyve::prelude::*;

#[server(POST "/jobs")]
pub async fn schedule(after: std::time::Duration) -> HyveResult<()> {
    let _ = after;
    Ok(())
}

fn main() {}
