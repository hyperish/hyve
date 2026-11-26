use hyve::prelude::*;

#[model]
pub struct Config {
    #[validate(len = 1..=3)]
    pub retries: u32,
}

fn main() {}
