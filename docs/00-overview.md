# 00. 개요와 범위 (hyve)

> 상태: 역할·범위·레퍼런스는 **확정**, 세부 모듈 경계 및 구현 Phase 컷은 **제안** · 기준일: 2026-11-24

`hyve`는 하이퍼리쉬 프레임워크 패밀리(`hyve`, `orbic`, `serge`)의 중심축이자, 모든 레이어가 수직 통합을 위해 기준(`ref`)으로 삼는 **올인원(All-in-One) 코어 프레임워크 및 개발 툴체인** 레포입니다. 공통 추상 계약은 [`hyperish/docs`](https://github.com/hyperish/docs)를 따릅니다.

## 1. 핵심 포지셔닝: "Rust/Cargo에 대한 Bun" (확정)

JS 생태계에서 Node.js + npm + webpack + jest + 수많은 서드파티 패키지의 파편화를 **Bun**이 단일 고속 런타임·툴체인·내장 표준 API로 통합했듯, **`hyve`는 기존 Rust/Cargo 생태계의 파편화를 수직 통합으로 해결**합니다.

1. **올인원 개발 툴체인 (Toolchain & CLI — `bun` CLI 포지션)**
   - 프로젝트 스캐폴딩(`hyve init`), 실시간 핫리로드·미리보기(`hyve preview`), 정적 분석·포맷·진단(`hyve lint`), 고속 단위·통합·매크로 진단 테스트(`hyve test`), 타깃별 프로덕션 빌드·패키징(`hyve build`, 내부에 `codegen` 단계 포함)까지 개발 전 주기를 외부 플러그인 조합 없이 단일 바이너리로 관리합니다(대표 명령 목록과 `build`/`codegen` 관계는 [`01-dx-and-dsl.md`](01-dx-and-dsl.md) §2 참조).
2. **기본 내장 범용 코어 모듈 (`core` ref — `Bun.*` 내장 API 포지션)**
   - 웹·서버 개발에만 쓰이는 도구가 아니라, 그래픽 엔진·네이티브 앱·CLI·시스템 연산 등 **모든 Rust 범용 개발**에서 반복되는 기반 모듈을 내장합니다.
   - **상태성(State & Reactivity — `signal(init)`)**: UI(`serge`), 백엔드(`orbic`), 엔진·도메인 로직이 공통으로 참조하는 직관적인 신호(`signal(0)`)·상태 그래프·수명 모델.
   - **직렬화(`hyve-serde`) · 파서(`hyve-parser`) · 검증(`hyve-validate`)**: 고성능 내부 직렬화/역직렬화, 선언적 구문·에셋·설정 파서, 그리고 구조적 데이터 검증(`#[validate]` / `Valid<T>`) 및 범용 스키마 리플렉션.
   - **환경 설정 로더(`hyve-config`) & 구조화 추적(`hyve-trace`)**: `.env`·`hyve.toml`·환경 변수를 시작 시점에 `#[model]`로 즉시 검증하는 설정 로더와, 프론트부터 백엔드·DB·MSA까지 단일 `TraceId`/`RequestId`로 잇는 로깅·트레이싱.
   - **컴파일 타임 에셋(`asset!`) & 표준 프리미티브(`hyve-core`)**: 이미지·폰트·셰이더 경로를 컴파일 타임에 검증해 타깃별로 번들링하는 `asset!("/assets/...")`, 시간 정렬 고유 ID(`Ulid`), UTC 시각(`Timestamp`), 체크섬/해싱, 구조적 취소 토큰(`CancellationToken`).
3. **범용 규격 인터페이스 통합 (`hyve-api`)**
   - 모듈·프로세스·네트워크·플러그인 경계를 넘나들 때 사용하는 **추상 스키마 및 규격 인터페이스(Contract)**를 단일 선언으로 통합합니다(`serge` ↔ `orbic` 간 인라인 서버 함수 호출 및 Dual-Rail 전송 계약 포함).
4. **아키텍처를 제한하지 않는 고점(High-Ceiling) 설계**
   - 프레임워크가 고정된 폴더 구조나 단일 아키텍처를 강제하지 않습니다. 어떤 도메인이든 독립된 **Rust 스타일 컴포넌트(Component)**와 저수준 프리미티브로 조립할 수 있게 고점을 열어 둡니다.

## 2. 레퍼런스와 구현 전략 (확정)

| 축 | 레퍼런스 / 전략 | 채용 이유 |
|---|---|---|
| **툴체인 & 내장 코어 포지션** | **Bun** (기존 Rust/Cargo = Node.js 대비) | 파편화된 빌드·테스트·패키지 도구와 기본 런타임 유틸리티를 단일 고속 툴체인과 내장 코어로 수직 통합 |
| **범용 매크로 & 컴포넌트 조립** | **선언적 어트리뷰트(`#[model]`, `#[validate]`) + 순수 Rust 체이닝** | 모델·스키마·검증·테스트·에셋처럼 반복 보일러플레이트가 큰 영역은 어트리뷰트/매크로로 압축하고, 컴포넌트·컨텍스트 조립은 순수 Rust 체이닝으로 구성 (UI 뷰 DSL은 `serge` 소관) |
| **코어 구현 (`unit optimization`)** | **고성능 검증 크레이트 포크 + 단계적 `unit optimization`** | 1단계로 범용 추상화·미사용 피처를 걷어내고 `hyve` 코어 타입에 직결(수직화)하며, 추후 병목 구간에 한해 2단계(Arena/SoA·SIMD 등) 심화 개조 여지를 확보 |
| **계층 수직화** | **`hyve` 코어를 `ref`로 삼고 그 위에 호환 어댑터 배치** | `orbic`, `serge`가 제각각 모델을 정의하지 않고 `hyve` 코어를 단일 원천으로 공유하고, 기존 Rust 생태계(`serde`/Cargo 등)와는 어댑터로 호환 |

## 3. 책임 경계: 소유하는 것과 소유하지 않는 것

- **소유함**:
  - 타깃 중립 코어 모델(`hyve-core`), 상태성(`hyve-state`), 직렬화(`hyve-serde`), 검증(`hyve-validate`), 설정(`hyve-config`), 추적(`hyve-trace`), 파서(`hyve-parser`), 규격 인터페이스(`hyve-api`), 진단(`hyve-diagnostics`), 범용 매크로(`hyve-macros`), 통합 CLI(`hyve-cli`).
- **소유하지 않음**:
  - 백엔드 전용 인프라·DB·MSA·WS/WT·OpenAPI 런타임(`orbic` 소관)
  - UI 선언적 뷰 DSL(`ui!`) 및 브라우저 DOM·SSR·네이티브 GPU/CPU UI 렌더러(`serge` 소관)
- **의존 방향**:
  - `orbic`, `serge` → `hyve` (단방향). `hyve` 코어 크레이트는 특정 타깃(DOM, OS 윈도우, 특정 DB 드라이버)에 직접 의존하지 않으며, CLI(`apps/hyve-cli`)만 워크플로 오케스트레이션을 위해 각 타깃 러너를 호출합니다.
