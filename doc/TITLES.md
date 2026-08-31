# Titles — 칭호

보스를 잡은 사람에게 **서버만 줄 수 있는 표식**을 남긴다. [DUNGEON_REWARD.md](DUNGEON_REWARD.md)
"보스의 역할"에서 명예 표식으로 미뤄둔 2단계의 구현 설계다. 보상 상자는 열쇠로 개인화됐으니,
보스를 잡을 이유는 이 칭호(와 뒤에 올 경험치·환금 보너스)가 진다.

## 왜 칭호인가

- 상자는 이제 보스 생사와 무관하다. 보스를 그냥 지나쳐도 손해가 없으므로, 잡는 쪽에 눈에
  보이는 무언가가 있어야 한다.
- 아이템은 팔리고 거래되면 증명이 아니다. 칭호는 서버가 부여하고 서버가 보여주므로 위조도
  양도도 안 된다.
- 경제에 닿지 않는다. 골드·아이템 발행량([ECONOMY.md](ECONOMY.md))을 건드리지 않고 동기를 준다.

## 자격 — 보스에게 준 피해 비율

지금의 처치 크레딧은 **막타 + 파티 공유**뿐이다(`combat.rs`의 `party_members_sharing_kill`).
막타는 보스방에 늦게 들어와 한 대 치는 사람에게도 가고, 이슈 #151의 "한 대만 때리고"가
그대로 재현된다. 그래서 피해 로그를 둔다.

- **보스만** 기록한다(`monsters.csv` `boss=true`). 보스는 던전당 하나, 세 마리뿐이라
  5,000명 규모에서도 맵 하나 크기다. 일반 몬스터에는 두지 않는다 — 킬당 기록·정리 비용이
  전부 낭비다.
- 보스 인스턴스마다 `HashMap<character_id, damage>`. 캐릭터 id로 키를 잡아 죽어서 돌아오거나
  재접속해도 누적이 이어진다. 보스가 죽거나 리셋되면 버린다.
- 보스가 죽는 순간 피해 비율로 두 단을 판정한다. 막타는 상관없다.
  - **총 피해의 `TITLE_DAMAGE_SHARE`(시작값 50%) 이상** → "쓰러뜨린 자". 산술적으로 최대
    두 명이고, 보통은 사실상 혼자 잡은 한 명이다. 파티가 고르게 나눠 잡으면 아무도 못
    받는다 — 칭호는 파티 참가 기념이 아니라 "내가 잡았다"의 증명이므로 의도다.
  - **총 피해의 `TITLE_SOLO_SHARE`(시작값 90%) 이상** → "홀로 쓰러뜨린 자". 100%로 두면
    지나가던 사람의 한 대가 홀로를 망친다. 옆에서 구경만 한 파티원은 방해가 안 된다 —
    "혼자 맞았다"가 아니라 "사실상 혼자 때렸다"다.
  - 홀로 자격은 쓰러뜨린 자 자격을 포함한다. 둘 다 받는다.
- 파티 XP 공유 규칙(피해 0이어도 근처면 XP)은 **적용하지 않는다.** 칭호는 "내가 잡았다"의
  증명이라 곁에 있었다는 것으로는 부족하다. 파티로 잡은 보상은 뒤에 올 경험치·환금
  보너스(피해 맵을 같이 쓴다)가 맡는다.
- 이 피해 맵은 뒤에 올 경험치·환금 보너스도 같이 쓴다. 기여도 계산기를 둘 만들지 않는다.
- 봇도 같은 규칙으로 받는다. 예외는 운영 NPC 계정(`npc_` 접두사, `is_official_npc`)뿐이다 —
  이들은 플레이어가 아니라 배경이다.

메모리에만 둔다. 보스 상태 자체가 메모리 전용이라(재시작 = 리셋, [ITEM_TIERS.md](ITEM_TIERS.md))
피해 로그만 살아남아도 의미가 없다.

## 정의 — `data-src/titles.csv`

```
id,name,nameKo,source,bossId,itemId,solo,order,supersedes
goblin_slayer,Slayer of the Goblin Chief,고블린 족장을 쓰러뜨린 자,boss_kill,goblin_boss,,false,10,
goblin_slayer_solo,Who Slew the Goblin Chief Alone,홀로 고블린 족장을 쓰러뜨린 자,boss_kill,goblin_boss,,true,11,goblin_slayer
orc_slayer,Slayer of the Orc Warlord,오크 군주를 쓰러뜨린 자,boss_kill,orc_boss,,false,20,
orc_slayer_solo,Who Slew the Orc Warlord Alone,홀로 오크 군주를 쓰러뜨린 자,boss_kill,orc_boss,,true,21,orc_slayer
ogre_slayer,Slayer of the Ogre Warlord,오거 군주를 쓰러뜨린 자,boss_kill,ogre_boss,,false,30,
ogre_slayer_solo,Who Slew the Ogre Warlord Alone,홀로 오거 군주를 쓰러뜨린 자,boss_kill,ogre_boss,,true,31,ogre_slayer
sturgeon_angler,Who Landed the Golden Sturgeon,황금 철갑상어를 낚은 자,fishing,,golden_sturgeon,false,40,
```

