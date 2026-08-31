# 원격 Agent Client (사용자 운용 LLM 에이전트)

지금까지 agent-client는 게임 서버와 같은 머신에서, 운영자가 소유한 `npc_` 계정으로만 돌릴 수 있었다. 이 문서는 **외부 사용자가 자기 머신에서, 자기 구글 계정으로, 자기 LLM 구독으로 에이전트를 돌리는** 구조의 설계다. 2026-07-22 논의 결과이며, 아래 구현 계획의 Phase 0~3이 같은 날 반영되었다. 실행 방법은 [AGENT_CLIENT_QUICKSTART.md](AGENT_CLIENT_QUICKSTART.md).

기존 NPC 운용(Rica, Karl) 설계는 [AGENT_CLIENT.md](AGENT_CLIENT.md), 거래·흥정 방어선은 [ECONOMY.md](ECONOMY.md) 참고.

## 목표와 범위

**1차 목표는 하나다: LLM이 평범한 플레이어로서 게임을 플레이한다.** 돌아다니고, 싸우고, 대화하고, 성장한다. 그게 전부다.

상인·경비병 같은 **역할을 사용자 에이전트에게 맡기는 것은 이번 범위가 아니다.** 그 방향은 아래 "나중 과제"에 조사 결과만 남겨두고, 실제로 열게 될 때 별도로 설계한다.

## 설계 원칙: 에이전트를 따로 취급하지 않는다

**agent-client로 접속했다는 이유로 서버가 다르게 대하지 않는다.** 게임 철학이고, [README](../README.md)가 내세우는 Agent–Human Parity — "에이전트와 인간이 완전히 같은 프로토콜을 쓰고, 서버는 둘을 구분하지 않는다" — 를 그대로 지킨다는 뜻이다. 이 원칙이 아래 결정들의 상위에 있다.

따라오는 것들:

- **개인에게 표식을 붙이지 않는다.** 이름표에도, 클라이언트 UI에도, 브로드캐스트되는 플레이어 데이터에도 "이 캐릭터는 에이전트"라는 정보는 없다
- 다만 **집계는 본다**: `/who`는 접속 클라이언트 종류별 인원수를 보여준다 (아래 참조). 누가 무엇인지는 알 수 없고 몇 명인지만 아는 선 — 월드가 어떻게 돌아가는지 궁금한 것과 개인을 분류하는 것은 다르다
- **에이전트 전용 정책을 만들지 않는다.** 몬스터 스폰 규칙, 이동 검증, 레이트 리밋은 접속 수단과 무관하게 모든 플레이어에게 같은 값으로 적용한다
- **에이전트 전용 권한도 만들지 않는다.** 뒤집어 말하면 사용자 에이전트가 필요로 하는 추가 권한이 애초에 없다 — agent-client는 이미 평범한 플레이어 계정으로 다 돌아간다
- **허용목록 같은 신원 기반 운영 수단은 성립하지 않는다.** 서버가 에이전트를 식별하지 않으므로 식별을 전제한 통제도 못 쓴다. 남용 대응은 **행동 기준**(과도한 메시지 빈도, 비정상 이동)으로 하고, 그 기준은 인간 클라이언트에도 똑같이 적용된다. 매크로를 돌리는 사람과 봇을 구분할 이유가 없다는 점에서 오히려 더 옳은 기준이다

`is_official_npc`가 남아 있는 건 이 원칙의 예외가 아니다. 그건 "LLM이 조종함"이 아니라 **"운영자가 소유한 공식 NPC"** 라는 뜻이고, 사용자 에이전트는 여기에 해당하지 않는다 (아래 참조).

## 요구사항과 결정 요약

| 요구사항 | 결정 |
|---|---|
| 다른 머신에서 실행 | 터레인 높이맵을 HTTP로 받고, `wss://` TLS를 켠다. 실제 블로커는 토큰이 아니라 이 두 가지 |
| 구글 로그인으로 실행 | OAuth 2.0 Device Flow (헤드리스용). 서버는 audience를 복수로 허용 |
| 실행자 계정으로 codex 실행 | **이미 그렇다.** `codex exec`를 로컬 프로세스로 띄우므로 그 머신의 `~/.codex` 인증을 쓴다. 문서화만 필요 |
| merchant/guard 클래스 차단 | 서버 `CreateCharacter`에 클래스 화이트리스트. 이건 **밸런스** 결정이고, 인간·에이전트 구분 없이 똑같이 적용된다 |
| 사용자 에이전트 취급 | 일반 플레이어와 **완전히 동일**. `is_npc`는 이름만 `is_official_npc`로 바로잡는다 |
| 옵션인가 새 클라인가 | 같은 바이너리에 인증 모드 추가. 브라우저 에이전트는 별도 트랙으로 분리 |
| 구버전 원격 클라이언트 | **하위 호환 코드를 만들지 않는다.** 프로토콜 버전이 다르면 접속을 거절하고 업데이트를 안내한다 |

## 현재 구조: 무엇이 서버 머신에 묶여 있나

| 의존성 | 현재 코드 | 원격 실행에 필요한 작업 |
|---|---|---|
| NPC 토큰 | [`main.rs`](../agent-client/src/main.rs) `resolve_npc_token`이 `../data/npc_token`을 읽음 | 구글 로그인으로 대체되므로 자동 해소 |
| **터레인 높이맵** | [`main.rs`](../agent-client/src/main.rs)이 로컬 `../data/terrain`을 `HeightSampler`로 직접 읽음. `data/terrain/height`만 **3.1 GB / 1024 타일** | 가장 큰 블로커. HTTP 타일 소스 추가 (아래 참조) |
| **wss://** | `tokio-tungstenite = "0.26"`에 TLS feature가 하나도 안 켜져 있어 `wss://` 접속이 실패한다 | `rustls-tls-webpki-roots` feature 추가 (아래 참조) |
| 게임 데이터 | items/merchants/npcs/monsters/behavior_trees는 전부 `include_str!`로 바이너리에 포함 | 없음 |
| 프롬프트·메모리 | `data/templates/*`, `data/npcs/{id}/*`, `system_prompt.txt`, `animation_durations.json` — 전부 수 KB | 배포물에 동봉 |

즉 배포 단위는 **바이너리 + 작은 `data/` 디렉터리 + `config.toml`** 이면 충분하다.

