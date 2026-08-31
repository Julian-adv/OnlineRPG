# Animation Assets

- 애니메이션 파이프라인/매핑 규칙 문서: [ANIMATION.md](../ANIMATION.md)

## Mixamo Animations

- mixamo.com에서 받은 fbx를 blender에서 scale 10으로 임포트한다

- Medea By M. Arrebola https://www.mixamo.com/#/?page=1&query=&type=Character
- Walking https://www.mixamo.com/#/?page=1&query=walk&type=Motion%2CMotionPack
- Catwalk Walk Forward https://www.mixamo.com/#/?page=2&query=walk&type=Motion%2CMotionPack
- Standing Torch Walk Forward
- Catwalk Walking

- Run (허리구부리고) https://www.mixamo.com/#/?page=1&query=run&type=Motion%2CMotionPack
- Slow Run https://www.mixamo.com/#/?page=1&query=run&type=Motion%2CMotionPack
- Jogging https://www.mixamo.com/#/?page=1&query=jog&type=Motion%2CMotionPack

- Standing Idle https://www.mixamo.com/#/?page=1&query=idle&type=Motion%2CMotionPack
- Happy Idle
- Dwarf Idle
- Offensive Idle https://www.mixamo.com/#/?page=2&query=idle&type=Motion%2CMotionPack
- Sword And Shield Idle (combat_melee pack, `combat_idle`, 몬스터 `animAttackIdle` + 플레이어 스윙 사이 쿨다운)
  - 2026-08-15에 gnoll 스킨째 받은 FBX(`assets/Sword And Shield Idle.fbx`, 57본, 새끼손가락 없음)를
    `import_mixamo_animation(..., target_armature_name="Armature_combat")`로 bake하고
    `export_animations.py -- --packs combat_melee`로 재export. 힙 흔들림 포함(bake_root_location 기본값).
    액션에 fake user를 켜지 않으면 저장 시 사라진다 (`import_mixamo_animation`은 켜지 않음).
    아주 잔잔한 숨쉬기 자세(머리 ~1cm)라 긴 쿨다운의 몬스터에는 "Offensive Idle" 같은 클립이 더 맞을 수 있다

- Sword and Shield Slash https://www.mixamo.com/#/?page=1&query=slash&type=Motion%2CMotionPack
- Zombie Attack ×2 (combat_melee pack, `claw1`/`claw2`, gnoll `animAttack` — 맨손 발톱 공격, 매 스윙마다 랜덤)
  - 2026-08-15 Mixamo, Without Skin(57본). `assets/Zombie Attack.fbx`(140f, 양손 내려찍기) → `claw1`,
    `assets/Zombie Attack (1).fbx`(80f, 오른손 휘두르기) → `claw2`.
    `import_mixamo_animation(..., target_armature_name="Armature_combat")`로 bake한 뒤 좀비 속도라 잘라내고
    빨리 감았다: claw1은 원본 12–75f를 ×1.5(43f, 1.43s), claw2는 1–56f를 ×1.4(40f, 1.33s)로 정수 프레임에 재샘플.
    두 클립 다 타격 시점이 ≈0.75–0.8s라 gnoll `attackImpactDelay` 750 / `attackDamageTextDelay` 850.
    1700ms 쿨다운보다 짧아 스윙 사이에 `combat_idle`이 잠깐 들어간다.

- Standing React Small From Front 02 https://www.mixamo.com/#/?query=standing+react&type=Motion%2CMotionPack
  (combat_melee pack, `hit`, 플레이어 피격 리액션 + 공용 팩 몬스터 `animHit`)
  - 2026-08-23 Mixamo, Without Skin(65본), 24f/0.8s. `assets/Standing React Small From Front 02.fbx`를
    `import_mixamo_animation(..., target_armature_name="Armature_combat")`로 bake하고
    (fake user를 켜야 저장된다) `export_animations.py -- --packs combat_melee`로 뽑은 도너에서
    `graft-glb-clip.py`로 `hit` 클립만 배포본 `combat_melee.glb`에 이식했다 — 기존 8개 클립과
    스켈레톤은 바이트 그대로다. Hips는 첫/끝 프레임이 같아 드리프트 없음.
  - 플레이어 쪽은 `PlayerModel.svelte`가 이 클립을 additive(`makeClipAdditive`, position 트랙 제거)로
    깔아 얹는다. 이동·공격 클립을 끊지 않고 상체만 움찔하게 하려는 것.

