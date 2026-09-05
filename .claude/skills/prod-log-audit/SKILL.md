---
name: prod-log-audit
description: Daily prod health/log audit for OpenMMO — inspect journald + nginx since the last deploy, classify warnings/errors, check economy, bots, bandwidth, top players, compare with the previous audit note, and write a dated note to ~/work/notes. Use when the user says "로그 조사", "로그 점검", "수상한 점 없는지", "prod 점검", or asks for the daily audit.
---

# prod 로그 감사 (매일)

조사만 한다. 서버 재시작·설정 변경·커밋은 하지 않고, 조치는 후보로만 적는다.
결과 노트: `~/work/notes/openmmo-YYYY-MM-DD-log-audit.md` (플레이어명이 들어가므로 리포 밖).

## 0. 접속·시간 규칙

- `ssh prod` (VPN 필요). 안 되면 게임 다운으로 단정하지 말고 공개 엔드포인트 200 여부와 VPN을 먼저 본다.
- `journalctl --since/--until`은 **KST**, 로그 줄 안 시각은 **UTC**. 창을 자르면 `head -1`/`tail -1`로 경계 확인.
- 로그에 ANSI 색 코드가 있다. 모든 grep 전에 `sed -E "s/\x1b\[[0-9;]*m//g"` 를 거친다. 안 거치면 `WARN`/`ERROR` 매칭이 0으로 나온다.
- 큰 창은 한 번 `/tmp/w.log`로 떨어뜨려 재사용한다.
- 유닛: `openmmo-server`, `openmmo-agent-client`. 프로토콜 버전 문자열 `protocol vN (server speaks vM)`.

## 1. 창 정하기

```
systemctl show openmmo-server -p ActiveEnterTimestamp --value
journalctl -u openmmo-server --since "3 days ago" -o short-iso | grep -E "Started|Stopped|listening"
```
- 창 = 직전 배포(재시작) → 지금. 창 안에 재시작이 또 있으면 사용자가 모를 수 있으니 **첫 줄에 알린다**.
- 비교 기준 = 직전 노트(`ls ~/work/notes/openmmo-*-log-audit.md | tail -2`).

## 2. 전체 상태

- 레벨별 줄 수(서버·agent-client), 패닉/OOM(`panic|panicked|out of memory` — 플레이어명 오탐 주의), 디스크·메모리·load.
- 일별 입장 수: `Player X (id) joined the game` 를 날짜별 count. (`Session started`라는 문구는 없다 — `Session ended`만 있다.)
- 배포 직후 건강: NPC 4명이 `Auth failed: Protocol` 1회 뒤 `Authenticated` 됐는지, `Refusing client` kind별 잔여.

## 3. WARN/ERROR 분류

숫자·해시를 `N`/`H`로 치환해 `sort | uniq -c | sort -rn`. 상위 항목마다 "누가·언제·최대 빈도"를 본다:
- `Rejected player attack … (InvalidTarget|OutOfRange)`: 공격자 id별 count → id는 `Player NAME (id) joined`로 이름 매핑. **분당 고정 빈도로 수 시간 평탄**하면 클라 재시도 루프(과거 사례). 분산돼 있고 최대 10/s 이하이면 정상. 총량은 킬 수에 비례(막타 후 인플라이트 스윙 경합, 대략 3~7킬당 1건)이므로 ` killed ` 총량과 비율로 판단. reason 뒤 상세(corpse/absent from registry/alive but unreachable+층·좌표, OutOfRange는 거리)로 게이트 구분 가능 — 같은 (공격자, 몬스터) 페어가 쿨다운 간격으로 장시간 반복되면 붙박이 루프 버그(9/1 별이→m97_10 사례).
- `Refusing client: protocol vN … ip= kind= version=`: ip/kind/version별. `[+N more in the last 60s]` 억제분이 있으니 "로그 줄 수"로 표기. 같은 /24에 IP가 3개 이상이면 프록시 풀.
- `Blocked move … by r-X_+Y_N`: 집 콜라이더에 끼임. 같은 집에 여러 명이면 배치 문제.
- `storey change|floor change … off the stairs`: 계단 밖 층 변경 거부. 시간당 비율을 직전 노트와 비교.
- `Waypoint queue full`: 분당 피크 → 직후 사망이면 스팸 클릭.
- `Google token … ExpiredSignature`: 수십 건은 정상.
- `Dropping connection: unauthenticated`, `handshake failed`: 스캐너. 수만 보고 넘어간다.
- `bad house file … unknown variant`: 서버가 모르는 하우징 스키마 — 클라/서버 배포 순서 문제. 파일 mtime과 배포 시각을 대조.
- agent-client: `TradeFailed`, `Position lost`, `Failed to parse agent response` 정도는 평상시 잡음. 총 줄/초도 적는다.

