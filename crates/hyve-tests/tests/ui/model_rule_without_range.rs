use hyve::prelude::*;

#[model]
pub struct User {
    #[validate(len)]
    pub name: String,
}

fn main() {}