- Fishing Cast https://www.mixamo.com/#/?query=fishing&type=Motion%2CMotionPack (fishing pack, `fishing_cast`)
- Fishing Idle https://www.mixamo.com/#/?query=fishing&type=Motion%2CMotionPack (fishing pack, `fishing_idle`)
  - 두 fishing 액션의 `RightHand` 키에는 로컬 Z +40° 오프셋이 bake되어 있다 (탑다운
    카메라에서 로드가 시선 축과 겹쳐 안 보이던 것을 화면에 보이게 트는 각도).
    Mixamo에서 재임포트하면 오프셋이 사라지므로 다시 적용할 것.

- Guitar Playing https://www.mixamo.com/#/?page=1&query=Guitar+Playing&type=Motion%2CMotionPack (social pack, `guitar_playing`)
  - 체중 이동이 있어 Hips location bake가 필요한 첫 클립이다.
    `graft-glb-clip.py`로 social 팩에 이어붙였다.

- Clapping https://www.mixamo.com/#/?query=clapping&type=Motion%2CMotionPack (social pack, `clap`, `/emote clap`)
  - Excited와 같은 방식: `assets/Clapping.fbx`(버커니어 스킨, 33본)에서 리타겟 →
    `graft-glb-clip.py`로 이식.

- Twist Dance https://www.mixamo.com/#/?query=twist+dance&type=Motion%2CMotionPack (social pack, `twist`, `/emote twist`)
- Macarena Dance https://www.mixamo.com/#/?query=macarena&type=Motion%2CMotionPack (social pack, `macarena`, `/emote macarena`)
- Chicken Dance https://www.mixamo.com/#/?query=chicken+dance&type=Motion%2CMotionPack (social pack, `chicken`, `/emote chicken`)
  - 세 댄스 모두 Without Skin/30fps FBX를 social.glb 스켈레톤으로 리타겟 bake 후
    `graft-glb-clip.py`로 이식 (2026-08-14).

- Excited https://www.mixamo.com/#/?query=excited&type=Motion%2CMotionPack (social pack, `excited`, `/emote excited`)
  - 예외적으로 Without Skin이 아니라 night_merchant(버커니어) 스킨째 받은 FBX(`assets/Excited.fbx`,
    손가락 본 없는 33본)에서 리타겟했다 — social 팩 스킨 조인트도 33개라 손실 없음.
    social.glb를 타겟 armature로 임포트해 retarget bake 후 `graft-glb-clip.py`로 이식.
    작업 blend: `~/assets_original/excited_social_work.blend`.

- Stand To Sit / Sitting Idle / Sitting Talking / Sit To Stand https://www.mixamo.com/#/?query=sit&type=Motion%2CMotionPack
  (social pack, `stand_to_sit` / `sit_idle` / `sit_talk` / `sit_to_stand` — 의자 `interaction: "sit"`)
  - 2026-08-30 Mixamo, Without Skin(65본)/30fps, `assets/Stand To Sit.fbx` 등 4개를
    `import_mixamo_animation`으로 `Armature`에 bake(fake user)한 뒤 `all_animation.blend`에서
    손봤다: `stand_to_sit`는 끝 자세가 `sit_idle`보다 0.58m 뒤라 Hips를 -Y 0.58 이동(끝 = `sit_idle`
    첫 프레임), 네 클립 모두 Hips를 +Y 0.10 옮겨 등받이 쪽으로 붙였고, `sit_talk`는 44초짜리를 `sit_idle`과 가장 가까운 자세인 1–301f(10초)로 잘랐다.
    도너는 `export_animations.py`를 임시 팩으로 돌려 뽑고 `graft-glb-clip.py`로 4개를 이식했다
    (blend에 twist/macarena/chicken이 없어 social 팩 전체 재export는 막힌다).
  - 앉은 Hips 높이 0.584m, 의자 좌석 0.56m. 카탈로그 `interactOffset.y`는 눈으로 맞춘 값(0.03은
    엉덩이가 10cm 묻혔다) — 발끝은 그만큼 뜬다.

- Female Standing Pose ×3 https://www.mixamo.com/#/?query=standing+pose&type=Motion%2CMotionPack
  (social pack, `stand_pose2`/`stand_pose3`/`stand_pose4` — 메이드 클래스 idle + `/emote stand_pose2` 등 루프 이모트)
- Weight Shift https://www.mixamo.com/#/?query=weight+shift&type=Motion%2CMotionPack
  (social pack, `weight_shift`, `/emote weight_shift` — 루프 이모트)
