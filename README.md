# hyperish/hyve

하이퍼리쉬(Hyperish) 제품군과 프레임워크 패밀리(`orbic`, `serge`) 전체를 아우르는 **올인원(All-in-One) 코어 개발 프레임워크 및 툴체인** 레포입니다.

> **포지셔닝: 기존 Rust / Cargo 생태계가 Node.js라면, `hyve`는 Bun입니다.**

```rust
use hyve::prelude::*;

// 웹·서버뿐 아니라 그래픽 엔진, 네이티브 앱, CLI, 설정·에셋 포맷 등 모든 범용 개발에서
// 모델 하나만 선언하면:
// 1) 고성능 직렬화/역직렬화 (hyve-serde)
// 2) 구조적 데이터 검증 (hyve-validate)
// 3) 범용 스키마 메타데이터(hyve-validate)와 규격 인터페이스(hyve-api)
// 가 한 번에 도출됩니다.
#[model]
pub struct SceneConfig {
    #[validate(len = 1..=80)]
    pub name: String,
    #[validate(range = 1..=240)]
    pub target_fps: u32,
}
```

## 왜 `hyve`인가?

| 기존 Rust 생태계의 한계 | `hyve`의 해답 |
|---|---|
| **크레이트 조립 지옥**: 새 프로젝트마다 `serde`, 파서, 시그널, 검증기, 에러, 스키마 리플렉션 등 수십 개 크레이트를 직접 고르고 호환성을 맞춰야 함 | **기본 내장(Batteries-included) 범용 코어**: 반응형 상태(`signal`), 직렬화, 파서, Validation, 범용 스키마·규격 인터페이스가 단일 코어로 내장 |
| **파편화된 빌드·개발 도구**: `cargo-watch`, 번들러, `nextest`, 린트 러너를 따로 설치·설정해야 함 | **올인원 단일 바이너리 CLI**: `hyve init` → `hyve dev` → `hyve test` → `hyve lint` → `hyve build` 하나로 범용 크레이트·엔진·백엔드(`orbic`)·UI(`serge`)를 모두 구동 |
| **구조를 강제하는 프레임워크**: 고정된 폴더 구조나 매크로 감옥에 갇혀 대형 프로젝트에서 아키텍처 고점이 막힘 | **아키텍처를 제한하지 않는 고점(High-Ceiling)**: 빠른 시작용 매크로와 저수준 순수 Rust 빌더·코어 프리미티브를 1:1로 열어 개발자가 직접 아키텍처를 설계 |

상위 범용 명세와 문서 상태 규칙(`확정` / `제안` / `미정`)은 [`hyperish/docs`](https://github.com/hyperish/docs)를 따릅니다.

## 문서 목록

| 번호 | 문서 | 내용 |
|---|---|---|
| 00 | [개요와 범위](docs/00-overview.md) | `hyve`의 정체성, 책임 경계, 레퍼런스, 코어 `ref` 수직화 원칙 |
| 01 | [DX 및 코어 매크로·CLI 설계](docs/01-dx-and-dsl.md) | 범용 모델·검증·설정·상태 DX 및 `init`/`dev`/`lint`/`test`/`build` 올인원 CLI |
| 02 | [아키텍처와 크레이트 구조](docs/02-architecture.md) | `apps/`, `crates/` 모노레포 구성, 상태성·직렬화·파서·규격 인터페이스 코어 |
| 03 | [파이프라인과 호환 어댑터](docs/03-adapters-and-pipeline.md) | `orbic`·`serge` 타깃 연결 파이프라인, 규격 통합 인터페이스, 외부 호환 어댑터 |
| 04 | [결정 기록](docs/04-open-questions.md) | `hyve` 레포의 열린 항목과 확정된 계약 |
| 05 | [구현 계획](docs/05-implementation-plan.md) | Phase 1 작업 순서와 완료 기준 |