### 왜 웹 클라이언트는 TLS 작업이 필요 없었나

"인간용 클라는 wss 없이 잘 도는데 왜 에이전트만 필요한가"는 오해다. **웹 클라이언트도 wss를 쓴다** — `getDefaultServerUrl()`이 페이지가 https면 `wss://<host>/ws`를 만들고([`networkUtils.ts`](../client/src/lib/utils/networkUtils.ts)), 프로드는 https이므로 브라우저는 항상 wss로 붙는다. 리버스 프록시가 TLS를 끊고 뒤의 평문 `ws://127.0.0.1:10006`으로 넘긴다.

차이는 프로토콜이 아니라 **TLS 스택을 누가 갖고 있느냐**다. 브라우저는 내장하고 있고, Rust 클라이언트는 빌드에 넣어야 한다. 지금 agent-client가 TLS 없이 도는 이유는 서버와 같은 머신에 있어 프록시를 건너뛰고 루프백에 직접 붙기 때문이다.

평문 ws 포트를 외부에 여는 선택지는 버린다. 구글 ID 토큰이 평문으로 흐르고, 서버 신원 검증이 없어 중간자가 월드 상태를 위조할 수 있다. (운영자 본인이 임시로 쓸 SSH 터널은 코드 변경 없이 지금도 가능하지만 공개 배포용은 아니다.) `reqwest`가 이미 rustls를 끌어오므로 feature 한 줄로 끝난다.

전제 조건 하나: 프로드에서 `/api/terrain/*`가 외부에 노출되어 있어야 한다. 서버 자체는 `--api-bind` 기본값이 `127.0.0.1`이고, 웹 클라이언트는 같은 오리진 프록시를 통해 접근한다 ([`networkUtils.ts`](../client/src/lib/utils/networkUtils.ts) `getTerrainApiUrl`). 리버스 프록시가 이미 웹 클라용으로 이 경로를 열어두고 있다면 추가 작업이 없다.

## 결정 1: 새 실행 파일이 아니라 agent-client의 인증 모드

**같은 바이너리에 인증 모드를 추가한다.**

근거는 코드 공유 비율이다. 드라이버·상태 관리·이동/경로탐색·몬스터 AI·프롬프트 조립이 약 5,300줄인데, 원격 운용에서 달라지는 것은 **인증 방식, 터레인 소스, 페르소나 선택 규칙** 세 갈래뿐이다. 새 크레이트로 포크하면 프로토콜이 바뀔 때마다 두 곳을 고쳐야 하고, 실제로 그 프로토콜은 자주 바뀐다.

```toml
# agent-client/data/config.toml (원격 사용자용 예시)

server  = "wss://<prod-host>/ws"
terrain = "https://<prod-host>"      # 로컬 경로 대신 HTTP 소스

[auth]
mode = "google"                       # 기본값은 기존 "npc_token"
# token_cache = "~/.config/onlinerpg/google.json"  # refresh token 저장 위치

[codex]
model = "gpt-5.4-mini"

[[npcs]]
character_name = "Jake's Agent"
character_class = "rogue"
gender = "female"
llm = "codex"
```

`mode = "google"`이 아래를 한꺼번에 함의하게 묶는다. 설정 실수로 보안 결정이 뒤집히지 않도록 개별 플래그로 쪼개지 않는다.

- `[[npcs]] id = "..."`(레지스트리 NPC) 금지 → 시작 시 에러. 공식 NPC 정의는 운영자 것이다
- `account` 필드 무시 → 계정은 구글 `sub`에서 서버가 결정한다
- 클래스 화이트리스트 (클라이언트 측 조기 실패용, 진짜 집행은 서버)
- 터레인은 HTTP 소스 강제
- 캐릭터 삭제 금지 → 계정은 사람의 것이고 웹 클라이언트로 키우는 캐릭터가 같이 산다. `character_name`이 일치하는 캐릭터만 이 에이전트 것으로 보고, 나머지는 건드리지 않는다 (npc_token 전용 계정만 불일치 캐릭터를 정리한다)

`[[npcs]]` 개수에 인위적 상한은 두지 않는다. 서버가 이미 자연스럽게 제한한다 — 계정당 캐릭터는 3개까지이고, 같은 계정으로 다시 인증하면 기존 세션을 끊으므로 계정 하나당 살아 있는 세션은 하나다. 이건 인간 플레이어에게도 똑같이 적용되는 제약이라 원칙에 어긋나지 않는다.

`[[npcs]]`라는 이름 자체는 이제 어색하다 (사용자 에이전트는 NPC가 아니다). 구현할 때 `[[agents]]` 등으로 바꾸고 기존 키를 별칭으로 남기는 편이 좋다.

**나중에 실행 파일을 갈라야 할 조건**: 원격 사용자용 기능(대화형 로그인 UI, 자격증명 관리, 자동 업데이트)이 상주 NPC 운용 코드와 섞이기 시작하면 그때 `agent-client` / `agent-cli` 로 나눈다. 지금은 그 분기점이 아니다.

## 결정 2: 브라우저 에이전트는 별도 트랙 — 지금은 하지 않는다

"인간용 클라가 브라우저에서 도는데 LLM용도 브라우저에서 돌면 안 되나"는 타당한 질문이고, 실제로 웹 클라이언트는 이미 에이전트에 필요한 기반을 대부분 갖고 있다.

**브라우저가 이미 가진 것**
- A* 경로탐색 + passability 캐시 + 몬스터 AI 브레인이 전부 wasm으로 노출되어 있다 ([`wasm_api.rs`](../shared/src/wasm_api.rs), [`monsterManager.ts`](../client/src/lib/managers/monsterManager.ts)의 `ai_create_brain` / `ai_tick_brain`). agent-client의 Rust 로직과 **같은 코드**다
- 터레인 타일, 하우징, 던전 지오메트리를 이미 스트리밍한다 — 3.1 GB 문제가 없다
- 구글 로그인이 이미 붙어 있다. 인증 작업이 0이다
- 에이전트가 무엇을 보고 무엇을 하는지 화면으로 관찰된다 ([AGENT_CLIENT.md](AGENT_CLIENT.md)의 "대안 2")

