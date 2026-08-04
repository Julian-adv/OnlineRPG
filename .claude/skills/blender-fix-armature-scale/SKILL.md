---
name: blender-fix-armature-scale
description: Blender에서 Mixamo 캐릭터의 transform 문제를 한 번에 수정한다. (1) Armature rotation=90° X → 0, (2) Armature/Mesh scale=0.01/100 → 1, (3) rest pose 발이 지면(z=0)에 닿도록 위치 보정.
disable-model-invocation: true
argument-hint: [armature_name] [mesh_name]
---

Blender MCP를 사용해 Mixamo 캐릭터의 transform을 완전히 정규화한다.

세 가지를 순서대로 처리한다:
1. **Rotation 정규화**: Armature X축 90° rotation → 0
2. **Scale 정규화**: Armature/Mesh scale → 1.0
3. **발 위치 보정**: rest pose에서 발이 z=0에 닿도록 이동

---

## 올인원 스크립트

씬에 Armature 1개, Mesh 1개가 있는 표준 Mixamo 구조를 가정한다.
`$ARGUMENTS`로 오브젝트 이름이 주어지면 해당 이름을 사용하고, 없으면 자동으로 탐색한다.

```python
import bpy
from mathutils import Vector

# ── 오브젝트 탐색 ──────────────────────────────────────────
arm  = next(o for o in bpy.data.objects if o.type == 'ARMATURE')
mesh = next(o for o in bpy.data.objects if o.type == 'MESH')

def select_only(obj):
    bpy.ops.object.select_all(action='DESELECT')
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj

# ── STEP 1: Scale 비율 미리 기록 (Apply 전에 캡처해야 함) ──
scale_factor = arm.scale.x  # 예: 0.01

# ── STEP 2: Apply Rotation (rotation keyframe 보정 불필요) ──
select_only(arm)
bpy.ops.object.transform_apply(location=False, rotation=True, scale=False)
print(f"Rotation applied. arm.rotation={arm.rotation_euler[:]}")

# ── STEP 3: Apply Scale ─────────────────────────────────────
select_only(arm)
bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
select_only(mesh)
bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
print(f"Scale applied. arm.scale={arm.scale[:]}, mesh.scale={mesh.scale[:]}")

# ── STEP 4: location F-curve를 scale_factor 배로 보정 ───────
# Apply Scale은 bone rest 위치만 갱신하고 animation keyframe은
# 그대로 두기 때문에 직접 보정해야 함.
action = arm.animation_data.action

def iter_fcurves(action):
    """Blender 5.x Layered Action / 4.x Legacy 모두 지원"""
    if action.is_action_layered:
        for layer in action.layers:
            for strip in layer.strips:
                for slot in action.slots:
                    cb = strip.channelbag(slot)
                    if cb:
                        yield from cb.fcurves
    else:
        yield from action.fcurves

loc_count = 0
for fc in iter_fcurves(action):
    if 'location' in fc.data_path and 'pose.bones' in fc.data_path:
        for kp in fc.keyframe_points:
            kp.co[1]           *= scale_factor
            kp.handle_left[1]  *= scale_factor
            kp.handle_right[1] *= scale_factor
        fc.update()
        loc_count += 1

bpy.context.view_layer.update()
print(f"Location F-curves scaled ×{scale_factor}: {loc_count} curves")

# ── STEP 5: rest pose에서 발 위치 확인 ─────────────────────
arm.data.pose_position = 'REST'
bpy.context.view_layer.update()

foot_z_min = None
for bone in arm.data.bones:
    if any(k in bone.name for k in ['Toe_End', 'ToeBase', 'toe_end', 'toebase']):
        z = (arm.matrix_world @ bone.tail_local).z
        if foot_z_min is None or z < foot_z_min:
            foot_z_min = z

offset = -foot_z_min  # 위로 올려야 할 양 (양수)
print(f"Foot z in rest pose: {foot_z_min:.4f} → moving up by {offset:.4f}")

# ── STEP 6: Armature 위로 올리고 Apply Location ─────────────
arm.data.pose_position = 'POSE'
arm.location.z += offset
bpy.context.view_layer.update()

select_only(arm)
bpy.ops.object.transform_apply(location=True, rotation=False, scale=False)
print(f"Location applied. arm.location={arm.location[:]}")

# ── STEP 7: Root bone location keyframe 보정 ─────────────────
# Apply Location도 animation keyframe을 갱신하지 않으므로 보정 필요.
# world Z offset을 root bone의 local 좌표계로 변환해 해당 채널에서 뺀다.
R     = arm.matrix_world.to_3x3()   # rotation apply 후 identity에 가까움
R_inv = R.inverted()
root_bone = next(b for b in arm.data.bones if b.parent is None)
B         = root_bone.matrix_local.to_3x3()
B_inv     = B.inverted()

world_correction    = Vector((0, 0, -offset))
armature_local_corr = R_inv @ world_correction
bone_local_corr     = B_inv @ armature_local_corr
print(f"Root bone '{root_bone.name}' local correction: {[f'{v:.4f}' for v in bone_local_corr]}")

root_path = f'pose.bones["{root_bone.name}"].location'
for fc in iter_fcurves(action):
    if fc.data_path == root_path:
        corr = bone_local_corr[fc.array_index]
        if abs(corr) > 1e-5:
            for kp in fc.keyframe_points:
                kp.co[1]           += corr
                kp.handle_left[1]  += corr
                kp.handle_right[1] += corr
            fc.update()

bpy.context.view_layer.update()

# ── 검증 ────────────────────────────────────────────────────
arm.data.pose_position = 'REST'
bpy.context.view_layer.update()
for bone in arm.data.bones:
    if 'Toe_End' in bone.name:
        z = (arm.matrix_world @ bone.tail_local).z
        print(f"[REST]  {bone.name} z={z:.4f}  (기대: ≈0)")

arm.data.pose_position = 'POSE'
bpy.context.view_layer.update()
root_pb = arm.pose.bones[root_bone.name]
world_z = (arm.matrix_world @ root_pb.head).z
print(f"[POSE]  {root_bone.name} world z={world_z:.4f}")
print("완료.")
```

