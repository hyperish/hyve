use hyve::prelude::*;

#[model]
pub struct User {
    #[validate(pattern = "^[a-z]+$")]
    pub name: String,
}

fn main() {}
