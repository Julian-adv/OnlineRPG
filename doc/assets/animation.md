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

## Mixamo Animation Export Workflow

새 Mixamo 애니메이션을 offhand/locomotion 등의 pack에 추가할 때:

1. **Mixamo에서 FBX 다운로드**
   - Format: **FBX Binary**
   - Skin: **Without Skin**
   - FPS: **30**
   - Keyframe Reduction: **none**
   - 이동 동작은 반드시 **In Place** 체크 (현재 export는 Hips location을 bake하지 않음)

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

3. **`export_animations.py`의 `EXPORT_PACKS`에 액션 이름 추가** (예: `offhand` pack에 `"torch_walk"`)

4. **Export 실행**

   ```bash
   blender ~/assets_original/all_animation.blend --background --python tools/blender-scripts/export_animations.py
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

## Known Pitfalls

- **A-pose vs T-pose rest**: Mixamo 원본은 A-pose, 프로젝트 Armature는 T-pose. 리타게팅
  bake 없이 그대로 export하면 팔 등에 identity에 가까운 키프레임이 적용되어 T-pose로
  서 있는 자세가 나온다 (`import_mixamo_animation.py`가 이를 자동 처리).
- **Armature.001 바인딩**: FBX import는 항상 새 Armature.001에 연결된다. `Armature`에
  바인딩된 액션으로 만들려면 retarget bake 단계가 필수.
- **Hips location 스케일**: Mixamo는 센티미터 단위라 Hips pose location을 그대로 쓰면
  캐릭터가 수 km 밖으로 날아간다. `import_mixamo_animation.py`는 location 채널을
  bake하지 않으므로 in-place 애니메이션만 지원한다.