## 4. nginx (`/var/log/nginx/access.log*`, 로테이션 일별)

- 상태 코드 분포, 4xx 상위 경로. `/api/terrain/water-field|height-original` 404는 **정상**(없는 타일 = 물 없음, 클라가 null 캐시).
- 400/166 + 바이너리 요청줄 = 인터넷 스캔.
- 500은 전부 본다(경로·referer·UA).
- 리소스 404(예: `/portraits/*.png`)는 구 번들이 새 파일명을 요청하는 배포 간 불일치일 수 있다.
- 대역폭: `$10` 바이트를 날짜(`split($4,d,"[/:]"); d[1]`)·카테고리(`textures|models|bgm|assets|api/terrain|ws`)별로 합산, 웹 고유 IP로 인원 보정. 200 vs 304 비율, (IP,파일) 쌍 대비 200 수로 재다운로드 배율, 상위 IP. 헤더는 `curl -sI`로 실측(`cache-control`, `etag`, `last-modified`). prod nginx 설정은 `/etc/nginx/sites-available/openmmo`(수동 관리, 리포 미추적).

## 5. 경제

`gold_flow.py`(이 디렉터리)를 prod에 scp 해서 창을 파이프한다:
```
scp .claude/skills/prod-log-audit/gold_flow.py prod:/tmp/ && \
ssh prod 'journalctl -u openmmo-server --since "<KST>" -o cat | python3 /tmp/gold_flow.py'
```
- 상인(Rica·Wick)은 무한 지갑: 판매=생성, 구매=소멸. Karl·Signe는 이전. 1g = 10,000c.
- **되사기(`bought back`)는 스크립트가 양쪽에서 제외**한다(골드 소모 아님).
- 급여는 지갑 cap이면 `no payment`만 찍힌다.
- 대형 P2P 이전(`Trade:` 한쪽 ≥ 20g)은 계정·IP를 묶어 본다. 같은 IP·여러 계정·직후 캐릭터 삭제면 리롤용 자산 이전(위반 아님, 기록만).
- 직전 노트의 순증감과 비교해 방향(인플레/디플레)을 적는다.

## 5-1. 인챈트

로그 문구(모두 `game_state::inventory` / `combat`):
- 드롭: `Bonus drops ["scroll_of_enchant_weapon", …] at (x,z) PLACE` — 플레이어명 없음, 한 줄에 여러 아이템 가능하니 **장 수는 id 등장 횟수**로 센다.
- 처치: `Player NAME killed TYPE (lvl N) at (x,z) PLACE; weapon drop: …` — `lvl`은 유효레벨. `world_drop.csv`는 ≤8 0.5%, >8 1%(종류당)이므로 기대 드롭 = 0.005×(≤8 킬) + 0.01×(>8 킬), 종류당 하나씩.
- 시도: `NAME consumed scroll_of_enchant_(weapon|armor) at PLACE`. 결과: `NAME enchanted ITEM to +N` / `NAME destroyed ITEM enchanting at +N`(+N은 시도 전 값).

적을 것: 처치 수(장소별), 드롭 장 수(종류·장소별)와 킬당 실측률 대 설정 기대값, 시도 수와 성공/파괴 비율, 개인별 시도 상위, 파괴 목록(누가 무엇을 +몇에서), 창 안 최고 도달값, 상인 판매·P2P 건수.

DB(`character_items(item_def_id, enchant, equip_slot, quantity)` + `characters`):
- 최고 인챈트 무기·방어구(장착/가방 구분, 방어구 id는 `data-src/items.csv`의 `armor` 카테고리로 거른다), +5·+7 이상 개수.
- 주문서 재고 합계와 보유 상위 — 드롭/일 대 시도/일과 비교해 축적률을 적는다.
- 같은 캐릭이 +9 무기를 여러 자루 갖는 식의 이상치는 이전 창 시도 로그와 대조해 출처를 적는다.

## 6. DB (`/home/ubuntu/work/OnlineRPG/data/game_data.db`, SQLite)