**그럼에도 이번 요구사항을 못 채우는 이유**
- **codex CLI를 못 돌린다.** 브라우저에는 서브프로세스가 없다. "실행자 계정으로 codex 실행"은 로컬 프로세스가 `~/.codex`의 구독 인증을 쓴다는 뜻인데, 브라우저에서는 API 키를 직접 넣는 과금 모델로 바뀐다. 요구사항 3과 정면으로 충돌한다
- **상주성이 없다.** agent-client는 재접속 루프를 돌며 며칠씩 산다. 탭은 닫히고, 백그라운드 탭의 타이머는 브라우저가 강하게 스로틀한다. 전투 중 반응 지연이 곧 죽음이다
- **자격증명 노출.** API 키를 넣는 순간 브라우저 저장소에 남고, 확장 프로그램·XSS의 사정거리에 들어온다

**결론**: Rust 트랙을 먼저 한다. 브라우저 에이전트는 나중에 **다른 목적**으로 만든다 — 설치 없이 체험하는 "구경용 에이전트 모드"(BYO OpenRouter 키, 탭 열려 있는 동안만 동작). 두 트랙은 wasm으로 이미 로직을 공유하므로 중복 구현이 아니다.

## 인증: 구글 Device Flow

서버에는 이미 구글 경로가 있다 ([`connection.rs`](../server/src/connection.rs) `Authenticate { google_id_token }`). 브라우저 GSI는 헤드리스에서 못 쓰므로 CLI는 **Device Authorization Grant**(구글의 "TV·입력 제한 기기" 플로우)를 쓴다.

```
agent-client                     구글                      사용자
    │  device/code 요청  ────────▶ │
    │  ◀──── user_code + URL       │
    │  콘솔에 URL/코드 출력 ─────────────────────────────────▶ 아무 브라우저에서 로그인
    │  token 폴링       ────────▶ │
    │  ◀──── id_token + refresh_token
    │
    │  Authenticate { google_id_token }  ───▶ 게임 서버
```

로컬 브라우저가 있는 환경이면 loopback + PKCE도 가능하지만, 헤드리스 서버까지 커버하는 device flow 하나로 통일한다.

**서버 변경**: device flow는 별도 OAuth 클라이언트를 쓰므로 `aud`가 웹 클라이언트 ID와 다르다. [`google_auth.rs`](../server/src/google_auth.rs)의 `set_audience(&[&self.client_id])`를 복수 audience로 바꾸고 `--google-cli-client-id`를 추가한다. JWKS 캐시·검증 로직은 그대로 재사용된다.

**토큰 수명**: ID 토큰은 1시간이고 에이전트는 며칠을 산다. `refresh_token`을 `~/.config/onlinerpg/google.json`(0600)에 저장하고 **재접속마다** 새 ID 토큰을 발급받는다. 서버는 접속 시점에만 검증하므로 세션 도중 갱신은 불필요하다.

## 터레인: HTTP 높이 소스

서버는 이미 `GET /api/terrain/height/{x}/{z}`를 공개로 서빙한다 ([`routes.rs`](../server/src/terrain/routes.rs)). GET은 인증 면제이고([`api_auth.rs`](../server/src/api_auth.rs)) 웹 클라이언트가 같은 엔드포인트를 쓴다.

- `HeightSampler`가 지금은 구체 타입 `TerrainIO`에 묶여 있으므로([`height.rs`](../terrain/src/height.rs)) 타일 읽기를 trait 또는 enum(`Local(TerrainIO)` / `Http(base_url)`)으로 한 겹 추상화한다
- 타일 캐시는 이미 인메모리 온디맨드다. 여기에 **디스크 캐시**를 얹어 재시작 시 재다운로드를 막는다 (타일 1개 ≈ 3 MB, 에이전트 하나가 실제로 만지는 것은 보통 수 개)
- 서버 측에 `Cache-Control` / `ETag`를 붙여두면 다수 에이전트가 붙어도 트래픽이 늘지 않는다

## codex: 실행자 계정 — 이미 그렇다

[`codex.rs`](../agent-client/src/codex.rs)는 `codex exec`를 그냥 로컬 프로세스로 띄운다. 그 머신 사용자의 인증·요금·레이트리밋을 그대로 쓴다. 위생 처리도 이미 되어 있다.

- `--sandbox read-only` — 프롬프트 인젝션이 파일을 건드리지 못한다
- `--ephemeral`, `current_dir(temp_dir)`, `--skip-git-repo-check` — 실행자의 리포지토리·`AGENTS.md`를 끌어오지 않는다
- `max_concurrent` 스케줄러가 프로세스 단위로 동시 호출을 제한하고, `request_timeout_secs`(기본 120)가 먹통이 된 CLI를 죽여 슬롯을 돌려준다

문서에 명시할 것: **codex CLI가 정상 동작하도록 만드는 것은 실행자 책임**이고, 게임 서버는 LLM 비용을 전혀 부담하지 않는다. 프롬프트에는 게임 월드에서 온 다른 플레이어의 채팅이 그대로 들어가므로, 인젝션 시도는 항상 있다고 가정한다 — 위 샌드박스 옵션을 임의로 낮추지 말 것.

## 권한 모델: 아무것도 주지 않는다

원칙("에이전트를 따로 취급하지 않는다")과 목표(평범한 플레이어로 플레이)를 합치면 권한 설계는 한 줄로 끝난다. **사용자 에이전트에게 주는 특별 권한은 없다.** 구글 로그인 사용자는 인간이든 LLM이 조종하든 똑같은 플레이어다.

그래도 지금 코드가 무엇을 특별 취급하고 있는지는 알아둘 필요가 있다. `is_npc` 플래그가 성격이 다른 일곱 가지를 한 덩어리로 준다.

| `is_npc = true`가 주는 것 | 코드 | 사용자 에이전트에게 필요한가 |
|---|---|:--:|
| 거래 상대로 인정 (상점 열기, 가격 밴드) | [`trading.rs`](../server/src/game_state/trading.rs) `validate_trader` | 역할을 맡기기 전까지 불필요 |
| 플레이어 화면에 거래창을 **밀어넣기** | [`trading.rs`](../server/src/game_state/trading.rs) `open_trade` | 위에 종속 |
| 급여 수령 (골드 파우셋) | [`salary.rs`](../server/src/game_state/salary.rs) | 절대 아님 |
| 이동 시 충돌 검사 면제 | [`player.rs`](../server/src/game_state/player.rs) `check_collision: !is_npc` | 아님 (치트다) |
| 주변 몬스터 앰비언트 스폰 대상에서 제외 | [`ambient_spawn.rs`](../server/src/game_state/ambient_spawn.rs) | 아님 — 인간과 같은 규칙을 받아야 사냥으로 성장한다 |
| 클라이언트 UI에서 NPC 취급 (클릭 → 대화/거래) | [`PlayerControl.svelte`](../client/src/lib/components/PlayerControl.svelte) | 아님 — 공식 NPC로 오인시키면 안 된다 |
| `/who` 집계에서 NPC로 분류 | [`chat.rs`](../server/src/game_state/chat.rs) | 아님 — 클라이언트 종류로 다시 센다 (아래) |

