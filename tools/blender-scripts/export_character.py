"""Mixamo 스킨 FBX + Meshy GLB → 게임 캐릭터 GLB (client/public/models/characters/).

Mixamo가 돌려준 FBX는 스켈레톤·스킨만 쓰고, 머티리얼은 같은 메시의 Meshy GLB
(baseColor / metallicRoughness / normal)를 통째로 이식한다. FBX 쪽 텍스처는
Mixamo가 baseColor 한 장만 남기고 경로도 깨져 있다.

헤드리스:
  blender -b -P tools/blender-scripts/export_character.py -- \
      --fbx "assets/steward/Sitting Laughing.fbx" \
      --glb assets/steward/Meshy_AI_The_Master_Keykeeper_0905051902_texture.glb \
      --name steward --height 1.90 \
      --out client/public/models/characters/steward.glb \
      --blend assets/steward/steward.blend

세션 안에서:
  ARGS = ["--fbx", ..., "--out", ...]; exec(open(__file__).read())
"""

import argparse
import json
import os
import struct
import sys

import bpy
from mathutils import Matrix, Vector, kdtree

MIXAMO_PREFIX = "mixamorig:"
SMALL_MAP_SIZE = 1024   # normal / metallicRoughness; baseColor는 원본 크기 유지
UV_TOLERANCE = 1e-3


def parse_args():
    argv = globals().get("ARGS")
    if argv is None:
        argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--fbx", required=True, help="Mixamo 스킨 FBX")
    ap.add_argument("--glb", required=True, help="같은 메시의 Meshy GLB (머티리얼 소스)")
    ap.add_argument("--name", required=True)
    ap.add_argument("--height", type=float, default=1.90, help="rest pose 키 (m)")
    ap.add_argument("--out", required=True, help="출력 GLB")
    ap.add_argument("--blend", help="작업 .blend 저장 경로 (선택)")
    ap.add_argument("--double-sided", action="store_true",
                    help="열린 셸이 보이면 켠다; 기본은 backface culling")
    return ap.parse_args(argv)


def select_only(*objs):
    bpy.ops.object.select_all(action='DESELECT')
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[-1]


def import_new(op, **kw):
    before = set(bpy.data.objects.keys())
    op(**kw)
    return [o for o in bpy.data.objects if o.name not in before]


def face_signature(o):
    """정규화한 면 중심 + UV 중심. 정점 수는 임포터마다 갈리므로 면 단위로 비교한다."""
    me, M = o.data, o.matrix_world
    co = [M @ v.co for v in me.vertices]
    lo = Vector((min(c.x for c in co), min(c.y for c in co), min(c.z for c in co)))
    hi = Vector((max(c.x for c in co), max(c.y for c in co), max(c.z for c in co)))
    h = hi.z - lo.z
    mid = (lo + hi) / 2
    uv = me.uv_layers.active.data
    out = []
    for p in me.polygons:
        c = sum((co[i] for i in p.vertices), Vector()) / len(p.vertices)
        u = sum((Vector(uv[l].uv) for l in p.loop_indices), Vector((0, 0))) / len(p.loop_indices)
        out.append((Vector(((c.x - mid.x) / h, (c.y - mid.y) / h, (c.z - lo.z) / h)), u))
    return out


def check_uv_match(a, b):
    sa, sb = face_signature(a), face_signature(b)
    kd = kdtree.KDTree(len(sb))
    for i, (c, _) in enumerate(sb):
        kd.insert(c, i)
    kd.balance()
    worst = 0.0
    for c, u in sa:
        _, idx, _ = kd.find(c)
        worst = max(worst, (u - sb[idx][1]).length)
    if worst > UV_TOLERANCE:
        raise RuntimeError(f"UV mismatch between FBX and GLB meshes: max dev {worst:.5f}")
    return worst


def bake_transform(o):
    select_only(o)
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)


def strip_animation(arm):
    action = arm.animation_data.action if arm.animation_data else None
    arm.animation_data_clear()
    if action:
        bpy.data.actions.remove(action)
    for pb in arm.pose.bones:
        pb.matrix_basis = Matrix.Identity(4)


