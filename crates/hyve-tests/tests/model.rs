//! `#[model]`: codecs, validation, schema and fingerprints of the generated code.

use std::collections::BTreeMap;

use hyve::api::{CursorParams, Page};
use hyve::serde::json::{from_json, to_json};
use hyve::serde::{Binary, Fingerprint};
use hyve::validate::{Constraint, Definition, Schema, SchemaNode, SchemaRegistry, Valid};
use hyve::{Id, Timestamp, Ulid, Value};
use hyve_tests::*;

fn project() -> Project {
    Project {
        id: Id::from_ulid(Ulid::from_parts(1_700_000_000_000, 7)),
        owner: "ada".into(),
        title: "Atrium".into(),
        status: Status::InReview,
        tags: vec![Tag {
            name: "light".into(),
        }],
        note: None,
        version: (1 << 53) + 1,
        score: 0.25,
        created_at: Timestamp::from_unix_micros(1_700_000_000_123_456).unwrap(),
    }
}

#[test]
fn readme_and_sketch_models_compile_and_use_camel_case() {
    let scene = SceneConfig {
        name: "sunlit_atrium".into(),
        target_fps: 120,
    };
    assert_eq!(
        to_json(&scene),
        r#"{"name":"sunlit_atrium","targetFps":120}"#
    );
    let engine = EngineConfig {
        app_name: "demo".into(),
        max_fps: 144,
    };
    assert_eq!(to_json(&engine), r#"{"appName":"demo","maxFps":144}"#);
    assert!(Valid::new(scene).is_ok());
    assert!(
        Valid::new(EngineConfig {
            app_name: String::new(),
            max_fps: 10
        })
        .is_err()
    );
}

#[test]
fn both_codecs_round_trip() {
    let p = project();
    assert_eq!(Project::from_binary(&p.to_binary().unwrap()).unwrap(), p);
    let json = to_json(&p);
    assert!(json.contains(r#""status":"in_review""#), "{json}");
    assert!(
        json.contains(r#""version":"9007199254740993""#),
        "big integers travel as strings: {json}"
    );
    assert!(
        json.contains(r#""createdAt":"2023-11-14T22:13:20.123456Z""#),
        "{json}"
    );
    assert_eq!(from_json::<Project>(&json).unwrap(), p);
}

#[test]
fn json_decoding_rules() {
    // Optional fields may be omitted; unknown keys are skipped.
    let json = r#"{"id":"01HF7YAT00000000000000000Z","owner":"o","title":"t","status":"draft",
        "tags":[],"version":1,"score":1.0,"createdAt":"2024-01-01T00:00:00Z","extra":{"x":[1]}}"#;
    let p = from_json::<Project>(json).unwrap();
    assert_eq!((p.note, p.status), (None, Status::Draft));

    let missing = from_json::<Tag>("{}").unwrap_err();
    assert_eq!(missing.message(), "missing field `name`");
    assert!(from_json::<Tag>(r#"{"name":"a","name":"b"}"#).is_err());
    let bad_enum = from_json::<Status>(r#""archived""#).unwrap_err();
    assert!(
        bad_enum
            .message()
            .contains("expected one of `draft`, `in_review`, `published`"),
        "{bad_enum}"
    );
    let nested =
        from_json::<Project>(&json.replace(r#""tags":[]"#, r#""tags":[{"name":7}]"#)).unwrap_err();
    assert_eq!(nested.path(), "$.tags[0].name");
}

#[test]
fn validation_reports_every_rule_with_paths() {
    let mut p = project();
    p.title = String::new();
    p.score = 1.5;
    p.tags.push(Tag {
        name: "x".repeat(17),
    });
    let err = Valid::new(p).unwrap_err();
    let Some(Value::List(fields)) = err.details().get("fields") else {
        panic!("{err:?}")
    };
    let paths: Vec<_> = fields
        .iter()
        .filter_map(|f| f.get("path").cloned())
        .collect();
    assert_eq!(
        paths,
        vec![
            Value::from("$.title"),
            Value::from("$.tags[1].name"),
            Value::from("$.score")
        ]
    );
    assert_eq!(fields[2].get("max"), Some(&Value::Float(1.0)));
}

#[test]
fn schemas_describe_fields_rules_and_references() {
    let mut r = SchemaRegistry::new();
    assert_eq!(r.schema_of::<Project>(), SchemaNode::Ref("Project".into()));
    let Some(Definition::Object(fields)) = r.get("Project") else {
        panic!()
    };
    let names: Vec<_> = fields.iter().map(|f| f.json_name).collect();
    assert_eq!(
        names,
        [
            "id",
            "owner",
            "title",
            "status",
            "tags",
            "note",
            "version",
            "score",
            "createdAt"
        ]
    );
    let title = &fields[2];
    assert_eq!(
        title.constraints,
        vec![Constraint::Len {
            min: Some(1),
            max: Some(80)
        }]
    );
    assert!(
        title.required && !fields[5].required,
        "Option fields are not required"
    );
    assert_eq!(fields[3].schema, SchemaNode::Ref("Status".into()));
    assert_eq!(
        r.get("Status"),
        Some(&Definition::Enum(vec!["draft", "in_review", "published"]))
    );
    assert!(r.get("Tag").is_some());

    let Some(Definition::Object(scene)) = ({
        r.schema_of::<SceneConfig>();
        r.get("SceneConfig")
    }) else {
        panic!()
    };
    assert_eq!(
        scene[1].constraints,
        vec![Constraint::Range {
            min: Some(Value::Int(1)),
            exclusive_min: false,
            max: Some(Value::Int(240)),
            exclusive_max: false
        }]
    );
}

#[test]
fn generic_models_name_their_instances() {
    let mut r = SchemaRegistry::new();
    assert_eq!(
        r.schema_of::<Envelope<Tag>>(),
        SchemaNode::Ref("Envelope_Tag".into())
    );
    assert_eq!(
        r.schema_of::<Envelope<String>>(),
        SchemaNode::Ref("Envelope_String".into())
    );
    assert_ne!(
        <Envelope<Tag>>::FINGERPRINT,
        <Envelope<String>>::FINGERPRINT
    );
    let e = Envelope {
        item: Tag { name: "a".into() },
        count: 101,
    };
    assert_eq!(from_json::<Envelope<Tag>>(&to_json(&e)).unwrap(), e);
    assert!(Valid::new(e).is_err(), "range = ..=100");
}

#[test]
fn recursive_models_work() {
    let leaf = Comment {
        body: "leaf".into(),
        replies: vec![],
        parent: None,
    };
    let c = Comment {
        body: "root".into(),
        replies: vec![leaf.clone()],
        parent: Some(Box::new(leaf)),
    };
    assert_eq!(Comment::from_binary(&c.to_binary().unwrap()).unwrap(), c);
    assert_eq!(from_json::<Comment>(&to_json(&c)).unwrap(), c);
    let mut r = SchemaRegistry::new();
    assert_eq!(r.schema_of::<Comment>(), SchemaNode::Ref("Comment".into()));
    assert_ne!(Comment::FINGERPRINT, 0);
}

#[test]
fn fingerprints_follow_the_shape() {
    assert_ne!(SceneConfig::FINGERPRINT, EngineConfig::FINGERPRINT);
    assert_ne!(Tag::FINGERPRINT, <String as Fingerprint>::FINGERPRINT);
    assert_eq!(Status::FINGERPRINT, Status::FINGERPRINT);
}

#[test]
fn hand_written_api_shapes_match_their_model_twins() {
    let page = Page::new(vec![Tag { name: "a".into() }], Some("c".into()));
    let twin = twins::Page {
        items: page.items.clone(),
        next_cursor: page.next_cursor.clone(),
        has_more: page.has_more,
    };
    assert_eq!(to_json(&page), to_json(&twin));
    assert_eq!(<Page<Tag>>::FINGERPRINT, <twins::Page<Tag>>::FINGERPRINT);
    assert_eq!(&*page.to_binary().unwrap(), &*twin.to_binary().unwrap());

    let cursor = CursorParams {
        after: Some("c".into()),
        limit: Some(10),
    };
    let twin = twins::CursorParams {
        after: cursor.after.clone(),
        limit: cursor.limit,
    };
    assert_eq!(to_json(&cursor), to_json(&twin));
    assert_eq!(CursorParams::FINGERPRINT, twins::CursorParams::FINGERPRINT);

    let (mut a, mut b) = (SchemaRegistry::new(), SchemaRegistry::new());
    assert_eq!(
        <Page<Tag> as Schema>::schema(&mut a),
        <twins::Page<Tag> as Schema>::schema(&mut b)
    );
    assert_eq!(a.get("Page_Tag"), b.get("Page_Tag"));
    assert_eq!(
        CursorParams::schema(&mut a),
        twins::CursorParams::schema(&mut b)
    );
    assert_eq!(a.get("CursorParams"), b.get("CursorParams"));
}

#[test]
fn maps_and_values_inside_models_are_plain_fields() {
    // Not a model feature, but models compose with std containers.
    let m: BTreeMap<String, Tag> = [("k".to_string(), Tag { name: "v".into() })].into();
    assert_eq!(to_json(&m), r#"{"k":{"name":"v"}}"#);
}