`is_npc`는 `AuthenticateNpc`(공유 비밀 토큰) 경로에서만 세워진다. **구글 로그인 사용자는 어떤 클래스를 고르든 `is_npc = false`이므로 위 일곱 가지를 하나도 얻지 못한다** — 그리고 하나도 필요하지 않다. 캐릭터 이름은 전역 유니크라([`auth.rs`](../server/src/auth.rs)) "Rica"/"Karl" 사칭도 이미 막혀 있다.

여기서 원칙이 한 번 더 확인된다. 사용자 에이전트는 다른 플레이어의 화면에서도 그냥 플레이어로 보인다 — 클릭해도 상점이 열리지 않고, 이름표도 평범하다.

### 이름만 바로잡는다: `is_npc` → `is_official_npc`

플래그 이름이 오해를 부른다. 사용자 에이전트도 일상어로는 "NPC"지만 이 플래그의 뜻은 **"운영자가 소유한 공식 NPC"** 다. 에이전트가 늘어날수록 `!is_npc`를 읽는 사람이 "인간"으로 잘못 이해하게 된다.

- `is_npc` → `is_official_npc` 로 개명한다 (테스트 제외 31곳, 기계적 치환)
- 브로드캐스트 필드이므로([`entity.rs`](../shared/src/entity.rs)) 웹 클라이언트 `isNpc`도 같이 바꾼다
- **3-way enum(`Human`/`Agent`/`OfficialNpc`)은 만들지 않는다.** `Agent` 항목이 존재하는 순간 "에이전트 전용 정책"을 붙일 자리가 생기고, 그건 원칙에 어긋난다. 서버가 에이전트를 구분하지 못하는 상태를 **의도적으로 유지한다**

### `/who`: 클라이언트 종류별 집계

지금은 `Online: 12 (10 human, 2 NPC)`처럼 `is_npc` 기준으로 나눈다. 에이전트가 늘면 `human`이라는 단어가 사실과 달라진다 — 서버는 그쪽이 사람인지 확인한 적이 없다. **접속한 클라이언트 종류로 바꿔 센다.**

```
Online: 12 (9 web, 1 cli, 2 npc)
```

- `web` — 브라우저 클라이언트
- `cli` — agent-client (사용자 운용)
- `npc` — 운영자 공식 NPC. 이건 서버가 `is_official_npc`로 **확실히 아는** 값이라 따로 남긴다. 여기에 합치면 "cli 2명"이 Rica·Karl뿐인지 진짜 사용자 에이전트인지 구별이 안 된다

설계 제약 셋:

1. **집계 전용이다.** 클라이언트 종류는 브로드캐스트되는 `Player` 데이터에 넣지 않는다 (`#[serde(skip)]`). 넣는 순간 다른 클라이언트가 개인을 분류할 수 있게 되고, 그건 위 원칙 위반이다
2. **자기 신고값이고, 그래도 괜찮다.** 클라이언트가 스스로 "나는 web/cli"라고 밝히는 값이라 거짓말이 가능하다. 하지만 이 값은 카운터 말고 아무것에도 쓰이지 않으므로 속일 동기가 없다. 반대로 말하면 **여기에 어떤 정책도 걸면 안 된다** — 거는 순간 거짓 신고 동기가 생긴다
3. **버전 handshake에 얹어 보낸다.** 아래 프로토콜 버전 절의 `ClientInfo { protocol_version, client_kind, client_version }` 한 메시지로 끝난다 — 종류 표시를 위해 따로 만들 것이 없다

명칭 주의: 나중에 브라우저 에이전트 모드가 생기면 그건 `web`으로 잡힌다. 이 축은 "인간이냐 LLM이냐"가 아니라 **"어떤 클라이언트 프로그램이냐"** 다. 그게 의도한 바다 — 전자는 서버가 알 수 없고, 알 필요도 없다.

### 부수 효과: 남용 대응은 행동 기준으로만 가능하다

에이전트를 식별하지 않기로 했으므로 "에이전트 계정 허용목록", "봇 총량 상한" 같은 신원 기반 통제는 쓸 수 없다. 남는 수단은 행동 기준이고, 그 편이 옳다.

- 채팅 빈도·이동 검증·거래 요청 빈도에 **모든 클라이언트 공통** 상한을 둔다. 매크로 도는 사람과 LLM이 도는 클라이언트를 구분할 이유가 없다
- 실제 문제를 일으킨 계정은 계정 단위로 차단한다 (지금은 킥만 있고([`Kicked`](../shared/src/messages.rs)) 재접속 루프를 도는 클라이언트에는 일시 차단이 필요하다 — 이것도 에이전트 전용이 아니라 공용 수단이다)
- 부하가 문제가 되면 접속 수단이 아니라 **동시 접속 총량**으로 조절한다

## 프로토콜 버전: 안 맞으면 그냥 거절한다

지금은 서버·웹 클라·agent-client가 프로드 호스트에서 한 번에 배포된다 ([`deploy-prod.sh`](../tools/deploy-prod.sh)). agent-client는 프로드에서만 돌기 때문에 **버전이 어긋난 클라이언트라는 것이 아예 존재하지 않는다.** 원격 배포를 시작하는 순간 그게 깨진다 — 남의 머신에서 도는 구버전을 우리가 갱신할 수 없다. 그래서 아래 handshake는 첫 원격 배포보다 **먼저** 들어가야 한다 (Phase 0). 지금 넣으면 "handshake를 모르는 클라이언트"를 다루는 코드를 영영 쓰지 않아도 된다.

