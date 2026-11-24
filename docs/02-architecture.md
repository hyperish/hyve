# 02. 아키텍처와 크레이트 구조 (hyve)

> 상태: `apps/`·`crates/` 모노레포 및 코어 `ref` 역할은 **확정**, 세부 크레이트 분할은 **제안** · 기준일: 2026-11-24

`hyve` 레포는 Cargo 워크스페이스 기반의 모노레포(`apps/`, `crates/`)로 구성합니다(`hyperish/docs`의 `05-development-rules.md` §1).

## 1. 워크스페이스 디렉터리 구조 (제안)

```
hyve/
  apps/
    hyve-cli/          # `hyve` 바이너리 (대표 명령: init, preview, lint, test, build + 보조 codegen — 01-dx-and-dsl.md §2 참조)
  crates/
    hyve/              # 사용자 진입 크레이트 (단일 의존성 재수출 + prelude)
    hyve-core/         # 수명, 소유권, 오류, 취소 토큰, 기본 타입(Ulid, Timestamp), Lock-Free/Shm 공용 Ring Buffer(`SharedRingBuf`), Ctx<T>
    hyve-macros/       # 범용 코어 proc macro (`#[model]`, `#[validate]`, `#[hyve::test]`, `#[server]`/`#[server_ctx]`, `asset!`, proc-macro = true)
    hyve-state/        # 범용 반응형 상태(`signal(init)`, memo, effect, store) 및 스냅샷 모델
    hyve-serde/        # 고성능 내부 직렬화·역직렬화 코어 + 포맷 어댑터 트레이트
    hyve-validate/     # 범용 데이터 검증(`#[validate]`, `Valid<T>`) 및 타입/제약 스키마 메타데이터
    hyve-config/       # `.env`, `hyve.toml`, 환경 변수, CLI 인자 통합 로더 (시작 시점 `hyve-validate` 자동 검증)
    hyve-trace/        # 전 계층 통합 구조화 로깅·트레이싱·메트릭 (`TraceId`/`RequestId` 전파)
    hyve-parser/       # DSL·스키마·포맷용 고성능 파서 콤비네이터/토크나이저 코어
    hyve-api/          # 범용 규격 인터페이스(Contract), 인라인 서버 함수 계약, 추상 테스트 클라이언트 트레이트
    hyve-diagnostics/  # 컴파일 타임·런타임 구조화 진단 및 에러 리포팅
