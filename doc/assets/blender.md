# Blender Asset Workflow

## Version

- Use version 5.1.0

## Scripts

- `fix_mixamo_transforms.py`

  mixamo에서 import한 armature와 mesh가 각각 scale이 0.01, 100.0으로 되어 있는 것을 1.0, 1.0으로 맞춰준다.

- `add_action_to_nla.py`

  mixamo에서 import한 메쉬없는 애니메이션을 최초의 armature에 붙여준다

- `import_mixamo_animation.py`

  Mixamo FBX 하나를 `Armature`(T-pose 타겟)에 맞는 액션으로 변환까지 자동화한다.
  내부에서 FBX import → `fix_mixamo_transforms` 실행 → A-pose→T-pose
  리타게팅 bake (본별 `target_basis = target_rest.inv() × source_rest × source_basis`) →
  슬롯 식별자를 `OBArmature`로 설정 → 임시 Armature/액션 정리 → `.blend` 저장을
  한 번에 수행한다.

- `export_character.py`

  Mixamo 스킨 FBX + 같은 메시의 Meshy GLB로 캐릭터 GLB를 만든다: 애니 제거 →
  transform 정규화·키 맞춤·발 원점 → `mixamorig:` 제거 → Meshy 머티리얼 이식(UV 일치 검증)
  → emissive 제거·normal/MR 1024² → WebP q90 GLB export → 본 노드 float 오차 scale 제거.
  헤드리스(`blender -b -P ... -- --fbx ... --glb ... --name ... --out ...`)와 세션 안
  exec 양쪽을 지원한다. steward(2026-09-05)부터 사용.

## Import Tips

- .glb를 import 할 때 거대한 구가 나타나는 경우 bone shape scale을 0.01로 하면 거대한 구체가 나타나는 것을 방지할 수 있다.

## Export Tips

- Backface Culling: Material Properties → Settings → Backface Culling 켜기(뒷면 제거).
- Shader Editor 활성화
  - Alpha가 의도치 않게 들어가 있는지: Base Color 텍스처에 알파가 섞여 Alpha에 연결돼 있지 않은지 확인.
- .glb 내보내기 시 권장 옵션(Blender glTF 2.0 Exporter)
  - Apply Modifiers: 켜기
  - (노멀맵 쓴다면) Tangents: 켜기