- Yawn https://www.mixamo.com/#/?query=yawn&type=Motion%2CMotionPack (social pack, `yawn`, `/emote yawn` — 원샷 이모트)
  - 2026-08-31 Mixamo, Without Skin(65본)/30fps. `assets/Female Standing Pose (2).fbx`→`stand_pose2` 식으로
    FBX 번호를 클립 이름에 유지. `import_mixamo_animation`으로 `Armature`에 bake(fake user).
    Standing Pose 3개는 2프레임짜리 정지 포즈라 두 번째 키를 180f로 밀어 6초 홀드로 늘렸다.
    도너는 임시 팩으로 export 후 `graft-glb-clip.py`로 5개를 social.glb에 이식 (sit 클립과 동일 절차).
  - 메이드 클래스는 기본 idle1–5 대신 이 5개를 랜덤 재생한다 (`animations.ts`의
    `CLASS_IDLE_CLIP_NAMES` 클래스→클립 테이블, `PlayerModel.svelte`의 `pickClassIdleClip` —
    같은 클립 연속 재생은 피하고, 첫 miss에 social 팩 로드를 시작한다).

## Mixamo Animation Export Workflow

새 Mixamo 애니메이션을 offhand/locomotion 등의 pack에 추가할 때:

1. **Mixamo에서 FBX 다운로드**
   - Format: **FBX Binary**
   - Skin: **Without Skin**
   - FPS: **30**
   - Keyframe Reduction: **none**
   - 이동 동작은 반드시 **In Place** 체크 (Hips location이 bake되므로 그대로 두면
     캐릭터가 제자리를 벗어난다)

2. **Blender에서 import + retarget bake** (Text Editor/Python Console)

   ```python
   import sys
   sys.path.insert(0, r"C:\Users\jake\work\OnlineRPG\tools\blender-scripts")
   from import_mixamo_animation import import_mixamo_animation

   import_mixamo_animation(
       fbx_path=r"Y:\public\web_downloads\Standing Torch Walk.fbx",
       action_name="torch_walk",
   )
   ```

   배포 중인 팩에 클립 하나만 넣을 때는 아래 3~4단계 대신
   `python tools/graft-glb-clip.py 팩.glb 도너.glb 클립이름 출력.glb`를 쓴다
   ([ANIMATION.md](../ANIMATION.md) 참고).

3. **`export_animations.py`의 `EXPORT_PACKS`에 액션 이름 추가** (예: `offhand` pack에 `"torch_walk"`)

4. **Export 실행**

   ```bash
   blender assets/all_animation.blend --background --python tools/blender-scripts/export_animations.py
   # 바뀐 팩만: 나머지 GLB의 바이트(=assets.lock 해시)를 그대로 둔다
   blender assets/all_animation.blend --background --python tools/blender-scripts/export_animations.py -- --packs offhand
   ```

   또는 Blender 내부에서:

   ```python
   exec(open(r"C:\Users\jake\work\OnlineRPG\tools\blender-scripts\export_animations.py").read())
   ```

   Export script는 매 실행마다 `mixamorig:` 프리픽스를 fcurve에서 strip하고, 모든
   layered action의 슬롯 식별자를 대상 armature (`OBArmature`)에 맞게 재-바인딩한다.

5. **클라이언트 코드 연결** (새 애니메이션 타입인 경우)
   - `client/src/lib/types/animations.ts`의 `OffhandAnimationName`에 상수 추가
   - `client/src/lib/components/PlayerModel.svelte`에서 해당 상태에 클립 선택 로직 추가

## 팩에는 메쉬가 없다

배포되는 팩 GLB는 클립과 스켈레톤만 담는다. 런타임은 팩의 메쉬를 그리지 않고
클립(`PlayerModel.svelte`)과 리타게팅용 rest 포즈(`characterAnimationUtils.ts`)만 읽기
때문에, `tools/strip-animation-pack-mesh.py`가 export 직후 지오메트리·머티리얼·텍스처를
걷어낸다 (36.5MB → 2.4MB). 스킨 자체는 남긴다 — three의 GLTFLoader는 skinned mesh가
참조하는 노드만 `Bone`으로 만들기 때문에, 스킨을 지우면 리타게팅이 조용히 실패한다.

## all_animation.blend

모든 팩의 소스. 2026-08-13에 배포본에서 13개 클립을 역임포트해 5팩 전부를 재현할 수
있게 복구했다 (`pickup`, `guitar_playing`, `excited`, `clap`, `torch_idle1/2`,
`torch_walk`, `torch_run`, `dying`, `slash1`–`slash4`).

