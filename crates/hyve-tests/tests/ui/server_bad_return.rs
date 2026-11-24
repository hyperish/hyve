use hyve::prelude::*;

#[server(GET "/count")]
pub async fn count() -> u32 {
    1
}

fn main() {}
