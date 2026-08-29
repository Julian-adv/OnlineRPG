"""Render the six dungeon floor-key icons (doc/DUNGEON_REWARD.md).

The key is modelled here — a torus bow, a shaft and one tooth per five
floors, so a 10F key shows two teeth and a 15F key three — and tinted per
dungeon. Keys are icon-only items (no ground GLB: the client floats the icon
over the drop), so this writes just the 128² icons, per the recipe in
.claude/skills/blender-item-asset.

    ~/opt/blender/blender -b -P tools/blender-scripts/render_key_icons.py
"""

import math
import os
import sys

import bpy

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from icon_render import add_light, principled, render_icon  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT_DIR = os.path.join(REPO, "client", "public", "items", "objects")

# (item id stem, base colour, roughness) per dungeon; the depth picks the teeth.
METALS = {
    "crypt_key": ((0.38, 0.36, 0.34, 1.0), 0.55),
    "orc_key": ((0.62, 0.40, 0.16, 1.0), 0.45),
    "ogre_key": ((0.10, 0.09, 0.09, 1.0), 0.40),
}
KEYS = [
    ("crypt_key", 5),
    ("orc_key", 5),
    ("orc_key", 10),
    ("ogre_key", 5),
    ("ogre_key", 10),
    ("ogre_key", 15),
]

SHAFT_LEN = 0.20
SHAFT_R = 0.014
BOW_R = 0.045
BOW_TUBE = 0.011
TOOTH = (0.022, 0.012, 0.030)
KEY_MIDDLE = (0.05, 0, 0)


def add(obj):
    bpy.context.collection.objects.link(obj)
    return obj


def build(depth, material):
    """Key lying in the XY plane: bow at -X, shaft along +X, teeth toward -Y."""
    parts = []
    bpy.ops.mesh.primitive_torus_add(
        major_radius=BOW_R, minor_radius=BOW_TUBE, major_segments=48, minor_segments=16
    )
    bow = bpy.context.active_object
    bow.location = (-BOW_R, 0, 0)
    parts.append(bow)

    bpy.ops.mesh.primitive_cylinder_add(radius=SHAFT_R, depth=SHAFT_LEN, vertices=32)
    shaft = bpy.context.active_object
    shaft.rotation_euler = (0, math.pi / 2, 0)
    shaft.location = (SHAFT_LEN / 2, 0, 0)
    parts.append(shaft)

    # Collar where the bow meets the shaft.
    bpy.ops.mesh.primitive_cylinder_add(radius=SHAFT_R * 1.6, depth=0.02, vertices=32)
    collar = bpy.context.active_object
    collar.rotation_euler = (0, math.pi / 2, 0)
    collar.location = (0.02, 0, 0)
    parts.append(collar)

    teeth = depth // 5
    for i in range(teeth):
        bpy.ops.mesh.primitive_cube_add(size=1)
        tooth = bpy.context.active_object
        tooth.scale = TOOTH
        x = SHAFT_LEN - TOOTH[0] / 2 - i * (TOOTH[0] + 0.008)
        tooth.location = (x, -TOOTH[1] / 2 - SHAFT_R * 0.6, 0)
        parts.append(tooth)

    for obj in parts:
        obj.data.materials.append(material)
        for poly in obj.data.polygons:
            poly.use_smooth = obj is not tooth if teeth else True
    return parts


def render_key(stem, depth):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    color, roughness = METALS[stem]
    parts = build(depth, principled(stem, color, roughness, metallic=0.85))
    add_light((0.6, -0.5, 1.2), 60, 1.2, aim=KEY_MIDDLE)
    add_light((-0.6, 0.4, 0.7), 25, 1.4, aim=KEY_MIDDLE)
    # Metal wants something to reflect: a neutral grey world.
    world = bpy.data.worlds.new("world")
    world.use_nodes = True
    bg = world.node_tree.nodes["Background"]
    bg.inputs[0].default_value = (0.55, 0.57, 0.62, 1.0)
    bg.inputs[1].default_value = 1.2
    bpy.context.scene.world = world
    # Diagonal in frame, bow top-left, a slight tip so the teeth read.
    render_icon(
        parts,
        os.path.join(OUT_DIR, f"{stem}_{depth}.png"),
        (math.radians(-20), 0, math.radians(-40)),
        margin=1.08,
    )


def main():
    for stem, depth in KEYS:
        render_key(stem, depth)


main()