칭호는 등급 이름이 아니라 **한 줄짜리 이야기**다("홀로 오거 군주를 쓰러뜨린 자"). 몬스터
이름을 그대로 써서 어느 보스인지 바로 읽히고, "홀로"가 위 90% 조건에 대응한다.

- `source`는 부여 경로. `boss_kill`은 `bossId`·`solo`가, `fishing`(2026-08-31)은
  `itemId` — 그 물고기를 낚아 올리면 부여 — 가 그 조건이다. 경로가 더 생기면 열이 늘지
  행이 바뀌지 않는다.
- `supersedes`는 이 칭호가 대신하는 칭호. 그 칭호를 보이고 있던 사람이 이걸 얻으면 자동으로
  바뀐다. 코드가 "같은 보스의 홀로"를 추론하지 않고 데이터가 말한다.
- `order`는 목록 정렬용. 가치의 서열은 아니다.
- 다른 데이터와 같이 `node tools/convert.mjs`로 `data/titles.json`을 만들고 서버·클라이언트가
  같은 파일을 읽는다. 필드에 쉼표 금지.
- 서버는 시작할 때 `bossId`를 `MonsterDefs`와, `itemId`를 `ItemDefs`와 대조해 없으면
  패닉한다(`dungeon_defs.rs`가 `chestDrops`를 검증하는 것과 같은 자세).

첫 세트는 보스당 둘(쓰러뜨린 자·홀로)로 시작한다. **누적 처치**(10회·100회)와 **서버 최초 처치**는 후보로만
둔다. 서버 최초는 "이미 누가 잡았는가"를 재시작 너머로 기억해야 하므로 영속 마커가 하나 더
필요하다 — 하고 싶어지면 그때 `titles` 테이블에 `first_holder` 같은 것을 붙인다.

## 저장

- `character_titles(character_id, title_id, earned_at)` — `character_dungeon_chests`와 같은
  꼴의 부속 테이블, PK `(character_id, title_id)`, 캐릭터 삭제 시 CASCADE.
- `characters.active_title TEXT NULL` — 기존 `ALTER TABLE characters ADD COLUMN` 경로로
  추가. NULL이면 칭호 없이 보인다.
- 부여는 보스 사망 처리 안에서 **동기적으로 DB에 쓴다.** 상자 클레임과 같은 이유다 —
  잃어버린 칭호는 되돌릴 수 없고, 그 순간 접속이 끊긴 캐릭터도 받아야 한다.
- 이미 가진 칭호는 무시한다(`INSERT OR IGNORE`). 두 번째 처치는 로그만 남는다.

## 와이어

- `Player.title: Option<String>` — **구조체 끝에** `#[serde(default)]`로 붙인다.
  `entity.rs`의 경고대로 msgpack이 위치 배열이라 중간에 끼우면 뒤 필드가 밀린다.
- `ServerMessage::PlayerTitleChanged { player_id, title: Option<String> }` —
  `PlayerBackChanged`와 같은 AOI 브로드캐스트.
- `ServerMessage::TitleEarned { title }` — 본인에게만. 클라이언트는 이걸로 토스트를 띄운다.
- `ClientMessage::SetActiveTitle { title: Option<String> }` — 서버는 `character_titles`에
  있는지 검사하고 없으면 무시한다.
- `Character.titles: Vec<String>`(캐릭터 목록)와 `VisibleEquipment` 옆에 `active_title` —
  캐릭터 선택 화면에서도 같은 이름으로 보이게.
- 필드가 전부 끝 추가 + `default`라 옛 클라이언트도 읽는다. 다만 `SetActiveTitle`은
  새 메시지이므로 **PROTOCOL_VERSION을 올린다.** 서버·웹 클라이언트·agent-client가 같이
  나간다(열쇠 배포와 묶으면 된다).

## 표시

칭호는 한 문장이라 이름 옆에 붙이면 길어지므로, 어디서나 **이름 위 한 줄**로 둔다.
줄이지 않는다.

| 자리 | 표시 | 비고 |
|---|---|---|
| 머리 위 이름표(`PlayerModel`의 `TextLabel`) | `이름` 위에 작은 글씨로 전체 칭호 | 칭호 없는 사람은 지금과 동일 |
| 채팅(`ChatPanel`) | 이름만 | 칭호는 붙이지 않는다(2026-08-30) |
| 캐릭터 선택 | 이름 아래 한 줄 | `active_title` |
| 캐릭터 창(본인) | `titles` 탭 — 칭호 목록 + 라디오로 선택, "없음" 포함 | `SetActiveTitle` |

