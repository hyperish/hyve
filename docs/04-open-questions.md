# 04. 열린 결정 (hyve)

> 상태: "열려 있는 결정"은 **미정**(방향은 **제안**), "결정됨"은 **확정** 기록(제안으로 추가한 내용은 항목 안에 표시) · 기준일: 2026-11-24

`hyve` 레포 범위의 미정 사항 목록입니다. 범용 명세 전체의 결정 기록은 [`hyperish/docs/docs/07-open-questions.md`](https://github.com/hyperish/docs/blob/main/docs/07-open-questions.md)에서 추적합니다.

## 열려 있는 결정

| # | 결정 항목 | 왜 중요한가 | 지금의 방향(제안) |
|---|---|---|---|
| 1 | `hyve preview`의 바이너리 핫패칭 세부 메커니즘 (Phase 3) | Rust 컴파일 시간을 극복하고 즉각적인 피드백 루프를 만드는 고급 단계 | Phase 2 상태 보존 HMR 우선 구축 후, Phase 3에서 Subsecond 계열 바이너리 핫패칭 검토 |
| 2 | `#[server]` 호출 스텁의 인자·오류 전달 규칙 | `#[server_ctx]`가 아닌 인자가 여럿일 때 묶는 방식과, 서버의 구조화 오류(종류·코드·세부)가 스텁에서 복원되는 방식이 정해져야 클라이언트 분기 코드가 레일(고속·표준)에 따라 달라지지 않음 | 인자는 선언 순서대로 하나의 요청 모델로 묶고, 오류는 `HyveError`의 종류·코드·`requestId`를 두 레일에서 똑같이 복원하는 방향 |

## 결정됨

- **Rust/Cargo 대비 Bun 포지셔닝 확정** (2026-11-17): 올인원 CLI(`init`, `preview`, `lint`, `test`, `build`) + 기본 내장 범용 코어 모듈(`00-overview.md`)
- **`hyve-state` 반응성 코어의 `signal(0)` 표기 및 단일 `Copy` 아레나 핸들(`Signal<T>`, `!Send`만 `LocalSignal<T>` 옵트인) 확정** (2026-11-17 ~ 2026-11-19): `02-architecture.md` §2.2
- **웹 전용 OpenAPI 제외, UI 선언적 DSL(`ui!`) 제외 및 범용 스키마·검증(`hyve-validate`)·인터페이스(`hyve-api`) 경계 확정** (2026-11-18): `00-overview.md`, `02-architecture.md`
- **crates.io 비충돌 고유명 `hyve`(구 `hyper`) 및 순수 `MIT` 라이선스 확정** (2026-11-19): 레포명·crates.io 패키지명·Rust 크레이트명·CLI명을 `hyve`로 1:1 일치
- **MIT 포크 스택(`rkyv`·`reactive_graph`·`winnow`·`ariadne`·`ulid`·`bytemuck`·`rtrb`/`crossbeam-queue`), `reactive_graph` SoA 내부 개조 방향 및 Phase 1~3 우선순위 컷 확정** (2026-11-19):
  - Phase 1(`v0.1.0`): `hyve-core`·`hyve-state`·`hyve-serde`(`rkyv`+JSON)·`hyve-validate`·`hyve-macros`(`#[model]`·`#[validate]`·`#[hyve::test]`·`#[server]`)·`hyve-api`·`hyve-parser`·`hyve-diagnostics`·`hyve-config`·`hyve-cli`(`init`·`preview` 기본 실행·`test`·`build` 기본)
  - Phase 2(`v0.2.0`): `hyve preview` 상태 보존 HMR·`hyve lint`·`asset!`·`hyve-trace`·`SharedRingBuf` OS `shm` IPC
  - Phase 3(`v0.3.0+`): 바이너리 핫패칭·스토어 패키징·코드 사이닝·델타 업데이터 (`02-architecture.md` §3)
- **Phase 1 의존 폐포 보정** (2026-11-24): `orbic`·`serge`의 Phase 1이 필요로 하는 `hyve-macros`·`hyve-api`와 매크로의 빌드 의존성인 `hyve-parser`·`hyve-diagnostics`를 Phase 1에 포함(위 목록에 반영). `serge` Phase 1의 `serge-web-dev`가 붙을 수 있도록 `hyve preview`의 기본 실행(HMR 없는 실행·재시작)도 Phase 1에 두고, 상태 보존 HMR은 Phase 2 유지. `asset!`는 Phase 2 유지
- **`#[server]` 매크로 소유와 해석 경계 확정** (2026-11-24): `#[server]`/`#[server_ctx]`는 `hyve-macros`가 소유. 메서드·경로 인자는 `hyve`가 해석하지 않는 바인딩 메타데이터이며 `orbic`만 HTTP 라우트로 해석 (`02-architecture.md` §1, §2.4, `03-adapters-and-pipeline.md` §2)
- **포크 표 보정** (2026-11-24): `slotmap`(`Zlib` 단독)은 포크 대상에서 제외하고 아레나 설계 레퍼런스로만 사용(아레나는 자체 구현). `rtrb` 라이선스 기재, 표의 버전은 확인 시점 스냅샷으로 취급 (`02-architecture.md` §3)
- **수명·호환·의존 규칙 추가(제안)** (2026-11-24): `Ctx<T>`와 시그널의 스코프 수명, 고속 레일의 스키마 일치 규칙, `hyve-parser`·`hyve-diagnostics`의 의존 방향을 `02-architecture.md` §2.1~§2.3에 **제안**으로 추가