- 같은 디렉터리에 `game_data-backup-*.db`가 있다. `find … -name "*.db" | head -1`로 잡지 말고 경로를 고정한다.
- `characters(character_name, account_name, level, xp, gold, class, created_at, last_seen_at, admin_role)`. `last_seen_at`은 2026-08-29부터 채워지므로 그 이전 창의 액티브는 저널 `joined the game` 이름으로 잡는다(저널 보존 약 20일 — `journalctl -o short-iso | head -1`로 시작일 확인).
- 상위 레벨 20명, 레벨 분포, 액티브 캐릭/계정 수, 골드 합·평균·중앙값·구간 분포, 전체 골드 대비 액티브 비중. 레벨 대비 골드 과다인 캐릭을 표시.
- **계정 수 (매번 §1 표에 기록)**: `accounts(player_name, created_at, google_sub)`에는 last_seen이 없으므로 액티브는 `characters.last_seen_at`으로 계정을 묶는다.
  ```
  select (select count(*) from accounts) total,
         (select count(*) from accounts where created_at >= strftime('%s','<창 시작 KST→UTC 아님, 그대로>')) new_in_window,
         (select count(*) from accounts where created_at >= strftime('%s','now')-7*86400) new_7d,
         (select count(distinct account_name) from characters where last_seen_at >= strftime('%s','<창 시작>')) active_in_window,
         (select count(*) from accounts where player_name not in (select account_name from characters)) no_character;
  ```
  직전 노트의 총계와 차이 = 창 안 가입 수인지 대조한다(삭제된 계정이 있으면 어긋난다). `strftime('%s', ...)`의 인자는 UTC로 해석되므로 창 시작 시각을 **UTC 문자열**로 넣는다.

## 7. 봇·멀티 계정

- 의심 IP가 있으면 그 IP로 `Session ended for NAME (IP)`를 전 기간 모아 이름 목록 → 각 이름의 다른 IP → DB에서 계정 묶기 → 개명 로그(`renamed to`)·삭제 로그(`Character id=N deleted`)로 이름 변천 추적.
- 서버 몬스터 AI 가동 확인: `monster ai: brains N active N … over_budget 0`. 봇이 맞고 있는지는 `consumed healing_potion` 수로 본다.
- 금지 이름은 `data/banned_names.txt`(재시작 때 로드, 정확 일치). 추가는 사용자 결정 — 제안만.

## 8. 직전 노트 점검표 대조 — 반드시

직전 노트의 `## 요약 — 조치 후보`/`## 후속 후보`/"내일 재측정" 항목을 **하나씩** 이번 창에서 재측정해 표로 남긴다(항목 / 이번 결과 / 판정: 해소·개선·미해결·악화). 빠뜨리기 쉬우니 노트 작성 전 마지막에 한다.

## 8-1. 영웅담 원장 (doc/HEROIC_TALES.md)

바드 Signe가 저녁 여관 공연에서 부를 사실 원장. `tales.py`가 창 안 저널에서 **후보**만 뽑는다:
```
scp .claude/skills/prod-log-audit/tales.py prod:/tmp/ && \
ssh prod 'journalctl -u openmmo-server --since "<KST>" -o cat | python3 /tmp/tales.py YYYY-MM-DD'
```
- 후보를 읽고 노래감인 것만 사용자에게 보인 뒤 prod의 `~/work/OnlineRPG/agent-client/data/tales/ledger.txt`에 **append**한다(리포 밖, gitignore). 형식은 `DATE KIND NAME args key=value`. 스크립트는 절대 직접 쓰지 않는다.
- `solo=`/`first=`는 원장에 같은 보스가 이미 있는지, `record=`는 DB 최고 인챈트, `level_record`는 DB 최고 레벨과 대조해 채운다.
- `most_xp`: DB `SELECT character_name, level, xp FROM characters WHERE level >= 5`를 `~/work/notes/openmmo-YYYY-MM-DD-xp.tsv`로 남기고 직전 tsv와 xp 차이 1위를 적는다. 같은 사람이 이어지면 새 줄 대신 `streak=N` 줄 하나.
- 같은 사람의 같은 종류 사건은 첫 번과 스트릭만. 개명(`renamed to`)·삭제(`Character id=N deleted`)가 보이면 원장의 그 이름을 고치거나 줄을 지운다 — 원장을 고쳐 쓰는 유일한 경우.
- 봇도 동일하게 오른다. `npc_` 계정만 제외. 금액·IP·계정명은 원장에 넣지 않는다.

## 9. 노트 구성

1. 창·재시작 이력 → 2. 전체 상태(계정 총계·신규·액티브 포함) → 3. 플레이어에게 보였던 문제 → 4. 수상하지만 무해 → 5. 사소 → 6. 대역폭 → 7. 경제 → 8. 인챈트 → 8-1. 영웅담 후보(원장에 붙인 줄) → 9. 상위/액티브 플레이어 → 10. 봇 → 11. 직전 점검표 대조 → 12. 후속 후보 → 13. 조회 메모(이번에 새로 알게 된 경로·문구·함정).

사용자에게는 표 위주로 짧게 보고한다. 숫자는 사용자가 툴 출력을 못 보므로 본문에 직접 적는다. 추정은 추정이라고 쓰고, 나중에 틀린 게 드러나면 정정한다.
