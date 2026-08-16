# Monster Assets

## Monster

- SCP939 https://sketchfab.com/3d-models/scp939-79a749a5073b453d9d85875797bf45d7
- Orc https://create.verse8.io/ 에서 2d -> 3d 생성함
  - 원화는 chatgpt.com에서 다음 프롬프트로 생성함

    > T-pose, fantasy concept art of an orc warrior, muscular humanoid with greyish-green leathery skin, protruding lower tusks, heavy brow ridge, pointed ears, battle-scarred face, crude iron and bone armor, tribal war paint, dramatic rim lighting, dark earthy color palette, detailed character design sheet, painterly digital art, D&D fantasy aesthetic, highly detailed, 4k

    > change it to simple background, character only, no texts

    > make the background light grey

    > make his skin more green

    ![원화](../images/orc-concept.png)
- female orc https://create.verse8.io/ 에서 2d -> 3d 생성함; 원화는 chatgpt.com에서 생성 ![원화](../images/female-orc-concept.png)
- goblin https://create.verse8.io/ 에서 2d -> 3d 생성함; 원화는 chatgpt.com에서 생성 ![원화](../images/goblin-concept.png)
- hobgoblin Meshy.ai (유료 생성, 2026-08-14, "Ironclad Warlord") 에서 2d -> 3d 생성 후
  mixamo.com에서 auto-rig (65본). 원화는 chatgpt.com에서 생성 ![원화](../images/hobgoblin-concept.png)
  - Blender: Mixamo FBX가 metallic=1 / specular 2배로 들어와 검은 크롬처럼 보이므로 되돌리고,
    텍스처는 Mixamo FBX에 임베드된 PNG(2048², 1024²·JPEG q88로 export)로 재연결. 본 이름의 `mixamorig:` 접두사를 떼어
    캐릭터 리그(knight.glb) 규약에 맞춤. 높이 1.90m(사람 크기)로 스케일 적용, 원점=바닥 중심,
    `export_yup=True`로 GLB export. 작업 blend는 `assets/hobgoblin.blend`(HF 동기화)
  - 소스는 `assets/`의 Mixamo FBX 하나만 보관한다. Meshy obj zip은 리깅 없는 메시 + 동일한 PNG라 삭제 (2026-08-15)
  - Meshy가 베이스 컬러만 주므로 metallic-roughness 맵은 albedo의 채도·명도에서 유도해 만들었다
    (어둡고 무채색인 판금 → metallic 0.85 / roughness 0.54, 피부는 metallic 0 / roughness 0.92). 정확한 PBR이
    필요하면 Meshy에서 PBR 맵 세트를 다시 받아 교체할 것
  - 애니메이션 클립 미탑재 — 캐릭터 공용 팩(locomotion/combat_melee)을 런타임에 리타게팅해서 쓴다
    (`monsters.csv`의 `sharedAnims` → `loadSharedPackClipsForModel`, 모델당 1회 캐시).
    팩의 리그(Medea, 힙 1.0m)와 몬스터 리그의 비율이 달라 그대로 재생하면 팔다리가 늘어난다.
    리타게팅은 힙을 소스 리그 기준으로 잡아 몸이 지면에 파묻히므로(dying이 12cm) 클립마다
    가장 깊이 잠기는 프레임만큼 힙 트랙을 올려 접지시킨다 (`groundRetargetedClips`).
    공용 팩에는 hit 리액션이 없어 `animHit`은 비워 둠
- gnoll Meshy.ai (유료 생성, 2026-08-15, "Hyena Warlord") 에서 2d -> 3d 생성 후
  mixamo.com에서 auto-rig (57본, 새끼손가락 없음). 원화는 chatgpt.com에서 생성 ![원화](../images/gnoll-concept.png)
  - hobgoblin과 같은 Blender 파이프라인(Mixamo 재질 되돌리기, `mixamorig:` 접두사 제거, albedo에서 유도한
    metallic-roughness 맵, 1024²·JPEG q88, `export_yup=True`). 높이 2.15m — D&D 놀은 7~7.5ft로 사람보다 크다.
    Meshy 원본이 1cm 크기로 들어와 mesh/armature data를 직접 스케일했다. 작업 blend는 `assets/gnoll.blend`(HF 동기화)
  - 소스는 `assets/`의 Mixamo FBX 하나만 보관한다
- bugbear Meshy.ai (유료 생성, 2026-08-16, "Fanghide Warlord") 에서 2d -> 3d 생성 후
  mixamo.com에서 auto-rig (65본). 원화는 chatgpt.com에서 생성 ![원화](../images/bugbear-concept.png)
  - gnoll과 같은 Blender 파이프라인. 높이 2.20m — D&D 버그베어는 7ft 이상으로 놀(2.15m)보다 조금 크게.
    Meshy 원본이 1cm 크기로 들어와 mesh/armature data를 직접 스케일했다(`Mesh.transform`/`Armature.transform`).
    작업 blend는 `assets/bugbear.blend`(HF 동기화). 소스는 `assets/bugbear.fbx` 하나만 보관
  - 무기는 기존 iron_sword를 들려줬다(D&D 버그베어의 모닝스타 모델이 없음)