```

- 외부 개발자와 상위 레이어(`orbic`, `serge` 및 개별 제품)는 기본적으로 진입 크레이트(`hyve`) 또는 명시된 코어 크레이트(`hyve-core`, `hyve-state`, `hyve-serde`, `hyve-validate`, `hyve-config`, `hyve-trace`, `hyve-api`)를 참조(`ref`)합니다.
- 포크 소스는 그것을 쓰는 크레이트 안의 모듈로 편입하는 것을 기본으로 하고(`hyperish/docs` `05-development-rules.md` §7), 외부에는 `hyve-*`의 안정 인터페이스만 노출합니다.
- 타입·제약 스키마 메타데이터는 `hyve-validate`가 정의하고, `hyve-api`의 계약 디스크립터는 이를 참조만 합니다. 그래서 `hyve-api`는 `hyve-validate`에 의존합니다.
- 바이너리 패키지 이름은 `hyve-cli`이고 설치되는 실행 파일 이름이 `hyve`입니다. 라이브러리 진입 크레이트 `hyve`와 이름이 겹치지 않습니다.

## 2. 코어 모듈별 책임 (제안)

### 2.1 `hyve-core` (기반 계약 · 공용 Ring Buffer · 컴포넌트 컨텍스트)
- 식별자(ID), 시각, 상관관계 ID(`RequestId`/`TraceId`), 구조화된 오류 분류(`Domain` / `Infra` / `Internal`), 구조적 취소(`CancellationToken`), 컴포넌트 컨텍스트(`Ctx<T>`) 계약을 제공합니다. `Ctx<T>`는 주입된 값의 스코프(앱·컴포넌트·요청) 동안만 유효한 대여 핸들이며, 요청 스코프 값은 요청이 끝나면 해제됩니다.
- **오류 값 (확정, 2026-11-24)**: 구조화 오류(`HyveError`, `HyveResult<T>`는 이를 오류로 갖는 별칭)는 종류(`Domain`/`Infra`/`Internal`), 안정된 `code`(snake_case), 사람용 `message`, 구조화된 `details`, 재시도 가능 여부, 상관관계 ID(`requestId`)를 가집니다. 호출자는 `code`로 분기하고 `message`는 해석하지 않습니다. 외부 규약의 상태 코드 대응은 이 타입이 아니라 어댑터가 가집니다(`03-adapters-and-pipeline.md` §2). 이 필드 구성은 `#[server]` 호출 스텁 계약(§2.4)의 전제입니다.
- **식별자**: `Id<T>`는 대상 타입으로 구분되는 `Ulid` 래퍼이며 `Id::new()`로 새 값을 만듭니다. `Ulid`는 ULID 규격(48비트 밀리초 타임스탬프 + 80비트 난수)을 따르고, 난수부는 암호학적 난수원에서 채웁니다(ULID 규격, 2026-11-24 확인: https://github.com/ulid/spec). 그래도 ID는 식별용이며 세션·토큰 같은 자격 증명으로 쓰지 않습니다(`hyperish/docs` `02-core-contracts.md` §6). 규격상 같은 밀리초 안의 정렬 순서는 보장되지 않으므로, 한 프로세스 안에서는 규격의 단조 증가 방식을 적용합니다(제안).
- **스코프**: 스코프를 만들고, 값을 제공하고, 해제하는 API는 `hyve-core`의 공개 인터페이스이며 `orbic`(앱·컴포넌트·요청)과 `serge`(앱·기능·뷰)가 이를 호출해 `Ctx<T>`를 채웁니다. 조립 규칙 자체는 각 레포가 가집니다(제안).
- **취소**: `CancellationToken`은 부모의 취소가 자식에게 전파되는 트리이며, 폴링과 대기(`await`) 양쪽으로 관찰할 수 있습니다(제안).
- 인프로세스 스레드 간 및 로컬 멀티프로세스 공유 메모리(`shm`/`mmap`) 간에 대용량·120fps 실시간 스트림을 락·복사 없이 주고받는 **Lock-Free SPSC/MPMC 공용 링 버퍼(`SharedRingBuf<T>`)** 프리미티브를 제공합니다. OS 공유 메모리(`shm`) 모드는 Rust 메모리 안전성(`unsafe` 근거 주석 필수)을 위해 다음 계약을 강제합니다:
  - **레이아웃 제약**: `#[repr(C)]` 고정 크기 POD(`Pod + Zeroable`) 타입 또는 검증된 바이트 프레이밍 전용이며, 힙 포인터(`String`, `Vec` 등)나 레이아웃 미보장 타입의 직접 배치를 컴파일 타임에 금지합니다.
  - **찢어짐(Tearing) 방지**: Producer가 고속으로 덮어쓰는 환경에서 불안정한 원시 슬라이스 대여를 허용하지 않고, 스냅샷 복사(seqlock/더블·트리플 버퍼) 또는 명시적 읽기 가드(`ReadGuard`) 모델을 강제합니다.
  - **수명·권한·샌드박스**: 프로세스 비정상 종료 시 잔여 `shm` 객체 정리 규칙, 타 사용자 접근 차단(`0600` 권한 격리), macOS App Sandbox / iOS App Group 컨테이너 경계를 준수합니다.

### 2.2 `hyve-state` (상태성 / Reactivity — `signal()` & 단일 `Copy` 아레나 핸들 확정)
- React 훅 모방(`use_signal(|| 0)`) 대신 직관적인 **`signal(0)`** 생성자를 기본으로 제공하며, UI(`serge`)와 백엔드(`orbic`), 그래픽 엔진·일반 애플리케이션이 공통으로 사용할 수 있는 **플랫폼 중립 반응형 그래프**를 제공합니다.
- **단일 `Copy` 핸들(`Signal<T>`) + 내부 스레드 모드 분리 (확정, 2026-11-19)**:
  - 크로스플랫폼 컴포넌트 시그니처와 비동기 캡처 시 타입 마찰을 없애기 위해 기본 공개 타입은 단일 `Copy` 아레나 핸들 **`Signal<T>`**로 통일합니다(`!Send` 로컬 값만 `LocalSignal<T>` 옵트인).
  - 내부 구현은 타깃별로 분기해 단일 스레드 경로(WASM 웹 및 네이티브 메인 스레드)에서는 원자적 락 없는 인덱스 아레나로, 멀티스레드 공유 경로에서는 `Send + Sync` 스토리지로 동작합니다.
- 읽기 참조 대여와 명시적 변경 요청 규칙을 타입 수준에서 강제합니다.
- 시그널·메모·이펙트는 생성된 소유 스코프(컴포넌트·기능·요청)에 속하고, 스코프가 해제되면 아레나 슬롯을 반환합니다. 해제된 핸들 접근은 조용한 오동작이 아니라 감지되는 실패여야 합니다.
- `memo`는 읽기 전용 파생 값(`Memo<T>`)을 돌려주며 호출자가 직접 쓸 수 없습니다. 한 번의 변경에 대해 이펙트는 의존 경로가 여러 갈래여도 한 번만 실행됩니다. `update`·`with` 클로저 안에서 다른 시그널을 읽고 쓰는 것은 허용되며 패닉이 아닙니다(제안). 소유 스코프 밖에서 만든 시그널은 프로세스 수명의 루트 스코프에 속합니다(제안).

### 2.3 `hyve-serde` · `hyve-validate` · `hyve-parser` (직렬화·검증·파서·범용 스키마)
- **단일 내부 바이너리 와이어 포맷 (`rkyv` 확정, 2026-11-19)**: 내부 직렬화는 `rkyv`(MIT, `bytecheck` 검증 기반 제로카피 역직렬화)를 단일 기준으로 사용해 Tier 2 Shm IPC·MSA·SSR 상태·고속 Dual-Rail을 한 코덱으로 통합하고, JSON(`serde_json`) 등 외부 표준 포맷은 어댑터로 분리합니다.
  - **호환 규칙 (제안)**: `rkyv` 아카이브는 스키마 진화를 지원하지 않으므로(공식 기능 비교표, 2026-11-24 확인: https://rkyv.org/feature-comparison) 고속 레일은 양쪽의 `#[model]` 스키마가 같을 때만 쓰고, 다르면 표준 레일(JSON 어댑터)로 폴백합니다(`hyperish/docs` `02-core-contracts.md` §5). 신뢰 경계를 넘어온 바이트는 `bytecheck` 검증을 거친 뒤에만 접근합니다. 포맷 제어 피처(엔디언 `little_endian`/`big_endian`, 정렬 `aligned`/`unaligned`, 포인터 폭 `pointer_width_16`/`32`/`64`)는 패밀리 전체에서 `little_endian` + `aligned` + `pointer_width_32` 한 조합으로 고정하고, 기본값에 기대지 않고 명시적으로 켭니다. 수신 측은 본문을 정렬된 버퍼로 받아 복사 없이 접근합니다. `usize`/`isize`는 타깃의 포인터 폭이 아니라 이 피처가 정한 폭(기본 32비트)으로 직렬화되므로, WASM(32비트)과 네이티브(64비트)가 같은 조합을 써야 같은 아카이브를 읽습니다(`rkyv` 크레이트 문서 "Format control", 2026-11-24 확인: https://docs.rs/rkyv/latest/rkyv/ · 피처 이름은 `cargo info rkyv@0.8.18`로 확인). 스키마 지문은 `#[model]` 정의에서 컴파일 타임에 계산하며, 그 모델이 포함하는 모든 모델과 코덱 포맷 버전(위 포맷 제어 조합 포함)을 반영합니다. 지문을 싣는 자리와 불일치 시의 폴백은 `03-adapters-and-pipeline.md` §3이 정합니다.
  - **JSON 어댑터 (제안)**: `#[model]`의 JSON 표현은 값을 잃지 않습니다. 64비트 이상 정수는 JSON 수(배정밀도 부동소수)가 정확히 표현하는 범위 안이면 수로, 벗어나면 10진 문자열로 싣고, 디코더는 그런 필드에서 두 표현을 모두 받습니다. `NaN`과 무한대는 문자열 `"NaN"`, `"Infinity"`, `"-Infinity"`로 싣습니다. 그래서 같은 값이 두 레일에서 같은 결과를 내고, `version` 같은 흔한 정수 필드는 `hyperish/docs` `03-api-protocol.md`대로 JSON 수로 남습니다. 이 표현으로도 옮길 수 없는 타입(스트리밍 핸들 등)을 가진 모델은 JSON 어댑터 구현을 컴파일 타임에 거부합니다. `#[model]`의 JSON 필드명은 `camelCase`입니다(`hyperish/docs` `03-api-protocol.md` §1). 신뢰할 수 없는 입력을 받는 디코더는 중첩 깊이와 크기 한도를 가지며, 수치는 구현이 정하고 문서화합니다.
- `hyve-validate`는 웹/서버에 종속되지 않는 **범용 구조체·데이터 검증 및 스키마 리플렉션**(`len`, `range`, 패턴, 커스텀 불변조건 등)을 제공합니다. Phase 1의 `#[validate]` 문법은 `len = <범위>`와 `range = <범위>` 둘이며(`README.md` 예시), 패턴과 커스텀 불변조건의 문법은 Phase 2에서 정합니다. 그 밖의 키는 컴파일 타임 진단입니다(제안). 설정 파일, 에셋 데이터, UI 입력, 서버 요청 어디서든 동일하게 재사용됩니다.
- `hyve-parser`는 매크로 구문 분석(`hyve-macros`, `serge-macros`, `orbic-macros`)과 런타임 스키마·에셋·데이터 포맷 파싱을 모두 뒷받침하는 저할당 파서 콤비네이터/토크나이저 기반을 제공합니다. `hyve-parser`와 `hyve-diagnostics`는 매크로 크레이트의 빌드 의존성이므로 `hyve-macros`나 진입 크레이트(`hyve`)에 의존하지 않습니다.

### 2.4 `hyve-api` (범용 규격 인터페이스 통합)
- 모듈·프로세스·네트워크 경계를 넘는 서비스·리소스·이벤트 계약(Contract)과 `#[server]` 인라인 서버 함수 계약을 정의합니다. `hyve` 자체는 HTTP나 OpenAPI 같은 특정 프로토콜 규격을 포함하지 않으며, 백엔드(`orbic`)가 이 범용 스키마·인터페이스를 읽어들여 컴포넌트 라우터 바인딩이나 OpenAPI 문서로 변환합니다. `#[server(POST "/projects")]`의 메서드·경로 인자도 `hyve`는 해석하지 않고 계약 디스크립터에 바인딩 메타데이터로 그대로 싣습니다. 이를 HTTP 라우트로 해석하는 주체는 `orbic`뿐입니다.
- 경로·쿼리 값을 모델로 받는 `Path<T>`와 `Query<T>`, 목록 응답과 커서 요청의 `Page<T>`와 `CursorParams`는 `hyve-api`가 정의하고, `orbic` 핸들러와 `serge` 라우트 컴포넌트가 함께 씁니다. 경로 자리표시자는 `{이름}` 형태입니다(제안).
- **문법 (제안, 2026-11-24)**: `#[server(<메서드> "<경로>")]` 한 가지 형태이며 인자는 필수입니다. 메서드는 `hyperish/docs` `03-api-protocol.md` §2의 `GET`, `POST`, `PATCH`, `DELETE` 중 하나이고 경로는 `/`로 시작합니다. `#[server_ctx]`는 `Ctx<T>` 타입의 함수 인자에만 붙입니다. 그 밖의 형태는 컴파일 타임 진단입니다. 본문 제거는 함수가 선언된 크레이트의 `server` 피처로 갈립니다(`orbic/docs/01-dx-and-dsl.md` §2).
- **호출 스텁 계약 (확정, 2026-11-24)**: 함수 이름으로 부르는 것은 어느 빌드에서나 스텁입니다. 스텁은 원본 함수의 전송 인자를 같은 순서로 받고, 고속·표준 두 레일에서 같은 요청 모델과 같은 오류를 주고받습니다. `server` 피처가 켜진 빌드에서는 핸들러 본문이 계약 디스크립터에 함께 묶이며, 본문에 닿는 길은 `orbic`의 `.handle(...)` 등록을 거친 요청 파이프라인뿐입니다. 그래서 서버와 UI가 한 프로세스에 있어도(포트 없는 테스트, 단일 바이너리) 호출은 가드를 건너뛰지 않습니다.
  - **인자**: `#[server_ctx]`가 아닌 인자만 전송합니다. 0개면 빈 요청, 1개면 그 값 자체가 요청 모델(래퍼 없음), 2개 이상이면 선언 순서대로 인자 이름을 필드명으로 하는 하나의 요청 모델로 묶습니다. `Valid<T>` 인자는 스텁에서 `T`로 받고, 검증은 서버가 신뢰 경계에서 항상 다시 수행합니다. 전송 인자는 `hyve-serde`로 직렬화되는 타입이어야 하며, 아니면 컴파일 타임 진단을 냅니다.
  - **오류**: 스텁은 `HyveResult<T>`를 그대로 돌려주며, 서버 오류의 종류(`Domain`/`Infra`/`Internal`)·`code`·`details`·재시도 가능 여부·`requestId`를 어느 레일에서든 똑같이 복원합니다. 고속 레일은 이 필드를 결과에 그대로 싣고, 표준 레일은 어댑터의 오류 변환이 같은 정보를 왕복 가능하게 보존합니다(`orbic/docs/03-adapters-and-pipeline.md` §1).
  - **신뢰 경계**: 고속 레일도 클라이언트와의 경계입니다. `Internal` 오류의 원문과 내부 타입명은 어느 레일로도 내보내지 않습니다(`hyperish/docs` `02-core-contracts.md` §2).
  - **전송 실패와 재시도 (제안, 2026-11-24)**: 전송 자체의 실패는 스텁이 `Infra` 오류로 돌려주며 `code`는 `transport_unavailable`(연결 불가), `timeout`, `decode_failed` 중 하나입니다. 스텁은 `Valid<T>` 인자를 보내기 전에 같은 규칙으로 먼저 검증해, 실패하면 네트워크 호출 없이 `invalid_request`를 돌려줍니다(서버 재검증은 그대로입니다). 스키마 불일치 폴백(`03-adapters-and-pipeline.md` §3.3, 전송이 수행)을 빼면 스텁과 전송은 스스로 재시도하지 않고 멱등 키도 붙이지 않습니다. 재시도는 호출자가 정합니다.
  - **전송과 와이어 형식 (제안, 2026-11-24)**: 스텁이 전송을 찾는 방법, 고속 레일의 와이어 형식, 표준 레일 매핑은 `03-adapters-and-pipeline.md` §3이 정합니다.

### 2.5 비동기 런타임과 외부 의존성 (확정, 2026-11-24)

- §1의 코어 크레이트(`hyve-core`, `hyve-state`, `hyve-serde`, `hyve-validate`, `hyve-config`, `hyve-trace`, `hyve-api`)는 비동기 런타임에 의존하지 않습니다. 스폰, 타이머, 비동기 I/O는 코어에 두지 않고 `orbic`, `serge`, 테스트 런타임이 가집니다.
- 패밀리의 네이티브 비동기 런타임은 `tokio`입니다(외부 의존성, 포크하지 않음). `orbic`의 포크 대상 중 `tokio-tungstenite`가 `tokio` 전용이고 `quinn`의 기본 런타임도 `tokio`라서(2026-11-24 `cargo info` 확인), 다른 런타임을 고르면 경계마다 변환 계층이 생기기 때문입니다. 웹(WASM)에서는 브라우저 이벤트 루프를 씁니다.
- `#[hyve::test]`는 네이티브에서 `tokio` 현재 스레드 런타임으로 비동기 테스트를 실행합니다. 이 런타임은 진입 크레이트 `hyve`의 `test` 피처에만 들어 있고 기본으로는 꺼져 있어, 일반 빌드와 WASM 빌드는 `tokio`를 링크하지 않습니다. `wasm32` 타깃에서 `#[hyve::test]`를 쓰면 Phase 1에서는 컴파일 타임 진단입니다(제안).
- proc macro 크레이트는 Rust 항목(구조체, 함수 시그니처)의 해석에 `syn`·`quote`·`proc-macro2`를 외부 의존성으로 쓰고, DSL 고유 문법과 진단은 `hyve-parser`·`hyve-diagnostics`가 맡습니다(제안).

## 3. 확정 MIT 포크 스택 및 내부 코어 `unit optimization` 타깃 (2026-11-19 확정)

전 크레이트는 **순수 `MIT` 라이선스**를 유지하기 위해 `MIT` 단독 또는 `MIT OR Apache-2.0`(MIT 선택) 크레이트만 포크합니다(`hyperish/docs` `05-development-rules.md` §7).

표의 버전·라이선스는 crates.io 레지스트리(`cargo info <크레이트>@<버전>`)에서 2026-11-24에 확인한 스냅샷입니다. 확정된 것은 베이스 크레이트의 선택이며, 포크 기준 리비전은 착수 시점에 다시 확인해 고정합니다.

| 모듈 | 베이스 크레이트 (확인 버전 · 라이선스) | 포크 후 내부 코어 개조(1·2단계 `unit optimization`) 방향 |
|---|---|---|
| `hyve-serde` | `rkyv` (`0.8.18`, `MIT`) · 외부 어댑터 `serde_json` (`1.0.154`, `MIT/Apache-2.0`) | `#[model]` 직결 및 `bytecheck` 검증 제로카피 경로 고정 (0할당 포인터 캐스팅 유지) |
| `hyve-state` | `reactive_graph` (`0.3.0-rc1`, `MIT`) · 아레나 설계 레퍼런스 `slotmap` (`1.1.1`, `Zlib` 단독 — 포크·편입 없이 레퍼런스 전용) | 범용 멀티-런타임·`AnySubscriber` 동적 디스패치 전면 제거 → 자체 구현 단일 스레드 무잠금 인덱스 아레나 + **SoA(값 배열/구독자 비트셋 분리)** 개조 |
| `hyve-parser` | `winnow` (`1.0.5`, `MIT`) | 제로카피 슬라이스 파싱을 `hyve` 진단 스팬과 직결 |
| `hyve-diagnostics` | `ariadne` (`0.6.0`, `MIT`) | 컴파일 타임/런타임 스팬·수정 가이드 출력 경량화 |
| `hyve-core` | `ulid` (`3.0.0`, `MIT`), `bytemuck` (`1.25.2`, `MIT/Apache-2.0/Zlib`), `rtrb` (`0.4.1`, `MIT OR Apache-2.0`) / `crossbeam-queue` (`0.3.14`, `MIT/Apache-2.0`) | `#[repr(C)]` POD 검증 + `SharedRingBuf<T>` 인프로세스/OS `shm` 안전 스냅샷·`ReadGuard` 직결 |

외부 의존성(포크하지 않음, 2026-11-24 `cargo info` 확인): `tokio` (`1.54.1`, `MIT`), `syn` (`3.0.10`, `MIT OR Apache-2.0`), `quote` (`1.0.47`, `MIT OR Apache-2.0`), `proc-macro2` (`1.0.107`, `MIT OR Apache-2.0`), `bytecheck` (`0.8.3`, `MIT`), 컴파일 실패 테스트용 `trybuild` (`1.0.125`, `MIT OR Apache-2.0`). 이 목록에 없는 의존성은 `hyperish/docs` `05-development-rules.md` §7의 외부 의존성 규칙에 따라 추가 기록합니다.
