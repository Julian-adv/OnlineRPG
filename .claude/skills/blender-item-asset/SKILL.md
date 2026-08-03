---
name: blender-item-asset
description: Meshy 등에서 받은 GLB/FBX를 Blender로 가져와 게임용 아이템 애셋으로 만든다 — 기존 애셋과 비교해 스케일 결정·적용, 원점 바닥 중심, Meshy emissive 제거, 텍스처 512² 축소, client/public/models/ 로 GLB export, 128×128 아이콘 렌더, doc/assets/items.md 기록까지. "이 glb 임포트해서 스케일 맞춰줘", "아이템 애셋 추가", "아이콘 렌더링해줘" 같은 요청에 사용한다.
argument-hint: [입력 glb/fbx 경로] [애셋 이름]
---

# 아이템 애셋 파이프라인

Blender MCP(`mcp__blender__execute_blender_code`)로 실행한다. Blender가 켜져 있고 애드온이 연결돼 있어야 한다.

산출물 3개:
- `client/public/models/<카테고리>/<name>.glb` — 원점 바닥 중심, 스케일 적용됨
- `client/public/items/<name>.png` — 128×128 투명 배경 아이콘
- `doc/assets/items.md` 항목 — 소스·라이선스·치수·날짜

카테고리: 착용구=`armor/`, 장신구=`accessory/`, 무기=`weapons/`, 그 외 사물=`objects/`.

---

## STEP 0 — 크기 기준 정하기

새 애셋 크기는 **기존 애셋과 비교해서** 정한다. 사용자가 "○○.glb랑 비슷하게"라고 하면 넘겨짚지 말고 직접 재본다:

```bash
tools/fetch-assets.sh                                          # models/는 대부분 gitignore + assets.lock 핀
python .claude/skills/blender-item-asset/measure_glb.py "client/public/models/**/*.glb"
```

기준 파일이 로컬에 없으면 **애셋을 안 받았거나 체크아웃이 뒤처진 것**이지 없는 게 아니다. `grep <이름> assets.lock`으로 먼저 확인할 것.

아래는 스냅샷 (2026-08-04, 단위 m, W×H×D). 최신값은 위 명령으로 다시 뽑는다:

| 애셋 | 치수 |
|---|---|
| apple | 0.15 × 0.16 × 0.15 |
| cross_shield_ring | 0.15 × 0.07 × 0.15 |
| plate_helmet | 0.20 × 0.30 × 0.29 |
| plate_gauntlets (한 쌍) | 0.35 × 0.15 × 0.32 |
| ornate_cross_belt | 0.40 × 0.15 × 0.40 |
| plate_greaves (한 쌍) | 0.37 × 0.45 × 0.36 |
| dagger | 0.42 × 0.11 × 0.03 |
| chain_mail | 0.53 × 0.28 × 0.60 |
| campfire | 0.60 × 0.26 × 0.58 |
| iron_leggings | 0.38 × 0.26 × 0.70 |
| sword | 1.20 × 0.26 × 0.07 |

실물보다 크게 잡는 게 이 게임의 규약이다 — 사과가 0.15m, 반지 외경이 0.15m다. 실측대로 만들면 화면에서 안 보인다. 확대한 경우 items.md에 이유를 남긴다.

음식류는 `client/public/models/objects/`에 들어간다. `doc/assets/items.md`의 "Hunger icons (9)" 항목에 따르면 현재 음식 아이콘은 PIL 프로시저럴 플레이스홀더이고 AI 아이콘으로 교체 예정이다 — 이 스킬이 주로 그 교체에 쓰인다.

---

## STEP 1 — 임포트

FBX zip이면 먼저 풀고 `bpy.ops.import_scene.fbx(filepath=...)`, GLB면 아래.

```python
import bpy
REPO = r"<리포 절대경로>"   # 예: C:\Users\jake\work\OnlineRPG — 머신마다 다르므로 매번 확인할 것
SRC  = r"<입력 경로>"
NAME = "<asset_name>"

before = set(bpy.data.objects.keys())
bpy.ops.import_scene.gltf(filepath=SRC)
new = [n for n in bpy.data.objects if n not in before]
bpy.context.view_layer.update()
result = {"new": [{"name": n, "type": bpy.data.objects[n].type,
                   "dims": [round(v, 4) for v in bpy.data.objects[n].dimensions]} for n in new]}
```

메시가 여러 개면 `bpy.ops.object.join()`으로 합친다.

---

## STEP 2 — 머티리얼 정리

Meshy 산출물은 emissive 텍스처가 물려 있다. **노드만 지우면 안 된다** — Principled의 Emission Color/Strength 소켓 값이 남아 glTF에 `emissiveFactor [1,1,1]`로 나가고 모델이 하얗게 발광한다.

