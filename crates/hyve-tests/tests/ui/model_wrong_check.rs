use hyve::model;

#[model(check = check)]
struct Name {
    name: String,
}

fn check(_: &i32) -> bool { true }
fn main() {}
