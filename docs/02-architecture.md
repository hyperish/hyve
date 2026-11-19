# 02. 아키텍처와 크레이트 구조 (hyve)

> 상태: `apps/`·`crates/` 모노레포 및 코어 `ref` 역할은 **확정**, 세부 크레이트 분할은 **제안** · 기준일: 2026-11-19

`hyve` 레포는 Cargo 워크스페이스 기반의 모노레포(`apps/`, `crates/`)로 구성합니다(`hyperish/docs`의 `05-development-rules.md` §1).

## 1. 워크스페이스 디렉터리 구조 (제안)

```
hyve/
  apps/
    hyve-cli/          # `hyve` 바이너리 (대표 명령: init, preview, lint, test, build + 보조 codegen — 01-dx-and-dsl.md §2 참조)
  crates/
    hyve/              # 사용자 진입 크레이트 (단일 의존성 재수출 + prelude)
    hyve-core/         # 수명, 소유권, 오류, 취소 토큰, 기본 타입(Ulid, Timestamp), Lock-Free/Shm 공용 Ring Buffer(`SharedRingBuf`), Ctx<T>
    hyve-macros/       # 범용 코어 proc macro (`#[model]`, `#[validate]`, `#[hyve::test]`, `asset!`, proc-macro = true)
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
- 내부 포크 크레이트는 `crates/` 하위(또는 워크스페이스 내 포크 전용 경로)에서 관리하며, 외부에는 `hyve-*`의 안정 인터페이스만 노출합니다.

## 2. 코어 모듈별 책임 (제안)

### 2.1 `hyve-core` (기반 계약 · 공용 Ring Buffer · 컴포넌트 컨텍스트)
- 식별자(ID), 시각, 상관관계 ID(`RequestId`/`TraceId`), 구조화된 오류 분류(`Domain` / `Infra` / `Internal`), 구조적 취소(`CancellationToken`), 컴포넌트 컨텍스트(`Ctx<T>`) 계약을 제공합니다.
- 인프로세스 스레드 간 및 로컬 멀티프로세스 공유 메모리(`shm`/`mmap`) 간에 대용량·120fps 실시간 스트림을 락·복사 없이 주고받는 **Lock-Free SPSC/MPMC 공용 링 버퍼(`SharedRingBuf<T>`)** 프리미티브를 제공합니다. OS 공유 메모리(`shm`) 모드는 Rust 메모리 안전성(`unsafe` 근거 주석 필수)을 위해 다음 계약을 강제합니다:
  - **레이아웃 제약**: `#[repr(C)]` 고정 크기 POD(`Pod + Zeroable`) 타입 또는 검증된 바이트 프레이밍 전용이며, 힙 포인터(`String`, `Vec` 등)나 레이아웃 미보장 타입의 직접 배치를 컴파일 타임에 금지합니다.
  - **찢어짐(Tearing) 방지**: Producer가 고속으로 덮어쓰는 환경에서 불안정한 원시 슬라이스 대여를 허용하지 않고, 스냅샷 복사(seqlock/더블·트리플 버퍼) 또는 명시적 읽기 가드(`ReadGuard`) 모델을 강제합니다.
  - **수명·권한·샌드박스**: 프로세스 비정상 종료 시 잔여 `shm` 객체 정리 규칙, 타 사용자 접근 차단(`0600` 권한 격리), macOS App Sandbox / iOS App Group 컨테이너 경계를 준수합니다.

### 2.2 `hyve-state` (상태성 / Reactivity — `signal()` & 이원화 스레드 모델 확정)
- React 훅 모방(`use_signal(|| 0)`) 대신 직관적인 **`signal(0)`** 생성자를 기본으로 제공하며, UI(`serge`)와 백엔드(`orbic`), 그래픽 엔진·일반 애플리케이션이 공통으로 사용할 수 있는 **플랫폼 중립 반응형 그래프**를 제공합니다.
- **이원화(Dual-mode) 스레드 모델**:
  - 단일 스레드 경로(WASM 웹 타깃 및 네이티브 메인 스레드 로컬 상태)에서는 원자적 락 오버헤드가 없는 고속 로컬 시그널(`LocalSignal`)을 기본으로 씁니다.
  - 멀티스레드·백그라운드 공유 경로(서버 상태, 멀티스레드 워커 공유 상태)에서는 `Send + Sync` 원자적 시그널(`SyncSignal`)을 제공하되, 두 모드가 동일한 반응형 읽기/바인딩 문법을 공유합니다(크로스플랫폼 컴포넌트 시그니처와 비동기 캡처 시 타입 마찰을 없애기 위한 단일 `Copy` 핸들 노출 여부는 [`04-open-questions.md`](04-open-questions.md)에서 확정).
- 읽기 참조 대여와 명시적 변경 요청 규칙을 타입 수준에서 강제합니다.

### 2.3 `hyve-serde` · `hyve-validate` · `hyve-parser` (직렬화·검증·파서·범용 스키마)
- 내부 규약 모델은 표준 호환보다 **성능과 수직 통합 생산성이 극대화되는 바이너리/제로카피 지향 포맷**을 기본으로 다루고, JSON 등 외부 표준 포맷은 어댑터로 분리합니다.
- `hyve-validate`는 웹/서버에 종속되지 않는 **범용 구조체·데이터 검증 및 스키마 리플렉션**(`len`, `range`, 패턴, 커스텀 불변조건 등)을 제공합니다. 설정 파일, 에셋 데이터, UI 입력, 서버 요청 어디서든 동일하게 재사용됩니다.
- `hyve-parser`는 매크로 구문 분석(`hyve-macros`, `serge-macros`, `orbic-macros`)과 런타임 스키마·에셋·데이터 포맷 파싱을 모두 뒷받침하는 저할당 파서 콤비네이터/토크나이저 기반을 제공합니다.

### 2.4 `hyve-api` (범용 규격 인터페이스 통합)
- 모듈·프로세스·네트워크 경계를 넘는 서비스·리소스·이벤트 계약(Contract)과 추상 클라이언트/호출 인터페이스를 정의합니다. `hyve` 자체는 HTTP나 OpenAPI 같은 특정 프로토콜 규격을 포함하지 않으며, 백엔드(`orbic`)가 이 범용 스키마·인터페이스를 읽어들여 라우터 바인딩이나 OpenAPI 문서로 변환합니다.

- 내부 포크 선정 및 단계적 `unit optimization` 절차는 `hyperish/docs`의 `05-development-rules.md` §7을 따릅니다.
