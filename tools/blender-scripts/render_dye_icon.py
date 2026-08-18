"""Build the cape dye bottle and write both of its assets.

The bottle is modelled here rather than sourced: it is a lathe of a profile,
so the script is shorter than the GLB it produces. Writes the ground drop
(client/public/models/objects/cape_dye.glb) and the inventory icon, 512²
rendered down to the 128² the game loads, per the icon recipe in
.claude/skills/blender-item-asset.

    ~/opt/blender/blender -b -P tools/blender-scripts/render_dye_icon.py
"""

import math
import os
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from icon_render import add_light, export_glb, principled, render_icon  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

# Pigment filling the bottle — obviously not the healing potion's crimson,
# since both sit in the same bag. Opaque like the rest of the icon set: the
# drop is thumb-sized on the ground and refraction costs more than it shows.
PIGMENT = (0.20, 0.06, 0.36, 1.0)
GLASS = (0.50, 0.56, 0.58, 1.0)
CORK = (0.42, 0.28, 0.14, 1.0)

# Outer profile, (radius, height) in metres: base, belly, shoulder, neck, lip.
PROFILE = [
    (0.000, 0.000),
    (0.062, 0.000),
    (0.075, 0.022),
    (0.078, 0.090),
    (0.060, 0.140),
    (0.026, 0.175),
    (0.024, 0.215),
    (0.030, 0.228),
    (0.026, 0.236),
    (0.000, 0.236),
]
# How far up the bottle the dye stands.
FILL = 0.135
# Where the key lights aim.
BOTTLE_MIDDLE = (0, 0, 0.11)
SEGMENTS = 48


def lathe(profile, name, material):
    """Revolve a (radius, height) profile into a closed solid."""
    verts, faces = [], []
    rings = len(profile)
    for radius, height in profile:
        for s in range(SEGMENTS):
            angle = s / SEGMENTS * math.tau
            verts.append(
                Vector((radius * math.cos(angle), radius * math.sin(angle), height))
            )
    for r in range(rings - 1):
        for s in range(SEGMENTS):
            a = r * SEGMENTS + s
            b = r * SEGMENTS + (s + 1) % SEGMENTS
            faces.append((a, b, b + SEGMENTS, a + SEGMENTS))

    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, [], faces)
    mesh.validate()
    for poly in mesh.polygons:
        poly.use_smooth = True
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    obj.data.materials.append(material)
    return obj


def split_profile(low, high):
    """The bottle's own profile between two heights, capped flat at each cut —
    the fill line splits it into the dyed body and the clear neck."""
    kept = [(r, h) for r, h in PROFILE if low < h < high]
    return [(0.0, low), (kept[0][0], low)] + kept + [(kept[-1][0], high), (0.0, high)]


def build():
    body = lathe(
        split_profile(-0.001, FILL), "cape_dye", principled("dye", PIGMENT, 0.42)
    )
    neck = lathe(split_profile(FILL, 0.237), "neck", principled("glass", GLASS, 0.18))
    cork = lathe(
        [(0.0, 0.228), (0.021, 0.230), (0.023, 0.262), (0.0, 0.264)],
        "cork",
        principled("cork", CORK, 0.75),
    )
    return body, neck, cork


def render(objects):
    add_light((0.9, -0.8, 1.3), 70, 1.4, aim=BOTTLE_MIDDLE)
    add_light((-0.9, -0.5, 0.6), 22, 1.6, aim=BOTTLE_MIDDLE)
    # The camera looks down -Z, so stand the bottle up in view and tip it
    # slightly so the icon sees a little of the shoulder. A lathe has no yaw
    # worth choosing.
    render_icon(
        objects,
        os.path.join(REPO, "client", "public", "items", "objects", "cape_dye.png"),
        (math.radians(-82), 0, 0),
        margin=1.06,
    )


def main():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    objects = list(build())
    export_glb(
        objects,
        os.path.join(REPO, "client", "public", "models", "objects", "cape_dye.glb"),
    )
    render(objects)


main()
