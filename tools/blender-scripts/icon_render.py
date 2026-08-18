"""Shared icon recipe for the item icons rendered from Blender.

Every icon in client/public/items is the same shot: the subject framed edge to
edge under an orthographic camera looking down -Z, lit by area lamps, rendered
in Cycles at 512² and downscaled to the 128² the game loads. Only the subject,
its pose and its lights differ, so those are the arguments.

Import it from a `-b -P` script:

    import importlib.util, os, sys
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    from icon_render import add_light, render_icon
"""

import bpy
from mathutils import Vector

ICON_SIZE = 128
RENDER_SIZE = 512
SAMPLES = 128


def principled(name, color, roughness, metallic=0.0):
    """A plain opaque material — what every icon subject is made of."""
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = color
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Metallic"].default_value = metallic
    return mat


def export_glb(objects, out_path) -> None:
    """Write `objects` as the item's ground-drop GLB."""
    for obj in bpy.context.scene.objects:
        obj.select_set(obj in objects)
    bpy.ops.export_scene.gltf(
        filepath=out_path,
        export_format="GLB",
        use_selection=True,
        export_apply=True,
    )
    print(f"wrote {out_path}")


def add_light(location, energy, size, aim=(0, 0, 0)) -> None:
    light = bpy.data.lights.new("key", "AREA")
    light.energy = energy
    light.size = size
    obj = bpy.data.objects.new("key", light)
    obj.location = location
    obj.rotation_euler = (
        (Vector(aim) - Vector(location)).to_track_quat("-Z", "Y").to_euler()
    )
    bpy.context.collection.objects.link(obj)


def render_icon(objects, out_path, pivot_rotation, margin=1.04) -> None:
    """Pose `objects` about a shared pivot, frame them, and write the icon.

    Lights are the caller's — add them before calling.
    """
    pivot = bpy.data.objects.new("pivot", None)
    bpy.context.collection.objects.link(pivot)
    for obj in objects:
        obj.parent = pivot
    pivot.rotation_euler = pivot_rotation

    bpy.context.view_layer.update()
    corners = [
        obj.matrix_world @ Vector(corner) for obj in objects for corner in obj.bound_box
    ]
    xs = [c.x for c in corners]
    ys = [c.y for c in corners]

    cam_data = bpy.data.cameras.new("cam")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = max(max(xs) - min(xs), max(ys) - min(ys)) * margin
    cam = bpy.data.objects.new("cam", cam_data)
    cam.location = ((min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2, 2.0)
    bpy.context.collection.objects.link(cam)
    bpy.context.scene.camera = cam

    scene = bpy.context.scene
    scene.view_settings.view_transform = "Standard"
    scene.render.engine = "CYCLES"
    scene.cycles.samples = SAMPLES
    scene.cycles.use_denoising = True
    scene.render.film_transparent = True
    scene.render.resolution_x = scene.render.resolution_y = RENDER_SIZE
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.render.filepath = out_path
    bpy.ops.render.render(write_still=True)

    # Render large and downscale here so a re-render reproduces the shipped
    # file rather than needing a second tool.
    img = bpy.data.images.load(out_path, check_existing=False)
    img.scale(ICON_SIZE, ICON_SIZE)
    img.file_format = "PNG"
    img.save(filepath=out_path)
    bpy.data.images.remove(img)
    print(f"wrote {out_path}")
