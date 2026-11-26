//! The documentation examples, compiled and run (`docs/05-implementation-plan.md`
//! §3): the README's `SceneConfig` and the `docs/01-dx-and-dsl.md` §1 sketch.
//! `config::load` and `asset!` use files from the fixture package.

#![allow(missing_docs)]

mod readme {
    use hyve::prelude::*;

    #[model]
    pub struct SceneConfig {
        #[validate(len = 1..=80)]
        pub name: String,
        #[validate(range = 1..=240)]
        pub target_fps: u32,
    }

    #[test]
    fn scene_config_round_trips_and_validates() {
        let scene = SceneConfig {
            name: "sunlit_atrium".into(),
            target_fps: 120,
        };
        let json = hyve::serde::to_json(&scene);
        assert_eq!(json, r#"{"name":"sunlit_atrium","targetFps":120}"#);
        let back: SceneConfig = hyve::serde::from_json(&json).unwrap();
        assert!(Valid::new(back).is_ok());
        assert!(
            Valid::new(SceneConfig {
                name: String::new(),
                target_fps: 0
            })
            .is_err()
        );
    }
}

mod sketch {
    use hyve::prelude::*;

    // 1) 모델 + 데이터 검증 + 환경 설정 스키마를 한 번에 선언
    #[model]
    pub struct EngineConfig {
        #[validate(len = 1..=64)]
        pub app_name: String,
        #[validate(range = 30..=240)]
        pub max_fps: u32,
    }

    #[hyve::test]
    async fn config_and_reactive_state_work() -> HyveResult<()> {
        // 2) 설정 로드 시점에 즉시 Validation 수행
        let cfg = config::load::<EngineConfig>()?;
        let fps = signal(cfg.max_fps);
        let default_shader = asset!("/shaders/pbr.wgsl");

        fps.set(120);
        assert_eq!(fps.get(), 120);
        assert!(default_shader.exists());
        assert_eq!(default_shader.path(), "/shaders/pbr.wgsl");
        assert_eq!(
            default_shader.bytes(),
            include_bytes!("../shaders/pbr.wgsl")
        );
        assert_eq!((cfg.app_name.as_str(), cfg.max_fps), ("hyve-tests", 144));
        Ok(())
    }
}