- ogre Meshy.ai (유료 생성, 2026-08-16, "Ironhide Brute") 에서 2d -> 3d 생성 후
  mixamo.com에서 auto-rig. 원화는 chatgpt.com에서 생성 ![원화](../images/ogre-concept.png)
  - bugbear와 같은 Blender 파이프라인(Mixamo 재질 되돌리기, `mixamorig:` 접두사 제거, albedo에서 유도한
    metallic-roughness 맵, 1024²·JPEG q88, 원점=바닥 중심, `export_yup=True`). 높이 2.4m.
    Meshy 원본이 1cm 크기로 들어와 mesh/armature data를 직접 스케일했다.
    작업 blend는 `assets/ogre.blend`(HF 동기화). 소스는 `assets/ogre.fbx` 하나만 보관
  - 리그가 33본뿐이라(손가락은 검지 체인만) 공용 팩을 리타게팅해도 나머지 손가락은 움직이지 않는다.
    무기는 `RightHand`에 greatclub(1.5m, 오거 키에 맞춰 sword 1.20m보다 길게).
    손 본이 손목에 있어 `weaponOffset`(본 로컬 +Y)으로 손가락 밑동까지 0.24m 밀어야 쥔 모양이 된다.
    값은 `RightHand` 가중치 정점이 본 축으로 뻗은 길이의 80% — 같은 규칙으로 벅베어 0.20, 홉고블린 0.12
  - 공용 팩을 쓰는 몬스터의 `walkSpeed`는 **Hips 본의 지면 위 높이**에 비례한다 — 리타게팅은 회전만
    옮기므로 보폭이 다리를 편 길이(엉덩이→접지점)로 정해지기 때문. 발목 높이로 재면 안 된다:
    벅베어처럼 발목이 높은 디지티그레이드 리그에서 크게 빗나간다(발목 기준으로는 1.4가 맞다고 나오는데
    실제로는 미끄러졌다). 팩 리그(Hips 1.165m)의 walk 클립은 사이클당 1.72m/0.958s = 1.8m/s이고,
    여기에 Hips 높이 비율을 곱한 값이 미끄러지지 않는 속도다:
    홉고블린 0.98→1.52(설정 1.5), 오거 1.17→1.81(1.8), 놀 1.14→1.76(설정 1.5, 아직 안 맞춤),
    벅베어 1.25→1.93(1.9). 벅베어·오거를 이 값으로 올려 미끄러짐이 사라진 걸 확인했다
  - run 클립은 체공 구간 때문에 같은 방식으로 못 잰다. 기존 값에서 역산한 팩 기준 5.3~5.5에
    Hips 비율을 곱해 잡는다 (오거 5.1, 벅베어 4.8→5.4)
- troll Meshy.ai (유료 생성, 2026-08-16, "Grimclaw Goblin") 에서 2d -> 3d 생성 후
  mixamo.com에서 auto-rig (65본). 원화는 chatgpt.com에서 생성 ![원화](../images/troll-concept.png)
  - ogre와 같은 Blender 파이프라인(Mixamo 재질 되돌리기, `mixamorig:` 접두사 제거, 1024²·JPEG q88,
    원점=바닥 중심, `export_yup=True`). 높이 2.7m — D&D 트롤은 9ft. Meshy 원본이 1cm 크기로 들어와
    mesh/armature data를 직접 스케일했다. 작업 blend는 `assets/troll.blend`(HF 동기화),
    소스는 `assets/troll.fbx` 하나만 보관
  - 금속 부위가 없는 모델(맨살·천 요포·머리카락·발톱)이라 metallic-roughness 맵을 만들지 않고
    metallic 0 / roughness 0.9 상수로 뒀다. albedo에서 유도하는 기존 공식은 어두운 머리카락과
    발톱을 금속으로 오인한다
  - 무기 없이 손톱으로 때린다 — 놀과 같은 `claw1|claw2` + `bleed`. Hips 높이가 1.51m로 커서
    walkSpeed 2.3(팩 1.8 × 1.51/1.165), runSpeed 6.5(팩 기준 5.05 × 같은 비율)

- kobold https://create.verse8.io/ 에서 2d -> 3d 생성함
  - 원화는 chatgpt.com에서 다음 프롬프트로 생성함
    > d&d 혹은 nethack에 나오는 kobold를 3d로 제작할 수 있게 T자형 포즈로 그려줘

    ![원화](../images/kobold-concept.png)
