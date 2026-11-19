# 04. 열린 결정 (hyve)

> 기준일: 2026-11-19

`hyve` 레포 범위의 미정 사항 목록입니다. 범용 명세 전체의 결정 기록은 [`hyperish/docs/docs/07-open-questions.md`](https://github.com/hyperish/docs/blob/main/docs/07-open-questions.md)에서 추적합니다.

## 열려 있는 결정

| # | 결정 항목 | 왜 중요한가 | 지금의 방향(제안) |
|---|---|---|---|
| 1 | `hyve preview`의 바이너리 핫패칭 세부 메커니즘 (Phase 3) | Rust 컴파일 시간을 극복하고 즉각적인 피드백 루프를 만드는 고급 단계 | Phase 2 상태 보존 HMR 우선 구축 후, Phase 3에서 Subsecond 계열 바이너리 핫패칭 검토 |

## 결정됨

- **Rust/Cargo 대비 Bun 포지셔닝 확정** (2026-11-17): 올인원 CLI(`init`, `preview`, `lint`, `test`, `build`) + 기본 내장 범용 코어 모듈(`00-overview.md`)
- **`hyve-state` 반응성 코어의 `signal(0)` 표기 및 단일 `Copy` 아레나 핸들(`Signal<T>`, `!Send`만 `LocalSignal<T>` 옵트인) 확정** (2026-11-17 ~ 2026-11-19): `02-architecture.md` §2.2
- **웹 전용 OpenAPI 제외, UI 선언적 DSL(`ui!`) 제외 및 범용 스키마·검증(`hyve-validate`)·인터페이스(`hyve-api`) 경계 확정** (2026-11-18): `00-overview.md`, `02-architecture.md`
- **crates.io 비충돌 고유명 `hyve`(구 `hyper`) 및 순수 `MIT` 라이선스 확정** (2026-11-19): 레포명·crates.io 패키지명·Rust 크레이트명·CLI명을 `hyve`로 1:1 일치
- **MIT 포크 스택(`rkyv`·`reactive_graph`+`slotmap`·`winnow`·`ariadne`·`ulid`·`bytemuck`·`rtrb`/`crossbeam-queue`), `reactive_graph` SoA 내부 개조 방향 및 Phase 1~3 우선순위 컷 확정** (2026-11-19):
  - Phase 1(`v0.1.0`): `hyve-core`·`hyve-state`·`hyve-serde`(`rkyv`+JSON)·`hyve-validate`·`hyve-config`·`hyve-cli`(`init`·`test`·`build` 기본)
  - Phase 2(`v0.2.0`): `hyve preview` HMR·`hyve lint`·`asset!`·`hyve-trace`·`SharedRingBuf` OS `shm` IPC
  - Phase 3(`v0.3.0+`): 바이너리 핫패칭·스토어 패키징·코드 사이닝·델타 업데이터 (`02-architecture.md` §3)