**결정: 하위 호환 코드를 쓰지 않는다.** 버전이 정확히 일치하지 않으면 접속을 거절하고 업데이트하라고 안내한다. 옵셔널 필드, 버전별 분기, 메시지 스킵 같은 호환 장치는 하나도 만들지 않는다 — 그런 코드는 조합 폭발과 "구버전에서만 나는 버그"를 만들고, 재현이 가장 어려운 종류의 버그다. 대신 **잘못된 조합이 애초에 접속하지 못하게** 한다.

### 규칙

1. `shared`에 정수 하나를 둔다: `pub const PROTOCOL_VERSION: u32`. **와이어 포맷이나 메시지 의미가 바뀌면 무조건 올린다.** 판단이 애매하면 올린다 — 올려서 손해 보는 건 사용자의 재시작 한 번뿐이다
2. 클라이언트는 접속 직후, 인증보다 **먼저** `ClientMessage::ClientInfo { protocol_version, client_kind, client_version }`을 보낸다
3. 서버는 `protocol_version`이 자기 값과 **정확히 같지 않으면** 거절한다. `>=` 같은 범위 규칙도 두지 않는다 — 범위를 허용하는 순간 그 범위를 유지하는 코드가 필요해진다
4. `ClientInfo`가 안 왔으면 그것도 거절이다. 옵션이 아니라 **필수 첫 메시지**로 둔다 (지금 모든 클라이언트가 우리 것이므로 예외 경로를 만들 필요가 없다)
5. 거절은 **기존 `AuthError { message }`** 로 보낸다. 새 거절 전용 메시지를 만들면 그걸 모르는 버전에게는 전달되지 않는다 — **거절 통로만큼은 절대 바꾸지 않는다**
6. 메시지 내용에 **무엇을 해야 하는지**를 담는다: `"Protocol v7 required (you sent v5). Update: <다운로드 URL 또는 명령>"`
7. agent-client는 이 에러를 받으면 **재접속하지 않고 종료한다.** 세션 실패를 전부 재접속으로 처리하는 구조라([`orchestrator.rs`](../agent-client/src/orchestrator.rs) `run_npc_loop`) 그냥 두면 버전 불일치도 무한 루프가 된다. 고칠 수 없는 실패는 즉시 죽어야 사용자가 알아챈다. (프로드 유닛은 `Restart=always`라 그래도 10초 뒤 재시작되지만, 프로드는 두 바이너리를 함께 배포하므로 불일치 자체가 나지 않는다)
8. 거절은 `AuthError`에 이어 **close code `CLOSE_CODE_PROTOCOL_MISMATCH`(4001)** 로도 닫는다. close code는 직렬화된 메시지 바깥이라 `PROTOCOL_VERSION`을 또 올리지 않고도 전달되고, 메시지를 못 읽는 클라이언트에게도 "재시도해도 소용없다"가 닿는 유일한 신호다. 단 close reason은 123바이트 제한이 있으니 안내 문구는 `AuthError`에 싣고 reason은 짧게 둔다

### 던전 레이아웃 지문

프로토콜 버전이 잡지 못하는 불일치가 하나 있다: **던전 레이아웃은 네트워크로 오가지 않는다.** 양쪽이 `shared`의 생성기를 각자 컴파일해 입구 id에서 같은 미로를 만들어내므로, 생성기가 바뀌면 메시지 모양은 그대로인 채 **구버전 클라만 다른 미로를 그린다.** 자기에게만 있는 벽을 계속 밀고, 서버는 계속 되돌려 놓는다 (2026-08-19 배포 후 실제로 발생).

- `shared/build.rs`가 `src/dungeon/`의 `.rs`(테스트 제외 — 테스트 수정으로 전체가 리로드될 이유는 없다)와 `data-src/dungeons.csv`를 해시해 `LAYOUT_VERSION`을 박는다. 디렉터리를 훑으므로 생성기 파일이 새로 생겨도 자동으로 포함된다. 상수를 손으로 올릴 필요가 없다
  - 다만 지문 밖의 변경은 여전히 조용하다: `housing`/`pathfinding`처럼 생성기가 참조하는 다른 모듈, 그리고 `rand`/`rand_chacha` 버전 범프(RNG 스트림이 바뀌면 미로가 전부 바뀐다). 그런 변경을 할 때는 프로토콜 범프처럼 취급한다
- 해시는 FNV-1a를 직접 쓴다. `DefaultHasher`는 러스트 릴리스 간 안정성이 보장되지 않아 서버(리눅스)와 agent-client(윈도우) 빌드가 같은 소스로 갈라진다. CR도 걸러 CRLF 체크아웃을 흡수한다
- 클라이언트는 `client_version` 문자열 끝에 `+layout.<hash>`로 얹어 보낸다(`stamp_layout_version`). **`ClientInfo`에 필드를 더하지 않는 것이 요점이다** — rmp_serde는 구조체를 위치 배열로 인코딩하므로 필드를 하나 더하면 **새 서버가 구버전의 `ClientInfo`를 아예 디코드하지 못한다.** 그 실패는 `error!` 로그로만 끝나서 `AuthError`도 close 4001도 나가지 않고, 구버전은 이유도 모른 채 백오프로 재시도만 반복한다. 태그는 문자열이라 `+`로 이어 붙는 목록이며, 나중에 다른 지문(가구·주택 등)이 생기면 그 자리에 함께 실으면 된다
- 불일치(또는 지문 없음)는 **프로토콜 불일치와 똑같은 경로**로 거절한다: `AuthError` + close 4001. 웹은 안내를 띄우고 세션당 한 번 자동 새로고침하며, agent-client는 재접속하지 않고 종료한다
- 공식 NPC(`is_official_npc`)는 이 킥에서 제외한다. 서버와 함께 빌드·배포되므로 레이아웃이 어긋날 수가 없고, 그래도 걸렸다면 낡은 빌드가 아니라 **우리 쪽 버그**라는 뜻이다. 스트릭은 그대로 세되 경고만 남긴다 — 끊어봐야 agent-client가 재시작 루프에 빠지고 마을에서 상인이 사라진다
- 게이트를 통과한 뒤에야 어긋나는 경우(같은 벽을 계속 미는 클라이언트)는 서버가 세션을 끊으면서 **close `CLOSE_CODE_CLIENT_DESYNC`(4004)** 를 실어 보낸다. 웹은 4001과 같은 처리를 하고(안내 + 1회 자동 새로고침 + 재접속 중단), agent-client도 재접속하지 않고 종료한다 — 같은 빌드로 다시 붙어봐야 같은 자리에 선다. 운영자 `/kick`이나 중복 로그인은 코드 없이 평범하게 닫으므로 새로고침이 걸리지 않는다
- 따라서 **생성기를 바꾸는 배포는 agent-client 사용자를 잠근다.** 프로토콜 범프와 같은 급의 변경으로 취급하고, 릴리스 바이너리를 함께 올린다

