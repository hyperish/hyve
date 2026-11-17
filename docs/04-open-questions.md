# 04. 열린 결정 (hyper)

> 기준일: 2026-11-17

`hyper` 레포 범위의 미정 사항 목록입니다. 범용 명세 전체에 걸친 미정 사항은 [`hyperish/docs/docs/07-open-questions.md`](https://github.com/hyperish/docs/blob/main/docs/07-open-questions.md)에서 추적합니다.

## 열려 있는 결정

| # | 결정 항목 | 왜 중요한가 | 지금의 방향(제안) |
|---|---|---|---|
| 1 | `hyper` 매크로·DSL의 구체 키워드와 문법 | `ring`, `lace`, `hera` 전체의 코드 첫인상을 좌우함 | Dioxus `rsx!` 및 `#[component]` 스타일을 레퍼런스로 삼아 선언적 블록과 어트리뷰트 매크로 조합 설계 |
| 2 | 포크 대상 베이스 크레이트 선정 (직렬화·파서·매크로·반응성) | 포크 + `unit optimization` 전략의 첫 대상 확정 필요 | 라이선스(MIT/Apache-2.0)와 벤치마크 확인 후 선정 |
| 3 | `hyper-state`의 동시성·스레드 모델 | 웹(단일 스레드 WASM 중심)과 네이티브/백엔드(멀티스레드) 양쪽에서 참조해야 함 | 로컬 아레나/단일 스레드 경로와 `Send + Sync` 원자적 경로를 분리하거나 피처/타입으로 구분 |
| 4 | `hyper preview`의 핫리로드 메커니즘 | Rust 컴파일 시간을 극복하고 즉각적인 피드백 루프를 만드는 핵심 | Dioxus subsecond/핫패칭 아키텍처 및 템플릿/상태 분리 리로드 방식 검토 |

## 결정됨

- **결정**: `hyper`가 프레임워크 패밀리 전체의 개발 환경(`init`, `preview`, `lint`, `test`), 범용 코어 모듈(상태성, 직렬화, 파서), API 규격 인터페이스를 통합 관리하며, Dioxus의 `rsx!`·매크로를 DX 레퍼런스로 삼음
  - **날짜**: 2026-11-17
  - **영향**: `hyper` 전체 문서(`00`~`03`)
