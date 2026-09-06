# 대상 캐릭터 전투 감사

의심 대상을 캐릭터 ID로 지정해 전투·회복을 관측한다. 게임 판정이나 보상은 바꾸지 않는다. 추적은 **수동 해제할 때까지** 유지하며, 서버 재시작 때 같은 설정 파일을 다시 읽는다.

## 설정

서버의 `--state-dir` 아래 `combat-audit.txt`에 캐릭터 ID를 한 줄에 하나씩 적는다. 기본 state-dir은 `./data`이다. 최초에 파일이 없으면 추적하지 않는다.

가상의 캐릭터 ID를 사용한 설정 예:

```text
1234
```

수동 해제하려면 해당 ID 줄을 삭제한다. 전부 해제하려면 파일 내용을 비운다.

- ID는 양의 정수이며 최대 128명을 지정한다. 빈 줄과 `#`로 시작하는 주석 줄은 무시하고 중복 ID는 합친다.
- 영구 캐릭터 ID를 사용하므로 이름 변경·재접속·서버 재시작 후에도 같은 캐릭터를 추적한다. 접속마다 달라지는 `player_id`와 구분한다.
- 로그 보관 기간은 서버 실행 옵션 `--combat-audit-retention-days` 또는 환경 변수 `COMBAT_AUDIT_RETENTION_DAYS`로 지정한다. 기본 30일, 최소 1일이며 **추적 만료 기간이 아니다.** 변경 시 서버를 재시작한다.
- 대상 파일은 서버 시작 시 읽고 이후 10분마다 다시 읽는다. 추가·해제는 다음 조회 때 반영된다. 임시 파일에 목록을 쓴 뒤 rename으로 교체하면 된다.
- 잘못된 ID 형식·파일 읽기 실패는 기존 설정을 유지하고 운영 로그에 경고한다. 시작 시 읽기가 실패하면 추적을 시작하지 않는다. 실행 중 파일 삭제도 해제로 취급하지 않는다.
- 대상 추가·삭제는 운영 로그의 `Combat audit targets updated`에서 확인한다. 배포만으로 특정 캐릭터를 자동 지정하지 않는다.

## 출력

`<state-dir>/combat-audit/combat-audit-YYYY-MM-DD.jsonl`에 대상의 1분 집계를 기록한다. 파일 날짜는 구간 시작 시각의 UTC 날짜이다. 접속 중에는 활동이 없는 구간도 기록한다. 로그아웃·수동 해제·정상 종료 때 남은 구간을 마감한다. 로그아웃한 구간은 다음 기록 주기(통상 1초 이내)에 저장한다.

| 필드 | 의미 |
|---|---|
| `schema`, `character_id`, `player_id`, `name` | 스키마 버전, 영구 캐릭터 ID, 접속 ID, 이름 |
| `start_ms`, `end_ms`, `reason` | Unix 밀리초, 마감 사유(`interval`, `logout`, `disabled`, `shutdown`) |
| `start_hp`, `end_hp`, `max_hp`, `level` | 시작·종료 HP, 종료 시점 최대 HP·레벨 |
| `health_gained` | 실제 증가한 HP를 `potion`, `food`, `natural`, `level_up`, `respawn`, `revive` 등으로 분리 |
| `health_lost` | 실제 감소한 HP를 `monster`, `debuff`, `death_penalty` 등으로 분리 |
| `deaths`, `level_ups` | 사망·레벨업 처리 횟수 |
| `monsters` | 몬스터 종류별 집계 |
| `history_overflow` | 공격 이력 추적 상한에 도달했는지 여부 |

`monsters`의 각 종류에는 `server_attempts`, `client_requests`, `rejected`(사유별), `hits`, `misses`, `damage`, `kills`, `kills_without_observed_attempt`가 들어간다.

- 공격 시도는 AI가 공격 명령을 실행하거나 클라이언트 요청이 들어온 횟수이다. AI의 탐색·접근·대기 자체는 시도로 세지 않는다. 클라이언트 요청은 서버 시도와 별도 집계한다.
- 거부 사유는 `missing_monster`, `not_controllable`, `cooldown`, `target_not_damageable`, `unreachable_floor`, `out_of_range`, `wall`, `target_disappeared_or_dead`, `client_disabled`이다. 없는 몬스터와 무시된 클라이언트 요청의 종류는 `unknown`이다.
- `damage`는 실제 HP 감소량이다. 남은 HP를 넘는 피해와 최대 HP를 넘는 회복을 제외한다. 일반적인 구간에서 `end_hp - start_hp = sum(health_gained) - sum(health_lost)`로 대조할 수 있다.
- `kills`는 본인이 마지막 타격을 가한 처치 수이다. 파티 공유 경험치 횟수가 아니다.
- `kills_without_observed_attempt`는 **추적 중 해당 캐릭터를 향한 공격 시도를 관측하지 못한 처치**다. 다른 사람에게 공격했거나, 추적 전에 공격했거나, 한 방에 죽은 경우도 포함한다. 이것만으로 악용을 판정하지 않는다. 시도 이력은 1분 경계를 넘어 유지한다.
- 몬스터 이력은 대상당 최대 4,096개이며 사라진 몬스터를 주기적으로 정리한다. 상한을 넘으면 해당 추적 세션의 `history_overflow`를 표시하고 이력이 없는 처치를 무반격 수치에 더하지 않는다.

파일 쓰기는 게임 상태 잠금 밖의 작업에서 수행한다. 쓰기 실패 시 오류를 남기고 마감 구간을 재시도한다. 미저장 구간은 최대 16,384개이며 초과하면 가장 오래된 구간을 버리고 오류를 남긴다. 강제 종료 시 진행 중이거나 아직 저장하지 못한 구간은 유실될 수 있다. 정상 종료에서는 남은 구간을 저장한다.

## 조사할 때

먼저 날짜 범위와 `character_id`를 선택하고, 구간별 HP 수지와 거부 사유를 확인한다. 명중률은 `hits / (hits + misses)`로 계산하고, 거부된 공격은 분모에서 제외한다. 몬스터별 피해와 회복원을 대조한 뒤 반격 시도가 없는 처치 비율을 함께 본다. 관측 시작·재접속·기록 실패·이력 초과 여부도 함께 확인한다.

구현: [combat_audit.rs](../server/src/game_state/combat_audit.rs).
