use hyve::model;

#[model]
struct Name {
    #[validate(pattern = "[")]
    name: String,
}

fn main() {}
