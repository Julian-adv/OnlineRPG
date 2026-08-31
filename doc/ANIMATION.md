# Animation Pipeline

OnlineRPG 클라이언트의 캐릭터 애니메이션 로딩/매핑 규칙 문서.

## 1. 관련 파일

- 캐릭터 베이스 모델: `client/src/lib/utils/modelPaths.ts`의 `getCharacterModelPath(...)`가 반환하는 모델
- 이동 전용 클립: `client/public/models/animations/locomotion.glb`
- 근접 전투 클립: `client/public/models/animations/combat_melee.glb`
- 애니메이션 이름/순서 정의: `client/src/lib/types/animations.ts`
- 공통 유틸: `client/src/lib/utils/characterAnimationUtils.ts`
- 런타임 캐릭터: `client/src/lib/components/PlayerModel.svelte`
- 캐릭터 선택 프리뷰: `client/src/lib/components/CharacterPreview.svelte`

## 2. 표준 클립 이름

아래 이름은 코드에서 직접 참조하므로 대소문자까지 정확히 일치해야 한다.

| Category | Clips | Animation Pack |
|---|---|---|
| Idle | `idle1`, `idle2`, `idle3`, `idle4`, `idle5` | `locomotion` |
| Move | `walk`, `jog`, `run`, `jump` | `locomotion` |
| Attack | `slash1`, `slash2`, `slash3`, `slash4`, `slash5` | `combat_melee` |
| Attack alt | `attack1`, `attack2`, `attack3`, `attack4` | `combat_melee` |
| Death | `dying` | `combat_melee` |
| Attack idle | `combat_idle` | `combat_melee` (몬스터 `animAttackIdle` + 플레이어 스윙 사이 쿨다운) |
| Claw attack | `claw1`, `claw2` | `combat_melee` (몬스터 `animAttack` 전용, 플레이어 미사용) |

순서 기준은 `AnimationName` enum 선언 순서(`client/src/lib/types/animations.ts`)를 따른다.

몬스터 `animAttack`(monsters.csv)은 `claw1|claw2`처럼 `|`로 여러 클립을 나열할 수 있다 — 클라이언트가
스윙마다 하나를 랜덤으로 고르고, 서버의 스윙 홀드 시간(`data/monster_attack_clips.json`,
`tools/measure-monster-attack-clips.mjs`)은 그중 가장 긴 클립을 따른다.

## 2-1. 본 구조 (Mixamo)

현재 휴먼 본 구조는 Mixamo(`mixamorig`) 계열을 따른다.

- `glb-editor` 기준 본 이름: `mixamorig:Hips`, `mixamorig:Spine` 형태
- 현재 export된 node 이름: 접두어/콜론이 제거된 `Hips`, `Spine` 형태

## 2-2. 본 계층 구조 (부모-자식)

![locomotion bone hierarchy](./images/diagrams/animation-bone-hierarchy.svg)

- 파일: `doc/images/diagrams/animation-bone-hierarchy.svg`
- `locomotion.glb`에서 추출한 65개 본의 부모-자식 구조를 시각화한 이미지

## 3. 현재 매핑 정책

`selectOrderedCharacterAnimations(...)`에서 아래 우선순위를 사용한다.

1. `idle1~idle5`, `walk`, `jog`, `run`, `jump`는 `locomotion.glb` 우선
2. `slash1~slash5`, `attack1~attack4`, `dying`은 `combat_melee.glb` 우선
3. 지정된 source에 해당 이름의 클립이 없으면 같은 source의 첫 번째 클립으로 fallback
4. 지정된 source에도 fallback 클립이 없으면 빈 배열을 반환

초기 로딩 시 캐릭터 베이스 모델, `locomotion.glb`, `combat_melee.glb`, 기본 무기 모델 로드를 모두 기다린다.

## 4. 컴포넌트별 사용 방식

### PlayerModel

