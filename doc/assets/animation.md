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
- Sword And Shield Idle

- Sword and Shield Slash https://www.mixamo.com/#/?page=1&query=slash&type=Motion%2CMotionPack

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

- Excited https://www.mixamo.com/#/?query=excited&type=Motion%2CMotionPack (social pack, `excited`, `/emote excited`)
  - 예외적으로 Without Skin이 아니라 night_merchant(버커니어) 스킨째 받은 FBX(`assets/Excited.fbx`,
    손가락 본 없는 33본)에서 리타겟했다 — social 팩 스킨 조인트도 33개라 손실 없음.
    social.glb를 타겟 armature로 임포트해 retarget bake 후 `graft-glb-clip.py`로 이식.
    작업 blend: `~/assets_original/excited_social_work.blend`.

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
  `Armature_combat`(combat_melee 전용, 손가락·눈·`mixamorigSleeve*` 포함, scale 0.1).
- 어느 팩도 참조하지 않는 잔재 액션: `torch_idle`(= `torch_idle1`),
  `run_down`, `run_sword`, `walk_cat`, `walk_female`, `walk_tough`, `mixamo.com*`.

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