```python
import bpy
o = bpy.data.objects[<임포트된 이름>]
o.name = o.data.name = NAME
mat = o.data.materials[0]
mat.name = NAME
nt = mat.node_tree
bsdf = nt.nodes["Principled BSDF"]

for l in list(bsdf.inputs["Emission Color"].links):
    nt.links.remove(l)
for n in list(nt.nodes):
    if n.type == 'TEX_IMAGE' and n.image and n.image.name.startswith("emissive"):
        nt.nodes.remove(n)
bsdf.inputs["Emission Color"].default_value = (0, 0, 0, 1)
bsdf.inputs["Emission Strength"].default_value = 0.0

for im in list(bpy.data.images):
    if im.name.startswith("emissive"):
        bpy.data.images.remove(im)
    elif im.size[0] > 512 and im.users:
        im.scale(512, 512)   # 2048² 원본은 아이템에 과함

result = {"images": [(i.name, list(i.size)) for i in bpy.data.images]}
```

---

## STEP 3 — 스케일 적용 · 원점 바닥 중심

`TARGET`과 기준 축은 애셋에 맞게 고른다 (착용구는 보통 높이, 눕는 사물은 길이).

```python
import bpy, mathutils
o = bpy.data.objects[NAME]
bpy.ops.object.select_all(action='DESELECT')
o.select_set(True)
bpy.context.view_layer.objects.active = o

TARGET = 0.15                      # m
cur = max(o.dimensions.x, o.dimensions.y)   # 또는 o.dimensions.z
o.scale = (TARGET / cur,) * 3
bpy.context.view_layer.update()
bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)

bb = [o.matrix_world @ mathutils.Vector(c) for c in o.bound_box]
cx = (min(v.x for v in bb) + max(v.x for v in bb)) / 2
cy = (min(v.y for v in bb) + max(v.y for v in bb)) / 2
bpy.context.scene.cursor.location = (cx, cy, min(v.z for v in bb))
bpy.ops.object.origin_set(type='ORIGIN_CURSOR')
o.location = (0, 0, 0)
bpy.context.scene.cursor.location = (0, 0, 0)
bpy.context.view_layer.update()
result = {"dims": [round(v, 4) for v in o.dimensions]}
```

착용구·사물은 원점=바닥 중심. **무기는 예외** — `spear.glb`/`sword.glb`의 손 소켓 규약(원점이 그립, 칼날이 +X)을 따른다.

기울여 눕히는 게 자연스러운 애셋(건틀릿 등)은 여기서 X축 회전을 함께 적용한다.

---

## STEP 4 — 렌더 리그 구성

씬에 Key/Fill/Rim + 직교 카메라가 없으면 만든다. 이미 있으면 STEP 5로.

카메라 각도는 **정면 금지, 약간 측면·위에서** (X 62°, Z 35°). 리그 높이는 오브젝트 중심 높이만큼 올린다.

```python
import bpy, math, mathutils
sc = bpy.context.scene
o = bpy.data.objects[NAME]
cz = o.dimensions.z / 2
aim = mathutils.Vector((0, 0, cz))

for name, base, energy in (("Key",  (0.7, -0.9, 1.1), 45),
                           ("Fill", (-1.0, -0.6, 0.4), 14),
                           ("Rim",  (-0.4, 1.0, 0.8), 22)):
    ob = bpy.data.objects.get(name)
    if not ob:
        d = bpy.data.lights.new(name, 'AREA'); d.size = 1.5
        ob = bpy.data.objects.new(name, d); sc.collection.objects.link(ob)
    ob.data.energy = energy
    ob.location = (base[0], base[1], base[2] + cz)
    ob.rotation_euler = (aim - ob.location).to_track_quat('-Z', 'Y').to_euler()

cam = bpy.data.objects.get("Camera")
if not cam:
    cam = bpy.data.objects.new("Camera", bpy.data.cameras.new("Camera"))
    sc.collection.objects.link(cam)
sc.camera = cam
cam.data.type = 'ORTHO'
cam.rotation_euler = (math.radians(62), 0, math.radians(35))
bpy.context.view_layer.update()   # matrix_world 읽기 전 필수
cam.location = cam.matrix_world.to_3x3() @ mathutils.Vector((0, 0, 2.0)) + aim

sc.render.engine = 'CYCLES'
sc.cycles.samples = 128
sc.cycles.use_denoising = True
sc.render.film_transparent = True
sc.render.resolution_x = sc.render.resolution_y = 512
sc.render.image_settings.file_format = 'PNG'
sc.render.image_settings.color_mode = 'RGBA'
sc.view_settings.view_transform = 'Standard'
result = {"cam": [round(v, 4) for v in cam.location]}
```