- 아마추어가 둘이다: 33본 `Armature`(locomotion/social/offhand/fishing)와 69본
  `Armature_combat`(combat_melee 전용, 손가락·눈·`mixamorigSleeve*` 포함). 둘 다 오브젝트
  transform은 identity, rest는 T-pose 직립(Hips z≈1.0)이다.
  - `Armature_combat`은 2026-08-15까지 오브젝트가 X -90°·scale 0.1이고 본이 10배로 구워져 있었고
    rest도 누워 있었다(배포본은 `Armature` 노드 scale 0.1 + 본 10배). 정규화하면서 transform을
    본에 굽고(`Armature.transform`), 6개 액션의 location 키를 ×0.1, rest를 세운 뒤 `Hips` 키를
    `rest_new⁻¹·rest_old·basis`로 재기준했다. 월드 포즈·클립 길이·리타게팅 결과는 정규화 전과 동일함을
    확인했다(three 리타게팅 하네스로 gnoll/knight 힙·발끝 좌표 비교). 정규화 전 blend 백업은
    보관하지 않는다 — 필요하면 HF에서 이전 리비전을 받을 것.
- 어느 팩도 참조하지 않는 잔재 액션: `torch_idle`(= `torch_idle1`),
  `run_down`, `run_sword`, `walk_cat`, `walk_female`, `walk_tough`, `mixamo.com*`.
- `dying`은 2026-08-14에 힙을 4cm 내렸다 (시체가 지면에서 떠 있었다). blend와
  배포본 `combat_melee.glb` 양쪽에 반영돼 있으므로 재export해도 유지된다
  (blend는 `Armature_combat`의 `Hips` location[2]에 +0.04 — 정규화 전 단위로 +0.4, 배포본은
  `tools/shift-glb-clip-hips.py`). 이 오프셋은 캐릭터에만 듣는다 — 몬스터는
  로드할 때 클립을 다시 접지시키므로 (`groundRetargetedClips`) 상수 이동이 상쇄된다.

## Known Pitfalls

- **A-pose vs T-pose rest**: Mixamo 원본은 A-pose, 프로젝트 Armature는 T-pose. 리타게팅
  bake 없이 그대로 export하면 팔 등에 identity에 가까운 키프레임이 적용되어 T-pose로
  서 있는 자세가 나온다 (`import_mixamo_animation.py`가 이를 자동 처리).
- **Armature.001 바인딩**: FBX import는 항상 새 Armature.001에 연결된다. `Armature`에
  바인딩된 액션으로 만들려면 retarget bake 단계가 필수.
- **Hips location 스케일**: Mixamo는 센티미터 단위라 Hips pose location을 그대로 쓰면
  캐릭터가 수 km 밖으로 날아간다. `import_mixamo_animation.py`는 rest 대비 월드 변위를
  두 리그의 Hips rest 높이 비로 스케일해 bake한다. 제자리 클립에서 드리프트를 없애려면
  `bake_root_location=False`로 rest에 고정한다.
- **`bake_root_location=False`는 수직 성분까지 죽인다**: Hips가 rest 높이에 못 박히면
  다리가 땅에 닿지 못해 캐릭터가 공중에 뜬다. 걷기·달리기처럼 상하 바운스가 있는
  클립에는 절대 쓰지 말 것 (In Place FBX는 어차피 수평 드리프트가 없다).
  2026-08-14에 offhand 팩(`torch_walk` 5~13cm, `torch_run` 7~17cm, `torch_idle2` 0~6cm
  부양)이 이 문제로 확인되어 `tools/blender-scripts/ground_hips_curve.py`로 복원했다
  — 매 프레임 가장 낮은 발뼈를 `walk`의 접지 높이에 맞춰 Hips 수직 커브를 다시 키잉한다
  (체공 구간이 있는 `torch_run`은 `--shift`로 상수 오프셋만). 적용한 명령:

  ```bash
  blender assets/all_animation.blend --background \
      --python tools/blender-scripts/ground_hips_curve.py -- \
      --ground torch_walk,torch_idle2 --shift torch_idle1,torch_run \
      --clearance 0.007 --apply
  ```

  `--clearance 0.007`은 눈으로 맞춘 값이다: 지상 `walk`와 같은 높이로 접지시키면
  평평한 던전 바닥에서 발이 파묻혀 보인다 (지형은 굴곡·풀이 가려준다).
  델타는 기존 커브에 더해지므로 같은 명령을 다시 돌려도 결과가 같다.
  원본 Mixamo 커브가 필요하면 FBX를 다시 받아 `bake_root_location=True`로 재임포트할 것.
