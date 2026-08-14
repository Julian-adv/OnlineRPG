"""Rebuild the vertical Hips curve of clips that were baked with the root pinned.

`import_mixamo_animation(bake_root_location=False)` pins Hips at its rest height
for every frame. Horizontal drift goes away, but so does the vertical bounce, and
the legs can no longer reach the ground: the character walks in the air (the
offhand torch clips floated 5-17 cm).

This grounds the clip again — per frame, the lowest foot bone is pulled to the
plant height of a reference clip by keying Hips' vertical offset.

Usage (from project root):
  blender assets/all_animation.blend --background \
      --python tools/blender-scripts/ground_hips_curve.py -- \
      --ground torch_walk,torch_idle2 --shift torch_run [--apply]

  --ground     per-frame grounding; for clips with no flight phase (walks, idles)
  --shift      one constant offset for the whole clip, so a run keeps its
               airborne frames airborne
  --clearance  metres above the reference plant height to land on (default 0).
               The soles read as sunk on a dead-flat dungeon slab when they sit
               exactly at the terrain baseline, so the torch pack carries a
               little.
Without --apply nothing is written: the measured heights are printed and the
blend is left alone.
"""

import statistics
import sys

import bpy

FOOT_BONES = ["LeftFoot", "RightFoot", "LeftToeBase", "RightToeBase"]
HIPS_PATH = 'pose.bones["Hips"].location'
VERTICAL_INDEX = 1  # bone-local Y maps 1:1 to world Z on this rig
REFERENCE_CLIP = "walk"
ARMATURE_NAME = "Armature"


def arg_list(flag):
    if flag not in sys.argv:
        return []
    return [n for n in sys.argv[sys.argv.index(flag) + 1].split(",") if n]


def fcurves(action):
    if action.is_action_layered:
        for layer in action.layers:
            for strip in layer.strips:
                for slot in action.slots:
                    cb = strip.channelbag(slot)
                    if cb:
                        yield from cb.fcurves
    else:
        yield from action.fcurves


def assign(armature, action):
    """Make `action` drive `armature`, rebinding its slot if it was authored
    elsewhere — an unbound slot leaves the rig at rest and every measurement
    below comes out constant."""
    if armature.animation_data is None:
        armature.animation_data_create()
    identifier = f"OB{armature.name}"
    for slot in action.slots:
        if slot.target_id_type == "OBJECT" and slot.identifier != identifier:
            slot.identifier = identifier
    armature.animation_data.action = action
    slots = [s for s in action.slots if s.target_id_type == "OBJECT"]
    if slots:
        armature.animation_data.action_slot = slots[0]


def foot_min_z(armature):
    evaluated = armature.evaluated_get(bpy.context.evaluated_depsgraph_get())
    return min(
        (evaluated.matrix_world @ point).z
        for name in FOOT_BONES
        for point in (evaluated.pose.bones[name].head, evaluated.pose.bones[name].tail)
    )


def profile(armature, action):
    """Lowest foot height per frame."""
    assign(armature, action)
    first, last = int(action.frame_range[0]), int(action.frame_range[1])
    heights = {}
    for frame in range(first, last + 1):
        bpy.context.scene.frame_set(frame)
        heights[frame] = foot_min_z(armature)
    return heights


def write_curve(action, deltas):
    curve = next(
        (
            fc
            for fc in fcurves(action)
            if fc.data_path == HIPS_PATH and fc.array_index == VERTICAL_INDEX
        ),
        None,
    )
    if curve is None:
        raise SystemExit(f"{action.name}: no {HIPS_PATH}[{VERTICAL_INDEX}] fcurve")
    # The deltas were measured against whatever the curve already holds, so they
    # compose with it — replacing the values instead would throw away an earlier
    # correction and re-float the clip.
    existing = {frame: curve.evaluate(frame) for frame in deltas}
    while curve.keyframe_points:
        curve.keyframe_points.remove(curve.keyframe_points[0])
    for frame, delta in deltas.items():
        key = curve.keyframe_points.insert(frame, existing[frame] + delta)
        key.interpolation = "BEZIER"
        key.handle_left_type = key.handle_right_type = "AUTO_CLAMPED"
    curve.update()


def main():
    apply = "--apply" in sys.argv
    per_frame = arg_list("--ground")
    constant = arg_list("--shift")
    if not per_frame and not constant:
        raise SystemExit("nothing to do: pass --ground and/or --shift")

    clearance = float(sys.argv[sys.argv.index("--clearance") + 1]) if "--clearance" in sys.argv else 0.0

    armature = bpy.data.objects[ARMATURE_NAME]
    plant_z = statistics.median(profile(armature, bpy.data.actions[REFERENCE_CLIP]).values())
    plant_z += clearance
    print(
        f"plant height from `{REFERENCE_CLIP}` + {clearance * 100:.1f} cm clearance:"
        f" {plant_z * 100:.2f} cm"
    )

    for name in per_frame + constant:
        action = bpy.data.actions[name]
        before = profile(armature, action)
        frames = sorted(before)
        if name in constant:
            shift = plant_z - min(before.values())
            deltas = {f: shift for f in frames}
        else:
            deltas = {f: plant_z - before[f] for f in frames}
            # A cycled clip must open and close on the same offset or it pops.
            seam = (deltas[frames[0]] + deltas[frames[-1]]) / 2
            deltas[frames[0]] = deltas[frames[-1]] = seam

        low, high = min(deltas.values()), max(deltas.values())
        print(
            f"{name:12s} foot {min(before.values()) * 100:5.1f}..{max(before.values()) * 100:5.1f} cm"
            f" -> hips {low * 100:6.1f}..{high * 100:6.1f} cm (bob {(high - low) * 100:.1f} cm)"
        )
        if not apply:
            continue

        write_curve(action, deltas)
        after = profile(armature, action)
        print(
            f"{name:12s} after: foot {min(after.values()) * 100:.1f}..{max(after.values()) * 100:.1f} cm"
        )

    if apply:
        armature.animation_data.action = None
        bpy.ops.wm.save_mainfile()
        print("saved", bpy.data.filepath)


if __name__ == "__main__":
    main()
