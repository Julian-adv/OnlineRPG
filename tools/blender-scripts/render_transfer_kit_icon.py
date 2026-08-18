"""Build the cape transfer kit and write both of its assets.

A waxed sheet rolled and corded, with the pot of fixative that sets it. Both
are primitives, so the script is shorter than the GLB it produces. Writes the
ground drop (client/public/models/objects/cape_transfer_kit.glb) and the
inventory icon, 512² rendered down to the 128² the game loads, per the icon
recipe in .claude/skills/blender-item-asset.

    ~/opt/blender/blender -b -P tools/blender-scripts/render_transfer_kit_icon.py
"""

import math
import os
import sys

import bpy

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from icon_render import add_light, export_glb, principled, render_icon  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

WAX = (0.72, 0.63, 0.42, 1.0)
ROLL_CORE = (0.44, 0.37, 0.24, 1.0)
CORD = (0.34, 0.22, 0.12, 1.0)
POT = (0.44, 0.22, 0.13, 1.0)
FIXATIVE = (0.68, 0.74, 0.78, 1.0)

ROLL_LENGTH = 0.24
ROLL_RADIUS = 0.036
POT_RADIUS = 0.045
POT_HEIGHT = 0.062
KIT_MIDDLE = (0, 0, 0.045)


def shade(obj, material):
    obj.data.materials.append(material)
    for poly in obj.data.polygons:
        poly.use_smooth = True
    return obj


def build():
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=40,
        radius=ROLL_RADIUS,
        depth=ROLL_LENGTH,
        rotation=(0, math.radians(90), 0),
        location=(-0.03, 0.0, ROLL_RADIUS),
    )
    roll = shade(bpy.context.object, principled("wax", WAX, 0.62))
    roll.name = "sheet"

    # Two bands rather than one: a single cord reads as a seam at 128², two
    # read as something tied shut.
    cords = []
    for i, x in enumerate((-0.10, 0.04)):
        bpy.ops.mesh.primitive_torus_add(
            major_radius=ROLL_RADIUS + 0.002,
            minor_radius=0.006,
            major_segments=32,
            minor_segments=12,
            rotation=(0, math.radians(90), 0),
            location=(x, 0.0, ROLL_RADIUS),
        )
        cord = shade(bpy.context.object, principled(f"cord{i}", CORD, 0.85))
        cord.name = f"cord{i}"
        cords.append(cord)

    # The rolled-up end, sunk in and darker: without it the flat cap reads as
    # a peg rather than as something rolled.
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=32,
        radius=ROLL_RADIUS - 0.006,
        depth=0.012,
        rotation=(0, math.radians(90), 0),
        location=(-0.03 - ROLL_LENGTH / 2 + 0.004, 0.0, ROLL_RADIUS),
    )
    core = shade(bpy.context.object, principled("core", ROLL_CORE, 0.80))
    core.name = "core"

    bpy.ops.mesh.primitive_cylinder_add(
        vertices=32,
        radius=POT_RADIUS,
        depth=POT_HEIGHT,
        location=(0.14, -0.03, POT_HEIGHT / 2),
    )
    pot = shade(bpy.context.object, principled("pot", POT, 0.70))
    pot.name = "pot"

    # The fixative sits proud of the pot's top so the icon sees it: a closed
    # cylinder would hide anything modelled inside.
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=32,
        radius=POT_RADIUS - 0.007,
        depth=0.004,
        location=(0.14, -0.03, POT_HEIGHT + 0.001),
    )
    fixative = shade(bpy.context.object, principled("fixative", FIXATIVE, 0.22))
    fixative.name = "fixative"

    # Rim around it, so the pot reads as open rather than corked.
    bpy.ops.mesh.primitive_torus_add(
        major_radius=POT_RADIUS - 0.002,
        minor_radius=0.005,
        major_segments=32,
        minor_segments=12,
        location=(0.14, -0.03, POT_HEIGHT),
    )
    rim = shade(bpy.context.object, principled("rim", POT, 0.70))
    rim.name = "rim"

    return [roll, core, *cords, pot, fixative, rim]


def render(objects):
    add_light((0.7, -0.9, 1.1), 40, 1.4, aim=KIT_MIDDLE)
    add_light((-0.8, -0.6, 0.5), 20, 1.6, aim=KIT_MIDDLE)
    # Tipped up towards the camera and turned a little, so the icon sees the
    # roll's length and the pot beside it rather than a plan view.
    render_icon(
        objects,
        os.path.join(
            REPO, "client", "public", "items", "objects", "cape_transfer_kit.png"
        ),
        (math.radians(-62), 0, math.radians(-26)),
        margin=1.06,
    )


def main():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    objects = build()
    export_glb(
        objects,
        os.path.join(
            REPO, "client", "public", "models", "objects", "cape_transfer_kit.glb"
        ),
    )
    render(objects)


main()