- 언어는 설정 창의 "Title Language"(Auto/한국어/English, 브라우저별 저장)로 고른다. Auto는
  브라우저 로케일(`navigator.language`가 `ko`면 한국어). 서버는 id만 다루므로 남의 칭호도 내
  설정대로 보인다. `/title` 응답과 봇 프롬프트는 영어 `name`이다.
- 색·아이콘 같은 등급 표현은 두지 않는다. 문장 자체가 서열을 말한다("홀로"가 붙었는가).
  등급을 넣기 시작하면 그 자체가 또 하나의 수집 시스템이 된다.

## 고르기

칭호는 쌓이지만 **보이는 건 하나**다. 어느 것을 보일지는 본인이 고른다.

- 캐릭터 창에 "칭호" 칸. 가진 칭호 전부를 `order` 순으로 나열하고 하나를 고르거나 "없음"을
  고른다. 고르면 `SetActiveTitle`로 서버에 보내고, 서버가 `character_titles`에 있는지 확인한
  뒤 `characters.active_title`에 쓰고 `PlayerTitleChanged`를 주변에 뿌린다.
- 채팅 명령 `/title`도 둔다 — 인자 없이 치면 가진 칭호를 번호와 함께 보여주고, `/title 2`로
  고르고, `/title off`로 뗀다. 봇(agent-client)은 이 경로를 쓴다.
- 자동 선택은 두 경우뿐이다: 칭호가 하나도 없다가 처음 얻었을 때, 그리고 활성 칭호를
  `supersedes`하는 칭호를 새로 얻었을 때. 그 외에는 새 칭호를 얻어도 활성은 바꾸지 않는다 —
  본인이 고른 것을 서버가 멋대로 갈아치우지 않는다.
- 활성 칭호는 캐릭터에 붙어 로그아웃해도 남는다.

## agent-client

- 상태 줄에 자기 칭호 목록과 활성 칭호를 적는다. 고르기는 `/title` 채팅 명령으로 한다.
- 남의 칭호는 주변 플레이어 목록에 그대로 실어 LLM이 "오거 사냥꾼이 옆에 있다" 정도는
  알게 한다. 그 이상의 로직은 두지 않는다.

## 서버 변경 범위

- `data-src/titles.csv` + `title_defs.rs`(로드·검증).
- `game_state`: 보스 피해 맵(`boss_damage: HashMap<monster_id, HashMap<character_id, u32>>`),
  `apply_damage` 경로에서 보스면 누적, 보스 사망 시 문턱 판정 → `character_titles` INSERT →
  `TitleEarned` + `PlayerTitleChanged`(활성 칭호가 없던 캐릭터는 첫 칭호를 자동 활성;
  홀로 칭호를 새로 얻으면 같은 보스의 쓰러뜨린 자가 활성이던 경우 홀로로 올려준다).
  리셋(`reset_dungeons`)·디스폰에서 맵 정리.
- `auth.rs`: 테이블·컬럼, `load/grant/set_active`.
- `Player`에 `title` 실어 보내는 곳(`PlayerJoined`/`PlayerAppeared`) 채우기.
- 클라이언트: 위 표시 네 곳 + 토스트.

## 구현 메모 (2026-08-30)

- 서버: `title_defs.rs`(정의·부팅 검증), `game_state/titles.rs`(피해 로그·부여·활성 선택·`/title`),
  `auth.rs`(`character_titles`, `characters.active_title`). 보스 판정은 `dungeon_monsters`
  인덱스의 `is_boss`. 피해는 실제로 깎인 HP만 센다(오버킬 제외).
- 봇(agent-client)은 상태 줄에 자기 칭호 목록을, 주변 플레이어 줄에 상대 칭호를 싣는다.
- 오프라인 캐릭터도 보스 사망 시점의 피해 로그에 있으면 DB에 부여된다. 다음 접속 때 목록에
  보이지만 자동으로 활성되지는 않는다 — DB의 NULL은 "처음"과 "/title off"를 구분하지 못한다.
  본인이 고르면 된다.

## 열어둔 것

- 문턱 50%·90%는 시작값이다. 파티 플레이가 주류가 되어 칭호가 아무에게도 안 나가면 그때
  낮춘다 — 실제 파티 크기 분포를 본 뒤에.
- 보스 사망 직전 접속을 끊은 캐릭터는 피해 맵에 남아 있으므로 받는다. 의도다.
- 칭호 회수는 없다. 운영상 필요하면 DB에서 지운다.
