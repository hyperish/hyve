# 01. DX 및 코어 매크로·CLI 설계 (hyper)

> 상태: 범용 모델·검증·설정·상태(`signal`)·에셋(`asset!`) DX 및 `init`/`preview`/`lint`/`test`/`build` CLI 범위는 **확정**, 세부 키워드는 **제안** · 기준일: 2026-11-18

`hyper`의 목표는 그래픽 엔진, CLI, 네이티브 툴, 백엔드(`ring`), UI(`lace`) 등 **어떤 종류의 Rust 프로젝트를 작성하든 파편화된 서드파티 크레이트 조립 없이 단일 코어와 단일 CLI 도구로 즉시 개발을 시작**하게 하는 것입니다. 범용 원칙은 `hyperish/docs`의 `01-dx-principles.md`를 따릅니다. (UI 뷰 마크업 DSL은 [`lace`](https://github.com/hyperish/lace)가 전담합니다.)

## 1. 범용 코어 DX: 모델 · 검증 · 설정 · 상태 · 에셋

- **단일 `#[model]` 선언으로 직렬화·검증·스키마 일원화**:
  - `serde`, `validator`, 스키마 리플렉션 크레이트를 따로 붙이지 않고 `#[model]`과 `#[validate(...)]`만으로 고성능 직렬화(`hyper-serde`), 구조적 검증(`hyper-validate`), 범용 스키마 메타데이터를 한 번에 얻습니다.
- **시작 시점 환경 설정 검증 (`hyper-config`)**:
  - `.env`, `hyper.toml`, 환경 변수를 `#[model]` 구조체로 읽어 앱 시작 즉시 검증합니다.
- **플랫폼 중립 반응형 상태 (`signal(init)`)**:
  - UI뿐 아니라 범용 애플리케이션·시뮬레이션·엔진 상태 그래프에서도 직관적인 `signal(0)`을 동일하게 사용합니다.
- **컴파일 타임 에셋 검증 (`asset!`)**:
  - 파일·셰이더·설정 리소스 경로 오타를 컴파일 타임에 잡아냅니다.

### 개념 스케치 (제안)

```rust
use hyper::prelude::*;

// 1) 모델 + 데이터 검증 + 환경 설정 스키마를 한 번에 선언
#[model]
pub struct EngineConfig {
    #[validate(len = 1..=64)]
    pub app_name: String,
    #[validate(range = 30..=240)]
    pub max_fps: u32,
}

#[hyper::test]
async fn config_and_reactive_state_work() {
    // 2) 설정 로드 시점에 즉시 Validation 수행
    let cfg = config::load::<EngineConfig>()?;
    let fps = signal(cfg.max_fps);
    let default_shader = asset!("/shaders/pbr.wgsl");

    fps.set(120);
    assert_eq!(fps.get(), 120);
    assert!(default_shader.exists());
}
```

## 2. 올인원 개발 환경 CLI (`hyper` — Bun 포지션)

Node.js에서 `npm` + `vite` + `vitest` + `eslint`를 따로 설치·설정하던 것을 `bun` 하나로 통합했듯, Rust에서 `cargo` + `cargo-watch` + `trunk` + `wasm-bindgen` + `cargo-nextest` + `clippy`를 개발자가 직접 조합하지 않도록 `hyper` CLI가 **Cargo 워크스페이스 위에서 동작하는 올인원 슈퍼셋 진입점**을 제공합니다.

| 명령 | 역할 (Bun 대응) | 동작 방향 (제안) |
|---|---|---|
| `hyper init` | 프로젝트·크레이트 초기화 (`bun init` / `create`) | 풀스택(`ring`+`lace`), UI 단독(`lace` 웹/네이티브), 백엔드 단독(`ring`) 스타터 템플릿을 즉시 생성하고 수 분 내 실행 가능한 기본 예제를 포함 |
| `hyper preview` | 실시간 미리보기·핫리로드 (`bun --hot` / dev server) | 코드·DSL 변경 시 `lace` 웹 뷰포트·네이티브 창 및 `ring` 로컬 서버를 즉시 리로드하고 상태(state)를 보존 (변경된 계약에 대한 증분 `codegen` 자동 수행) |
| `hyper lint` | 정적 검사·포맷·규약 검증 | `rustfmt`/`clippy`뿐 아니라 하이퍼리쉬 DSL 규약, 코어 경계 위반(코어에 어댑터 타입 유입 등), API 스키마 호환성을 함께 진단 |
| `hyper test` | 통합 고속 테스트 러너 (`bun test`) | 단위 테스트, proc macro 전개·컴파일 타임 오류 스냅샷 테스트, 어댑터 계약 테스트를 단일 러너로 일괄 고속 실행 |
| `hyper build` | 타깃별 프로덕션 빌드·패키징 (`bun build`) | `ring` 서버 바이너리, `lace` 웹 번들(WASM/SSG), `lace` 네이티브 앱 번들(macOS `.app`/`.dmg`, Windows `.msi`/`.exe`, Linux `AppImage`/`.deb`, iOS `.ipa`, Android `.apk`/`.aab` + 코드 사이닝 및 데스크톱 델타 자동 업데이터) 패키징 수행 (`codegen` 및 `asset!` 번들링 선행 포함) |
| `hyper codegen` | 단독 코드 생성 (보조 명령) | `hyper build`나 `hyper preview` 내부에서 자동 실행되는 `codegen` 단계만 단독으로 실행할 때 사용하는 보조 명령(IDE 자동완성 갱신, 외부 스키마·바인딩 사전 생성 및 점검용) |

> 이 표가 `hyper` CLI 대표 명령의 기준 목록입니다. `codegen`은 별개의 독립 워크플로가 아니라 `build`·`preview` 파이프라인에 기본 포함되는 선행 단계이며, 전체 컴파일 없이 생성 코드만 갱신·검증할 때 `hyper codegen`으로 단독 호출합니다.

## 3. 진단 메시지 원칙

- 에러 메시지는 항상 **(1) 무엇이 잘못되었는지, (2) 어느 매크로/스팬에서 발생했는지, (3) 어떻게 고쳐야 하는지(예시 코드 포함)** 세 가지를 함께 출력합니다.
- `cargo expand` 또는 `hyper lint --expand`로 proc macro가 생성한 순수 Rust 코드를 언제든 투명하게 확인할 수 있어야 합니다.
