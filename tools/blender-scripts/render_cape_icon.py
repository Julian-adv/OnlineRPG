"""Render the inventory icon for a procedural cape.

Capes have no GLB — the worn cape is a verlet sheet built at runtime (client/
src/lib/effects/cape-rig.ts) — so the icon is modelled here instead: the same
tapered sheet, pleated and draped, with a clasped collar. Cloth colour comes
from the item's `capeColor`, so every cape's icon is this one script.

Renders 512² and writes the 128² downscale the game loads, per the icon recipe
in .claude/skills/blender-item-asset (the camera differs: the cape is modelled
hanging, so the subject is tilted into view instead of the camera orbited).

    ~/opt/blender/blender -b -P tools/blender-scripts/render_cape_icon.py \
        -- wool_cape
"""

import json
import math
import os
import sys

import bpy
from mathutils import Vector

CLASP_COLOR = (0.62, 0.44, 0.12, 1.0)

TOP_WIDTH = 0.38
BOTTOM_WIDTH = 0.62
LENGTH = 0.80
FOLDS = 5
FOLD_DEPTH = 0.055
COLLAR_ROLL = 0.10
COLLAR_RADIUS = 0.045
# The sheet is analytic and subsurf smooths it, so a coarse control grid gives
# the same silhouette at icon size.
COLS, ROWS = 61, 67

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def item_def() -> dict:
    """The item being rendered, from the generated item table."""
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    item_id = argv[0] if argv else "wool_cape"
    with open(os.path.join(REPO, "data", "items.json")) as f:
        items = json.load(f)
    if item_id not in items:
        raise SystemExit(f"no such item: {item_id}")
    return items[item_id]


def srgb_to_linear(hex_color: str) -> tuple:
    """Blender wants linear; items.csv stores the sRGB hex the client uses."""
    h = hex_color.lstrip("#")
    channels = [int(h[i : i + 2], 16) / 255 for i in (0, 2, 4)]
    return tuple(
        c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4 for c in channels
    ) + (1.0,)


def sheet_point(u: float, v: float) -> Vector:
    """One point of the draped sheet. `u` runs across; `v` runs down from the
    top edge, and negative `v` is the collar rolled over toward the viewer."""
    if v < 0:
        return collar_point(u, -v / COLLAR_ROLL)
    width = TOP_WIDTH + (BOTTOM_WIDTH - TOP_WIDTH) * v
    across = (u - 0.5) * width
    phase = u * math.pi * FOLDS
    # Pleats: sharpened sine so the ridges read at 128², deepening down the
    # length, gathering the cloth sideways and leaving a scalloped hem.
    s = math.sin(phase)
    pleat = math.copysign(abs(s) ** 0.7, s) * FOLD_DEPTH * (0.35 + 0.65 * v)
    # The sheet wraps forward around the shoulders, hardest at the edges.
    wrap = 0.20 * (across / width * 2) ** 2 * (1.0 - 0.35 * v)
    return Vector(
        (
            across + math.cos(phase) * 0.012 * v,
            -v * LENGTH + abs(s) * 0.025 * v,
            pleat + wrap + 0.06 * v * v,
        )
    )


def collar_point(u: float, t: float) -> Vector:
    """The top edge rolled forward over itself, `t` running 0→1 around the roll."""
    top = sheet_point(u, 0.0)
    a = t * math.pi
    return Vector(
        (
            top.x * (1.0 + 0.03 * math.sin(a)),
            top.y + COLLAR_RADIUS * math.sin(a),
            top.z + COLLAR_RADIUS * (1.0 - math.cos(a)),
        )
    )


def build_cape() -> bpy.types.Object:
    verts, faces = [], []
    span = 1.0 + COLLAR_ROLL
    for r in range(ROWS):
        v = -COLLAR_ROLL + r / (ROWS - 1) * span
        for c in range(COLS):
            verts.append(sheet_point(c / (COLS - 1), v))
    for r in range(ROWS - 1):
        for c in range(COLS - 1):
            a = r * COLS + c
            faces.append((a, a + 1, a + COLS + 1, a + COLS))

    mesh = bpy.data.meshes.new("wool_cape")
    mesh.from_pydata(verts, [], faces)
    mesh.validate()
    obj = bpy.data.objects.new("wool_cape", mesh)
    bpy.context.collection.objects.link(obj)

    solidify = obj.modifiers.new("Solidify", "SOLIDIFY")
    solidify.thickness = 0.008
    solidify.offset = 0
    subsurf = obj.modifiers.new("Subsurf", "SUBSURF")
    subsurf.levels = subsurf.render_levels = 2
    for poly in mesh.polygons:
        poly.use_smooth = True
    return obj


