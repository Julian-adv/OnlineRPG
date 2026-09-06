# Blender Asset Workflow

## Version

- Use version 5.1.0

## Scripts

- `export_item_asset.py`

  정적인 GLB/glTF/FBX 아이템을 하나의 메쉬로 합쳐 회전·크기를 적용하고 바닥 중앙에 원점을 둔다.
  텍스처는 종횡비를 유지하며 최대 512px로 축소하고, WebP q90 GLB와 투명 128px 아이콘,
  512px 미리보기 및 packed `.blend`를 만든다. 리깅·shape key 모델과 손잡이 원점이 필요한 무기는 별도 작업한다.
  모델은 `client/public/models/CATEGORY/NAME.glb`, 아이콘은 `client/public/items/CATEGORY/NAME.png`,
  작업 파일은 `assets/NAME/`에 저장한다. 원본은 수정하지 않는다.

  Land Deed 재현 명령 (저장소 루트에서 실행):

  ```bash
  blender -b --python-exit-code 1 -P tools/blender-scripts/export_item_asset.py -- \
    --source assets/land_deed/Meshy_AI_Blackridge_Estate_Dee_0905071232_texture.glb \
    --name land_deed --size 0.5 --rotation -90 0 0 \
    --icon-rotation 28 -8 -12 --exposure -1.2
  ```

  `--size`는 회전 후 가장 긴 변의 미터 단위 길이이며, `--size-axis x|y|z`로 기준 축을 바꿀 수 있다.
  축과 회전은 Blender Z-up 기준이다. `--category` 기본값은 `objects`이며,
  `--texture-size`로 텍스처 최대 변 길이, `--keep-emission`으로 의도된 발광 유지를 지정한다.
  아이콘용 복사본은 일정 크기로 정규화한 뒤 촬영하므로 게임 모델 크기와 독립적으로 노출을 조절할 수 있다.
  `--output-root /tmp/item-preview`로 별도 디렉토리에 결과를 만들고, `--help`로 전체 인자를 확인한다.

  가구는 카탈로그의 `solid: true` 등록 후 `node tools/measure-furniture-footprints.mjs`로
  충돌 영역을 갱신한다. WebP GLB 측정에는 `@gltf-transform/core`와
  `@gltf-transform/extensions`가 필요하다 (`npm install --prefix tools --no-save --package-lock=false @gltf-transform/core@4.4.2 @gltf-transform/extensions@4.4.2`).

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
