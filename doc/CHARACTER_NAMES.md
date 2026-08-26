# Character Names (캐릭터 이름 제한)

캐릭터 이름은 두 단계로 검사한다. 형식 검사는 코드에 있고(`server/src/auth.rs`의 `valid_character_name`), 금지어 목록은 서버가 시작할 때 파일에서 읽는다.

## 금지 이름 목록

`<state_dir>/banned_names.txt` (개발 환경 기본값은 `data/banned_names.txt`). 한 줄에 이름 하나, `#`으로 시작하는 줄은 주석이다. 로딩은 `server/src/banned_names.rs`.

- 앞뒤 공백과 대소문자만 무시하고 **정확히 일치**하는 이름을 막는다. 부분 일치는 쓰지 않는다 — "Sigmund"가 "gm"에 걸리는 오탐이 생긴다. 변형(`GM_`, `운영자1` 등)을 막으려면 줄을 추가한다.
- 파일이 없거나 읽히지 않으면 목록은 빈 상태가 되고 서버는 그대로 뜬다. 보안 장치가 아니라 운영 장치다.
- 목록은 서버 재시작 시점에만 읽는다. 재배포는 필요 없다.
- 운영자가 돌리는 NPC 계정(`npc_` 접두사)은 이 검사를 건너뛴다. 헤드리스 봇은 이름 변경 창에 답할 수 없기 때문이다.

도커에서는 이미지의 `data/banned_names.txt`가 상태 볼륨으로 시드된다(`docker/server-entrypoint.sh`). 볼륨에 이미 파일이 있으면 덮어쓰지 않으므로 운영자 수정본이 살아남는다.

## 생성 시점

`create_character`가 `check_new_character_name`을 부른다 — 형식, 금지어, 대소문자 무시 중복 순서다. 개명도 같은 함수를 쓴다. 금지어에 걸리면 `AuthError::BannedCharacterName`이 나가고 클라이언트는 캐릭터 생성 화면에 사유를 띄운다.

## 이미 만들어진 캐릭터

목록은 캐릭터가 생긴 뒤에 늘어난다. 그래서 기존 캐릭터는 **입장 시점**에 걸러진다.

1. 클라이언트가 `EnterGame`을 보낸다.
2. 이름이 목록에 있으면 서버가 `CharacterRenameRequired`로 거절한다 (입장하지 않는다).
3. 캐릭터 선택 화면이 새 이름 입력 창을 연다.
4. 클라이언트가 `RenameCharacter`를 보내고, 서버는 생성과 **같은 함수**로 검사한 뒤 `CharacterRenamed`로 답한다.
5. 이름이 바뀌면 클라이언트가 곧바로 입장을 다시 시도한다.

이름을 바꾸면 `character_blocks`의 `blocked_name`도 같은 트랜잭션에서 따라 바뀐다. 친구 목록은 캐릭터 id를 쓰므로 영향이 없다.
