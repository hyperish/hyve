# 03. 파이프라인과 호환 어댑터 (hyve)

> 상태: 코어 `ref` + 어댑터 분리 구조는 **확정**, 세부 파이프라인은 **제안** · 기준일: 2026-11-19

`hyve`는 모든 계층의 코어 기준(`ref`)이므로, 내부 코어 연산과 외부/타깃 호환 어댑터 사이의 경계를 가장 엄격하게 정의합니다(`hyperish/docs` `02-core-contracts.md` §5).

## 1. 코어(`ref`) 중심 수직 통합 파이프라인

```
               ┌─────────────────────────────────────────────────────┐
               │                        hyve                         │
               │  (범용 매크로 · 상태 · 직렬화 · 검증 · 파서 · API)  │
               └───────────────┬──────────────────┬──────────────────┘
                               │ ref              │ ref
                               ▼                  ▼
                            [ orbic ]           [ serge ]
                            (백엔드)     (Web DOM + Native GPU/CPU UI)
                               │                  │
                               ▼                  ▼
                         [호환 어댑터]        [타깃 & 호환 어댑터]
                         HTTP/WS/DB         Web(DOM/SSR/Ionic) · Native(GPU/CPU/OS)
```

1. **단일 모델·단일 계약**: 데이터 모델, 검증 규칙, 상태 시그널, API 인터페이스는 `hyve` 코어 크레이트에서 한 번만 정의합니다.
2. **제로 보일러플레이트 연결**: `orbic`은 `hyve-api`와 `hyve-serde`를 참조해 핸들러와 전송 파이프를 구성하고, `serge`(웹·네이티브)는 동일한 `hyve-api`와 `hyve-state`를 참조해 UI 상태와 원격 호출을 직결합니다.
3. **개발 툴 파이프라인 (`hyve preview` / `lint` / `test` / `build`)**: CLI가 워크스페이스 그래프를 분석해 코어 변경이 백엔드(`orbic`)와 웹·네이티브 UI(`serge`)에 미치는 영향을 한 번에 빌드·검증합니다.

## 2. 어댑터 경계와 변환 규칙

`hyve`에서 정의하는 어댑터 트레이트(`Adapter`, `CodecAdapter`, `TransportAdapter` 등)는 다음 계약을 강제합니다:

- **코어의 외부 규약 무지성**: `hyve-core`, `hyve-state`에는 HTTP 상태 코드, 브라우저 DOM 노드, 특정 GPU API 타입이 절대 들어가지 않습니다.
- **손실 정책의 명시화**: 내부 모델의 타입(예: 고정밀 수치, 커스텀 열거형, 스트리밍 핸들)을 외부 JSON이나 웹 브리지로 변환할 때 표현 불가능한 값이 있으면 `Reject`(거부) / `Approximate`(근사) / `PassThrough`(보존 필드 통과) 중 하나를 어댑터가 명시적으로 선택해야 합니다.
- **API 규격 어댑터 연결**:
  - 내부 통신(예: `orbic` 내부 MSA, `serge` SSR ↔ `orbic`, 네이티브 IPC)은 고성능 내부 코덱(`hyve-serde`)을 직접 사용할 수 있습니다.
  - 외부 공개 API나 서드파티 연동이 필요한 경계에서는 `hyperish/docs`의 `03-api-protocol.md`(HTTP/JSON) 및 `04-realtime-communication.md`(WebSocket) 규약을 구현하는 어댑터를 장착합니다.
