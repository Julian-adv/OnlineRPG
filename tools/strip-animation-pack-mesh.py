"""Strip the character mesh out of an animation pack GLB.

The packs are only read for their clips and their skeleton rest pose
(`client/src/lib/utils/characterAnimationUtils.ts`); the skinned mesh they carry
is never rendered. Its geometry and textures are ~90% of the file and are the
Mixamo character redistributed as a standalone file.

The skin itself has to stay: three's GLTFLoader only builds `THREE.Bone`s and a
`THREE.Skeleton` for nodes a skinned mesh references, so each mesh keeps one
degenerate triangle bound to joint 0.

Usage:
  python tools/strip-animation-pack-mesh.py IN.glb OUT.glb
"""

import json
import struct
import sys

COMPONENT_SIZE = {5120: 1, 5121: 1, 5122: 2, 5123: 2, 5125: 4, 5126: 4}
TYPE_COUNT = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT2": 4, "MAT3": 9, "MAT4": 16}


def read_glb(path):
    with open(path, "rb") as f:
        magic, _, _ = struct.unpack("<III", f.read(12))
        if magic != 0x46546C67:
            raise SystemExit(f"{path}: not a GLB")
        json_len, _ = struct.unpack("<II", f.read(8))
        gltf = json.loads(f.read(json_len))
        bin_len, _ = struct.unpack("<II", f.read(8))
        return gltf, bytearray(f.read(bin_len))


def write_glb(path, gltf, blob):
    js = json.dumps(gltf, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    blob += b"\x00" * (-len(blob) % 4)
    total = 12 + 8 + len(js) + 8 + len(blob)
    with open(path, "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, total))
        f.write(struct.pack("<II", len(js), 0x4E4F534A))
        f.write(js)
        f.write(struct.pack("<II", len(blob), 0x004E4942))
        f.write(blob)


def element_size(accessor):
    return COMPONENT_SIZE[accessor["componentType"]] * TYPE_COUNT[accessor["type"]]


def read_accessor(gltf, blob, accessor):
    """Return the accessor's data tightly packed, dropping any byteStride."""
    if "sparse" in accessor:
        raise SystemExit("sparse accessors are not supported")
    size = element_size(accessor)
    view = gltf["bufferViews"][accessor["bufferView"]]
    start = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
    stride = view.get("byteStride") or size
    if stride == size:
        return bytes(blob[start:start + size * accessor["count"]])
    return b"".join(
        bytes(blob[start + i * stride:start + i * stride + size]) for i in range(accessor["count"])
    )


def strip(src, dst):
    gltf, blob = read_glb(src)

    keep = []  # accessor index -> new index, in insertion order
    remap = {}

    def keep_accessor(index):
        if index not in remap:
            remap[index] = len(keep)
            keep.append(index)
        return remap[index]

    for animation in gltf.get("animations", []):
        for sampler in animation["samplers"]:
            sampler["input"] = keep_accessor(sampler["input"])
            sampler["output"] = keep_accessor(sampler["output"])
    for skin in gltf.get("skins", []):
        if "inverseBindMatrices" in skin:
            skin["inverseBindMatrices"] = keep_accessor(skin["inverseBindMatrices"])

    out = bytearray()
    views = []
    accessors = []
    for old in keep:
        accessor = gltf["accessors"][old]
        data = read_accessor(gltf, blob, accessor)
        out += b"\x00" * (-len(out) % 4)
        views.append({"buffer": 0, "byteOffset": len(out), "byteLength": len(data)})
        out += data
        accessors.append(
            {
                k: v
                for k, v in accessor.items()
                if k in ("componentType", "count", "type", "min", "max", "name")
            }
            | {"bufferView": len(views) - 1}
        )

    # One degenerate triangle, fully weighted to joint 0, shared by every mesh.
    out += b"\x00" * (-len(out) % 4)
    position_offset = len(out)
    out += struct.pack("<9f", *([0.0] * 9))
    joints_offset = len(out)
    out += struct.pack("<12B", *([0] * 12))
    weights_offset = len(out)
    out += struct.pack("<12f", *([1.0, 0.0, 0.0, 0.0] * 3))
    views += [
        {"buffer": 0, "byteOffset": position_offset, "byteLength": 36},
        {"buffer": 0, "byteOffset": joints_offset, "byteLength": 12},
        {"buffer": 0, "byteOffset": weights_offset, "byteLength": 48},
    ]
    base = len(accessors)
    accessors += [
        {
            "bufferView": len(views) - 3,
            "componentType": 5126,
            "count": 3,
            "type": "VEC3",
            "min": [0.0, 0.0, 0.0],
            "max": [0.0, 0.0, 0.0],
        },
        {"bufferView": len(views) - 2, "componentType": 5121, "count": 3, "type": "VEC4"},
        {"bufferView": len(views) - 1, "componentType": 5126, "count": 3, "type": "VEC4"},
    ]

    for mesh in gltf.get("meshes", []):
        mesh["primitives"] = [
            {"attributes": {"POSITION": base, "JOINTS_0": base + 1, "WEIGHTS_0": base + 2}}
        ]

    gltf["accessors"] = accessors
    gltf["bufferViews"] = views
    gltf["buffers"] = [{"byteLength": len(out)}]
    for key in ("materials", "textures", "images", "samplers", "extensionsUsed", "extensionsRequired"):
        gltf.pop(key, None)

    write_glb(dst, gltf, out)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__.strip().splitlines()[-1])
    strip(sys.argv[1], sys.argv[2])
