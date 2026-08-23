"""Bake a rear-up-and-collapse death clip for scp939 from its attack clip.

939_Attack1 frames CUT rear the body up onto its hind legs; that slice becomes
the start of the clip. From HOLD the pose freezes and, over FALL, the root
pitches forward about the hips until the spine lies flat while the neck, head
and limb segments swing out to the sides (SPREAD, world directions) and the
body sinks into the floor, ending sprawled prone. Hands and feet are IK
controls parented to the root, so they get the same rotations as their limb
segments and are pinned to the limb ends. Exports only the armature + skinned
meshes (no materials) as a donor GLB; graft the clip into the shipped model
with tools/graft-glb-clip.py.

Usage:
  blender -b --python tools/blender-scripts/bake_prone_death.py -- SRC.glb OUT.glb
"""
import bpy, sys, math
from mathutils import Matrix, Quaternion, Vector

SRC, OUT = sys.argv[-2], sys.argv[-1]
SRC_CLIP, OUT_CLIP = '939_Attack1', '939_DieProne'
CUT = (45, 63)          # source frames copied to the start of the clip
HOLD = CUT[1] - CUT[0]  # pose frozen from here on
SPREAD_RANGE = (12, 34)
FALL = (HOLD, 34)
LENGTH = 44
CLEARANCE = 0.01
# How far the trunk (hips/spine-skinned vertices) sinks into the floor once down.
SINK = 0.0
TRUNK = {'Hips_50', 'Spine1_43', 'Spine2_42', 'Spine3_41'}
# Extra pitch past flat, in the same direction, so it ends rolled onto its back a bit.
FALL_EXTRA = math.radians(40)
# World direction each segment ends up pointing once down (out to the side,
# a little headward and down toward the floor).
SPREAD = {
    'Neck1_34': Vector((0, -1, -0.15)), 'Neck2_33': Vector((0, -1, -0.05)), 'Head_32': Vector((0, -1, 0)),
    'UpArm.L_39': Vector((0.9, -0.35, -0.15)), 'UpArm.R_36': Vector((-0.9, -0.35, -0.15)),
    'DownArm.L_38': Vector((0.9, -0.3, -0.1)), 'DownArm.R_35': Vector((-0.9, -0.3, -0.1)),
    'Thigh.L_49': Vector((0.9, 0.3, -0.15)), 'Thigh.R_46': Vector((-0.9, 0.3, -0.15)),
    'Shin.L_48': Vector((0.9, 0.3, 0.0)), 'Shin.R_45': Vector((-0.9, 0.3, 0.0)),
    # The shortest rotation to a mirrored target rolls the right limbs the other
    # way, leaving that foot and hand twisted up; aim the controls explicitly.
    'FootCtrl.R_127': Vector((0.12, -0.49, -0.86)), 'HandCtrl.R_73': Vector((-0.97, 0.17, -0.15)),
    'FootPad.R_124': Vector((0.97, -0.23, 0.01)), 'Hand.R_70': Vector((-0.4, -0.91, 0.13)),
}
PIN = {'HandCtrl.L_91': 'DownArm.L_38', 'HandCtrl.R_73': 'DownArm.R_35',
       'FootCtrl.L_109': 'Shin.L_48', 'FootCtrl.R_127': 'Shin.R_45'}
CTRL = {'UpArm.L_39': 'HandCtrl.L_91', 'DownArm.L_38': 'HandCtrl.L_91', 'UpArm.R_36': 'HandCtrl.R_73',
        'DownArm.R_35': 'HandCtrl.R_73', 'Thigh.L_49': 'FootCtrl.L_109', 'Shin.L_48': 'FootCtrl.L_109',
        'Thigh.R_46': 'FootCtrl.R_127', 'Shin.R_45': 'FootCtrl.R_127'}

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=SRC)
arm = next(o for o in bpy.data.objects if o.type == 'ARMATURE')
meshes = [o for o in bpy.data.objects if o.type == 'MESH' and o.find_armature() == arm]
body = max(meshes, key=lambda m: len(m.data.vertices))
scene = bpy.context.scene
root = arm.pose.bones['Root_129']
hips = arm.pose.bones['Hips_50']
neck = arm.pose.bones['Neck1_34']
Mw = arm.matrix_world
Mwi = Mw.inverted()
group_names = {g.index: g.name for g in body.vertex_groups}
trunk_idx = [v.index for v in body.data.vertices
             if v.groups and group_names[max(v.groups, key=lambda g: g.weight).group] in TRUNK]

