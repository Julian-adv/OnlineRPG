# agent-client 실행 가이드

LLM이 당신의 캐릭터를 조종해 게임에 참여한다. 구글 계정으로 로그인하고, LLM은
당신 머신에서 당신 계정으로 돈다 — 게임 서버는 LLM 비용을 부담하지 않는다.

에이전트는 **평범한 플레이어**다. 서버는 사람이 조종하든 LLM이 조종하든 똑같이
대한다. 특별한 권한도, 특별한 제약도 없다.

## 0. 받기

Linux tarball은 **Ubuntu 24.04 이상**(glibc 2.39+), Windows zip은 **64비트
Windows 10/11**용이다. 압축을 풀고 그 디렉터리에서 실행한다.

Linux:

```bash
tar -xzf agent-client-*.tar.gz
cd agent-client-*/
```

Windows (PowerShell):

```powershell
Expand-Archive .\agent-client-*-x86_64-windows-msvc.zip
cd .\agent-client-*-x86_64-windows-msvc
```

지원하지 않는 배포판이나 아키텍처에서는 소스에서 빌드한다. Rust 툴체인 말고는
준비물이 없고, 게임 데이터는 빌드가 알아서 생성한다.

```bash
git clone https://github.com/Julian-adv/OpenMMO.git
cd OpenMMO
cargo build --release -p agent-client
```

빌드한 바이너리는 Linux에서 `target/release/agent-client`, Windows에서
`target\release\agent-client.exe`다. `agent-client/` 디렉터리에서 실행하고,
`data/config.toml`은 `data/config.toml.example`을 참고해 만든다.

이때 **구글 로그인용 `client_secret`이 저장소에는 없다.** 위에서 푼 배포본의
`data/config.toml`에서 값을 복사한다 — Linux는 `grep client_secret data/config.toml`,
Windows는 `Select-String client_secret data\config.toml`. 값은 반드시 `GOCSPX-`로
시작한다. 아니면 배포본이 잘못된 것이고 로그인이 `invalid_client`로 실패한다.

기밀이 아니다 — 설치형 앱은 비밀을 지킬 수 없고(RFC 8252 §8.5) 모든 배포본이
같은 값을 쓴다. 저장소에 두지 않는 이유는 보안이 아니라 시크릿 스캐너 마찰이다.

## 1. LLM 준비

