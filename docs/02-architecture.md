# 02. 아키텍처와 크레이트 구조 (hyper)

> 상태: `apps/`·`crates/` 모노레포 및 코어 `ref` 역할은 **확정**, 세부 크레이트 분할은 **제안** · 기준일: 2026-11-17

`hyper` 레포는 Cargo 워크스페이스 기반의 모노레포(`apps/`, `crates/`)로 구성합니다(`hyperish/docs`의 `05-development-rules.md` §1).

## 1. 워크스페이스 디렉터리 구조 (제안)

```
hyper/
  apps/
    hyper-cli/          # `hyper` 바이너리 (init, preview, lint, test, codegen)
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

### 2.2 `hyper-state` (상태성 / Reactivity)
- `lace`(웹 반응형 파이프라인)와 `hera`(GPU 네이티브 UI 상태), 나아가 에디터/제품군의 장면·도메인 상태가 공통으로 사용할 수 있는 **플랫폼 중립 반응형 그래프**를 제공합니다.
- 읽기 참조 대여와 명시적 변경 요청 규칙을 타입 수준에서 강제합니다.

### 2.3 `hyper-serde` & `hyper-parser` (직렬화·파서)
- 내부 규약 모델은 표준 호환보다 **성능과 수직 통합 생산성이 극대화되는 바이너리/제로카피 지향 포맷**을 기본으로 다루고, JSON 등 외부 표준 포맷은 어댑터로 분리합니다.
- `hyper-parser`는 proc macro DSL 파싱과 런타임 스키마/표현식 파싱 양쪽에서 재사용할 수 있는 저할당 파서 기반을 제공합니다.

### 2.4 `hyper-api` (API 규격 인터페이스 통합)
- 하나의 서비스·리소스·이벤트 계약 선언으로부터 백엔드 라우터 스켈레톤(`ring`), 웹/네이티브 클라이언트 바인딩(`lace`, `hera`), 그리고 HTTP/JSON·WebSocket 어댑터 검증 코드를 동시에 도출하는 중심 인터페이스입니다.

## 3. 내부 포크 + Unit Optimization 운영

- 직렬화, 파서, 반응성, 매크로 구문 분석 등에서 검증된 고성능 오픈소스 크레이트를 포크해 내부 코어에 맞게 불필요한 범용 분기를 걷어내고(`unit optimization`) 수직 통합합니다.
- 포크 선정·업스트림 추적·패치 분리·벤치마크 검증은 `hyperish/docs`의 `05-development-rules.md` §7을 따릅니다.
