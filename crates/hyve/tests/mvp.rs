//! Phase 1 MVP integration tests for `hyve`.

use hyve::prelude::*;

/// Scene rendering configuration model from `hyve/README.md`.
#[model]
pub struct SceneConfig {
    /// Human-readable scene name.
    #[validate(len = 1..=80)]
    pub name: String,
    /// Target frame rate cap.
    #[validate(range = 1..=240)]
    pub target_fps: u32,
}

/// Engine runtime configuration model from `hyve/docs/01-dx-and-dsl.md`.
#[model]
pub struct EngineConfig {
    /// Application identifier.
    #[validate(len = 1..=64)]
    pub app_name: String,
    /// Maximum frame rate limit.
    #[validate(range = 30..=240)]
    pub max_fps: u32,
}

#[hyve::test]
fn scene_config_readme_mvp_roundtrip_validation_and_schema() {
    let valid = SceneConfig {
        name: "sunlit_atrium".into(),
        target_fps: 120,
    };

    // 1) Binary round-trip (hyve-serde)
    let encoded = to_bytes(&valid);
    assert!(encoded.starts_with(b"HYVE"));
    let decoded: SceneConfig = from_bytes(&encoded).expect("binary decode");
    assert_eq!(decoded, valid);

    // 2) JSON round-trip using camelCase API fields
    let json = to_json_string(&valid);
    assert!(json.contains("\"targetFps\":120"));
    let from_json: SceneConfig = from_json_str(&json).expect("json decode");
    assert_eq!(from_json, valid);

    // 3) Structural validation (hyve-validate)
    assert!(Valid::new(valid).is_ok());
    let invalid = SceneConfig {
        name: String::new(),
        target_fps: 999,
    };
    let err = Valid::new(invalid).unwrap_err();
    assert_eq!(err.code(), "invalid_request");
    assert_eq!(err.http_status(), 400);

    // 4) Schema reflection
    let schema = SceneConfig::schema();
    assert_eq!(schema.name, "SceneConfig");
    assert_eq!(schema.fields.len(), 2);
    assert_eq!(
        schema.fields[0].constraints,
        vec![FieldConstraint::Len { min: 1, max: 80 }]
    );
    assert_eq!(
        schema.fields[1].constraints,
        vec![FieldConstraint::Range { min: 1, max: 240 }]
    );
}

#[hyve::test]
async fn config_and_reactive_state_work() -> HyveResult<()> {
    config::set_test_override("APP_NAME=HyperishEngine\nMAX_FPS=60\n");
    let cfg = config::load::<EngineConfig>()?;
    config::clear_test_override();

    assert_eq!(cfg.app_name, "HyperishEngine");
    let fps = signal(cfg.max_fps);
    let doubled = memo(move || fps() * 2);
    let default_shader = asset!("/shaders/pbr.wgsl");

    fps.set(120);
    assert_eq!(fps.get(), 120);
    assert_eq!(doubled(), 240);
    assert!(default_shader.exists());
    assert!(!asset!("/shaders/missing_does_not_exist.wgsl").exists());
    Ok(())
}

#[hyve::test]
fn shared_ring_buffer_and_cancellation_tree_work() {
    #[derive(Clone, Copy, Default, PartialEq, Debug)]
    struct FrameSample {
        frame_index: u32,
        gpu_ms: f32,
    }

    let rb = SharedRingBuf::new(3);
    rb.write(FrameSample {
        frame_index: 1,
        gpu_ms: 4.1,
    });
    rb.write(FrameSample {
        frame_index: 2,
        gpu_ms: 3.9,
    });
    rb.write(FrameSample {
        frame_index: 3,
        gpu_ms: 4.2,
    });
    rb.write(FrameSample {
        frame_index: 4,
        gpu_ms: 3.8,
    });

    let mut out = [FrameSample::default(); 2];
    let count = rb.read_latest_snapshot(&mut out);
    assert_eq!(count, 2);
    assert_eq!(out[0].frame_index, 3);
    assert_eq!(out[1].frame_index, 4);

    let root = CancellationToken::new();
    let child = root.child_token();
    root.cancel();
    assert!(child.is_cancelled());

    let diag = parser::Diagnostic::new(
        "missing closing brace",
        "Card { ... }",
        "add `}` at the end of the Card block",
    );
    let rendered = diag.to_string();
    assert!(rendered.contains("missing closing brace"));
    assert!(rendered.contains("add `}`"));
}
