# 01. DX 및 코어 매크로·CLI 설계 (hyve)

> 상태: 범용 모델·검증·설정·상태(`signal`)·에셋(`asset!`) DX 및 `init`/`preview`/`lint`/`test`/`build` CLI 범위는 **확정**, 세부 키워드 및 Phase 컷은 **제안** · 기준일: 2026-11-24

`hyve`의 목표는 그래픽 엔진, CLI, 네이티브 툴, 백엔드(`orbic`), UI(`serge`) 등 **어떤 종류의 Rust 프로젝트를 작성하든 파편화된 서드파티 크레이트 조립 없이 단일 코어와 단일 CLI 도구로 즉시 개발을 시작**하게 하는 것입니다. 범용 원칙은 `hyperish/docs`의 `01-dx-principles.md`를 따릅니다. (UI 뷰 마크업 DSL은 [`serge`](https://github.com/hyperish/serge)가 전담합니다.)

## 1. 범용 코어 DX: 모델 · 검증 · 설정 · 상태 · 에셋

- **단일 `#[model]` 선언으로 직렬화·검증·스키마 일원화**:
  - `serde`, `validator`, 스키마 리플렉션 크레이트를 따로 붙이지 않고 `#[model]`과 `#[validate(...)]`만으로 고성능 직렬화(`hyve-serde`), 구조적 검증(`hyve-validate`), 범용 스키마 메타데이터를 한 번에 얻습니다.
- **시작 시점 환경 설정 검증 (`hyve-config`)**:
  - `.env`, `hyve.toml`, 환경 변수를 `#[model]` 구조체로 읽어 앱 시작 즉시 검증합니다.
  - 우선순위는 CLI 인자 > 환경 변수 > `.env` > `hyve.toml` > 모델 기본값이며, 뒤의 원천은 앞의 값을 덮지 못합니다(제안).
  - 모델 기본값은 설정 모델의 `Default` 구현입니다. `Default`가 없는 모델은 모든 필드가 원천 중 하나에 있어야 합니다(제안).
  - `hyve.toml`은 최상위 테이블로 소유자를 나눕니다. `[app]`은 앱의 설정 모델 필드이고, `[hyve]`·`[orbic]`·`[serge]`는 각 프레임워크와 도구가 읽는 설정입니다. CLI 인자는 `--set <필드>=<값>` 형태로 `[app]`의 값을 덮습니다(제안).
  - 환경 변수 이름은 필드명의 SCREAMING_SNAKE_CASE에 앱이 정한 접두어를 붙입니다. 접두어는 `hyve.toml`의 `[hyve] env_prefix`로 정하고 기본값은 `APP`입니다(`fps` → `APP_FPS`). 프레임워크가 읽는 변수는 `HYPERISH_` 접두어를 씁니다(`hyperish/docs` `06-naming-conventions.md` §1). 검증에 실패하면 어느 원천의 어느 키가 틀렸는지 알립니다(제안).
  - **로더 세부 (제안, 2026-11-24)**: 뒤의 원천은 앞의 원천이 준 최상위 필드를 통째로 덮습니다. `hyve.toml`은 작업 디렉터리나 그 위로 가장 가까운 디렉터리에서 찾고 `.env`는 그 옆에서 읽습니다. 키는 Rust 필드 이름입니다. 환경 변수와 `--set` 값은 문자열 필드면 그대로, 그 밖의 필드면 TOML 값(`144`, `true`, `["a", "b"]`, `{ width = 800 }`)으로 읽습니다. 접두어가 맞지만 필드가 없는 환경 변수는 다른 프로그램과 공유하는 이름일 수 있어 무시하고, `[app]`의 모르는 키, 모르는 최상위 테이블, `--set`의 모르는 필드는 오류(오타 제안 포함)입니다. `--set`은 최상위 필드만 받습니다. 프레임워크·도구는 자기 테이블(`[orbic]` 등)을 같은 로더로 읽고, 그 환경 변수 접두어는 `HYPERISH_<테이블>`(`HYPERISH_ORBIC_DIALECT`)이며 `--set`은 `[app]`에만 적용됩니다. `hyve.toml`은 TOML 1.0의 부분집합을 받고 여러 줄 문자열·날짜와 시각·테이블 배열(`[[...]]`)은 지원하지 않는다는 진단을 냅니다.
- **플랫폼 중립 반응형 상태 (`signal(init)`)**:
  - UI뿐 아니라 범용 애플리케이션·시뮬레이션·엔진 상태 그래프에서도 직관적인 `signal(0)`을 동일하게 사용합니다.
- **컴파일 타임 에셋 검증 (`asset!`)**:
  - 파일·셰이더·설정 리소스 경로 오타를 컴파일 타임에 잡아냅니다.

### 개념 스케치 (제안)

```rust
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
    Ok(())
}
```

## 2. 올인원 개발 환경 CLI (`hyve` — Bun 포지션)

Node.js에서 `npm` + `vite` + `vitest` + `eslint`를 따로 설치·설정하던 것을 `bun` 하나로 통합했듯, Rust에서 `cargo` + `cargo-watch` + `trunk` + `wasm-bindgen` + `cargo-nextest` + `clippy`를 개발자가 직접 조합하지 않도록 `hyve` CLI가 **Cargo 워크스페이스 위에서 동작하는 올인원 슈퍼셋 진입점**을 제공합니다.

| 명령 | 역할 (Bun 대응) | 동작 방향 (제안) |
|---|---|---|
| `hyve init` | 프로젝트·크레이트 초기화 (`bun init` / `create`) | 풀스택(`orbic`+`serge`), UI 단독(`serge` 웹/네이티브), 백엔드 단독(`orbic`) 스타터 템플릿을 즉시 생성하고 수 분 내 실행 가능한 기본 예제를 포함 |
| `hyve preview` | 실시간 미리보기·핫리로드 (`bun --hot` / dev server) | 코드·DSL 변경 시 `serge` 웹 뷰포트·네이티브 창 및 `orbic` 로컬 서버를 즉시 리로드하고 상태(state)를 보존 (변경된 계약에 대한 증분 `codegen` 자동 수행) |
| `hyve lint` | 정적 검사·포맷·규약 검증 | `rustfmt`/`clippy`뿐 아니라 하이퍼리쉬 워크스페이스 규약(각 레포 린트 플러그인 포함), 코어 경계 위반(코어에 어댑터 타입 유입 등), API 스키마 호환성을 함께 진단 |
| `hyve test` | 통합 고속 테스트 러너 (`bun test`) | 단위 테스트, proc macro 전개·컴파일 타임 오류 스냅샷 테스트, 어댑터 계약 테스트를 단일 러너로 일괄 고속 실행 |
| `hyve build` | 타깃별 프로덕션 빌드·패키징 (`bun build`) | `orbic` 서버 바이너리, `serge` 웹 번들(WASM/SSG), `serge` 네이티브 앱 번들 패키징(`codegen` 및 `asset!` 번들링 선행 포함; macOS `.app`/`.dmg`·Windows `.msi`/`.exe`·Linux `AppImage`/`.deb`·iOS `.ipa`·Android `.apk`/`.aab` 스토어 패키징·코드 사이닝·델타 자동 업데이터는 Phase 2~3 단계적 확장) |
| `hyve codegen` | 단독 코드 생성 (보조 명령) | `hyve build`나 `hyve preview` 내부에서 자동 실행되는 `codegen` 단계만 단독으로 실행할 때 사용하는 보조 명령(IDE 자동완성 갱신, 외부 스키마·바인딩 사전 생성 및 점검용) |

> 이 표가 `hyve` CLI 대표 명령의 기준 목록입니다. `codegen`은 별개의 독립 워크플로가 아니라 `build`·`preview` 파이프라인에 기본 포함되는 선행 단계이며, 전체 컴파일 없이 생성 코드만 갱신·검증할 때 `hyve codegen`으로 단독 호출합니다.

### CLI 공통 계약 (제안, 2026-11-24)

- 아직 구현되지 않은 명령이나 옵션은 성공으로 끝내지 않습니다. 미구현임을 알리고 0이 아닌 종료 코드로 끝납니다.
- `hyve test`, `hyve build`, `hyve lint`의 종료 코드는 실제 결과를 따릅니다. 대상 워크스페이스를 찾지 못하면 오류입니다.
- `hyve init`은 기존 파일을 덮어쓰지 않습니다. 대상 위치에 파일이 있으면 중단하고, 덮어쓰기는 명시한 옵션으로만 합니다.
- 알 수 없는 템플릿 이름과 옵션은 오류입니다.
- **Phase 1의 이름**: `hyve init --template <이름>`의 템플릿은 `fullstack`, `serge-app`, `orbic-service` 셋입니다. 덮어쓰기 옵션은 `--force`입니다. `hyve preview`는 `--web`과 `--native`로 UI 타깃을 고르고, 둘 다 없으면 워크스페이스에 있는 대상을 모두 띄웁니다. 템플릿이 적는 `hyve`·`orbic`·`serge` 버전 요구는 그 CLI 릴리스와 함께 검증한 버전입니다.
- **`init` 구현 (제안, 2026-11-25)**: 템플릿을 주지 않으면 `fullstack`입니다. 대상은 `<path>`(없으면 작업 디렉터리)이고 패키지 이름은 그 디렉터리 이름입니다(영문자로 시작하고 영문자·숫자·`-`·`_`만, 아니면 `2`). 템플릿이 쓸 파일 중 이미 있는 것이 하나라도 있으면 그 목록을 알리고 아무것도 쓰지 않은 채 `1`로 끝나며, `--force`는 그 파일들을 덮어씁니다. 템플릿이 쓰지 않는 파일은 건드리지 않습니다. 세 템플릿은 모두 패키지 하나짜리 워크스페이스이고 같은 예제(노트 목록)를 씁니다. `fullstack`은 웹·데스크톱 UI(`[package.metadata.serge] web = true`), `#[server]` 함수, 백엔드 실행 파일(`<이름>-server`, `[package.metadata.orbic.server]`, `server` 피처)과 SQLite 마이그레이션, `hyve.toml`의 `[serge] api_proxy`·`[orbic] dialect`·`database_url`을 담습니다. `serge-app`은 카운터 UI만, `orbic-service`는 HTTP 라우트와 DB만(`server` 피처가 기본) 담습니다. 서버는 `Db::from_config()`로 붙고 시작할 때 마이그레이션을 적용하므로 `hyve preview`(로컬 DB)와 `cargo run`(`database_url`) 모두 바로 돕니다. 적는 버전은 `hyve`는 CLI 자신의 버전, `orbic`·`serge`는 그 CLI와 함께 검증한 버전, `wasm-bindgen`은 `serge`가 고정한 버전이며 모두 정확한 버전 요구(`=`)입니다.
- **설치**: 실행 파일 `hyve`는 `hyve-cli` 패키지가 제공합니다.
- **종료 코드와 Phase 1 옵션 (제안, 2026-11-24)**: `0` 성공, `1` 실패(작업 실패·워크스페이스 없음; 실행한 `cargo`가 실패하면 그 종료 코드를 그대로 씀), `2` 알 수 없는 명령·옵션·템플릿, `3` 이 릴리스에 없는 명령·기능. 대상 워크스페이스는 작업 디렉터리를 감싸는 Cargo 워크스페이스(`cargo metadata`)입니다. `hyve test [-p <패키지>]... [--release] [-- <테스트 인자>...]`는 `cargo test --workspace`(또는 고른 패키지)를, `hyve build [-p <패키지>]... [--debug]`는 기본이 최적화 빌드인 `cargo build`를 실행합니다. 멤버가 `orbic`·`serge`에 직접 의존하면 `build`와 `codegen`은 아래의 프레임워크 단계를 호출합니다(`build`는 프레임워크가 `build` 단계를 제공할 때만 그것을 먼저 실행하고 `cargo build`로 이어감). 프레임워크 의존이 없는 워크스페이스의 `codegen`(`--check` 포함)은 할 일이 없어 성공합니다.
- **프레임워크 단계 호출**: `hyve-cli`는 `orbic`과 `serge`를 링크하지 않습니다(의존 방향은 `orbic`, `serge` → `hyve`). 각 프레임워크는 자기 단계를 `hyve-<프레임워크>` 실행 파일(`hyve-orbic`, `hyve-serge`)로 제공하고, `hyve`는 대상 워크스페이스가 의존하는 프레임워크의 실행 파일을 같은 하위 명령 이름으로 호출합니다(`hyve codegen` → `hyve-orbic codegen`). `hyve`는 먼저 각 실행 파일에 제공하는 단계 목록을 물어(`hyve-<프레임워크> stages`) 제공하는 단계만 `orbic`, `serge` 순서로 호출하고, 버전은 실행 파일이 알려 준 값과 워크스페이스 `Cargo.lock`의 프레임워크 버전을 비교합니다. 실행 파일이 없거나 워크스페이스가 쓰는 프레임워크 버전과 다르면 설치 방법을 알리고 오류로 끝냅니다. 하위 프로세스의 종료 코드가 0이 아니면 `hyve`도 실패합니다.
  - **세부 (제안, 2026-11-24)**: `hyve-<프레임워크> stages`의 출력은 첫 줄이 그 실행 파일의 프레임워크 버전이고, 이어서 한 줄에 하나씩 제공하는 단계 이름입니다. 단계는 워크스페이스 루트에서 `hyve-<프레임워크> <단계> [옵션]`으로 실행하며 `hyve codegen --check`의 `--check`를 그대로 넘기고, 실패하면 그 종료 코드로 끝납니다. `hyve build --debug`는 `build` 단계에 `--debug`를, `hyve preview`는 `--web`·`--native`를 그대로 넘깁니다(둘 다 없으면 옵션 없이 호출하고, 단계가 워크스페이스의 대상을 모두 띄웁니다). `preview`를 제공하는 실행 파일이 하나도 없으면 "미리 볼 것이 없음" 오류로 끝납니다(2026-11-25). `stages`는 그 워크스페이스에서 제공하는 단계를 답합니다(예: `hyve-orbic`은 서버를 선언한 패키지가 있을 때만 `preview`를 답함). 단계 이름은 `-`로 시작하지 않습니다. 그래서 실행 파일을 직접 쓸 때를 위해 `--version`(`-V`)은 `hyve-<프레임워크> <버전>` 한 줄을, `--help`(`-h`)는 사용법을 표준 출력에 쓰고 0으로 끝납니다. 뒤에 다른 인자가 붙으면 사용 오류(`2`)입니다(제안, 2026-11-25).
  - **`preview`의 동시 실행 (제안, 2026-11-25)**: `preview` 단계는 오래 도는 서버와 창이라 순서대로가 아니라 한꺼번에 띄웁니다. 각 단계의 표준 입력은 `hyve`가 쥔 파이프이고 환경 변수 `HYVE_STAGE_LIFELINE=stdin`으로 이를 알립니다. 이 표식을 본 도구는 표준 입력이 닫히면(`hyve`가 멈추게 하거나 `hyve`가 끝나면) 자기가 띄운 서버와 창을 정리하고 끝납니다. 진행 중이던 `cargo build`는 끝날 때까지 돌고 그 결과는 다음 실행에 쓰입니다. 어느 단계든 끝나면(실패, 또는 창을 닫아 성공으로 끝남) 미리보기 전체가 끝납니다. `hyve`는 나머지의 표준 입력을 닫고 5초 안에 끝나지 않는 것만 강제 종료한 뒤 그 단계의 종료 코드로 끝납니다. 그래서 도구는 미리 볼 것이 있을 때만 `stages`에 `preview`를 답합니다. 이 규칙은 다른 단계의 "순서대로 실행하고 실패하면 그 종료 코드로 끝냄"을 `preview`에서만 바꾼 것입니다. 도구 프로세스를 직접 강제 종료하면 그 도구가 띄운 프로세스가 남을 수 있습니다(신호 처리는 `unsafe` 없이 할 수 없어 표식으로 대신함). 버전은 워크스페이스 `Cargo.lock`의 그 프레임워크 패키지 버전과 문자열로 비교하고, `Cargo.lock`이 없으면 `cargo metadata`로 만든 뒤 읽습니다. 설치 안내는 `cargo install <프레임워크>-cli --version =<버전>`입니다(실행 파일을 담은 패키지가 `orbic-cli`, `serge-cli`).
- **`codegen`**: Phase 1의 `hyve codegen`은 프레임워크 단계를 순서대로 실행하는 것이 전부이고, `hyve codegen --check`는 생성물이 현재 소스와 일치하는지만 확인해 다르면 실패합니다. Phase 1에서 단계를 제공하는 것은 `orbic`의 `sql!` 캐시입니다(`orbic/docs/01-dx-and-dsl.md` §4).

## 3. 진단 메시지 원칙

- 에러 메시지는 항상 **(1) 무엇이 잘못되었는지, (2) 어느 매크로/스팬에서 발생했는지, (3) 어떻게 고쳐야 하는지(예시 코드 포함)** 세 가지를 함께 출력합니다.
- `cargo expand` 또는 `hyve lint --expand`로 proc macro가 생성한 순수 Rust 코드를 언제든 투명하게 확인할 수 있어야 합니다.
