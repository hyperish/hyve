//! The same model checks and paths on the native and browser targets.

use hyve::model;
use hyve::validate::{Constraint, Definition, SchemaRegistry, Validate};

#[model(check = ordered)]
struct Interval {
    #[validate(pattern = r"^[A-Z]{2}$")]
    country: Option<String>,
    #[validate(check = positive)]
    start: i32,
    end: i32,
}

fn positive(n: &i32) -> bool {
    *n > 0
}
fn ordered(value: &Interval) -> bool {
    value.start < value.end
}

#[model]
struct Form {
    items: Vec<Interval>,
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn constraints_are_applied_at_the_declared_paths() {
    for country in [None, Some("KR".into())] {
        assert!(
            Interval {
                country,
                start: 1,
                end: 2
            }
            .validate()
            .is_ok()
        );
    }
    let invalid = Form {
        items: vec![Interval {
            country: Some("kr".into()),
            start: 0,
            end: -1,
        }],
    };
    let errors = invalid.validate().unwrap_err();
    let paths = errors
        .iter()
        .map(|v| (v.path.as_str(), v.rule))
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [
            ("$.items[0].country", "pattern"),
            ("$.items[0].start", "check"),
            ("$.items[0]", "check"),
        ]
    );
    let mut registry = SchemaRegistry::new();
    registry.schema_of::<Form>();
    assert!(registry.has_check("Interval"));
    assert!(!registry.has_check("Form"));
    let Some(Definition::Object(fields)) = registry.get("Interval") else {
        panic!("missing model")
    };
    assert_eq!(fields[0].constraints, [Constraint::Pattern(r"^[A-Z]{2}$")]);
    assert_eq!(fields[1].constraints, [Constraint::Check]);
}
