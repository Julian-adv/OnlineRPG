"""
import_mixamo_animation.py
==========================
Mixamo FBX 애니메이션 하나를 import하고, A-pose 스켈레톤에서
T-pose 타겟 Armature로 retargeting bake한 뒤 정리까지 자동화하는 스크립트.

사용법 (Blender Text Editor 또는 Python Console):
  import sys
  sys.path.insert(0, r"C:\\Users\\jake\\work\\OnlineRPG\\tools\\blender-scripts")
  from import_mixamo_animation import import_mixamo_animation
  import_mixamo_animation(
      fbx_path=r"Y:\\public\\web_downloads\\Standing Torch Walk.fbx",
      action_name="torch_walk",
  )

동작:
  1. FBX import (자동으로 Armature.XXX 생성, A-pose rest)
  2. fix_mixamo_transforms 실행 (rotation/scale apply, 발 지면 정렬)
  3. Armature.XXX (A-pose) → TARGET_ARMATURE (T-pose) retarget bake
     - 본별 rotation_quaternion을 rest-pose 차이만큼 보정해 다시 keyframe
     - Hips location도 bake (rest 대비 월드 변위를 두 리그의 Hips rest 높이 비로
       스케일). 체중 이동처럼 골반이 움직이는 클립에 필요하다.
       bake_root_location=False로 끄면 rest에 고정된다.
  4. 액션 이름 rename + 슬롯 식별자를 TARGET_ARMATURE에 맞춤
  5. 임시로 들어온 Armature.XXX와 mesh, 중간 액션 제거
  6. .blend 파일 저장

주의:
  - Mixamo에서 FBX 다운로드시 "Without Skin" 권장 (스켈레톤만 필요)
  - 30 FPS, Keyframe Reduction: none 권장
  - 실제로 이동하는 클립은 Hips location이 그대로 bake되므로 캐릭터가 제자리를
    벗어난다. 이동 계열은 Mixamo에서 "In Place"를 켜고 받을 것.
"""

import importlib.util
import os
import re
from pathlib import Path

import bpy
from mathutils import Vector

TARGET_ARMATURE_NAME = "Armature"
_FIX_SCRIPT_PATH = Path(__file__).parent / "fix_mixamo_transforms.py"


def _run_fix_mixamo_transforms(target_armature_name: str) -> None:
    """fix_mixamo_transforms.py를 지정 armature 대상으로 실행."""
    src = _FIX_SCRIPT_PATH.read_text(encoding="utf-8")
    src = src.replace(
        "TARGET_ARMATURE = None",
        f'TARGET_ARMATURE = "{target_armature_name}"',
    )
    module_globals = {"__name__": "__fix_mixamo_temp__", "__file__": str(_FIX_SCRIPT_PATH)}
    exec(compile(src, str(_FIX_SCRIPT_PATH), "exec"), module_globals)


def _rest_local_to_parent_quat(bone):
    """본의 rest 포즈를 부모 rest 기준으로 quaternion으로 반환."""
    if bone.parent:
        return (bone.parent.matrix_local.inverted() @ bone.matrix_local).to_quaternion()
    return bone.matrix_local.to_quaternion()


