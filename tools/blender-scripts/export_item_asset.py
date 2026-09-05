"""Export a static item's GLB, transparent icon, and packed Blender scene."""

import argparse
import json
import math
from pathlib import Path
import re
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from icon_render import add_light, render_icon

REPO = Path(__file__).resolve().parents[2]


def positive_float(value):
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError("must be finite and greater than zero")
    return number


def asset_name(value):
    if not re.fullmatch(r"[a-z][a-z0-9_]*", value):
        raise argparse.ArgumentTypeError("use a lowercase snake_case name")
    return value


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True, help="source GLB, glTF, or FBX")
    parser.add_argument("--name", type=asset_name, required=True)
    parser.add_argument("--size", type=positive_float, required=True, help="target size in meters")
    parser.add_argument("--size-axis", choices=("longest", "x", "y", "z"), default="longest",
                        help="dimension to size after rotation, in Blender Z-up coordinates")
    parser.add_argument("--category", type=asset_name, default="objects")
    parser.add_argument("--rotation", type=float, nargs=3, default=(0, 0, 0),
                        metavar=("X", "Y", "Z"), help="ground rotation in degrees")
    parser.add_argument("--icon-rotation", type=float, nargs=3, default=(28, -8, -12),
                        metavar=("X", "Y", "Z"), help="icon rotation in degrees")
    parser.add_argument("--exposure", type=float, default=0)
    parser.add_argument("--texture-size", type=int, default=512, help="maximum texture edge in pixels")
    parser.add_argument("--keep-emission", action="store_true")
    parser.add_argument("--output-root", type=Path, default=REPO, help="output checkout or staging directory")
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    args = parser.parse_args(argv)
    if not args.source.is_file():
        parser.error(f"source does not exist: {args.source}")
    if args.source.suffix.lower() not in {".glb", ".gltf", ".fbx"}:
        parser.error("source must be GLB, glTF, or FBX")
    if args.texture_size <= 0:
        parser.error("texture-size must be greater than zero")
    if not all(math.isfinite(value) for value in (*args.rotation, *args.icon_rotation, args.exposure)):
        parser.error("rotations and exposure must be finite")
    return args


args = parse_args()
source_dir = args.output_root / "assets" / args.name
model = args.output_root / "client/public/models" / args.category / f"{args.name}.glb"
icon = args.output_root / "client/public/items" / args.category / f"{args.name}.png"
if args.source.resolve() == model.resolve():
    raise ValueError("Source must be separate from the exported model")

bpy.ops.wm.read_factory_settings(use_empty=True)
if args.source.suffix.lower() == ".fbx":
    bpy.ops.import_scene.fbx(filepath=str(args.source.resolve()))
else:
    bpy.ops.import_scene.gltf(filepath=str(args.source.resolve()))
meshes = [obj for obj in bpy.context.scene.objects if obj.type == "MESH"]
if not meshes:
    raise ValueError("Source contains no meshes")
if any(obj.type == "ARMATURE" for obj in bpy.context.scene.objects):
    raise ValueError("Rigged models require the character pipeline")
if any(obj.data.shape_keys for obj in meshes):
    raise ValueError("Shape-key models are not supported by this static-item pipeline")
world_matrices = {obj: obj.matrix_world.copy() for obj in meshes}
for obj in meshes:
    obj.parent = None
    obj.matrix_world = world_matrices[obj]
for obj in list(bpy.context.scene.objects):
    if obj.type != "MESH":
        bpy.data.objects.remove(obj, do_unlink=True)
bpy.ops.object.select_all(action="DESELECT")
for obj in meshes:
    obj.select_set(True)
bpy.context.view_layer.objects.active = meshes[0]
if len(meshes) > 1:
    bpy.ops.object.join()
item = bpy.context.view_layer.objects.active
item.name = item.data.name = args.name
bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)

item.rotation_mode = "XYZ"
item.rotation_euler = tuple(math.radians(angle) for angle in args.rotation)
bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
bpy.context.view_layer.update()
dimension = max(item.dimensions) if args.size_axis == "longest" else item.dimensions["xyz".index(args.size_axis)]
if dimension <= 0:
    raise ValueError("Cannot scale a zero-sized dimension")
item.scale = (args.size / dimension,) * 3
bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
bpy.context.view_layer.update()
corners = [item.matrix_world @ Vector(corner) for corner in item.bound_box]
scene = bpy.context.scene
scene.cursor.location = (
    (min(v.x for v in corners) + max(v.x for v in corners)) / 2,
    (min(v.y for v in corners) + max(v.y for v in corners)) / 2,
    min(v.z for v in corners),
)
bpy.ops.object.origin_set(type="ORIGIN_CURSOR")
item.location = (0, 0, 0)
scene.cursor.location = (0, 0, 0)

images = set()
for material in item.data.materials:
    if material is None or not material.use_nodes:
        continue
    material.name = args.name
    nodes = material.node_tree.nodes
    if not args.keep_emission:
        for shader in (node for node in nodes if node.type == "BSDF_PRINCIPLED"):
            for name, value in (("Emission Color", (0, 0, 0, 1)), ("Emission Strength", 0)):
                socket = shader.inputs[name]
                for link in list(socket.links):
                    material.node_tree.links.remove(link)
                socket.default_value = value
    for node in nodes:
        if node.type != "TEX_IMAGE" or node.image is None:
            continue
        images.add(node.image)
for image in images:
    longest = max(image.size)
    if longest > args.texture_size:
        image.scale(*(max(1, round(edge * args.texture_size / longest)) for edge in image.size))
    image.pack()

bpy.context.view_layer.update()
source_dir.mkdir(parents=True, exist_ok=True)
model.parent.mkdir(parents=True, exist_ok=True)
icon.parent.mkdir(parents=True, exist_ok=True)
bpy.ops.export_scene.gltf(
    filepath=str(model),
    export_format="GLB",
    use_selection=True,
    export_apply=True,
    export_yup=True,
    export_animations=False,
    export_image_format="WEBP",
    export_image_quality=90,
)

render_item = item.copy()
render_item.data = item.data.copy()
render_item.name = f"{args.name}_icon"
render_item.scale = (0.5 / max(item.dimensions),) * 3
scene.collection.objects.link(render_item)
item.hide_render = True
item.hide_set(True)
for location, energy in (
    ((0.7, -0.9, 1.1), 45),
    ((-1.0, -0.6, 0.4), 14),
    ((-0.4, 1.0, 0.8), 22),
):
    add_light(location, energy, 1.5)
scene.world = bpy.data.worlds.new("IconWorld")
scene.world.use_nodes = True
background = scene.world.node_tree.nodes["Background"]
background.inputs["Color"].default_value = (0.55, 0.57, 0.62, 1)
background.inputs["Strength"].default_value = 1.2
scene.view_settings.exposure = args.exposure
render_icon(
    [render_item],
    str(icon),
    tuple(math.radians(angle) for angle in args.icon_rotation),
    margin=1.10,
)
bpy.data.images["Render Result"].save_render(str(source_dir / f"{args.name}-render.png"))
bpy.ops.wm.save_as_mainfile(filepath=str(source_dir / f"{args.name}.blend"))
print(
    "ITEM_ASSET_RESULT",
    json.dumps({
        "name": args.name,
        "dimensions_blender_xyz": list(item.dimensions),
        "scale": list(item.scale),
        "rotation": list(item.rotation_euler),
        "textures": [(image.name, list(image.size)) for image in images],
        "glb_bytes": model.stat().st_size,
    }),
)