---

## 각 단계 설명

### Rotation Apply (STEP 2)
- Armature의 X축 90° 회전을 bone 방향에 굽힘
- rotation keyframe은 bone-local space 기준이라 armature 회전 변경의 영향을 받지 않음 → **keyframe 보정 불필요**

### Scale Apply (STEP 3 + 4)
- Apply Scale은 bone rest 위치는 갱신하지만 animation keyframe은 그대로 → 캐릭터가 92m 상공으로 날아감
- 해결: 모든 `pose.bones.*.location` F-curve 값을 `scale_factor`(예: 0.01) 배로 곱함

### Location Apply (STEP 6 + 7)
- 발이 지면에 닿도록 armature를 위로 올린 후 Apply Location
- Apply Location도 animation keyframe을 갱신하지 않음 → pose position에서 캐릭터가 하늘로 올라감
- 해결: root bone(Hips)의 location keyframe에서 world Z offset을 bone local space로 변환한 보정값을 뺌
- Mixamo 기준 root bone = `mixamorig:Hips`, 보정은 Y 채널(array_index=1)에만 적용됨

## 주의사항

- 작업 전 Blender에서 파일을 저장해둘 것
- 이미 rotation이나 scale이 이미 정규화된 경우, 해당 단계를 건너뛰고 필요한 단계만 실행
- Blender 5.x (Layered Action)와 4.x 이하 (Legacy Action) 모두 `iter_fcurves()` 헬퍼로 처리됨
- Mixamo가 아닌 다른 rig는 root bone 이름과 Toe bone 이름이 다를 수 있으니 확인 필요
