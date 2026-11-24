//! The generated code works when `hyve` is only reachable through a re-export.

use hyve_tests::hyve;
use hyve_tests_indirect::{Boxed, Color, Note, add_note, remove_note};

use hyve::api::{ServerFn, StubFn};
use hyve::serde::{Binary, from_json, to_json};
use hyve::validate::{SchemaNode, SchemaRegistry, Valid};

fn descriptor<F: StubFn<A>, A>(_: F) -> &'static hyve::api::ServerFnDescriptor {
    <F::Marker as ServerFn>::descriptor()
}

#[test]
fn models_expand_through_the_override() {
    let n = Note {
        text: "hi".into(),
        pinned: true,
    };
    assert_eq!(to_json(&n), r#"{"text":"hi","pinned":true}"#);
    assert_eq!(Note::from_binary(&n.to_binary().unwrap()).unwrap(), n);
    assert!(
        Valid::new(Note {
            text: String::new(),
            pinned: false
        })
        .is_err()
    );
    assert_eq!(from_json::<Color>(r#""blue""#).unwrap(), Color::Blue);
    let mut r = SchemaRegistry::new();
    assert_eq!(
        r.schema_of::<Boxed<Note>>(),
        SchemaNode::Ref("Boxed_Note".into())
    );
}

#[test]
fn server_functions_expand_through_the_override() {
    let d = descriptor(add_note);
    assert_eq!(
        (d.id, d.args),
        ("hyve_tests_indirect::add_note", &["note", "color"][..])
    );
    assert_eq!(descriptor(remove_note).method, hyve::api::Method::Delete);
}

#[hyve::test(crate = ::hyve_tests::hyve)]
async fn stubs_and_the_test_attribute_work_through_the_override() -> hyve::HyveResult<()> {
    let err = add_note(
        Note {
            text: "x".into(),
            pinned: false,
        },
        Color::Red,
    )
    .await
    .unwrap_err();
    assert_eq!(err.code(), hyve::api::codes::TRANSPORT_UNAVAILABLE);
    Ok(())
}

#[cfg(feature = "server")]
#[hyve::test(crate = ::hyve_tests::hyve)]
async fn handler_bodies_run_through_the_override() -> hyve::HyveResult<()> {
    use hyve::api::ServerHandler;
    let scope = hyve::Scope::current().expect("test scope");
    scope.provide(hyve_tests_indirect::Store {
        prefix: "> ".into(),
    });
    let request = hyve_tests_indirect::__hyve_request_add_note {
        note: Note {
            text: "a".into(),
            pinned: false,
        },
        color: Color::Blue,
    };
    let note = hyve_tests_indirect::__hyve_server_add_note::handle(scope.child(), request).await?;
    assert_eq!(note.text, "> a:Blue");
    let err =
        hyve_tests_indirect::__hyve_server_remove_note::handle(scope.child(), hyve::Id::new())
            .await
            .unwrap_err();
    assert_eq!(err.code(), "note_not_found");
    Ok(())
}
