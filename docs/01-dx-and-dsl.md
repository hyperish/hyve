# 01. DX 및 DSL 설계 (hyper)

> 상태: Dioxus `rsx!`·매크로 레퍼런스 및 `init`/`preview`/`lint`/`test` 범위는 **확정**, 구체 문법은 **제안/미정** · 기준일: 2026-11-17

`hyper`의 목표는 개발자가 백엔드(`ring`), 웹(`lace`), 네이티브(`hera`) 어느 계층을 작성하든 **하나의 일관된 매크로 감각과 하나의 CLI 도구**로 즉시 결과물을 확인하게 하는 것입니다. 범용 원칙은 `hyperish/docs`의 `01-dx-principles.md`를 따릅니다.

## 1. 매크로 & DSL 설계 (레퍼런스: Dioxus `rsx!`)

- **읽기 쉬운 선언적 구조**: Dioxus의 `rsx!`와 컴포넌트·속성 매크로처럼 중첩 구조와 상태 바인딩, 이벤트/핸들러 연결이 한눈에 읽히는 문법을 지향합니다.
- **컴파일 타임 검증**: 스키마 불일치, 누락된 필수 필드, 잘못된 상태 참조, 허용되지 않는 속성 조합은 런타임이 아니라 **proc macro 전개 시점의 컴파일러 진단 메시지**로 잡습니다.
- **Rust로의 자연스러운 탈출구**: 매크로 내부 표현식은 표준 Rust 코드와 자연스럽게 섞여야 하며, 매크로 없이 하위 빌더/트레이트 API(`hyper_core`)를 직접 호출해도 100% 동일한 동작을 구성할 수 있어야 합니다.

### 개념 스케치 (제안 — 구체 문법은 미정)

```rust
use hyper::prelude::*;

// API/도메인 계약과 상태 모델을 선언적으로 정의하고 ring·lace·hera가 함께 참조
#[hyper::model]
pub struct ProjectSummary {
    pub id: Id<Project>,
    pub title: String,
    pub updated_at: Timestamp,
}

#[hyper::api]
pub trait ProjectService {
    #[get("/v1/projects/{id}")]
    async fn get_project(&self, id: Id<Project>) -> Result<ProjectSummary, ApiError>;
}
```

> 위 코드 블록은 DX 방향을 설명하기 위한 스케치이며, 구체 매크로 이름과 어트리뷰트 문법은 [`04-open-questions.md`](04-open-questions.md) #1에서 확정합니다.

## 2. 통합 개발 환경 CLI (`hyper`)

복잡한 빌드 스크립트나 타깃별 도구 체인을 개발자가 직접 조합하지 않도록 `hyper` CLI가 단일 진입점을 제공합니다.

| 명령 | 역할 | 동작 방향 (제안) |
|---|---|---|
| `hyper init` | 프로젝트·크레이트 초기화 | 풀스택(`ring`+`lace`/`hera`), 웹 단독(`lace`), 네이티브 단독(`hera`), 백엔드 단독(`ring`) 스타터 템플릿을 즉시 생성하고 수 분 내 실행 가능한 기본 예제를 포함 |
| `hyper preview` | 실시간 미리보기·핫리로드 | 코드·DSL 변경 시 웹(`lace`) 뷰포트, 네이티브(`hera`) 창, 백엔드(`ring`) 로컬 서버를 즉시 리로드하고 상태(state)를 보존 |
| `hyper lint` | 정적 검사·포맷·규약 검증 | `rustfmt`/`clippy`뿐 아니라 하이퍼리쉬 DSL 규약, 코어 경계 위반(코어에 어댑터 타입 유입 등), API 스키마 호환성을 함께 진단 |
| `hyper test` | 통합 테스트 러너 | 단위 테스트, proc macro 전개·컴파일 타임 오류 스냅샷 테스트, 어댑터 계약 테스트를 일괄 실행 |

## 3. 진단 메시지 원칙

- 에러 메시지는 항상 **(1) 무엇이 잘못되었는지, (2) 어느 매크로/스팬에서 발생했는지, (3) 어떻게 고쳐야 하는지(예시 코드 포함)** 세 가지를 함께 출력합니다.
- `cargo expand` 또는 `hyper lint --expand`로 proc macro가 생성한 순수 Rust 코드를 언제든 투명하게 확인할 수 있어야 합니다.
