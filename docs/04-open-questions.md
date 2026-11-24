# 04. 열린 결정 (hyve)

> 상태: "열려 있는 결정"은 **미정**(방향은 **제안**), "결정됨"은 **확정** 기록(제안으로 추가한 내용은 항목 안에 표시) · 기준일: 2026-11-24

`hyve` 레포 범위의 미정 사항 목록입니다. 범용 명세 전체의 결정 기록은 [`hyperish/docs/docs/07-open-questions.md`](https://github.com/hyperish/docs/blob/main/docs/07-open-questions.md)에서 추적합니다.

## 열려 있는 결정

| # | 결정 항목 | 왜 중요한가 | 지금의 방향(제안) |
|---|---|---|---|
| 1 | `hyve preview`의 바이너리 핫패칭 세부 메커니즘 (Phase 3) | Rust 컴파일 시간을 극복하고 즉각적인 피드백 루프를 만드는 고급 단계 | Phase 2 상태 보존 HMR 우선 구축 후, Phase 3에서 Subsecond 계열 바이너리 핫패칭 검토. **결정 시점: Phase 3 착수 전** (Phase 1·2를 막지 않음) |

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
- **`#[server]` 호출 스텁의 인자·오류 전달 규칙 확정** (2026-11-24): 전송 인자는 0개면 빈 요청, 1개면 그 값 자체, 2개 이상이면 선언 순서대로 하나의 요청 모델로 묶음. `Valid<T>`는 스텁에서 `T`로 받고 서버가 항상 재검증. 오류는 종류·`code`·`details`·재시도 가능 여부·`requestId`를 두 레일에서 똑같이 복원하며, 고속 레일에도 내부 세부 비노출을 적용 (`02-architecture.md` §2.4). 범위 밖으로 남긴 것: 본문 없는 메서드 바인딩의 표준 레일 매핑, 전송 실패 코드 이름, 클라이언트 사전 검증, 자동 멱등 키 — 구현 단계에서 **제안**으로 다룸
- **오류 값의 필드 구성 확정** (2026-11-24): 종류·`code`·`message`·`details`·재시도 가능 여부·`requestId`. 상태 코드 대응은 어댑터가 가짐 (`02-architecture.md` §2.1)
- **비동기 런타임과 외부 의존성 확정** (2026-11-24): 코어 크레이트는 런타임 비의존, 네이티브 런타임은 `tokio`(외부 의존성), `#[hyve::test]`는 `tokio` 현재 스레드 런타임으로 실행. proc macro의 `syn`·`quote`·`proc-macro2` 사용은 **제안** (`02-architecture.md` §2.5, §3)
- **구현 전 세부 규칙 추가(제안)** (2026-11-24): 설정 우선순위와 환경 변수 이름, CLI 공통 계약, `Ulid` 난수부와 단조 증가, 취소 관찰 방식, `Memo<T>` 읽기 전용과 이펙트 1회 실행, JSON 어댑터의 입력 한도, 스키마 지문 전달, 스텁의 전송 실패 코드·사전 검증·재시도 없음 (`01-dx-and-dsl.md` §1, §2, `02-architecture.md` §2.1~§2.4)
- **Phase 1 구현 계획 문서 추가** (2026-11-24): 작업 순서와 완료 기준을 `05-implementation-plan.md`에 둠
- **`#[server]` 스텁을 모든 빌드에 둠** (2026-11-24): 함수 이름으로 부르는 것은 서버 빌드에서도 스텁이며, 핸들러 본문에는 `orbic`의 등록을 거친 요청 파이프라인으로만 닿음. 포트 없는 풀스택 테스트가 성립하도록 2026-11-24의 스텁 계약을 넓힘 (`02-architecture.md` §2.4)
- **인라인 서버 함수 전송 계약 추가(제안)** (2026-11-24): 매크로가 남기는 것, 스텁의 전송 조회, 고속 레일의 HTTP 운반 형식과 불일치 폴백, 표준 레일의 오류 복원, 자격 증명의 책임을 한 절에 모음. 미디어 타입과 헤더 이름은 `hyve-api` 상수가 기준 (`03-adapters-and-pipeline.md` §3)
- **경계 세부 추가(제안)** (2026-11-24): `rkyv` 포맷 제어 조합(`little_endian`·`aligned`·`pointer_width_32`), JSON의 손실 없는 표현(JSON 수로 정확히 표현할 수 없는 정수와 비유한 부동소수는 문자열), `#[server]`·`#[validate]`의 Phase 1 문법, 스키마 메타데이터의 소유(`hyve-validate`), `Id<T>`·`HyveError`·`Path<T>`·`Query<T>`의 소유, `#[hyve::test]` 런타임의 위치(`hyve`의 `test` 피처), `hyve.toml` 구조와 환경 변수 접두어, CLI의 템플릿·옵션 이름과 프레임워크 단계 호출 방식 (`01-dx-and-dsl.md` §1, §2, `02-architecture.md` §1, §2)
- **전송 트레이트의 수준과 폴백 조건 정리(제안)** (2026-11-24): 전송 트레이트는 타입이 있는 호출 수준이고 레일 선택·HTTP 변환·폴백은 전송 구현의 일. 고속 레일 요청에 `415`가 오면 `code`와 무관하게 표준 레일로 한 번 폴백. 인프로세스 전송은 기본적으로 직렬화하지 않음. `Page<T>`·`CursorParams`를 `hyve-api` 소유로 옮김. `hyve`가 프레임워크 실행 파일에 제공 단계를 묻는 절차 추가 (`01-dx-and-dsl.md` §2, `02-architecture.md` §2.4, `03-adapters-and-pipeline.md` §3)
- **포크 착수 방식 적용** (2026-11-24): `rkyv`·`winnow`·`ariadne`·`bytemuck`은 정확한 버전 고정 의존, `hyve-state`와 `Ulid`·`SharedRingBuf`는 레퍼런스를 둔 자체 구현, JSON은 `serde_json` 대신 `#[model]`이 생성하는 자체 코덱. 확정된 포크 스택의 실행 방식을 바꾼 것 (`02-architecture.md` §3)
- **`#[model]` 입력 범위와 시그널 표면 확정(제안)** (2026-11-24, 미정 #2 해소): `#[model]`은 이름 있는 필드의 구조체(제네릭 포함)와 단위 배리언트만 가진 enum을 받고, 그 밖은 컴파일 타임 진단. enum의 JSON 값은 배리언트 이름의 `snake_case` 문자열. 소유하지 않은 속성(`#[table]`, `#[pk]` 등)은 그대로 통과. `Signal<T>`·`Memo<T>`는 `T: Clone`일 때 `sig()`로 읽을 수 있고(호출식이 `Deref` 대상 `dyn Fn() -> T`로 자동 역참조되는 것을 이용), `Signal<T>`는 `+=` 같은 복합 대입과 `Signal<bool>::toggle()`을 제공. `SharedRingBuf<T>`는 `T: bytemuck::Pod`인 덮어쓰기 링이며 `read_latest_snapshot(&mut [T]) -> usize`로 최신 샘플을 찢어짐 없이 복사 (`02-architecture.md` §2.1, §2.2)
- **구현 중 보정(제안)** (2026-11-24, Phase 1 1~7단계): `rkyv` 고수준 API가 깊이 한도 없이 검증해 작은 악성 아카이브로 스택이 넘칠 수 있어 깊이 128 한도를 둔 검증기 사용. `usize`/`isize`는 와이어 타입에서 제외(바이너리 32비트와 JSON·스키마 64비트가 어긋남). 스키마 없는 `Value`는 JSON에서도 정수 정확·부동소수 표기를 유지해 두 레일의 오류 `details`가 같음. 제네릭 모델의 스키마 이름은 타입 인자를 붙임(`Page_Project`). 직접 재귀 모델 지원, 상호 재귀는 미지원. `hyve`를 재수출하는 프레임워크를 위한 매크로의 `crate = <경로>` 인자. 7단계를 6단계보다 먼저 진행 (`02-architecture.md` §2.3·§2.4, `05-implementation-plan.md` §2)
- **설정 로더 세부(제안)** (2026-11-24, Phase 1 8단계): 원천은 최상위 필드 단위로 덮음, `hyve.toml`은 가장 가까운 상위 디렉터리에서 찾음, 환경 변수·`--set`의 비문자열 값은 TOML 값 문법, 접두어만 맞는 환경 변수는 무시, 프레임워크 테이블의 환경 변수 접두어는 `HYPERISH_<테이블>`, `hyve.toml`은 TOML 1.0 부분집합. 모델 기본값은 제네릭이 아닌 `#[model]`이 `Default` 구현 여부를 알려 주는 방식으로 읽음 (`01-dx-and-dsl.md` §1). 문서 예시 테스트와 코어 경계 검사는 픽스처 크레이트 `hyve-tests`에 둠 (`05-implementation-plan.md` §2)