def normalise_rig(arm, mesh, height):
    """Armature 회전/스케일 apply → 키 맞춤 → 발바닥 z=0. 메시는 잠시 떼어내
    같은 변환을 자기 데이터에 굽고 다시 붙인다 (parent inverse가 identity로 남도록)."""
    select_only(mesh)
    bpy.ops.object.parent_clear(type='CLEAR_KEEP_TRANSFORM')
    bake_transform(arm)
    bake_transform(mesh)

    zs = [v.co.z for v in mesh.data.vertices]
    k = height / (max(zs) - min(zs))
    dz = -min(zs) * k
    for o in (arm, mesh):
        o.scale = (k, k, k)
        o.location.z = dz
    bake_transform(arm)
    bake_transform(mesh)

    select_only(mesh, arm)
    bpy.ops.object.parent_set(type='OBJECT', keep_transform=True)
    bpy.context.view_layer.update()
    for o in (arm, mesh):
        if any(abs(o.matrix_world[i][j] - (i == j)) > 1e-6 for i in range(4) for j in range(4)):
            raise RuntimeError(f"{o.name} matrix_world is not identity after bake")
    return k


def rename_bones(arm):
    renamed = 0
    for b in arm.data.bones:
        if b.name.startswith(MIXAMO_PREFIX):
            b.name = b.name[len(MIXAMO_PREFIX):]
            renamed += 1
    return renamed


def transplant_material(mesh, mat, name, double_sided):
    mesh.data.materials.clear()
    mesh.data.materials.append(mat)
    mat.name = name
    nt = mat.node_tree
    bsdf = next(n for n in nt.nodes if n.type == 'BSDF_PRINCIPLED')
    for sock in ("Emission Color", "Emission Strength", "Alpha"):
        for l in list(bsdf.inputs[sock].links):
            nt.links.remove(l)
    bsdf.inputs["Emission Color"].default_value = (0, 0, 0, 1)
    bsdf.inputs["Emission Strength"].default_value = 0.0
    bsdf.inputs["Alpha"].default_value = 1.0
    mat.blend_method = 'OPAQUE'
    mat.surface_render_method = 'DITHERED'
    mat.use_backface_culling = not double_sided

    suffix = {"base_color": "baseColor", "metallic_roughness": "metallicRoughness", "normal": "normal"}
    images = []
    for n in nt.nodes:
        if n.type != 'TEX_IMAGE' or not n.image:
            continue
        img = n.image
        key = next((k for k in suffix if img.name.startswith(k)), None)
        if key is None:
            raise RuntimeError(f"unexpected texture node {n.name}: {img.name}")
        img.name = f"{name}_{suffix[key]}"
        if key != "base_color" and img.size[0] > SMALL_MAP_SIZE:
            img.scale(SMALL_MAP_SIZE, SMALL_MAP_SIZE)
        images.append((img.name, list(img.size)))
    return images


def open_edges(mesh):
    counts = {}
    for p in mesh.data.polygons:
        for e in p.edge_keys:
            counts[e] = counts.get(e, 0) + 1
    return sum(1 for c in counts.values() if c == 1)


def export_glb(arm, mesh, out):
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    select_only(mesh, arm)
    bpy.ops.export_scene.gltf(
        filepath=out, export_format='GLB', use_selection=True,
        export_apply=False, export_yup=True,
        export_animations=False, export_skins=True, export_rest_position_armature=True,
        export_def_bones=False, export_leaf_bone=False, export_hierarchy_flatten_bones=False,
        export_image_format='WEBP', export_image_quality=90,
        export_tangents=False, export_morph=False,
        export_lights=False, export_cameras=False, export_extras=False,
        export_original_specular=False,
    )


def read_glb_json(path):
    with open(path, "rb") as f:
        data = f.read()
    length = struct.unpack_from("<I", data, 12)[0]
    return json.loads(data[20:20 + length]), len(data)


