---
name: blender-item-asset
description: Meshy 등에서 받은 GLB/FBX를 Blender로 가져와 게임용 아이템 애셋으로 만든다 — 기존 애셋과 비교해 스케일 결정·적용, 원점 바닥 중심, Meshy emissive 제거, 텍스처 512² 축소·WebP export, client/public/models/ 로 GLB export, 128×128 아이콘 렌더, 원화 배치, pc5090으로 산출물 복사, pc5090의 doc/assets/items.md 또는 props.md에 유래 기록까지. "이 glb 임포트해서 스케일 맞춰줘", "아이템 애셋 추가", "아이콘 렌더링해줘" 같은 요청에 사용한다.
argument-hint: [입력 glb/fbx 경로] [애셋 이름]
---

# 아이템 애셋 파이프라인

헤드리스 Blender로 실행한다 — 스크립트를 하나 써서 `blender -b -P <script>.py -- <인자>`로 돌린다. (Blender MCP는 2026-08-21에 제거됐다. pc5090의 `~/opt/blender` 포터블 5.2.0 LTS가 `blender`로 잡혀 있다.)

**Meshy에서는 OBJ가 아니라 GLB로 받는다.** OBJ 포맷에는 PBR 슬롯이 없어 패키지에 `map_Kd`(베이스 컬러) 한 장만 들어온다 — metallic/roughness·normal이 통째로 빠지고, 금속·유리 부분이 납작한 색으로 나온다. 같은 모델의 GLB에는 `base_color` / `metallic_roughness` / `normal` 3장이 다 들어 있다. OBJ로 이미 받았다면 다시 받는 게 절차적 보정보다 빠르고 결과도 낫다 (2026-08-22에 물약 2개 + whetstone_oil을 이 이유로 재작업).

산출물:
- `client/public/models/<카테고리>/<name>.glb` — 원점 바닥 중심, 스케일 적용됨
- `client/public/items/<CAT>/<name>.png` — 128×128 투명 배경 아이콘 (CAT은 GLB와 동일: objects | armor | accessory | weapons)
- `doc/images/items/<name>.png` (프롭이면 `doc/images/props/`) — Meshy에 넣은 원화 원본 (있는 경우)
- `doc/assets/items.md` 또는 `props.md` 항목 — 소스·라이선스·치수·날짜

앞의 파일 3개는 STEP 8에서 pc5090으로 복사하고, 문서 항목은 STEP 9에서 **pc5090 쪽에** 쓴다 (커밋이 그쪽에서 나가므로).

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
new = [o.name for o in bpy.data.objects if o.name not in before]   # 순회는 Object를 내놓는다, 이름이 아니라
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

emissive_imgs = {n.image.name for n in nt.nodes
                 if n.type == 'TEX_IMAGE' and n.image and n.image.name.startswith("emissive")}
for l in list(bsdf.inputs["Emission Color"].links):
    nt.links.remove(l)
for n in list(nt.nodes):
    if n.type == 'TEX_IMAGE' and n.image and n.image.name.startswith("emissive"):
        nt.nodes.remove(n)
bsdf.inputs["Emission Color"].default_value = (0, 0, 0, 1)
bsdf.inputs["Emission Strength"].default_value = 0.0

for name in emissive_imgs:
    if bpy.data.images.get(name):
        bpy.data.images.remove(bpy.data.images[name])
# 이 머티리얼의 텍스처만 축소한다 — 씬에 비교용 오브젝트가 있으면 그쪽까지 건드린다
for n in nt.nodes:
    if n.type == 'TEX_IMAGE' and n.image and n.image.size[0] > 512:
        n.image.scale(512, 512)   # 2048² 원본은 아이템에 과함

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

착용구·사물은 원점=바닥 중심. **무기는 예외** — `spear.glb`/`sword.glb`의 손 소켓 규약(원점이 그립, 칼날이 +X)을 따른다:

1. 가장 긴 축을 +X로 돌린다. 어느 끝이 머리인지는 눈이 아니라 굵기로 정한다 — 양 끝 20% 구간의 최대 반경을 재서 굵은 쪽이 +X.
2. 전장을 목표값으로 스케일 적용.
3. 원점 = 자루 중심선 위, 손잡이 끝에서 전장의 **13.6%** 지점(`sword.glb` 비율). 그 X 근처 얇은 슬랩의 Y·Z 중점을 잡으면 중심선이 나온다.
4. `measure_glb.py`의 `centreXZ` X값이 `전장 × (0.5 - 0.136)`과 맞는지로 검증한다 (전장 1.5m면 +0.546).

긴 무기는 아이콘이 프레임을 못 채운다. **오브젝트가 아니라 카메라를 시선축으로 롤**시켜 대각선 구도를 만들고(사본만 렌더, export되는 GLB는 소켓 자세 그대로), 오쏘 핏은 카메라 공간에서 계산하므로 롤 뒤에도 그대로 동작한다:

```python
q = mathutils.Euler((math.radians(62), 0, math.radians(35))).to_quaternion()
cam.rotation_euler = (q @ mathutils.Quaternion((0, 0, 1), CAM_ROLL)).to_euler()
```

