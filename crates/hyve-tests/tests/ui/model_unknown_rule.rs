use hyve::prelude::*;

#[model]
pub struct User {
    #[validate(prefix = "user")]
    pub name: String,
}

fn main() {}