- 플레이 상태(`idle`, `moving`, `attack`, `dead`) 기반으로 클립 선택
- `moving` 상태에서는 시작 시점에 `walk/jog/run` 중 하나를 lock
- `idle`은 idle 계열 클립 중 랜덤 반복
- `attack`은 `slash1`, `dead`는 `dying` 사용

### CharacterPreview

- 선택 화면에서 idle 계열만 재생
- 선택되지 않은 슬롯은 action pause + time reset

## 5. 자주 발생하는 문제

### `THREE.PropertyBinding: No target node found for track: mixamorigHips.position`

원인:

- 애니메이션 트랙 본 이름과 타깃 모델 본 이름이 다를 때 발생
- 예: `locomotion.glb`는 `mixamorigHips`, 캐릭터 베이스 모델은 `Hips`

대응:

1. `glb-editor`에서 `본 이름 표준화`를 실행한다.

참고: 경고가 발생하면 클립이 부분/전체 미적용될 수 있으므로 반드시 확인한다.

## 6. 신규 애니메이션 추가 체크리스트

1. `glb-editor`에서 `본 이름 표준화` 버튼을 눌러 본 이름을 정리한다.
2. `애니메이션 추출` 버튼을 눌러 애니메이션을 추출한다.
3. 추출한 클립을 애니메이션 팩 중 하나(`locomotion`, `combat_melee`, `social`, `offhand`)에 넣는다.
   - 배포 중인 팩에 넣을 때는 `python tools/graft-glb-clip.py 팩.glb 도너.glb 클립이름 출력.glb`. 기존 클립과 스켈레톤을 바이트 단위로 보존한다.
   - `export_animations.py`로 팩을 통째로 다시 뽑아도 된다(2026-08-13 검증: 5팩 전부 배포본과 채널 단위 일치, 최대 오차 1.4e-5). 단 새 클립을 `all_animation.blend`에 먼저 넣어야 하고, glTF를 임포트해 넣었다면 키를 정수 프레임으로 스냅할 것 — 마지막 키가 `125.99999`로 들어오면 export에서 1프레임이 깎인다.
   - `combat_melee`만 69본 `Armature_combat`(손가락·눈·소매 본)에서 뽑는다. 33본 `Armature`로 뽑으면 채널 절반이 사라지고 rest 포즈가 어긋난다. 요청한 액션이 하나라도 없으면 스크립트가 해당 팩을 abort하고 기존 GLB를 건드리지 않는다.
4. 클립 이름을 `AnimationName`에 추가
5. `AnimationIndex` 동기화
6. 필요한 경우 `selectOrderedCharacterAnimations` 우선순위 반영
7. `PlayerModel` 상태 전이에서 새 클립 사용 지점 연결
8. `CharacterPreview`에서 필요한 경우 재생 정책 반영
9. 실행 검증
   - `cd client && npm run lint`
   - `cd client && npm run check`

## 7. 버전 로그

- `v0.8` (2026-02-21): 표준 클립 표에 `Animation Pack` 컬럼 추가
- `v0.7` (2026-02-21): 본 계층 구조 표를 SVG 이미지 첨부 방식으로 변경 (`doc/images/diagrams/animation-bone-hierarchy.svg`)
- `v0.6` (2026-02-21): VSCode 프리뷰 가독성을 위해 본 계층 구조를 Mermaid에서 테이블+인덴트 형식으로 변경
- `v0.5` (2026-02-21): 본 계층 구조를 텍스트 트리에서 Mermaid 다이어그램으로 변경
- `v0.4` (2026-02-21): `locomotion.glb` 본 계층 구조(`부모 ㄴ 자식`) 섹션 추가
- `v0.3` (2026-02-21): 신규 애니메이션 추가 절차에 `glb-editor` 본 정리/Extract/4개 묶음 분류 규칙 추가
- `v0.2` (2026-02-21): Mixamo 본 구조 설명 및 `locomotion.glb` 본 이름 목록 추가
- `v0.1` (2026-02-21): 문서 생성, locomotion 우선 매핑 규칙 및 트러블슈팅 정리
