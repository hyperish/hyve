# 01. DX 및 DSL 설계 (hyper)

> 상태: 개발 생산성 중심 표현 개방성(QML/Slint·Dioxus `rsx!`·GPUI 빌더 레퍼런스), `lace` 웹·네이티브 단일 UI 문법, `init`/`preview`/`lint`/`test`/`build` CLI 범위는 **확정**, 세부 키워드는 **제안/미정** · 기준일: 2026-11-18

`hyper`의 목표는 개발자가 백엔드(`ring`)나 크로스플랫폼 UI(`lace`, 웹·네이티브) 어느 계층을 작성하든 **하나의 일관된 표현 감각과 하나의 CLI 도구**로 즉시 결과물을 확인하게 하는 것입니다. 범용 원칙은 `hyperish/docs`의 `01-dx-principles.md`를 따릅니다.

## 1. 생산성 중심 표현 원칙: DSL은 수단일 뿐, 더 효율적이면 어떤 방법이든 무관 (확정)

- **DSL 채택의 유일한 기준은 '개발 생산성'**: QML 기반 Slint 스타일이나 Dioxus `rsx!` 같은 매크로 DSL을 채택하는 이유는 오직 개발 생산성과 가독성을 높이기 위해서입니다. **이 목적이 충족되거나 더 효율적인 방법(GPUI식 순수 Rust 빌더·체이닝, 코드젠, 트레이트 조합 등)이 있다면 그 어떤 방법을 사용해도 무관**합니다.
- **`lace` 내 웹(DOM)과 네이티브(GPU/CPU)의 단일 공통 UI 문법**:
  - `hyper-parser`와 `hyper-macros`(또는 공통 빌더 코어)가 **공통 UI 문법**을 제공하여, `lace`의 웹 타깃과 네이티브 타깃은 완전히 같은 UI 문법 구조를 씁니다.
  - 기본 컴포넌트·레이아웃·바인딩 문법은 동일하고, 그 안에 웹(DOM) 전용 기능/어트리뷰트가 들어가느냐 네이티브(OS/GPU) 전용 기능이 들어가느냐에 따라 타깃 확장이 나뉩니다.
- **컴파일 타임 검증**: 스키마 불일치, 누락된 필수 프로퍼티, 잘못된 상태 바인딩, 타깃에 맞지 않는 기능 사용은 런타임이 아니라 **컴파일러 진단 메시지**로 잡습니다.

### 개념 스케치 (제안 — 구체 문법은 미정)

```rust
use hyper::prelude::*;

// 1) QML/Slint + Dioxus 감각의 선언적 UI 문법 (lace 웹·네이티브 타깃 공통)
ui! {
    Card {
        padding: 12px,
        title: project.title,
        Button {
            text: if liked() { "좋아요 취소" } else { "좋아요" },
            clicked => { liked.toggle(); }
        }
    }
}

// 2) GPUI처럼 DSL 없이도 가독성이 좋은 순수 Rust 빌더 체이닝 (1:1 대응)
Card::new()
    .padding(px(12.0))
    .title(project.title.clone())
    .child(
        Button::new(if liked() { "좋아요 취소" } else { "좋아요" })
            .on_click(move |_| liked.toggle()),
    )
```

> 위 코드 블록은 선언적 DSL과 GPUI식 순수 Rust 빌더의 1:1 대응 방향을 설명하기 위한 스케치이며, 세부 토큰·키워드는 [`04-open-questions.md`](04-open-questions.md) #1에서 확정합니다.

## 2. 올인원 개발 환경 CLI (`hyper` — Bun 포지션)

Node.js에서 `npm` + `vite` + `vitest` + `eslint`를 따로 설치·설정하던 것을 `bun` 하나로 통합했듯, Rust에서 `cargo` + `cargo-watch` + `trunk` + `wasm-bindgen` + `cargo-nextest` + `clippy`를 개발자가 직접 조합하지 않도록 `hyper` CLI가 **Cargo 워크스페이스 위에서 동작하는 올인원 슈퍼셋 진입점**을 제공합니다.

| 명령 | 역할 (Bun 대응) | 동작 방향 (제안) |
|---|---|---|
| `hyper init` | 프로젝트·크레이트 초기화 (`bun init` / `create`) | 풀스택(`ring`+`lace`), UI 단독(`lace` 웹/네이티브), 백엔드 단독(`ring`) 스타터 템플릿을 즉시 생성하고 수 분 내 실행 가능한 기본 예제를 포함 |
| `hyper preview` | 실시간 미리보기·핫리로드 (`bun --hot` / dev server) | 코드·DSL 변경 시 `lace` 웹 뷰포트·네이티브 창 및 `ring` 로컬 서버를 즉시 리로드하고 상태(state)를 보존 (변경된 계약에 대한 증분 `codegen` 자동 수행) |
| `hyper lint` | 정적 검사·포맷·규약 검증 | `rustfmt`/`clippy`뿐 아니라 하이퍼리쉬 DSL 규약, 코어 경계 위반(코어에 어댑터 타입 유입 등), API 스키마 호환성을 함께 진단 |
| `hyper test` | 통합 고속 테스트 러너 (`bun test`) | 단위 테스트, proc macro 전개·컴파일 타임 오류 스냅샷 테스트, 어댑터 계약 테스트를 단일 러너로 일괄 고속 실행 |
| `hyper build` | 타깃별 프로덕션 빌드·패키징 (`bun build`) | `ring` 및 `lace`(웹/네이티브) 타깃별 프로덕션 컴파일·번들링·패키징을 수행하며, 스키마·바인딩·어댑터 코드 생성(`codegen`)을 빌드 파이프라인의 선행 단계로 기본 포함 |
| `hyper codegen` | 단독 코드 생성 (보조 명령) | `hyper build`나 `hyper preview` 내부에서 자동 실행되는 `codegen` 단계만 단독으로 실행할 때 사용하는 보조 명령(IDE 자동완성 갱신, 외부 스키마·바인딩 사전 생성 및 점검용) |

> 이 표가 `hyper` CLI 대표 명령의 기준 목록입니다. `codegen`은 별개의 독립 워크플로가 아니라 `build`·`preview` 파이프라인에 기본 포함되는 선행 단계이며, 전체 컴파일 없이 생성 코드만 갱신·검증할 때 `hyper codegen`으로 단독 호출합니다.

## 3. 진단 메시지 원칙

- 에러 메시지는 항상 **(1) 무엇이 잘못되었는지, (2) 어느 매크로/스팬에서 발생했는지, (3) 어떻게 고쳐야 하는지(예시 코드 포함)** 세 가지를 함께 출력합니다.
- `cargo expand` 또는 `hyper lint --expand`로 proc macro가 생성한 순수 Rust 코드를 언제든 투명하게 확인할 수 있어야 합니다.