def strip_unit_scales(path, eps=1e-4):
    """transform_apply가 본 노드에 남기는 float 오차 scale(0.9999999…)을 걷어낸다."""
    with open(path, "rb") as f:
        data = f.read()
    json_len = struct.unpack_from("<I", data, 12)[0]
    g = json.loads(data[20:20 + json_len])
    rest = data[20 + json_len:]
    stripped = 0
    for n in g["nodes"]:
        s = n.get("scale")
        if s and all(abs(v - 1) < eps for v in s):
            del n["scale"]
            stripped += 1
    body = json.dumps(g, separators=(",", ":")).encode()
    body += b" " * ((-len(body)) % 4)
    out = (struct.pack("<4sII", b"glTF", 2, 12 + 8 + len(body) + len(rest))
           + struct.pack("<II", len(body), 0x4E4F534A) + body + rest)
    with open(path, "wb") as f:
        f.write(out)
    return stripped


def summarise(path):
    g, size = read_glb_json(path)
    pos = None
    for m in g["meshes"]:
        for p in m["primitives"]:
            a = g["accessors"][p["attributes"]["POSITION"]]
            pos = (a["min"], a["max"])
    joints = g["skins"][0]["joints"] if g.get("skins") else []
    return {
        "bytes": size,
        "nodes": len(g["nodes"]),
        "root": [g["nodes"][i]["name"] for i in g["scenes"][0]["nodes"]],
        "node_scales": [n.get("scale") for n in g["nodes"] if "scale" in n],
        "joints": len(joints),
        "joint_prefix_left": sum(1 for j in joints if MIXAMO_PREFIX in g["nodes"][j].get("name", "")),
        "materials": g.get("materials"),
        "images": [(i.get("name"), i.get("mimeType")) for i in g.get("images", [])],
        "animations": len(g.get("animations", [])),
        "bbox_min": pos[0], "bbox_max": pos[1],
        "height": round(pos[1][1] - pos[0][1], 4),
        "floorY": round(pos[0][1], 4),
    }


def main():
    args = parse_args()
    if bpy.context.object and bpy.context.object.mode != 'OBJECT':
        bpy.ops.object.mode_set(mode='OBJECT')

    fbx_objs = import_new(bpy.ops.import_scene.fbx, filepath=os.path.abspath(args.fbx))
    arm = next(o for o in fbx_objs if o.type == 'ARMATURE')
    mesh = next(o for o in fbx_objs if o.type == 'MESH' and o.parent == arm)
    glb_objs = import_new(bpy.ops.import_scene.gltf, filepath=os.path.abspath(args.glb))
    glb_mesh = next(o for o in glb_objs if o.type == 'MESH' and o.data.materials)
    bpy.context.view_layer.update()

    uv_dev = check_uv_match(mesh, glb_mesh)
    strip_animation(arm)
    scale = normalise_rig(arm, mesh, args.height)
    renamed = rename_bones(arm)
    orphan_vgroups = [g.name for g in mesh.vertex_groups if g.name not in arm.data.bones]
    if orphan_vgroups:
        raise RuntimeError(f"vertex groups without bones after rename: {orphan_vgroups}")

    old_mat = mesh.data.materials[0] if mesh.data.materials else None
    images = transplant_material(mesh, glb_mesh.data.materials[0], args.name, args.double_sided)
    boundary = open_edges(mesh)

    arm.name = arm.data.name = "Armature"
    mesh.name = mesh.data.name = args.name
    for o in glb_objs:
        bpy.data.objects.remove(o, do_unlink=True)
    if old_mat and old_mat.users == 0:
        bpy.data.materials.remove(old_mat)
    for img in list(bpy.data.images):
        if img.users == 0 and img.name not in ("Render Result", "Viewer Node"):
            bpy.data.images.remove(img)

    export_glb(arm, mesh, os.path.abspath(args.out))
    stripped = strip_unit_scales(os.path.abspath(args.out))
    if args.blend:
        os.makedirs(os.path.dirname(os.path.abspath(args.blend)), exist_ok=True)
        bpy.ops.wm.save_as_mainfile(filepath=os.path.abspath(args.blend), copy=True)

    report = {"uv_max_dev": round(uv_dev, 6), "scale": round(scale, 5), "bones_renamed": renamed,
              "bones": len(arm.data.bones), "verts": len(mesh.data.vertices),
              "open_edges": boundary, "unit_scales_stripped": stripped, "images": images,
              "glb": summarise(os.path.abspath(args.out))}
    print(json.dumps(report, indent=1, ensure_ascii=False))
    return report


result = main()
