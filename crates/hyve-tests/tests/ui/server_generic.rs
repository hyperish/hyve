use hyve::prelude::*;

#[server(POST "/echo")]
pub async fn echo<T>(value: T) -> HyveResult<T> {
    Ok(value)
}

fn main() {}
