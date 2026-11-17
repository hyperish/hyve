# hyperish/hyper

하이퍼리쉬(Hyperish) 제품군과 프레임워크 패밀리(`ring`, `lace`, `hera`) 전체를 아우르는 **코어 개발 프레임워크 및 통합 툴체인** 레포입니다.

프로젝트 초기화(`init`), 실시간 미리보기(`preview`), 정적 검사(`lint`), 테스트(`test`) 등 개발 생명주기 전반의 생산성을 관리하며, 반응형 상태성(state/reactivity)·직렬화(serialization)·파서(parser)·통합 API 규격 인터페이스 등 **모든 타깃 레이어가 수직 통합을 위해 참조(ref)하는 범용 코어 모듈**을 소유합니다.

상위 범용 명세는 [`hyperish/docs`](https://github.com/hyperish/docs)를 따릅니다.

## 문서 상태 규칙

| 상태 | 의미 |
|---|---|
| **확정** | 명시적으로 결정된 사실. 바꾸려면 리뷰 승인과 새 결정 기록이 필요하다 |
| **제안** | 논의를 위한 설계안. 바꾸기 쉽게 적은 것이며, 확정이 아니다 |
| **미정** | 아직 결정되지 않은 사항. [`docs/04-open-questions.md`](docs/04-open-questions.md)에서 추적한다 |

## 문서 목록 (4개 레포 공통 아키텍처)

| 번호 | 문서 | 내용 |
|---|---|---|
| 00 | [개요와 범위](docs/00-overview.md) | `hyper`의 정체성, 책임 경계, 레퍼런스, 코어 `ref` 수직화 원칙 |
| 01 | [DX 및 DSL 설계](docs/01-dx-and-dsl.md) | Dioxus `rsx!`·매크로 레퍼런스 기반 DSL, `init`/`preview`/`lint`/`test` 통합 CLI DX |
| 02 | [아키텍처와 크레이트 구조](docs/02-architecture.md) | `apps/`, `crates/` 모노레포 구성, 상태성·직렬화·파서·API 규격 코어, 내부 포크 운영 |
| 03 | [파이프라인과 호환 어댑터](docs/03-adapters-and-pipeline.md) | `ring`·`lace`·`hera` 타깃 연결 파이프라인, API 규격 통합 인터페이스, 외부 호환 어댑터 |
| 04 | [열린 결정](docs/04-open-questions.md) | `hyper` 레포의 미정 사항 목록과 결정 기록 |
