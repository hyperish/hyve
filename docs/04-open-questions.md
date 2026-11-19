# 04. 열린 결정 (hyve)

> 기준일: 2026-11-19

`hyve` 레포 범위의 미정 사항 목록입니다. 범용 명세 전체에 걸친 미정 사항(내부 와이어 포맷, 인라인 서버 함수 계약 등)은 [`hyperish/docs/docs/07-open-questions.md`](https://github.com/hyperish/docs/blob/main/docs/07-open-questions.md)에서 추적합니다.

## 열려 있는 결정

| # | 결정 항목 | 왜 중요한가 | 지금의 방향(제안) |
|---|---|---|---|
| 1 | 포크 대상 베이스 크레이트 선정 (직렬화·파서·매크로·반응성) | 포크 + 1단계 `unit optimization`(추상화 제거 및 코어 직결)의 첫 대상 확정 필요 | 라이선스(MIT/Apache-2.0)와 벤치마크 확인 후 선정 |
| 2 | `hyve preview`의 핫리로드·핫패칭 메커니즘 | Rust 컴파일 시간을 극복하고 즉각적인 피드백 루프를 만드는 핵심 | 바이너리 핫패칭(Subsecond 계열) 및 상태(`hyve-state`) 보존 리로드 아키텍처 검토 |
| 3 | `hyve-state`의 `LocalSignal` / `SyncSignal` 공개 타입 노출 형태 | 두 타입을 완전히 분리하면 크로스플랫폼 컴포넌트 시그니처(`Signal<T>`)와 비동기 캡처에서 마찰이 발생함 | 단일 `Copy` 핸들(`Signal<T>`)을 기본 노출하고 `!Send` 로컬 값은 opt-in하거나 타깃별로 내부 구현을 숨기는 방안 검토 |
| 4 | `hyve` 구현 Phase 1~3 우선순위 컷 | 올인원 CLI와 범용 코어 전부를 한 번에 착수하면 초기 수직 슬라이스 완성이 지연됨 | Phase 1(`hyve-core`/`state`/`serde`/`validate`/`config`/`cli` 기본 `init`·`test`·`build`) → Phase 2(`preview` HMR·`lint`·`asset!`·`trace`·`SharedRingBuf` Shm) → Phase 3(바이너리 핫패칭·스토어 패키징·델타 업데이터) 제안 |

## 결정됨

- **Rust/Cargo 대비 Bun 포지셔닝 확정** (2026-11-17): 올인원 CLI(`init`, `preview`, `lint`, `test`, `build`) + 기본 내장 범용 코어 모듈(`00-overview.md`)
- **`hyve-state` 반응성 코어의 `signal(0)` 표기 및 이원화 스레드 모델(`LocalSignal` / `SyncSignal`) 확정** (2026-11-17 ~ 2026-11-18): `02-architecture.md` §2.2
- **웹 전용 OpenAPI 제외, UI 선언적 DSL(`ui!`) 제외 및 범용 스키마·검증(`hyve-validate`)·인터페이스(`hyve-api`) 경계 확정** (2026-11-18): `00-overview.md`, `02-architecture.md`
- **crates.io 비충돌 고유명 `hyve`(구 `hyper`) 확정** (2026-11-19): 레포명·crates.io 패키지명·Rust 크레이트명·CLI명을 `hyve`로 1:1 일치시킴