### 못

**`ClientInfo`와 `AuthError`의 모양은 영구 동결이다.** 이 둘은 "버전이 안 맞는다"를 전달하기 위한 유일한 통로라서, 여기가 깨지면 안내 자체가 불가능해진다. 정보를 더 실어야 하면 이 둘을 고치지 말고 새 메시지를 만든다. **close code 4001도 같이 동결이다** — 번호를 바꾸면 이미 배포된 클라이언트가 못 알아본다.

### 거절당한 클라이언트가 무한 재접속하지 않게 하는 것

거절은 WebSocket **핸드셰이크가 성공한 뒤** 애플리케이션 레이어에서 일어난다. 그래서 "연결에 성공하면 재시도 카운터를 리셋한다"는 흔한 구현은 이 경우 상한에 영원히 닿지 못한다 — 2026-07-23에 실제로 이 루프가 시간당 2,900건씩 돌았다. 규칙:

- 재시도 카운터는 **인증까지 통과한 연결**(`AuthSuccess`)에서만 리셋한다. 소켓이 열렸다는 것만으로는 리셋하지 않는다
- 백오프는 지수 + 지터 + 상한. 5,000명 규모에서 서버 재시작 후 동시에 몰려오는 것을 막는다
- close code 4001을 받으면 재접속을 **완전히 중단**하고 안내를 띄운다

### 확인해둔 사실 (근거)

현재 인코딩(`rmp_serde::to_vec`, [`messages.rs`](../shared/src/messages.rs))을 실제로 돌려본 결과:

| 변경 | 버전이 어긋난 클라이언트에서 |
|---|---|
| enum에 새 variant 추가 | variant는 **이름**으로 인코딩 → 순서·개수 무관, 받지만 않으면 무해 |
| 기존 variant에 필드 추가 | 필드는 **배열**로 인코딩 → `LengthMismatch` 디코드 실패 |
| 모르는 `ServerMessage` 수신 | 디코드 실패 → [`ws.rs`](../agent-client/src/ws.rs) `recv`가 에러를 올려 세션 종료 |

즉 버전이 어긋난 클라이언트는 조용히 잘 도는 게 아니라 **어딘가에서 반드시 깨진다.** 그렇다면 깨지는 지점을 접속 시점으로 앞당기고 이유를 알려주는 편이 낫다는 것이 위 결정의 근거다. 세션 도중 알 수 없는 메시지에 죽는 현재 동작도 그대로 둔다 — 버전이 같으면 일어날 수 없는 일이고, 일어난다면 그건 감춰야 할 게 아니라 드러나야 할 버그다.

### 대가

와이어를 건드릴 때마다 원격 에이전트가 **전부 멈춘다.** 받아들이는 대가이고, 줄이는 방법은 호환 코드가 아니라 운영으로 푼다.

- 진짜 와이어가 바뀔 때만 올린다 (내부 리팩터링으로는 올리지 않는다)
- 업데이트가 한 줄이면 되도록 배포물을 만든다 (Phase 3의 배포 패키지)
- 서버 재시작 로그와 안내 메시지에 새 버전 번호를 남겨 원인을 즉시 알 수 있게 한다

## merchant/guard 클래스 차단은 밸런스 결정

공식 NPC 플래그를 안 주면 클래스는 사실상 스탯 블록과 프롬프트 템플릿 선택일 뿐이다. 그런데도 막는 이유는 보안이 아니라 밸런스이고, **인간·에이전트 구분 없이 똑같이 적용된다**.

- **Merchant**: CHA +3 ([`character.rs`](../shared/src/character.rs)). CHA는 흥정 밴드 폭을 직접 넓힌다 — 진짜 Rica에게서 최대 할인을 상시로 받는 셈. ECONOMY.md가 "히든 클래스"라 부르는 이유다
- **Maid**: CHA +2 — 여관 접객 NPC용 마을 역할 클래스. 플레이어에게 줄 니치가 없어 함께 잠가 둔다.
- **Guard**: STR +2 / CON +2에 히트다이스 d10 — 전 클래스 최고 사양. 웹 클라이언트가 7종(knight/barbarian/caveman/valkyrie/ranger/rogue/priest)만 노출하는 것도 같은 이유다

**현재 구멍**: 서버 `CreateCharacter`는 클래스를 검증하지 않는다 ([`connection.rs`](../server/src/connection.rs)). 웹 UI가 버튼을 안 보여줄 뿐, 직접 만든 클라이언트는 지금도 merchant/guard 캐릭터를 만들 수 있다.

**수정**: `shared/src/character.rs`에 `is_player_selectable()`을 두고, `CreateCharacter`와 `RollCharacterStats`에서 `!state.is_official_npc && !class.is_player_selectable()`이면 거부한다. 한 곳을 막으면 웹·agent-client·자작 클라이언트가 동시에 막히고, 지금 열려 있는 구멍도 같이 닫힌다.

## 나중 과제: 사용자 에이전트에게 역할을 맡길 때

이번 범위는 아니지만, 조사 결과를 남겨둔다. "상인을 시켜도 안전한가"에 대한 답은 **금전적으로는 이미 안전하다**이다.

가정: 어떤 사용자에게 진짜 상인 권한(공식 NPC 플래그 + 레지스트리 상인 정의)을 줬고, 그 사람이 자기 부계정과 짜고 이득을 뽑으려 한다. [ECONOMY.md](ECONOMY.md)의 "LLM은 결국 뚫린다고 가정하고 방어는 전부 서버에서 한다" 원칙이 여기서 그대로 방어선이 된다.

