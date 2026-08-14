"""Raise or lower one animation clip in a GLB by shifting its hip track.

Use when a shipped clip sits at the wrong height for every model that plays it
— `dying` ended with the body hovering, so the whole clip needed lifting. Only
the named node's translation samples change; every other clip, the skeleton and
the meshes stay byte-identical, which matters because the runtime uses a pack's
rest pose as its retarget source (`characterAnimationUtils.ts`).

The shift is given in world metres. Pack rigs are authored at odd scales under a
rotated Armature (combat_melee is 10x under a 0.1 Armature), so the delta is
converted through the node's parent chain rather than applied to a raw axis.

Usage:
  python tools/shift-glb-clip-hips.py IN.glb CLIP DY OUT.glb [--node Hips]

Writing OUT.glb over IN.glb in place is not supported — write elsewhere and
move it afterwards.
"""

import json
import struct
import sys

GLB_MAGIC = 0x46546C67
JSON_CHUNK = 0x4E4F534A
BIN_CHUNK = 0x004E4942


def read_glb(path):
    data = open(path, "rb").read()
    magic, _, _ = struct.unpack_from("<3I", data, 0)
    if magic != GLB_MAGIC:
        raise SystemExit(f"{path} is not a GLB")
    offset = 12
    gltf = None
    binary = None
    while offset < len(data):
        length, kind = struct.unpack_from("<2I", data, offset)
        chunk = data[offset + 8 : offset + 8 + length]
        if kind == JSON_CHUNK:
            gltf = json.loads(chunk)
        elif kind == BIN_CHUNK:
            binary = bytearray(chunk)
        offset += 8 + length + (-length % 4)
    if gltf is None or binary is None:
        raise SystemExit(f"{path} has no JSON or BIN chunk")
    return gltf, binary


def write_glb(path, gltf, binary):
    text = json.dumps(gltf, separators=(",", ":")).encode()
    text += b" " * (-len(text) % 4)
    binary = bytes(binary) + b"\0" * (-len(binary) % 4)
    total = 12 + 8 + len(text) + 8 + len(binary)
    out = bytearray()
    out += struct.pack("<3I", GLB_MAGIC, 2, total)
    out += struct.pack("<2I", len(text), JSON_CHUNK) + text
    out += struct.pack("<2I", len(binary), BIN_CHUNK) + binary
    open(path, "wb").write(out)


def quat_rotate(q, v):
    x, y, z, w = q
    tx, ty, tz = 2 * (y * v[2] - z * v[1]), 2 * (z * v[0] - x * v[2]), 2 * (x * v[1] - y * v[0])
    return (
        v[0] + w * tx + y * tz - z * ty,
        v[1] + w * ty + z * tx - x * tz,
        v[2] + w * tz + x * ty - y * tx,
    )


def parent_of(gltf, index):
    for i, node in enumerate(gltf["nodes"]):
        if index in node.get("children", []):
            return i
    return None


def world_to_node_delta(gltf, node_index, world_delta):
    """A world-space delta expressed in the node's own translation units, i.e.
    undoing the rotation and scale of every ancestor."""
    chain = []
    parent = parent_of(gltf, node_index)
    while parent is not None:
        chain.append(gltf["nodes"][parent])
        parent = parent_of(gltf, parent)

    delta = world_delta
    for node in reversed(chain):  # outermost ancestor first
        scale = node.get("scale", [1, 1, 1])
        rotation = node.get("rotation", [0, 0, 0, 1])
        delta = quat_rotate([-rotation[0], -rotation[1], -rotation[2], rotation[3]], delta)
        delta = tuple(d / s for d, s in zip(delta, scale))
    return delta


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if len(args) != 4:
        raise SystemExit(__doc__)
    src, clip_name, dy, dst = args[0], args[1], float(args[2]), args[3]
    node_name = "Hips"
    if "--node" in sys.argv:
        node_name = sys.argv[sys.argv.index("--node") + 1]

    gltf, binary = read_glb(src)
    names = [n.get("name") for n in gltf["nodes"]]
    if node_name not in names:
        raise SystemExit(f"{src} has no node named {node_name}")
    node_index = names.index(node_name)

    clips = [a for a in gltf.get("animations", []) if a.get("name") == clip_name]
    if not clips:
        raise SystemExit(f"{src} has no clip named {clip_name}")
    clip = clips[0]

    channels = [
        c
        for c in clip["channels"]
        if c["target"]["path"] == "translation" and c["target"]["node"] == node_index
    ]
    if not channels:
        raise SystemExit(f"{clip_name} does not move {node_name}")

    delta = world_to_node_delta(gltf, node_index, (0.0, dy, 0.0))
    print(f"{dy:+.3f} m world = {tuple(round(d, 4) for d in delta)} in {node_name} space")

    for channel in channels:
        accessor = gltf["accessors"][clip["samplers"][channel["sampler"]]["output"]]
        if accessor["componentType"] != 5126 or accessor["type"] != "VEC3":
            raise SystemExit("only float VEC3 translation samples are supported")
        view = gltf["bufferViews"][accessor["bufferView"]]
        if view.get("byteStride") not in (None, 12):
            raise SystemExit("interleaved translation samples are not supported")
        users = [a for a in gltf["accessors"] if a.get("bufferView") == accessor["bufferView"]]
        if len(users) != 1:
            raise SystemExit("translation samples share a bufferView with another accessor")
        base = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
        for i in range(accessor["count"]):
            at = base + 12 * i
            value = struct.unpack_from("<3f", binary, at)
            struct.pack_into("<3f", binary, at, *(v + d for v, d in zip(value, delta)))
        print(f"shifted {accessor['count']} samples of {clip_name}")

    write_glb(dst, gltf, binary)
    print(f"wrote {dst}")


main()
