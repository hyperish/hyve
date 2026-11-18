# 04. 열린 결정 (hyper)

> 기준일: 2026-11-17

`hyper` 레포 범위의 미정 사항 목록입니다. 범용 명세 전체에 걸친 미정 사항은 [`hyperish/docs/docs/07-open-questions.md`](https://github.com/hyperish/docs/blob/main/docs/07-open-questions.md)에서 추적합니다.

## 열려 있는 결정

| # | 결정 항목 | 왜 중요한가 | 지금의 방향(제안) |
|---|---|---|---|
| 1 | 공통 UI 문법(QML/Slint + Dioxus `rsx!`)의 세부 토큰·키워드 규격 | `lace`의 웹·네이티브 타깃이 공유할 단일 UI 문법의 프로퍼티·바인딩·이벤트 표기법을 고정해야 `hyper-parser` 구현 가능 | QML/Slint의 선언적 프로퍼티·바인딩 가독성 + Dioxus `rsx!`의 Rust 표현식 인라인 결합 + GPUI식 빌더 1:1 전개 |
| 2 | 포크 대상 베이스 크레이트 선정 (직렬화·파서·매크로·반응성) | 포크 + 1단계 `unit optimization`(추상화 제거 및 코어 직결)의 첫 대상 확정 필요 | 라이선스(MIT/Apache-2.0)와 벤치마크 확인 후 선정 |
| 3 | `hyper preview`의 핫리로드 메커니즘 | Rust 컴파일 시간을 극복하고 즉각적인 피드백 루프를 만드는 핵심 | Dioxus subsecond/핫패칭 아키텍처 및 QML/Slint형 선언적 프로퍼티 즉시 반영 방식 검토 |

## 결정됨

- **결정**: `hyper`는 **기존 Rust/Cargo 생태계가 Node.js라면 Bun에 해당하는 포지션**으로, 프레임워크 패밀리 전체의 올인원 개발 환경(`init`, `preview`, `lint`, `test`, `build`), 기본 내장 범용 코어 모듈(상태성, 직렬화, 파서), API 규격 인터페이스를 통합 관리함
  - **날짜**: 2026-11-17
  - **영향**: `hyper` 전체 문서(`00`~`03`)
- **결정**: UI/선언적 문법은 **QML 기반 Slint 스타일**과 **Dioxus `rsx!`**를 레퍼런스로 삼아 `lace`의 웹·네이티브 타깃이 동일한 문법을 공유하되, **GPUI처럼 DSL 없이 순수 Rust 빌더만으로도 가독성이 충분하면 DSL을 쓰지 않아도 되도록** 1:1 대응 설계
  - **날짜**: 2026-11-17 (2026-11-18 `lace` 통합 반영)
  - **영향**: `00-overview.md`, `01-dx-and-dsl.md`, `02-architecture.md`
- **결정**: `hyper-state`는 단일 스레드 고속 로컬 시그널(`!Send`)과 멀티스레드 공유 시그널(`Send + Sync`)을 이원화하되 동일한 반응형 API로 제공하며, `unit optimization`은 1단계(추상화·미사용 피처 제거 및 `hyper` 코어 직결)를 기본으로 하고 추후 병목 구간에 2단계(메모리 레이아웃·SIMD 등) 개조 여지를 둠
  - **날짜**: 2026-11-17
  - **영향**: `00-overview.md`, `02-architecture.md`