| 방어 장치 | 값 | 코드 |
|---|---|---|
| 가격 밴드 (상인) | CHA 연동 ±5~25% p, 서버가 클램프 | [`deals.rs`](../server/src/game_state/deals.rs) `deal_half_band_pct` |
| 머니 펌프 불변식 | 최저 구매가 75% > 최고 판매가 (Rica: 40% × 125% = 50%) | [`deals.rs`](../server/src/game_state/deals.rs) `band_invariant_holds`, 상인 정의 로드 시 검사 |
| NPC 일일 할인 예산 | 게임일당 10,000 | `MERCHANT_DAILY_DISCOUNT_BUDGET` |
| 플레이어 일일 수혜 상한 | 게임일당 4,000 | `PLAYER_DAILY_DISCOUNT_CAP` |
| 흥정 쿨다운 / 딜 유효기간 | 30초 / 5분, 1회용 | `DEAL_COOLDOWN_MS`, `DEAL_TTL_MS` |
| 급여 파우셋 | 게임일 1회, `wallet_cap`까지만 (Karl: 5,000/일, 상한 30,000) | [`salary.rs`](../server/src/game_state/salary.rs) |

시나리오별로 보면:

1. **최대 할인으로 사서 되팔기** → 75%에 사서 50%에 판다. 손해다. 불변식이 구조적으로 보장한다
2. **부계정에게 폭탄 할인** → 준다 해도 게임일당 4,000(수혜자 기준)/10,000(상인 기준)에서 끊긴다. 사교적인 플레이어가 진짜 Rica에게서 얻어낼 수 있는 것과 같은 크기다
3. **주민 NPC 지갑 털기** → 지갑은 급여 파우셋이 상한이다. 누가 말을 잘 걸든 하루 유출량 총합은 변하지 않는다. **누가 가져가느냐**만 바뀐다

즉 **돈은 문제가 아니다.** 역할을 맡길 때 실제로 걸리는 것은 셋이다.

- **정체성**: 공식 NPC 계정을 넘기면 그 사람이 게임의 목소리로 아무 말이나 할 수 있다. 가짜 공지, 사기 유도, 다른 LLM NPC를 향한 프롬프트 인젝션. 되돌리기 어렵고 게임 신뢰를 직접 깎는다
- **충돌 검사 면제**: 벽·건물을 통과해 이동할 수 있다. 그 자체로 치트다
- **거래창 푸시**: 플레이어 화면에 UI를 띄울 수 있다 → 스팸/피싱 벡터

그리고 셋 다 "상인 역할"의 본질이 아니라 **공식 NPC 계정을 통째로 넘기는 방식**에서 온다. 그래서 열게 된다면 방향은 이쪽이다.

**플레이어 상점**: 공식 상인 정의를 넘기는 게 아니라, 자기 인벤토리를 파는 상점을 누구나 열 수 있게 한다. 재고가 플레이어 소유이므로 본질이 P2P 거래이고, 골드 중립이라 파우셋이 없다. 가격 밴드는 서버가 계산해 그대로 씌우면 된다. 상점을 여는 주체가 사람이든 에이전트든 상관없어지므로 **이 문서의 원칙과도 충돌하지 않는다** — 에이전트 전용 권한이 아니라 모든 플레이어의 기능이 된다. 별도 경제 설계가 필요하고, [ECONOMY.md](ECONOMY.md)의 밴드·불변식·예산 구조는 그대로 재사용된다.

## 구현 계획

### Phase 0 — 프로토콜 버전 handshake (원격 배포보다 먼저) — **완료 2026-07-22**
1. [x] `shared::PROTOCOL_VERSION` + `ClientMessage::ClientInfo { protocol_version, client_kind, client_version }`
2. [x] 서버: 인증 전 버전 검사(`handle_handshake`), 불일치·핸드셰이크 누락 시 안내 문구를 담은 `AuthError` 후 연결 종료
3. [x] agent-client: `ClientInfo` 전송, 거절(`AuthRejected`)은 재접속하지 말고 즉시 종료
4. [x] 웹 클라이언트도 `ClientInfo`를 보낸다 (캐시된 구버전 번들이 조용히 깨지는 것도 같이 막힌다). wasm이 `protocol_version()`을 노출해 버전 소스가 하나로 유지된다

### Phase 1 — 원격 접속 (서버 변경 없음) — **완료 2026-07-22**
5. [x] `tokio-tungstenite`에 `rustls-tls-webpki-roots` feature 추가 → `wss://` 지원
6. [x] 높이 타일 소스를 `HeightTiles` trait으로 추상화하고 HTTP 구현(`terrain_http.rs`) + 디스크 캐시 추가. `terrain` 설정이 경로면 로컬, `http(s)://`면 서버 API (`terrain_dir`은 별칭으로 유지)
7. [x] `[auth] mode` 파싱 골격 (기본 `npc_token` — 기존 동작 유지, `google`은 Phase 2 안내 후 종료)

### Phase 2 — 구글 로그인 — **완료 2026-07-22**
8. [x] 서버: `GoogleAuthVerifier`가 복수 audience 허용 + `--google-cli-client-id` / `GOOGLE_CLI_CLIENT_ID`
9. [x] agent-client: device flow(`google_auth.rs`), refresh token 캐시(Linux `~/.config/onlinerpg/google.json`, Windows `%APPDATA%\onlinerpg\google.json`), 접속마다 ID 토큰 재발급
10. [x] google 모드 제약 집행 (레지스트리 id 금지, 클래스 화이트리스트, character_name 필수, account 무시)

**운영 메모**: CLI용 OAuth 클라이언트는 "TV 및 입력 제한 기기" 타입이어야 하고, 구글이 토큰 교환 때 **client_secret을 요구한다** (없으면 `invalid_request: Missing required parameter: client_secret`). 기본 client_id는 `google_auth.rs`의 `DEFAULT_CLIENT_ID`에 박혀 있지만, **secret은 저장소에 두지 않는다** — 커밋하면 GitHub 푸시 보호가 막고, 시크릿 스캐너 경고가 계속 따라붙는다. 대신 배포물을 만들 때 `GOOGLE_CLI_CLIENT_SECRET` 환경변수에서 읽어 Linux tarball이나 Windows zip의 `config.toml`에 써 넣는다 (`tools/package-agent-client.sh`, `tools/package-agent-client.ps1`). 사용자는 여전히 아무것도 입력하지 않는다.

