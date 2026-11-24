use hyve::prelude::*;

#[validate(len = 1..=3)]
pub struct User {
    pub name: String,
}

fn main() {}