기본 설정은 [Codex CLI](https://github.com/openai/codex)를 쓴다. 터미널에서
`codex` 명령이 정상 동작하는 상태여야 한다 (로그인·요금·레이트리밋 모두 당신
계정 기준). 다른 백엔드를 쓰려면 `data/config.toml`의 `llm`을 바꾼다:

| 값 | 필요한 것 |
|---|---|
| `codex` | `codex` CLI (기본값) |
| `claude` | `claude` CLI |
| `openrouter` | `OPENROUTER_API_KEY` 환경변수 |
| `openai` | `[openai]`에 `base_url`·`model` 설정 (LiteLLM 프록시, vLLM 등 OpenAI 호환 엔드포인트 아무거나). 키는 `api_key` 또는 `OPENAI_COMPAT_API_KEY` 환경변수 |

## 2. 캐릭터 설정

`data/config.toml`에서 이름과 클래스를 정한다.

```toml
[[npcs]]
character_name = "당신의 캐릭터 이름"   # 서버 전체에서 유일해야 한다
character_class = "rogue"
gender = "female"
llm = "codex"
```

고를 수 있는 클래스: `knight` `barbarian` `caveman` `valkyrie` `ranger`
`rogue` `priest`. (`merchant`, `guard`는 운영자 NPC 전용이라 거절된다.)
성별은 `male` 또는 `female`이다. 기존 캐릭터와 다른 성별을 지정하면 캐릭터를
삭제하고 다시 만들므로 레벨과 아이템이 초기화된다. 클래스마다 있는 모델이 다르다
— `ranger`는 남성만, `valkyrie`는 여성만 있고 나머지(`knight`, `barbarian`,
`caveman`, `rogue`, `priest`)는 둘 다 있다. 없는 조합을 고르면 클라이언트가
캐릭터를 그리지 못한다.

### 성격과 지식 (`data/user_prompt.txt`)

에이전트가 어떻게 굴지는 `data/user_prompt.txt`가 정한다. 기본값은
`data/user_prompts/newcomer.txt`의 사본으로, 세계에 대해 아무것도 모른 채
직접 부딪혀 배우는 캐릭터다. 이미 요령을 아는 캐릭터로 시작하려면 덮어쓴다:

```bash
cp data/user_prompts/veteran.txt data/user_prompt.txt
```

`veteran.txt`에는 몬스터 티어와 XP 공식, 마을이 안전지대라는 것, 스탯이
무슨 일을 하는지가 들어 있다. 둘 다 그냥 텍스트라 직접 고쳐 써도 된다 —
성격, 목표, 말투 무엇이든.

캐릭터를 처음 만들 때 스탯을 굴리는데, 이때 에이전트가 굴림 결과를 보고
다시 굴릴지 받아들일지 스스로 정한다. 얼마나 까다롭게 굴지도 이 파일에
적혀 있다. `veteran.txt`는 주스탯 16 이상을 노리고, `newcomer.txt`는
직감대로 고른다.

## 3. 실행

Linux:

```bash
./agent-client
```

Windows (PowerShell):

```powershell
.\agent-client.exe
```

처음 실행하면 구글 로그인 안내가 뜬다:

```
  Sign in to continue:
    1. open https://www.google.com/device
    2. enter code XXX-XXX-XXX
```

브라우저(어느 기기든)에서 코드를 입력하면 접속이 이어진다. 자격증명은 Linux에서
`~/.config/onlinerpg/google.json`, Windows에서
`%APPDATA%\onlinerpg\google.json`에 저장되므로 다음부터는 묻지 않는다.

로그아웃하려면 그 파일을 지운다.

## 관전 창

에이전트는 화면 없이 돌아가므로, 무엇을 보고 어떻게 판단하는지 궁금하면 실행 중인
상태에서 브라우저로 **http://127.0.0.1:8808** 을 연다. 실시간 지도(에이전트·주변
플레이어·몬스터), HP·소지금·게임 시각, 그리고 채팅/전투/거래와 **매 LLM 턴**의
프롬프트와 응답이 흐르는 피드가 보인다.

이 PC에서만 열리고 보기만 할 뿐 조종은 못 한다. 포트를 바꾸거나 끄려면 `config.toml`의
`watch_port`를 고친다 (`0`이면 꺼짐). 8808이 이미 쓰이는 중이면 실행 로그에
`Watch panel failed to bind` 경고가 뜨는데, 에이전트 자체는 정상 동작한다.

## 문제가 생기면

| 증상 | 원인과 대처 |
|---|---|
| `Connection failed: HTTP error: 200 OK` | `server` 주소에 `/ws` 경로가 빠졌다. 게임 페이지(HTML)를 받아온 것이다 — `wss://<호스트>/ws` 로 고친다 |
| `Protocol vN required, you sent vM` | 서버가 업데이트됐다. 새 배포물을 받는다 |
| `Auth failed: ...` 후 종료 | 로그인 거절. 위 자격증명 파일을 지우고 다시 로그인 |
| 캐릭터 생성 실패 (이름 중복) | `character_name`은 서버 전체에서 유일해야 한다 |
| `The merchant class is not available` | 플레이어가 고를 수 없는 클래스다 |
| 지형 위를 걷지 못하거나 높이가 이상함 | 서버의 터레인 API에 접근이 안 되는 상태. `terrain` 값을 확인한다 |

로그를 자세히 보려면 Linux에서는 `RUST_LOG=debug ./agent-client`, PowerShell에서는
`$env:RUST_LOG = "debug"; .\agent-client.exe`로 실행한다.

## 동작 방식

- 게임 서버와는 웹 클라이언트와 **완전히 같은 WebSocket 프로토콜**로 통신한다
- 지형 높이는 필요한 타일만 서버에서 받아 `data/cache/height/`에 캐시한다
- LLM에는 주변 상황을 텍스트로 요약해 넘기고, 응답(이동·대화·전투)을 게임
  명령으로 바꿔 보낸다. 경로탐색·충돌·전투 판정은 전부 클라이언트/서버가 한다

설계 배경은 [REMOTE_AGENT_CLIENT.md](REMOTE_AGENT_CLIENT.md) 참고.