def _retarget_bake(
    source_arm,
    source_action,
    target_arm,
    out_action_name: str,
    bake_root_location: bool = True,
    bone_aliases: dict | None = None,
) -> bpy.types.Action:
    """source_arm의 pose를 target_arm에 맞춰 retarget하여 새 액션 생성.

    회전: target_basis = target_rest_local.inv() × source_rest_local × source_basis
    (본-부모 상대 rest 차이 보정)

    루트 본(Hips) 위치: rest로부터의 월드 변위를 두 리그의 Hips rest 높이 비로
    스케일해 target에 옮긴다. bake_root_location=False면 rest에 고정한다
    (제자리 클립에서 미세한 드리프트를 없애고 싶을 때).
    """
    # Source action을 source armature에 할당 (frame_set이 pose를 평가할 수 있도록)
    if not source_arm.animation_data:
        source_arm.animation_data_create()
    source_arm.animation_data.action = source_action
    if (
        source_arm.animation_data.action_slot is None
        and len(source_action.slots) > 0
    ):
        source_arm.animation_data.action_slot = source_action.slots[0]

    # 본 이름 매핑: source의 mixamorig: 프리픽스를 제거한 이름 = target 본 이름
    prefix_pat = re.compile(r"^mixamorig\d*:")
    src_name_by_stripped = {
        prefix_pat.sub("", b.name): b.name for b in source_arm.pose.bones
    }
    # Mixamo 이름을 벗어난 리그를 위한 수동 매핑(source 본 -> target 본). 이름이
    # 아니라 계층 위치로 짝지어야 하는 경우가 있다 — 척추 순서가 뒤집힌 리그를
    # 이름만 보고 맞추면 상체가 비틀린다.
    for src_bone, tgt_bone in (bone_aliases or {}).items():
        if src_bone in source_arm.pose.bones:
            src_name_by_stripped[tgt_bone] = src_bone
        else:
            print(f"  WARNING: alias source bone '{src_bone}' not in this rig")
    shared = [
        (tb.name, src_name_by_stripped[tb.name])
        for tb in target_arm.pose.bones
        if tb.name in src_name_by_stripped
    ]
    print(
        f"  Shared bones: {len(shared)}/{len(target_arm.pose.bones)} "
        f"(source has {len(source_arm.pose.bones)})"
    )

    # 각 본의 rest-local-to-parent quaternion 미리 계산
    src_rest_q = {
        sn: _rest_local_to_parent_quat(source_arm.data.bones[sn]) for _, sn in shared
    }
    tgt_rest_q = {
        tn: _rest_local_to_parent_quat(target_arm.data.bones[tn]) for tn, _ in shared
    }

    # 기존 동일 이름 액션 제거 후 새로 생성, target armature에 바인딩
    if bpy.data.actions.get(out_action_name):
        bpy.data.actions.remove(bpy.data.actions[out_action_name])
    new_action = bpy.data.actions.new(out_action_name)
    try:
        new_action.slots.new(id_type="OBJECT", name=target_arm.name)
    except Exception:
        pass
    if not target_arm.animation_data:
        target_arm.animation_data_create()
    target_arm.animation_data.action = new_action
    if len(new_action.slots) > 0:
        target_arm.animation_data.action_slot = new_action.slots[0]
        try:
            new_action.slots[0].identifier = f"OB{target_arm.name}"
        except Exception as e:
            print(f"  WARNING: failed to set slot identifier: {e}")

    # 루트 본(Hips) 위치 bake 준비: 두 리그의 Hips rest 높이 비로 변위를 스케일
    root_pair = None
    root_scale = 1.0
    if bake_root_location:
        root_tgt = next((b.name for b in target_arm.data.bones if b.parent is None), None)
        root_src = next((sn for tn, sn in shared if tn == root_tgt), None)
        if root_tgt is None or root_src is None:
            print("  WARNING: 루트 본 매칭 실패 — location bake 생략")
        elif source_arm.data.bones[root_src].parent is not None:
            print(f"  WARNING: source '{root_src}'가 루트가 아님 — location bake 생략")
        else:
            root_pair = (root_tgt, root_src)
            src_rest_z = (source_arm.matrix_world @ source_arm.data.bones[root_src].head_local).z
            tgt_rest_z = (target_arm.matrix_world @ target_arm.data.bones[root_tgt].head_local).z
            if abs(src_rest_z) > 1e-6:
                root_scale = tgt_rest_z / src_rest_z
            print(
                f"  Root location bake: {root_src} -> {root_tgt}, "
                f"rest z {src_rest_z:.4f} -> {tgt_rest_z:.4f} (×{root_scale:.4f})"
            )

    frame_start = int(source_action.frame_range[0])
    frame_end = int(source_action.frame_range[1])
    print(f"  Baking frames {frame_start}..{frame_end}")

    scene = bpy.context.scene
    for frame in range(frame_start, frame_end + 1):
        scene.frame_set(frame)
        bpy.context.view_layer.update()
        # 각 본에 retargeted basis 적용
        for tgt_name, src_name in shared:
            sb = source_arm.pose.bones[src_name]
            source_basis_q = sb.rotation_quaternion.copy()
            target_basis_q = (
                tgt_rest_q[tgt_name].inverted()
                @ src_rest_q[src_name]
                @ source_basis_q
            )
            target_arm.pose.bones[tgt_name].rotation_quaternion = target_basis_q

        if root_pair:
            root_tgt, root_src = root_pair
            src_bone = source_arm.data.bones[root_src]
            tgt_bone = target_arm.data.bones[root_tgt]
            src_rest_w = source_arm.matrix_world @ src_bone.head_local
            src_posed_w = source_arm.matrix_world @ source_arm.pose.bones[root_src].head
            tgt_rest_w = target_arm.matrix_world @ tgt_bone.head_local
            desired_w = tgt_rest_w + (src_posed_w - src_rest_w) * root_scale
            desired_arm = target_arm.matrix_world.inverted() @ desired_w
            target_arm.pose.bones[root_tgt].location = (
                tgt_bone.matrix_local.inverted() @ desired_arm
            )

        # keyframe 삽입
        bpy.context.view_layer.update()
        for tgt_name, _ in shared:
            target_arm.pose.bones[tgt_name].keyframe_insert(
                "rotation_quaternion", frame=frame
            )
        if root_pair:
            target_arm.pose.bones[root_pair[0]].keyframe_insert("location", frame=frame)

    if not root_pair and "Hips" in target_arm.pose.bones:
        target_arm.pose.bones["Hips"].location = (0, 0, 0)

    return new_action