arm.animation_data_create()
arm.animation_data.action = bpy.data.actions[SRC_CLIP]
src = {}
for f in range(CUT[0], CUT[1] + 1):
    scene.frame_set(f)
    src[f - CUT[0]] = {pb.name: pb.matrix_basis.copy() for pb in arm.pose.bones}
scene.frame_set(CUT[1])
spine = (Mw @ neck.head - Mw @ hips.head)
# Pitch about X that lays the reared spine flat, pointing forward (-Y).
FALL_ANGLE = -math.atan2(spine.z, -spine.y)
FALL_ANGLE += math.copysign(FALL_EXTRA, FALL_ANGLE)

act = bpy.data.actions.new(OUT_CLIP)
for a in bpy.data.actions:
    a.use_fake_user = a is act
arm.animation_data.action = act


def ease(t):
    t = min(1.0, max(0.0, t))
    return t * t * (3 - 2 * t)


def verts(only=None):
    dg = bpy.context.evaluated_depsgraph_get()
    vs = []
    for m in (only or meshes):
        ev = m.evaluated_get(dg)
        vs += [ev.matrix_world @ v.co for v in ev.data.vertices]
    return vs


def trunk_min_z():
    ev = body.evaluated_get(bpy.context.evaluated_depsgraph_get())
    return min((ev.matrix_world @ ev.data.vertices[i].co).z for i in trunk_idx)


def key_all(f):
    for pb in arm.pose.bones:
        pb.keyframe_insert('location', frame=f)
        pb.keyframe_insert('rotation_quaternion', frame=f)
        pb.keyframe_insert('scale', frame=f)


for f in range(0, LENGTH + 1):
    scene.frame_set(f)
    for pb in arm.pose.bones:
        pb.matrix_basis = src[min(f, HOLD)][pb.name]
    bpy.context.view_layer.update()
    k = ease((f - SPREAD_RANGE[0]) / (SPREAD_RANGE[1] - SPREAD_RANGE[0]))
    kf = ease((f - FALL[0]) / (FALL[1] - FALL[0]))
    pv = Mw @ hips.head
    R = Matrix.Translation(pv) @ Matrix.Rotation(FALL_ANGLE * kf, 4, 'X') @ Matrix.Translation(-pv)
    root.matrix = Mwi @ R @ Mw @ root.matrix
    bpy.context.view_layer.update()
    for name, target in SPREAD.items():
        pb = arm.pose.bones[name]
        head = Mw @ pb.head
        cur = (Mw @ pb.tail - head).normalized()
        q = Quaternion((1, 0, 0, 0)).slerp(cur.rotation_difference(target.normalized()), k)
        Rl = Matrix.Translation(head) @ q.to_matrix().to_4x4() @ Matrix.Translation(-head)
        for b in [pb] + [arm.pose.bones[c] for c in [CTRL.get(name)] if c]:
            b.matrix = Mwi @ Rl @ Mw @ b.matrix
            bpy.context.view_layer.update()
    for ctrl, seg in PIN.items():
        cb, sb = arm.pose.bones[ctrl], arm.pose.bones[seg]
        d = (Mw @ sb.tail - Mw @ cb.head) * k
        cb.matrix = Mwi @ Matrix.Translation(d) @ Mw @ cb.matrix
        bpy.context.view_layer.update()
    lift = (1 - k) * (CLEARANCE - min(v.z for v in verts())) + k * (-SINK - trunk_min_z())
    if abs(lift) > 0.0005:
        root.matrix = Mwi @ Matrix.Translation((0, 0, lift)) @ Mw @ root.matrix
        bpy.context.view_layer.update()
    key_all(f)
    if f % 4 == 0:
        print(f"f{f:3d} spread {k:.2f} fall {kf:.2f} head z {(Mw @ arm.pose.bones['Head_32'].head).z:+.2f}")

bpy.ops.object.select_all(action='DESELECT')
arm.select_set(True)
for m in meshes:
    m.select_set(True)
bpy.ops.export_scene.gltf(filepath=OUT, export_format='GLB', use_selection=True,
    export_animations=True, export_animation_mode='ACTIONS',
    export_image_format='NONE', export_materials='NONE', export_skins=True)