값 자체는 기밀이 아니다 (설치형 앱은 비밀을 지킬 수 없다, RFC 8252 §8.5). 이걸로 얻을 수 있는 건 우리 앱 이름으로 동의 화면을 띄우는 것과 쿼터 소모 정도이고, 토큰을 받으려면 사람이 동의를 눌러야 하며 게임 서버는 ID 토큰의 `aud`/`sub`만 본다. 그럼에도 저장소 밖에 두는 이유는 보안이 아니라 **도구 마찰**이다.

### Phase 3 — 서버 측 하드닝 — **완료 2026-07-22**
11. [x] `CharacterClass::is_player_selectable()` + `CreateCharacter`/`RollCharacterStats` 검증 (agent-client도 같은 함수로 조기 검사)
12. [x] `is_npc` → `is_official_npc` 개명 (Rust 호출부 + 웹 클라이언트 `isOfficialNpc`). 와이어 포맷은 위치 기반이라 프로토콜 버전은 그대로
13. [x] `/who`를 클라이언트 종류별 집계로 변경 (`ClientKind`, `Player`에 `#[serde(skip)]`으로 실려 브로드캐스트되지 않음)
14. [x] 배포물: `tools/package-agent-client.sh` / `.ps1` → 바이너리 + `data/` + 프로드용 config + [AGENT_CLIENT_QUICKSTART.md](AGENT_CLIENT_QUICKSTART.md)를 Linux tarball / Windows zip으로

**패키징 메모**: `package-agent-client.ps1`은 `powershell.exe`(5.1)가 아니라 **`pwsh`(7)로 실행한다**. 5.1의 `Compress-Archive`는 zip 항목 경로를 역슬래시로 쓴다 (`...\data\config.toml`). ZIP 스펙(APPNOTE 4.4.17.1)은 슬래시를 요구하고, macOS·Linux 추출기나 파이썬 `zipfile`은 이걸 디렉터리가 아니라 이름에 역슬래시가 든 납작한 파일로 푼다 — 그러면 클라이언트가 `data/config.toml`을 못 찾는다. Windows 탐색기만 눈감아 준다. pwsh 7은 슬래시로 쓴다.

```
pwsh -NoProfile -Command "cd <repo>; $env:GOOGLE_CLI_CLIENT_SECRET=...; .\tools\package-agent-client.ps1"
```

실행 정책도 둘이 저장소가 따로다. 한쪽에서 `Set-ExecutionPolicy`를 해도 다른 쪽은 `Restricted`로 읽고 스크립트 로드를 거부한다.

`PROTOCOL_VERSION`([`shared/src/lib.rs`](../shared/src/lib.rs))이 올라가면 기존 배포본은 `Protocol vN required`로 거절되므로, 서버 배포와 함께 새 릴리스를 올리고 인게임 공지에 재다운로드 안내를 넣는다. v0.35.0부터는 클라이언트가 시작할 때 GitHub 릴리스를 확인해 스스로 업데이트한다(`agent-client/src/update.rs`): 새 버전이 있으면 업데이트 여부를 물어보고, 승낙하면 설치 후 "다시 실행하세요"를 안내하고 종료한다 (터미널이 아니면 건너뛴다). 바이너리와 패키지 소유 파일(`data/system_prompt.txt`, `animation_durations.json`, `user_prompts/`, `README.md`)만 교체하고 `config.toml`·`user_prompt.txt`·메모리·캐시는 건드리지 않는다. `config.toml`의 `auto_update = false`로 끈다.

**릴리스 절차**: `agent-client/Cargo.toml`의 version을 올리고(자동 업데이트가 이 값과 최신 태그를 비교한다 — 안 올리면 구버전들이 새 릴리스를 영원히 다시 받는다), 두 패키징 스크립트가 만든 `agent-client-v<version>-<platform>` 자산을 태그 `agent-client-v<version>`으로 `gh release create` 한다. 자산 이름의 플랫폼 접미사(`windows-msvc.zip`, `glibc*.tar.gz`)는 업데이터가 매칭하는 규약이므로 바꾸지 않는다.

## 운영·보안 고려사항

- **개방 범위**: 이 기능이 켜지는 순간 "구글 계정만 있으면 누구나 상시 접속하는 에이전트를 돌릴 수 있다"가 된다. 원칙상 신청제·허용목록으로 좁히지 않는다 — 열 거면 다 연다
- **레이트 리밋**: 필요해지면 채팅 빈도, 캐릭터 생성/삭제, 거래 요청에 상한을 둔다. 단 **모든 클라이언트 공통**이다. 에이전트만 조이는 값은 만들지 않는다 (거래 딜에는 이미 30초 쿨다운이 있다)
- **새 공격면인가**: 대체로 아니다. 자작 클라이언트로 할 수 있는 일의 범위는 그대로다. 다만 **편리한 에이전트 프레임워크를 우리가 배포한다**는 사실은 남는다. 이동 검증·레이트 리밋을 강화하면 웹 클라이언트에도 똑같이 적용되고, 그게 의도한 바다
- **5,000 동접 목표** ([`CLAUDE.md`](../CLAUDE.md)): LLM 호출은 전부 실행자 머신에서 일어나므로 서버의 계정당 비용은 일반 플레이어와 같다. 다만 (a) 에이전트는 상시 이동해서 브로드캐스트가 꾸준히 발생하고, (b) 터레인 타일 HTTP fetch가 새로 생긴다. (b)는 디스크 캐시 + `Cache-Control`로 사실상 0에 수렴시킬 수 있다. 부하가 한계에 닿으면 접속 수단이 아니라 **동시 접속 총량**으로 조절한다

## 미해결 질문

- **폭주 대응 수단**: 지금은 킥만 있어([`Kicked`](../shared/src/messages.rs)) 재접속 루프를 도는 클라이언트에는 소용이 없다. 계정 단위 일시 차단이 필요하다 — 에이전트 전용이 아니라 공용 운영 수단으로
- **레이트 리밋 값**: 사람의 정상 플레이를 막지 않으면서 폭주를 잡는 지점이 어디인가. 실제 트래픽을 보고 정해야 한다
- ~~프로드 리버스 프록시가 `/api/terrain`을 외부에 노출하는지~~ — 확인 완료 (2026-07-22): `https://<host>/api/terrain/height/0/0`이 8450바이트를 정상 반환한다. WebSocket은 `/ws` 경로에서만 업그레이드되고 루트는 게임 페이지를 서빙하므로, 원격 config의 `server`에는 반드시 `/ws`가 붙어야 한다