def _cleanup_source(source_arm_name: str, source_action_name: str) -> None:
    """Import로 생긴 임시 armature/mesh/action 제거."""
    src = bpy.data.objects.get(source_arm_name)
    if src:
        if src.animation_data:
            src.animation_data.action = None
        for child in [c for c in bpy.data.objects if c.parent == src]:
            bpy.data.objects.remove(child, do_unlink=True)
        bpy.data.objects.remove(src, do_unlink=True)
        print(f"  Removed source armature: {source_arm_name}")

    tmp = bpy.data.actions.get(source_action_name)
    if tmp:
        bpy.data.actions.remove(tmp)
        print(f"  Removed intermediate action: {source_action_name}")


def import_mixamo_animation(
    fbx_path: str,
    action_name: str,
    target_armature_name: str = TARGET_ARMATURE_NAME,
    save: bool = True,
    bake_root_location: bool = True,
    bone_aliases: dict | None = None,
) -> None:
    """Mixamo FBX를 import하여 target armature용 action으로 변환.

    Args:
        fbx_path: FBX 파일 경로
        action_name: 생성할 action 이름 (예: "torch_walk")
        target_armature_name: 타겟 T-pose armature (기본: "Armature")
        save: 완료 후 .blend 파일 저장 여부
        bake_root_location: Hips location을 bake할지 (기본 True)
        bone_aliases: Mixamo 이름이 아닌 리그용 {source 본: target 본} 매핑

    """
    if not os.path.isfile(fbx_path):
        raise FileNotFoundError(f"FBX file not found: {fbx_path}")

    target_arm = bpy.data.objects.get(target_armature_name)
    if not target_arm or target_arm.type != "ARMATURE":
        raise RuntimeError(f"Target armature '{target_armature_name}' not found")

    print(f"\n=== Importing '{fbx_path}' as action '{action_name}' ===")

    # --- Step 1: FBX import ---
    before_objects = set(o.name for o in bpy.data.objects)
    before_actions = set(a.name for a in bpy.data.actions)

    bpy.ops.object.select_all(action="DESELECT")
    bpy.ops.import_scene.fbx(
        filepath=fbx_path,
        automatic_bone_orientation=False,
        primary_bone_axis="Y",
        secondary_bone_axis="X",
        ignore_leaf_bones=False,
        use_anim=True,
    )

    new_objects = set(o.name for o in bpy.data.objects) - before_objects
    new_actions = set(a.name for a in bpy.data.actions) - before_actions

    imported_arm_name = next(
        (n for n in new_objects if bpy.data.objects[n].type == "ARMATURE"),
        None,
    )
    if imported_arm_name is None:
        raise RuntimeError("FBX import did not create an armature")
    if not new_actions:
        raise RuntimeError("FBX import did not create any animation action")
    # 한 FBX에 액션이 여러 개 들어오기도 한다(1프레임 baselayer 더미 등).
    # 알파벳순 첫 번째는 임의라, 프레임 폭이 가장 넓은 것을 실제 클립으로 본다.
    imported_action_name = max(
        new_actions,
        key=lambda n: bpy.data.actions[n].frame_range[1] - bpy.data.actions[n].frame_range[0],
    )
    print(f"  Imported: armature='{imported_arm_name}', action='{imported_action_name}'")

    # --- Step 2: Normalize transforms (X-90° rotation, 0.01 scale, ground-align feet) ---
    print("\n[Step 2] Running fix_mixamo_transforms...")
    _run_fix_mixamo_transforms(imported_arm_name)

    # --- Step 3: Retarget bake A-pose → T-pose ---
    print("\n[Step 3] Retargeting to T-pose skeleton...")
    source_arm = bpy.data.objects[imported_arm_name]
    source_action = bpy.data.actions[imported_action_name]
    _retarget_bake(
        source_arm, source_action, target_arm, action_name, bake_root_location,
        bone_aliases
    )

    # --- Step 4: Cleanup ---
    print("\n[Step 4] Cleaning up...")
    _cleanup_source(imported_arm_name, imported_action_name)

    # 타겟의 active action 해제 (파일에 저장될 active action을 남기지 않음)
    if target_arm.animation_data:
        target_arm.animation_data.action = None

    # --- Step 5: Save ---
    if save:
        bpy.ops.wm.save_mainfile()
        print(f"\nSaved: {bpy.data.filepath}")

    print(f"\n✓ Done. Action '{action_name}' is ready for export.")
    print(
        f"  다음 단계: tools/blender-scripts/export_animations.py의 EXPORT_PACKS에 "
        f"'{action_name}' 추가 후 export 실행."
    )


if __name__ == "__main__":
    # 직접 실행 시 간단한 테스트
    print("이 스크립트는 import_mixamo_animation(fbx_path, action_name) 함수를 제공합니다.")
    print("docstring을 참고하세요.")
