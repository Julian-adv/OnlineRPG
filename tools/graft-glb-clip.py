"""Graft one animation clip from a donor GLB into a target GLB.

Use when a clip must be added to a shipped animation pack whose source
`.blend` no longer matches it. `tools/blender-scripts/export_animations.py`
rebuilds a pack from `all_animation.blend` and would silently drop every clip
the blend lacks; importing the pack into Blender and re-exporting instead
costs a frame per clip to keyframe quantisation, and rescales every clip if
the scene fps differs from the pack's.

This appends only new bufferViews/accessors/samplers plus one animation
entry, so existing clips and the skeleton stay byte-identical. That matters:
the runtime uses a pack's skeleton rest pose as the retarget source
(`client/src/lib/utils/characterAnimationUtils.ts`).

Channel targets are remapped by node NAME, so donor and target must share
skeleton naming. Channels whose node is absent from the target are dropped
and reported.

Usage:
  python tools/graft-glb-clip.py TARGET.glb DONOR.glb CLIP_NAME OUT.glb

Writing OUT.glb over TARGET.glb in place is not supported — write elsewhere
and move it afterwards.
"""

import json
import os
import struct
import sys

COMP_SIZE = {5120: 1, 5121: 1, 5122: 2, 5123: 2, 5125: 4, 5126: 4}
NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}


def read_glb(path):
    data = open(path, "rb").read()
    if data[:4] != b"glTF":
        raise SystemExit(f"{path} is not a GLB")
    pos, gltf, blob = 12, None, b""
    while pos < len(data):
        length, ctype = struct.unpack("<I4s", data[pos : pos + 8])
        chunk = data[pos + 8 : pos + 8 + length]
        if ctype.strip(b"\x00") == b"JSON":
            gltf = json.loads(chunk.decode("utf-8"))
        else:
            blob = chunk
        pos += 8 + length
    if gltf is None:
        raise SystemExit(f"{path} has no JSON chunk")
    return gltf, blob


def write_glb(path, gltf, blob):
    js = json.dumps(gltf, separators=(",", ":")).encode("utf-8")
    js += b" " * ((4 - len(js) % 4) % 4)
    blob += b"\x00" * ((4 - len(blob) % 4) % 4)
    total = 12 + 8 + len(js) + 8 + len(blob)
    with open(path, "wb") as f:
        f.write(struct.pack("<4sII", b"glTF", 2, total))
        f.write(struct.pack("<I4s", len(js), b"JSON"))
        f.write(js)
        f.write(struct.pack("<I4s", len(blob), b"BIN\x00"))
        f.write(blob)
    return total


def accessor_bytes(gltf, blob, index):
    acc = gltf["accessors"][index]
    view = gltf["bufferViews"][acc["bufferView"]]
    width = NCOMP[acc["type"]] * COMP_SIZE[acc["componentType"]]
    stride = view.get("byteStride")
    if stride not in (None, width):
        raise SystemExit(f"accessor {index} uses byteStride={stride}; not supported")
    start = view.get("byteOffset", 0) + acc.get("byteOffset", 0)
    return blob[start : start + acc["count"] * width], acc


def graft(target_path, donor_path, clip_name, out_path):
    if os.path.abspath(target_path) == os.path.abspath(out_path):
        raise SystemExit("refusing to write over the target; pick a different OUT.glb")

    target, target_blob = read_glb(target_path)
    donor, donor_blob = read_glb(donor_path)

    clip = next(
        (a for a in donor.get("animations", []) if a.get("name") == clip_name), None
    )
    if clip is None:
        have = [a.get("name") for a in donor.get("animations", [])]
        raise SystemExit(f"clip {clip_name!r} not in donor; donor has {have}")
    if any(a.get("name") == clip_name for a in target.get("animations", [])):
        raise SystemExit(f"target already has a clip named {clip_name!r}")

    node_by_name = {}
    for i, node in enumerate(target.get("nodes", [])):
        if node.get("name"):
            node_by_name.setdefault(node["name"], i)

    target.setdefault("bufferViews", [])
    target.setdefault("accessors", [])
    target.setdefault("animations", [])
    blob = bytearray(target_blob)
    blob.extend(b"\x00" * ((4 - len(blob) % 4) % 4))
    copied = {}

    def copy_accessor(index):
        if index in copied:
            return copied[index]
        payload, acc = accessor_bytes(donor, donor_blob, index)
        blob.extend(b"\x00" * ((4 - len(blob) % 4) % 4))
        target["bufferViews"].append(
            {"buffer": 0, "byteOffset": len(blob), "byteLength": len(payload)}
        )
        blob.extend(payload)
        new_acc = {
            "bufferView": len(target["bufferViews"]) - 1,
            "componentType": acc["componentType"],
            "count": acc["count"],
            "type": acc["type"],
        }
        for key in ("min", "max", "normalized"):
            if key in acc:
                new_acc[key] = acc[key]
        target["accessors"].append(new_acc)
        copied[index] = len(target["accessors"]) - 1
        return copied[index]

    samplers = []
    for sampler in clip["samplers"]:
        samplers.append(
            {
                "input": copy_accessor(sampler["input"]),
                "output": copy_accessor(sampler["output"]),
                "interpolation": sampler.get("interpolation", "LINEAR"),
            }
        )

    channels, dropped = [], []
    for channel in clip["channels"]:
        name = donor["nodes"][channel["target"]["node"]].get("name")
        index = node_by_name.get(name)
        if index is None:
            dropped.append(name)
            continue
        channels.append(
            {
                "sampler": channel["sampler"],
                "target": {"node": index, "path": channel["target"]["path"]},
            }
        )

    if not channels:
        raise SystemExit("no channels matched a target node — check skeleton naming")

    target["animations"].append(
        {"name": clip_name, "samplers": samplers, "channels": channels}
    )
    target["buffers"][0]["byteLength"] = len(blob)

    total = write_glb(out_path, target, bytes(blob))
    print(
        f"grafted {clip_name!r}: {len(channels)} channels, "
        f"{len(samplers)} samplers, {len(copied)} accessors"
    )
    if dropped:
        print(f"  dropped (node absent from target): {sorted(set(dropped))}")
    print(f"  {out_path}: {total} bytes (target was {os.path.getsize(target_path)})")


if __name__ == "__main__":
    if len(sys.argv) != 5:
        raise SystemExit(__doc__)
    graft(*sys.argv[1:5])
