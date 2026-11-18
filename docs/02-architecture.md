# 02. 아키텍처와 크레이트 구조 (hyper)

> 상태: `apps/`·`crates/` 모노레포 및 코어 `ref` 역할은 **확정**, 세부 크레이트 분할은 **제안** · 기준일: 2026-11-17

`hyper` 레포는 Cargo 워크스페이스 기반의 모노레포(`apps/`, `crates/`)로 구성합니다(`hyperish/docs`의 `05-development-rules.md` §1).

## 1. 워크스페이스 디렉터리 구조 (제안)

```
hyper/
  apps/
    hyper-cli/          # `hyper` 바이너리 (대표 명령: init, preview, lint, test, build + 보조 codegen — 01-dx-and-dsl.md §2 참조)
  crates/
    hyper/              # 사용자 진입 크레이트 (단일 의존성 재수출 + prelude)
    hyper-core/         # 수명, 소유권 핸들, 구조화 오류, 취소 토큰, 기본 타입(ULID 등)
    hyper-macros/       # proc macro 기반 선언적 DSL (proc-macro = true)
    hyper-state/        # 범용 반응형 상태(signal, memo, effect, store) 및 스냅샷 모델
    hyper-serde/        # 고성능 내부 직렬화·역직렬화 코어 + 포맷 어댑터 트레이트
    hyper-parser/       # DSL·스키마·포맷용 고성능 파서 콤비네이터/토크나이저 코어
    hyper-api/          # 제품·툴 통합 API 규격 인터페이스 및 계약(Contract) 정의
    hyper-diagnostics/  # 컴파일 타임·런타임 구조화 진단 및 에러 리포팅
```

- 외부 개발자와 상위 레이어(`ring`, `lace`, `hera`)는 기본적으로 진입 크레이트(`hyper`) 또는 명시된 코어 크레이트(`hyper-core`, `hyper-state`, `hyper-serde`, `hyper-api`)를 참조(`ref`)합니다.
- 내부 포크 크레이트는 `crates/` 하위(또는 워크스페이스 내 포크 전용 경로)에서 관리하며, 외부에는 `hyper-*`의 안정 인터페이스만 노출합니다.

## 2. 코어 모듈별 책임 (제안)

### 2.1 `hyper-core` (기반 계약)
- 식별자(ID), 시각, 상관관계 ID(`RequestId`/`TraceId`), 구조화된 오류 분류(`Domain` / `Infra` / `Internal`), 구조적 취소(`CancellationToken`)와 커밋 지점 추상화를 제공합니다(`hyperish/docs` `02-core-contracts.md` §1~§4 구현체).

### 2.2 `hyper-state` (상태성 / Reactivity — 이원화 스레드 모델 확정)
- `lace`(웹 반응형 파이프라인)와 `hera`(GPU 네이티브 UI 상태), 나아가 `ring` 및 제품군의 도메인 상태가 공통으로 사용할 수 있는 **플랫폼 중립 반응형 그래프**를 제공합니다.
- **이원화(Dual-mode) 스레드 모델**:
  - 단일 스레드 UI 경로(`lace` WASM, `hera` 메인 UI 스레드 로컬 상태)에서는 원자적 락 오버헤드가 없는 고속 로컬 시그널(`LocalSignal`)을 기본으로 씁니다.
  - 멀티스레드·백그라운드 공유 경로(`ring` 서버 상태, `hera` 워커 ↔ UI 공유 상태)에서는 `Send + Sync` 원자적 시그널(`SyncSignal`)을 제공하되, 두 모드가 동일한 반응형 읽기/바인딩 문법을 공유합니다.
- 읽기 참조 대여와 명시적 변경 요청 규칙을 타입 수준에서 강제합니다.

### 2.3 `hyper-serde` & `hyper-parser` (직렬화·파서·공통 UI AST)
- 내부 규약 모델은 표준 호환보다 **성능과 수직 통합 생산성이 극대화되는 바이너리/제로카피 지향 포맷**을 기본으로 다루고, JSON 등 외부 표준 포맷은 어댑터로 분리합니다.
- `hyper-parser`는 **QML/Slint + Dioxus `rsx!` 스타일의 공통 UI 문법 AST**와 런타임 스키마 파싱을 모두 담당하여, `lace`(웹)와 `hera`(네이티브)가 동일한 UI 문법 파서를 공유하게 합니다.

### 2.4 `hyper-api` (API 규격 인터페이스 통합)
- 하나의 서비스·리소스·이벤트 계약 선언으로부터 백엔드 라우터 스켈레톤(`ring`), 웹/네이티브 클라이언트 바인딩(`lace`, `hera`), 그리고 HTTP/JSON·WebSocket 어댑터 검증 코드를 동시에 도출하는 중심 인터페이스입니다.

## 3. 내부 포크 + Unit Optimization 운영 (확정)

- 직렬화, 파서, 반응성, 매크로 구문 분석 등에서 검증된 고성능 오픈소스 크레이트를 포크해 다음과 같이 단계적 `unit optimization`을 적용합니다(`hyperish/docs` `05-development-rules.md` §7):
  1. **1단계 (기본 적용)**: 범용 추상화·미사용 피처/의존성을 제거하고 `hyper` 코어 타입과 무변환·제로카피로 직결(수직화).
  2. **2단계 (심화 개조 여지)**: 프로파일링·벤치마크에서 병목이 확인된 핫패스에 한해 메모리 레이아웃(Arena/SoA) 개조 및 SIMD 등 저수준 최적화를 적용.
