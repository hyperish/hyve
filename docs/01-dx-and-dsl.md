# 01. DX 및 DSL 설계 (hyper)

> 상태: QML/Slint·Dioxus `rsx!`·GPUI 빌더 병행 원칙, `lace`·`hera` 공통 UI 문법, `init`/`preview`/`lint`/`test` 범위는 **확정**, 세부 키워드는 **제안/미정** · 기준일: 2026-11-17

`hyper`의 목표는 개발자가 백엔드(`ring`), 웹(`lace`), 네이티브(`hera`) 어느 계층을 작성하든 **하나의 일관된 표현 감각과 하나의 CLI 도구**로 즉시 결과물을 확인하게 하는 것입니다. 범용 원칙은 `hyperish/docs`의 `01-dx-principles.md`를 따릅니다.

## 1. 생산성 중심 표현 원칙: DSL은 수단일 뿐, 더 효율적이면 어떤 방법이든 무관 (확정)

- **DSL 채택의 유일한 기준은 '개발 생산성'**: QML 기반 Slint 스타일이나 Dioxus `rsx!` 같은 매크로 DSL을 채택하는 이유는 오직 개발 생산성과 가독성을 높이기 위해서입니다. **이 목적이 충족되거나 더 효율적인 방법(GPUI식 순수 Rust 빌더·체이닝, 코드젠, 트레이트 조합 등)이 있다면 그 어떤 방법을 사용해도 무관**합니다.
- **`lace`(웹)와 `hera`(네이티브)의 단일 공통 UI 문법**:
  - `hyper-parser`와 `hyper-macros`(또는 공통 빌더 코어)가 **공통 UI 문법**을 제공하여, `lace`와 `hera`는 완전히 같은 UI 문법 구조를 씁니다.
  - 기본 컴포넌트·레이아웃·바인딩 문법은 동일하고, 그 안에 웹(DOM) 전용 기능/어트리뷰트가 들어가면 `lace`로, 네이티브(OS/GPU) 전용 기능이 들어가면 `hera`로 나뉩니다.
- **컴파일 타임 검증**: 스키마 불일치, 누락된 필수 프로퍼티, 잘못된 상태 바인딩, 타깃에 맞지 않는 기능 사용은 런타임이 아니라 **컴파일러 진단 메시지**로 잡습니다.

### 개념 스케치 (제안 — 구체 문법은 미정)

```rust
use hyper::prelude::*;

// 1) QML/Slint + Dioxus 감각의 선언적 UI 문법 (lace와 hera가 동일한 문법 공유)
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
| `hyper init` | 프로젝트·크레이트 초기화 (`bun init` / `create`) | 풀스택(`ring`+`lace`/`hera`), 웹 단독(`lace`), 네이티브 단독(`hera`), 백엔드 단독(`ring`) 스타터 템플릿을 즉시 생성하고 수 분 내 실행 가능한 기본 예제를 포함 |
| `hyper preview` | 실시간 미리보기·핫리로드 (`bun --hot` / dev server) | 코드·DSL 변경 시 웹(`lace`) 뷰포트, 네이티브(`hera`) 창, 백엔드(`ring`) 로컬 서버를 즉시 리로드하고 상태(state)를 보존 |
| `hyper lint` | 정적 검사·포맷·규약 검증 | `rustfmt`/`clippy`뿐 아니라 하이퍼리쉬 DSL 규약, 코어 경계 위반(코어에 어댑터 타입 유입 등), API 스키마 호환성을 함께 진단 |
| `hyper test` | 통합 고속 테스트 러너 (`bun test`) | 단위 테스트, proc macro 전개·컴파일 타임 오류 스냅샷 테스트, 어댑터 계약 테스트를 단일 러너로 일괄 고속 실행 |

## 3. 진단 메시지 원칙

- 에러 메시지는 항상 **(1) 무엇이 잘못되었는지, (2) 어느 매크로/스팬에서 발생했는지, (3) 어떻게 고쳐야 하는지(예시 코드 포함)** 세 가지를 함께 출력합니다.
- `cargo expand` 또는 `hyper lint --expand`로 proc macro가 생성한 순수 Rust 코드를 언제든 투명하게 확인할 수 있어야 합니다.
