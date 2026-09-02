# Character Assets

## Human

- https://sketchfab.com/3d-models/blake-slim-walk-c4d-c076264ca7394357bf3f17837edd72c9 — **[미사용]** 캐릭터 미사용; 걷기 애니는 Mixamo 사용
- https://sketchfab.com/3d-models/xbot-049e4a44ad8b449dba8a2c4824502f5c — **[미사용]** 사용한 적 없음
- "Beauty Girl Exercising - Undressed Workout" (https://skfb.ly/pxpoo) by Polygonal Studios is licensed under Creative Commons Attribution (http://creativecommons.org/licenses/by/4.0/). — **[미사용]** Mixamo 도입 후 삭제
- "Beautiful Realistic Undressed Girls - 14 Anims" (https://skfb.ly/pxpoH) by Polygonal Studios is licensed under Creative Commons Attribution (http://creativecommons.org/licenses/by/4.0/). — **[미사용]** 초기 테스트용, Mixamo 도입 후 삭제
- "Mutant Mixamo" (https://skfb.ly/6DvxK) by NAZTart is licensed under Creative Commons Attribution (http://creativecommons.org/licenses/by/4.0/). — **[미사용]** Mixamo 도입 후 삭제
- "MIXAMO" (https://skfb.ly/ottKO) by sdhkim is licensed under Creative Commons Attribution (http://creativecommons.org/licenses/by/4.0/). — **[미사용]** Mixamo 사이트 알기 전 Sketchfab에서 찾은 애니, Mixamo 도입 후 미사용
- "Bandit Armor and Clothes - Game Model" (https://skfb.ly/6UVot) by wolkoed is licensed under Creative Commons Attribution (http://creativecommons.org/licenses/by/4.0/). — **[미사용]**
- Maria https://sketchfab.com/3d-models/maria-a04cac95ab8046e4bbdc9dec30c7d92d — **[미사용]** 초기 사용, 현재 미사용
- dying https://sketchfab.com/3d-models/dying-98a1d5b2288d49d993039cb161913cd3 — **[미사용]** 정적 dead 포즈 모델(CC-BY, robotgoul); 인게임 death 애니와 다름을 확인 → 캐릭터·애니 소스 아님 (death 클립은 Mixamo 계열)
- medieval_knight https://sketchfab.com/3d-models/medieval-knight-sculpture-game-ready-6cdd055b4afa41eb9360dbbfe75c7f10 — **[미사용]**

## Female Knight

- (초기) ComfyUI에서 jibMixZIT_v10.safetensors로 원화 생성 ![원화](../images/characters/female-knight-concept.png)
- 현재 원화 ![원화](../../client/public/character_concepts/female_knight.webp) (그리기: ComfyUI jibMixZIT_v10, A포즈 변경만 Qwen Image Edit; ChatGPT Pro 20x로 배경 투명화·키 약간 축소, 2026-08-28; WebP q85)
- Tripo(유료 등급)에서 3d 모델로 변환 -> 10k 모델로 리매쉬
- mixamo.com에서 리깅 및 애니메이션 부착
- blender에서 스케일/위치 조정(rest pose 원점 발 밑에 오게) -> 매터리얼 조정 (Shader Editor에서 Alpha 끊기) -> .glb 내보내기
- tools/glb-editor에서 `본 이름 표준화`

## Thief → Rogue

`female_thief.glb`가 `female_rogue.glb`로 개명됨 (클래스 Thief → Rogue, 커밋 7eebc39). 현재 사용 중.

- female_knight와 같은 workflow (3D 생성은 meshy.ai)
- 원화 ![원화](../../client/public/character_concepts/female_rogue.webp) (캐릭터 선택 UI 원화; WebP q85 변환 2026-08-28; 초기 thief 원화 `../images/characters/thief-concept.png`에서 교체)

## Knight

- female_knight와 같은 workflow (3D 생성은 meshy.ai)
- 원화 ![원화](../images/characters/knight-concept.png)
- nano banana2로 A 포즈 ![T-pose](../images/characters/knight-A-pose.png)
- character_concepts 원화 `character_concepts/knight.webp` (Gemini 원본을 ChatGPT Pro 20x로 배경 투명화, 2026-08-28; WebP q85)

## Other Classes

아래 플레이어 클래스는 female_knight와 같은 AI 워크플로우 (ComfyUI 원화 → Nano Banana/Grok 포즈 → 3D 생성 → Mixamo 리깅). 3D 도구는 캐릭터별로 다름(Meshy/Tripo) — License 섹션의 3D 도구 매핑 참조.

- barbarian / female_barbarian — Warrior 대체; 원화: barbarian(남) `character_concepts/barbarian.webp` (Gemini 원본을 ChatGPT Pro 20x로 배경 투명화, 2026-08-28; WebP q85), female_barbarian `character_concepts/female_barbarian.webp` (ChatGPT Pro 20x로 배경 투명화, 2026-08-28; WebP q85)
- caveman / cavewoman — 원화: caveman(남) `character_concepts/caveman.webp` (ChatGPT Pro 20x 생성, 2026-08-28; WebP q85. 이전 Qwen Image Edit 원화는 **[미사용]**), cavewoman `character_concepts/cavewoman.webp` (ChatGPT Pro 20x로 배경 투명화, 2026-08-28; WebP q85)
- priest / female_priest — 원화: priest(남) `character_concepts/priest.webp` (Gemini 원본을 ChatGPT Pro 20x로 배경 투명화, 2026-08-28; WebP q85), female_priest `character_concepts/female_priest.webp` (ChatGPT Pro 20x로 배경 투명화, 2026-08-28; WebP q85)
- ranger — 남성 ranger; 원화 `character_concepts/ranger.webp` (Gemini 원본을 ChatGPT Pro 20x로 배경 투명화, 2026-08-28; WebP q85)
- valkyrie — 단일 성별; 원화 `character_concepts/valkyrie.webp` (ChatGPT Pro 20x로 배경 투명화, 2026-08-29; WebP q85)
- rogue (남) — 남성 rogue 모델; 원화 `character_concepts/rogue.webp` (Gemini/Nano Banana 원본을 ChatGPT Pro 20x로 배경 투명화, 2026-08-28; WebP q85); female_rogue는 [Thief → Rogue](#thief--rogue) 참고

## Bard

`female_bard.glb` — 단일 성별(female). Meshy.ai Premium 등급, 생성 2026-08-05 (프롬프트명 "Crimson Vanguard").

- 원화 ![원화](../../client/public/character_concepts/female_bard.webp) (ChatGPT Pro 20x로 배경 투명화, 2026-08-28; WebP q85)

기존 워크플로우와 다른 점: Meshy 출력이 63개 분리 셸에 뒤집힌 면 403개, 열린 경계 1,093개라 Mixamo 업로드가 실패했다. Blender에서 커스텀 스플릿 노멀 제거 → Recalculate Outside → 8변 이하 구멍만 메움으로 정리 후 업로드 성공. 큰 개구부는 다른 조각에 가려 보이지 않아 남겼다(머티리얼 `doubleSided`).

- Mixamo 업로드용 FBX는 머티리얼 제거 + 텍스처 임베드 해제 필요 — metallic/roughness/emissive가 물려 있으면 "unable to map your existing skeleton"으로 실패
- 텍스처: baseColor 2048² JPEG + normal 1024² JPEG (Meshy 원본 2048² PNG에서 축소)
- emissive 미연결 (Meshy 기본 `EmissiveColor [1,1,1]` 방치 시 백색 발광)

## NPC Models

플레이어 클래스 아님.

- guard — 경비병 NPC Karl (`guard.glb`, CharacterClass::Guard); 원화 `../images/characters/karl-concept.png`, 3D는 Meshy.ai (라이센스는 위 License 표 참조); 거래 창 초상화 `../images/characters/karl-portrait.png` (ChatGPT, 2026-06-12, `doc/assets/ui.md` 참조)
- npc_woman — 상인 NPC Rica (`npc_woman.glb`); 원화 `../images/characters/rica-concept.png` (Gemini) (커밋 fb299e7); 거래 창 초상화 `../images/characters/rica-portrait.png` (ChatGPT, 2026-06-10, `doc/assets/ui.md` 참조)
- maid — 여관 직원 NPC용 메이드 (`maid.glb`, NPC Miriel); 원화 `../images/characters/maid-concept.png` (ComfyUI krea2_turbo_fp8_scaled, 2026-08-31); 거래 창 초상화 `../images/characters/miriel-portrait.png` (2026-09-02, `doc/assets/ui.md` 참조); 3D는 Meshy.ai Image to 3D (유료 등급, 생성 2026-08-30), Meshy 원본 `assets/Meshy_AI_Elegant_Maid_Pose_0830152239_texture_obj.zip`(OBJ, Mixamo 업로드용) + `assets/Meshy_AI_Elegant_Maid_Pose_0830151822_texture (1).glb`(같은 모델의 GLB 재다운로드, 노멀·MR 맵 포함), Mixamo 리깅 65본 `assets/maid_mixamo.fbx` (2026-08-31, Stand To Sit 스킨째). 헤드리스 Blender에서 `fix_mixamo_transforms` → 애니 제거 → 키 1.90m(npc_woman 1.91m 기준), 발 원점 → `mixamorig:` 접두 제거 → 머티리얼은 Meshy GLB 것을 통째로 이식(UV 동일: baseColor 픽셀 일치 확인), emissive/specular 제거 → GLB export → 후처리로 본 노드의 float 오차 scale 제거·노멀/MR 1024² 축소. baseColor 2048² JPEG q94 4:4:4(exporter 기본 q92 4:2:0은 얼굴이 뭉개져 상향; q97 백업 `~/assets_original/maid/maid_q97.glb`), normal·metallicRoughness 1024² JPEG
- pink_maid — 여관 메이드 NPC Cocoly (`pink_maid.glb`); 원화 `../images/characters/pink-maid-concept.png` (ComfyUI krea2_turbo_fp8_scaled, 2026-08-31 이전 생성); 거래 창 초상화 `../images/characters/cocoly-portrait.png` (2026-09-02, `doc/assets/ui.md` 참조); 3D는 Meshy.ai Image to 3D (유료 등급, 생성 2026-08-31, 프롬프트명 "Pink Porcelain Maid"), Meshy 원본 `assets/Meshy_AI_Pink_Porcelain_Maid_0831180703_texture_obj.zip`(OBJ, Mixamo 업로드용) + `..._0831180550_texture.glb`(GLB, 머티리얼 이식용) + `..._0831180601_texture_fbx.zip`(FBX, 미사용), Mixamo 리깅 `assets/Taunt.fbx` (2026-09-01, 검지만 있는 33본 스킨째 — Meshy 메시의 손가락이 붙어 있어 Mixamo가 풀 스켈레톤을 못 만듦; 나머지 손가락 애니 트랙은 무시됨). 가공은 maid 항목과 동일 파이프라인 (키 1.90m, `mixamorig:` 제거, Meshy GLB 머티리얼 이식, baseColor 2048² JPEG q94 4:4:4, normal·MR 1024², q97 백업 `~/assets_original/pink_maid/pink_maid_q97.glb`)
- night_merchant — 야간 상인 NPC Wick (`night_merchant.glb`); 거래 창 초상화 `../images/characters/wick-portrait.png` (ChatGPT, 2026-08-27, `doc/assets/ui.md` 참조); Meshy.ai Premium 등급, 생성 2026-08-08 (프롬프트명 "The Jolly Buccaneer"). OBJ로 받아 Mixamo 리깅(Excited) — Mixamo에서 텍스처가 하얗게 깨져 Blender에서 baseColor 재연결. 손가락 본 없는 33본 스켈레톤(기존 65본과 달리 손가락 애니 안 먹음, 런타임 리타게팅이 없는 본 트랙은 무시). baseColor 2048² JPEG, 노멀맵 없음. .blend 소스 `~/assets_original/night_merchant.blend` (텍스처 팩 포함)

## 텍스처 재패킹 (2026-08-06)

Meshy/Tripo 내보내기가 노멀·metallicRoughness 맵을 2048² RGBA PNG로 임베드해
캐릭터당 12~15MB였다. `tools/repack-glb-textures.py`로 전체 재패킹:
노멀·MR은 JPEG 4:4:4 1024²(q92/q90), 베이스컬러는 해상도 유지한 채 JPEG q92
(female_knight만 PNG였음), 플랫 노멀맵(knight, npc_woman)과 알파가 상수라
무의미했던 specularTexture(female_knight)는 제거.
합계 168.5MB → 33.9MB, 텍스처 VRAM 900MB → 464MB.
스크립트는 멱등이라 재실행해도 재인코딩하지 않는다.

베이스컬러 축소는 보류. `--base-max 1024`면 23.5MB/VRAM 229MB까지 내려가지만
선택 화면 크기에서 사슬갑옷·문장 같은 패턴 면이 뭉갠다(guard 45dB가 최악).
1536은 NPOT 리샘플 탓에 1024보다도 나쁘니 중간값은 없다.

## License (AI 제작 캐릭터)

위 AI 워크플로우로 만든 플레이어 캐릭터 전부(knight, barbarian, caveman, priest, rogue, ranger, valkyrie의 male/female)의 도구별 라이센스. 3D 도구는 female_knight만 Tripo, 그 외 전부 Meshy. (조사 2026-07, 약관 변경 가능)

| 단계 | 도구 | 라이센스 | 비고 |
|------|------|---------|------|
| 원화 | ComfyUI + jibMixZIT / Z-Image Turbo / Qwen Image Edit | Apache 2.0 | 상업 OK, 표시 의무 없음 (로컬 실행) |
| T/A 포즈 | Nano Banana(Gemini) / Grok | 출력물 사용자 소유, 상업 OK | 전 등급 동일, IP 배상 없음 |
| 3D 메쉬 (대부분) | Meshy.ai (유료 생성) | 완전 소유권, 상업 OK | 무료 다운그레이드해도 유지 (CC-BY 전환 안 됨) |
| 3D 메쉬 (female_knight) | Tripo (유료 Pro+ 생성) | 유료=완전 상업권 | ⚠️ 다운그레이드 후 유지 여부 약관 미명시 — 인보이스 보관·support 문의 |
| 리깅/애니 | Mixamo (Adobe) | 무료·로열티 없음·상업 OK | 원본 파일 단독 재배포 금지, 임베드는 OK |

핵심 조건:

- Meshy: 유료 때 생성분은 상업권 영구 유지. 단 ① Meshy Community에 공개 게시 안 함, ② 입력물이 타 저작권 미침해(위 원화·포즈 체인은 Apache 2.0/사용자 소유라 충족).
- Tripo: 유료 생성 시점엔 완전 상업권이나 **다운그레이드 후 유지 여부가 약관에 없음** (Meshy보다 리스크). 상업화 전 support@tripo3d.ai 확인 권장.
- **3D 도구 매핑** (Tripo=리스크, Meshy=안전): Tripo = female_knight (유일) / Meshy = 그 외 캐릭터·NPC 전부.
- 입증 대비: **Meshy·Tripo 결제 인보이스 + 생성 날짜** 보관 (유료 시점 생성 증빙).
- AI 생성 이미지는 저작권 보호가 약해 독점권 주장은 어려움(사용은 무방).
- Mixamo "단독 재배포 금지" 판단(2026-08-13): 애니메이션은 독립 배포물이 아니라 OpenMMO
  게임의 일부로 딸려나가므로 임베드에 해당한다고 본다. HF 데이터셋(`assets.lock`)과
  `assets/all_animation.blend`도 같은 게임의 빌드 소스로 취급한다.
- 같은 날 애니메이션 팩 GLB에서 Mixamo 캐릭터 메쉬(Medea)와 텍스처를 걷어냈다
  (36.5MB → 2.4MB). 런타임이 안 읽는 데이터라 크기·성능 목적 — [animation.md](./animation.md) 참조.

출처: [Meshy 취소 시 라이센스](https://help.meshy.ai/en/articles/9992023-if-i-cancel-my-subscription-will-all-my-models-revert-to-a-cc-by-4-0-license), [Tripo 약관](https://www.tripo3d.ai/terms), [Tripo 라이센스 가이드](https://www.tripo3d.ai/game-development/3d-assets-license-game-development), [Mixamo FAQ](https://helpx.adobe.com/creative-cloud/faq/mixamo-faq.html), [jibMixZIT](https://civitai.com/models/2231351/jib-mix-zit), [Z-Image Turbo](https://huggingface.co/Tongyi-MAI/Z-Image-Turbo)
