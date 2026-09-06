"""Render landscaping material samples as square planes with terrain materials."""

import json
import math
from pathlib import Path
import sys

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parent))
from icon_render import add_light, render_icon

REPO = Path(__file__).resolve().parents[2]
PALETTE = json.loads((REPO / "shared/palette.json").read_text())["layers"]
SAMPLES = {
    "sand": 1,
    "red_soil": 2,
    "gravel": 4,
    "pebbles": 6,
    "stone_path": 7,
    "paving": 8,
}
OUTPUT = REPO / "client/public/items/objects"
PREVIEWS = REPO / "assets/landscaping_samples"


def render_sample(name, slot):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    layer = PALETTE[slot]
    source = REPO / "client/public/textures" / f"{layer['texture']}.glb"
    bpy.ops.import_scene.gltf(filepath=str(source))
    material = next(
        material
        for obj in bpy.context.scene.objects if obj.type == "MESH"
        for material in obj.data.materials if material is not None
    )
    for obj in list(bpy.context.scene.objects):
        bpy.data.objects.remove(obj, do_unlink=True)

    bpy.ops.mesh.primitive_plane_add(size=0.5)
    plane = bpy.context.object
    plane.name = f"landscaping_palette_{name}"
    plane.data.materials.append(material)
    if layer.get("swapUv", False):
        for loop in plane.data.uv_layers.active.data:
            u, v = loop.uv
            loop.uv = (v, 1 - u)

    for location, energy in (
        ((0.7, -0.9, 1.1), 45),
        ((-1.0, -0.6, 0.4), 14),
        ((-0.4, 1.0, 0.8), 22),
    ):
        add_light(location, energy, 1.5)
    scene = bpy.context.scene
    scene.world = bpy.data.worlds.new("SampleWorld")
    scene.world.use_nodes = True
    background = scene.world.node_tree.nodes["Background"]
    background.inputs["Color"].default_value = (0.55, 0.57, 0.62, 1)
    background.inputs["Strength"].default_value = 1.2
    scene.view_settings.exposure = -1.2

    render_icon(
        [plane],
        str(OUTPUT / f"{plane.name}.png"),
        tuple(math.radians(angle) for angle in (25, -8, -12)),
        margin=1.10,
    )
    bpy.data.images["Render Result"].save_render(str(PREVIEWS / f"{plane.name}.png"))
    bpy.ops.file.pack_all()
    bpy.ops.wm.save_as_mainfile(filepath=str(PREVIEWS / f"{plane.name}.blend"))
    print(f"SAMPLE_ICON: {plane.name} <- {source.name}")


OUTPUT.mkdir(parents=True, exist_ok=True)
PREVIEWS.mkdir(parents=True, exist_ok=True)
for name, slot in SAMPLES.items():
    render_sample(name, slot)