---

## STEP 5 — 아이콘 렌더 (512² → 128²)

`ortho_scale`은 오브젝트 바운딩박스를 카메라 공간에 투영해 자동으로 맞춘다. 손으로 넣지 말 것.

씬에 비교용 오브젝트가 남아 있으면 `hide_render = True`로 빼고, 끝나면 되돌린다.

```python
import bpy, mathutils
sc = bpy.context.scene
o, cam = bpy.data.objects[NAME], bpy.data.objects["Camera"]
for ob in bpy.data.objects:
    if ob.type == 'MESH' and ob is not o:
        ob.hide_render = True

inv = cam.matrix_world.inverted()
pts = [inv @ (o.matrix_world @ mathutils.Vector(c)) for c in o.bound_box]
ext = max(max(abs(p.x) for p in pts), max(abs(p.y) for p in pts)) * 2
cam.data.ortho_scale = ext * 1.10        # 여백 10%

out = rf"{REPO}\client\public\items\{NAME}.png"
sc.render.filepath = out
bpy.ops.render.render(write_still=True)

img = bpy.data.images.load(out, check_existing=False)
img.scale(128, 128)
img.file_format = 'PNG'
img.save(filepath=out)
bpy.data.images.remove(img)
result = {"ortho_scale": round(cam.data.ortho_scale, 4)}
```

렌더된 PNG는 Read 툴로 **직접 눈으로 확인한다.** 잘림·각도·조명이 이상하면 다시 돌린다.

---

## STEP 6 — GLB export

```python
import bpy, os
o = bpy.data.objects[NAME]
bpy.ops.object.select_all(action='DESELECT')
o.select_set(True)
bpy.context.view_layer.objects.active = o

out = rf"{REPO}\client\public\models\<카테고리>\{NAME}.glb"
bpy.ops.export_scene.gltf(filepath=out, export_format='GLB', use_selection=True,
                          export_apply=True, export_yup=True, export_animations=False)
result = {"bytes": os.path.getsize(out)}
```

검증 — `emissiveFactor`가 없어야 하고(없으면 기본 [0,0,0]), 치수가 STEP 3 값과 맞아야 한다:

```bash
python .claude/skills/blender-item-asset/measure_glb.py client/public/models/<카테고리>/<name>.glb
```

---

## STEP 7 — 기록

`doc/assets/items.md`에 한 줄 추가한다. 기존 항목 형식을 그대로 따른다:

> `<name>.glb` — Meshy.ai (유료 생성, `<생성일>`, "`<프롬프트 제목>`"). 완전 소유권·상업 OK (characters.md License 참조). GLB를 Blender로 임포트해 `<축>` `<TARGET>`m로 스케일 적용(W×H×D, `<비교 기준>`), 원점=바닥 중심, 텍스처 512²로 축소, 검은 emissive 제거. 아이콘은 Cycles 직교 측면·위 각도 렌더 512²→128² (`<오늘 날짜>`)

`items.csv`에 아직 안 붙였으면 끝에 `아직 items.csv 미연결 **[미사용]**`을 덧붙인다.

게임에 실제로 등장시키려면 별도 작업이 필요하다 — `data/items.csv` 항목, 아이콘 경로, 필요하면 `client/public/models/objects/catalog.json` 등록.

---

## 함정

- **Meshy emissive**: 노드 삭제만으론 부족 (STEP 2). 빼먹으면 인게임에서 하얗게 발광한다.
- **기준 애셋이 "없다"고 단정하지 말 것**: `models/`는 대부분 gitignore돼 있고 체크아웃이 수십 커밋 뒤처져 있을 수 있다. `assets.lock`과 `git fetch` 먼저 확인한다 (2026-08-04에 apple.glb를 없다고 판단해 엉뚱한 기준을 쓴 적 있음).
- **Y: 네트워크 드라이브**: `ls`가 120초 넘게 걸릴 수 있다. 파일 경로를 이미 알면 목록 조회하지 말고 바로 임포트할 것.
- **텍스처 축소는 임포트 직후에**: export 후에는 GLB에 이미 구워져 있다.
- **원점 지정 전에 scale apply**: 순서가 바뀌면 바운딩박스가 스케일 반영 전 값이라 원점이 어긋난다.
- **`export_yup=True`**: 안 넣으면 인게임에서 90° 누워서 나온다.
- **비교 렌더가 제일 빠른 검증**: 새 애셋 옆에 기준 애셋을 놓고 저해상도(400², 24 samples)로 한 장 뽑아 비율을 눈으로 본다. 끝나면 기준 오브젝트 위치·표시를 원복한다.