greatclub은 -65°였다. 각도는 20° 간격으로 한 바퀴 훑고 좁혀서 고른다.

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

# 금속은 반사할 대상이 없으면 검게 나온다 — metallic 맵이 붙은 애셋은 월드가 필수다.
# film_transparent는 카메라 광선에만 적용되므로 배경 투명은 그대로 유지된다.
world = bpy.data.worlds.new("IconWorld")
sc.world = world
world.use_nodes = True
bg = world.node_tree.nodes["Background"]
bg.inputs[0].default_value = (0.55, 0.57, 0.62, 1)
bg.inputs[1].default_value = 1.2

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

bpy.context.view_layer.update()   # STEP 4와 합쳐 실행하면 matrix_world가 옛 위치다
inv = cam.matrix_world.inverted()
pts = [inv @ (o.matrix_world @ mathutils.Vector(c)) for c in o.bound_box]
ext = max(max(abs(p.x) for p in pts), max(abs(p.y) for p in pts)) * 2
cam.data.ortho_scale = ext * 1.10        # 여백 10%
# 0.16m 오브젝트에 ortho_scale이 0.7 같은 값이면 카메라 행렬이 갱신 안 된 것이다

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
                          export_apply=True, export_yup=True, export_animations=False,
                          export_image_format='WEBP', export_image_quality=90)
result = {"bytes": os.path.getsize(out)}
```

`export_image_format='WEBP'`가 없으면 텍스처가 PNG로 나가 512²라도 파일의 대부분을 차지한다(기존 아이템 1 MB대 → WebP면 200 KB대). 노멀맵도 WebP q90으로 충분하다.

검증 — `emissiveFactor`가 없어야 하고(없으면 기본 [0,0,0]), 치수가 STEP 3 값과 맞아야 한다:

```bash
python .claude/skills/blender-item-asset/measure_glb.py client/public/models/<카테고리>/<name>.glb
```

---

## STEP 7 — 원화 배치

Meshy에 넣은 원화(ChatGPT 생성 이미지)가 있으면 원본 해상도 그대로 `doc/images/items/<name>.png`(프롭이면 `doc/images/props/`)에 둔다. 축소하지 않는다 — 기존 원화들은 2MB대다.

---

## STEP 8 — pc5090으로 복사

산출물 3개는 전부 gitignore 대상이거나(모델·아이콘) 용량이 커서, 커밋만으로는 다른 작업 머신에 안 넘어간다. 직접 복사한다.

pc5090은 `~/.ssh/config`에 별칭이 있어 pc4090에서 바로 붙는다. 리포는 `/home/jake/work/OnlineRPG`.

```bash
NAME=<asset_name>; CAT=<카테고리>   # objects | armor | accessory | weapons
IMGDIR=items                       # 프롭이면 props
DEST=pc5090:/home/jake/work/OnlineRPG

scp "client/public/models/$CAT/$NAME.glb" "$DEST/client/public/models/$CAT/"
scp "client/public/items/$CAT/$NAME.png"  "$DEST/client/public/items/$CAT/"
scp "doc/images/$IMGDIR/$NAME.png"       "$DEST/doc/images/$IMGDIR/"
```

원화가 없는 애셋이면 세 번째 줄은 건너뛴다.

sha256으로 검증한다 — 크기만 보면 전송 잘림을 놓친다:

```bash
for f in "client/public/models/$CAT/$NAME.glb" "client/public/items/$CAT/$NAME.png" "doc/images/$IMGDIR/$NAME.png"; do
  [ -f "$f" ] || continue
  l=$(sha256sum "$f" | cut -d' ' -f1)
  r=$(ssh pc5090 "sha256sum /home/jake/work/OnlineRPG/$f 2>/dev/null | cut -d' ' -f1")
  [ "$l" = "$r" ] && echo "ok   $f" || echo "FAIL $f  ($l vs ${r:-없음})"
done
```

---

## STEP 9 — pc5090에서 유래 기록

**문서는 pc5090에서 고친다.** 커밋은 그쪽에서 나가므로, 로컬에도 쓰면 같은 줄이 양쪽에 생겨 충돌한다. 로컬 리포의 `doc/assets/`는 건드리지 않는다.

어느 파일에 쓸지 먼저 정한다. **디렉터리가 아니라 용도로 갈린다** — `models/objects/`에 있어도 양쪽으로 나뉜다:

| 대상 | 파일 | 실제 사례 |
|---|---|---|
| 인벤토리에 들어가는 소지·착용 아이템 | `doc/assets/items.md` | plate_helmet, dagger, campfire_kit, apple, bread, cheese_wedge |
| 월드에만 배치되는 프롭 | `doc/assets/props.md` → `## Objects` | campfire, coin_pile_spill, torch_wall |

음식은 월드에 놓이더라도 소지품이므로 items.md다. 애매하면 가장 비슷한 기존 애셋이 어느 파일에 적혀 있는지 보고 따르거나 사용자에게 묻는다.

