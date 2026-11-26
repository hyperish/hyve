use hyve::prelude::*;

#[model]
pub struct Borrowed<'a> {
    pub name: &'a str,
}

fn main() {}