def build_clasp() -> bpy.types.Object:
    bpy.ops.mesh.primitive_uv_sphere_add(radius=0.036, segments=32, ring_count=16)
    clasp = bpy.context.object
    clasp.name = "clasp"
    clasp.location = sheet_point(0.5, -COLLAR_ROLL) + Vector((0, 0.0, 0.02))
    clasp.scale = (1.0, 1.0, 0.55)
    for poly in clasp.data.polygons:
        poly.use_smooth = True
    return clasp


def wool_material(color: tuple) -> bpy.types.Material:
    mat = bpy.data.materials.new("wool")
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = color
    bsdf.inputs["Roughness"].default_value = 0.88
    # Wool catches a rim of light at grazing angles; without it the sheet reads
    # as flat plastic at icon size.
    bsdf.inputs["Sheen Weight"].default_value = 0.6
    bsdf.inputs["Sheen Roughness"].default_value = 0.4

    # Woven grain, or the large smooth panels render as a bare gradient.
    nodes, links = mat.node_tree.nodes, mat.node_tree.links
    coord = nodes.new("ShaderNodeTexCoord")
    noise = nodes.new("ShaderNodeTexNoise")
    noise.inputs["Scale"].default_value = 260.0
    noise.inputs["Detail"].default_value = 4.0
    bump = nodes.new("ShaderNodeBump")
    bump.inputs["Strength"].default_value = 0.22
    links.new(coord.outputs["Object"], noise.inputs["Vector"])
    links.new(noise.outputs["Fac"], bump.inputs["Height"])
    links.new(bump.outputs["Normal"], bsdf.inputs["Normal"])
    return mat


def brass_material() -> bpy.types.Material:
    mat = bpy.data.materials.new("brass")
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = CLASP_COLOR
    bsdf.inputs["Metallic"].default_value = 1.0
    bsdf.inputs["Roughness"].default_value = 0.32
    return mat


def add_light(location, energy, size) -> None:
    light = bpy.data.lights.new("key", "AREA")
    light.energy = energy
    light.size = size
    obj = bpy.data.objects.new("key", light)
    obj.location = location
    obj.rotation_euler = (
        Vector((0, 0, 0)) - Vector(location)
    ).to_track_quat("-Z", "Y").to_euler()
    bpy.context.collection.objects.link(obj)


def main() -> None:
    item = item_def()
    bpy.ops.wm.read_factory_settings(use_empty=True)

    cape = build_cape()
    cape.data.materials.append(wool_material(srgb_to_linear(item["capeColor"])))
    clasp = build_clasp()
    clasp.data.materials.append(brass_material())

    # Side-and-above three-quarter view, like the other item icons. Parented to
    # an empty so the whole thing turns about the collar.
    pivot = bpy.data.objects.new("pivot", None)
    bpy.context.collection.objects.link(pivot)
    for obj in (cape, clasp):
        obj.parent = pivot
    pivot.rotation_euler = (math.radians(-12), math.radians(22), 0)

    # Frame it edge to edge like the rest of the icon set.
    bpy.context.view_layer.update()
    corners = [
        obj.matrix_world @ Vector(corner)
        for obj in (cape, clasp)
        for corner in obj.bound_box
    ]
    xs = [c.x for c in corners]
    ys = [c.y for c in corners]

    cam_data = bpy.data.cameras.new("cam")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = max(max(xs) - min(xs), max(ys) - min(ys)) * 1.04
    cam = bpy.data.objects.new("cam", cam_data)
    cam.location = ((min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2, 2.0)
    cam.rotation_euler = (0, 0, 0)
    bpy.context.collection.objects.link(cam)
    bpy.context.scene.camera = cam

    add_light((0.9, 0.7, 1.6), 70, 1.2)
    add_light((-1.1, -0.2, 0.9), 26, 1.8)

    scene = bpy.context.scene
    scene.view_settings.view_transform = "Standard"
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 128
    scene.cycles.use_denoising = True
    scene.render.film_transparent = True
    scene.render.resolution_x = scene.render.resolution_y = 512
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"

    out = os.path.join(REPO, "client", "public", "items", item["icon"])
    scene.render.filepath = out
    bpy.ops.render.render(write_still=True)

    # The icon set is 128²; render large and downscale here so a re-render
    # reproduces the shipped file rather than needing a second tool.
    img = bpy.data.images.load(out, check_existing=False)
    img.scale(128, 128)
    img.file_format = "PNG"
    img.save(filepath=out)
    bpy.data.images.remove(img)
    print(f"wrote {out}")


main()