**추가 전에 중복부터 확인한다.** pc5090 쪽 문서가 이미 커밋 안 된 상태로 편집돼 있을 수 있다:

```bash
ssh pc5090 "cd ~/work/OnlineRPG && git status --short doc/assets/ && grep -n '<name>' doc/assets/items.md doc/assets/props.md"
```

편집은 **가져와서 → 로컬에서 고치고 → 되돌려 놓는** 순서로 한다. ssh 너머로 한글 항목을 heredoc에 밀어넣으면 따옴표가 깨진다:

```bash
scp pc5090:/home/jake/work/OnlineRPG/doc/assets/props.md /tmp/props.md   # 1) 가져오기
# 2) Read/Edit 툴로 /tmp/props.md 수정 — 주변 항목 형식을 눈으로 보고 맞춘다
scp /tmp/props.md pc5090:/home/jake/work/OnlineRPG/doc/assets/props.md   # 3) 되돌리기
ssh pc5090 "cd ~/work/OnlineRPG && git diff --stat doc/assets/"          # 4) 확인
```

항목 형식 (기존 줄을 그대로 따를 것):

> - `<name>.glb` — Meshy.ai (유료 생성, `<생성일>`, "`<프롬프트 제목>`"). 완전 소유권·상업 OK (characters.md License 참조). GLB를 Blender로 임포트해 `<축>` `<TARGET>`m로 스케일 적용(W×H×D, `<비교 기준>`), 원점=바닥 중심, 텍스처 512² WebP q90, 검은 emissive 제거. 아이콘은 Cycles 직교 측면·위 각도 렌더 512²→128² (`<오늘 날짜>`)
>     - 원화는 ChatGPT 이미지 생성 (ChatGPT Pro 20x, `<생성일>`) `![원화](../images/<name>.png)`

`items.csv`에 아직 안 붙였으면 끝에 `아직 items.csv 미연결 **[미사용]**`을 덧붙인다. 쓰이지 않게 된 애셋은 **[미사용]** 표기.

게임에 실제로 등장시키려면 별도 작업이 필요하다 — `data-src/items.csv` 항목, 아이콘 경로, 필요하면 `client/public/models/objects/catalog.json` 등록.

---

## 함정

- **Meshy emissive**: 노드 삭제만으론 부족 (STEP 2). 빼먹으면 인게임에서 하얗게 발광한다.
- **glTF 임포트는 오브젝트를 QUATERNION 회전 모드로 남긴다**: 그 상태에서 `o.rotation_euler = (0, 0, yaw)`를 넣으면 **조용히 무시된다** — `transform_apply`는 `{'FINISHED'}`를 반환하고 회전값도 0으로 지워지는데 메시는 그대로다. 요 각도를 주기 전에 `o.rotation_mode = 'XYZ'`를 반드시 먼저 지정할 것. OBJ 임포트에는 없는 함정이다 (2026-08-22 whetstone_oil에서 발견).
- **회전이 먹었는지는 렌더로 판단하지 말 것**: Cycles 노이즈 때문에 각도별 렌더가 다 미묘하게 달라 보여서, 위 버그를 스윕 이미지 8장을 보고도 못 잡았다. export한 GLB의 W/D 값이 실제로 뒤바뀌는지로 검증한다 — `measure_glb.py`.
- **기준 애셋이 "없다"고 단정하지 말 것**: `models/`는 대부분 gitignore돼 있고 체크아웃이 수십 커밋 뒤처져 있을 수 있다. `assets.lock`과 `git fetch` 먼저 확인한다 (2026-08-04에 apple.glb를 없다고 판단해 엉뚱한 기준을 쓴 적 있음).
- **Y: 네트워크 드라이브**: `ls`가 120초 넘게 걸릴 수 있다. 파일 경로를 이미 알면 목록 조회하지 말고 바로 임포트할 것.
- **텍스처 축소는 임포트 직후에**: export 후에는 GLB에 이미 구워져 있다.
- **원점 지정 전에 scale apply**: 순서가 바뀌면 바운딩박스가 스케일 반영 전 값이라 원점이 어긋난다.
- **`export_yup=True`**: 안 넣으면 인게임에서 90° 누워서 나온다.
- **비교 렌더가 제일 빠른 검증**: 새 애셋 옆에 기준 애셋을 놓고 저해상도(400², 24 samples)로 한 장 뽑아 비율을 눈으로 본다. 끝나면 기준 오브젝트 위치·표시를 원복한다.
- **복사는 커밋으로 대체되지 않는다**: 모델·아이콘은 gitignore 대상이라 푸시해도 pc5090에 안 간다. STEP 8을 건너뛰면 그쪽에서 애셋 없는 상태로 빌드가 돈다 — `measure-monster-attack-clips` 같은 생성 스크립트가 빈 결과를 뱉을 수 있다.
- **문서는 한쪽에만 쓴다**: 로컬과 pc5090 양쪽 `doc/assets/`에 같은 항목을 쓰면 나중에 중복 줄로 충돌한다. STEP 9대로 pc5090에서만 쓸 것 (2026-08-04에 실제로 양쪽에 쓸 뻔했음).
