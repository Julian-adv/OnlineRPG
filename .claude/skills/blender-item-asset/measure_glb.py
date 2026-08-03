#!/usr/bin/env python3
"""Print the world-space bounding box of one or more GLB files, in metres.

Used to pick a scale for a new item asset by comparing it against existing ones.
    python measure_glb.py client/public/models/**/*.glb
"""
import glob
import json
import struct
import sys


def _accessor_bounds(gltf, idx):
    a = gltf["accessors"][idx]
    return a.get("min"), a.get("max")


def _node_matrix(node):
    if "matrix" in node:
        m = node["matrix"]  # column-major
        return [[m[c * 4 + r] for c in range(4)] for r in range(4)]
    t = node.get("translation", [0, 0, 0])
    r = node.get("rotation", [0, 0, 0, 1])
    s = node.get("scale", [1, 1, 1])
    x, y, z, w = r
    rot = [
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ]
    return [[rot[i][j] * s[j] for j in range(3)] + [t[i]] for i in range(3)] + [
        [0, 0, 0, 1]
    ]


def _mul(a, b):
    return [
        [sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)
    ]


def _xform(m, p):
    return [sum(m[i][j] * p[j] for j in range(3)) + m[i][3] for i in range(3)]


def measure(path):
    data = open(path, "rb").read()
    n = struct.unpack("<I", data[12:16])[0]
    gltf = json.loads(data[20 : 20 + n])
    lo = [float("inf")] * 3
    hi = [float("-inf")] * 3

    def walk(idx, parent):
        node = gltf["nodes"][idx]
        m = _mul(parent, _node_matrix(node))
        if "mesh" in node:
            for prim in gltf["meshes"][node["mesh"]]["primitives"]:
                pmin, pmax = _accessor_bounds(gltf, prim["attributes"]["POSITION"])
                if not pmin:
                    continue
                for cx in (pmin[0], pmax[0]):
                    for cy in (pmin[1], pmax[1]):
                        for cz in (pmin[2], pmax[2]):
                            w = _xform(m, [cx, cy, cz])
                            for i in range(3):
                                lo[i] = min(lo[i], w[i])
                                hi[i] = max(hi[i], w[i])
        for child in node.get("children", []):
            walk(child, m)

    ident = [[1 if i == j else 0 for j in range(4)] for i in range(4)]
    for scene in gltf.get("scenes", []):
        for root in scene.get("nodes", []):
            walk(root, ident)
    return lo, hi


if __name__ == "__main__":
    paths = []
    for arg in sys.argv[1:]:
        paths.extend(sorted(glob.glob(arg, recursive=True)) or [arg])
    for p in paths:
        try:
            lo, hi = measure(p)
        except Exception as e:  # noqa: BLE001
            print(f"{p}: {e}")
            continue
        d = [hi[i] - lo[i] for i in range(3)]
        # glTF is Y-up: X=width, Y=height, Z=depth
        print(
            f"{p:60s} W{d[0]:6.3f} H{d[1]:6.3f} D{d[2]:6.3f}  "
            f"floorY {lo[1]:+.3f}  centreXZ ({(lo[0]+hi[0])/2:+.3f}, {(lo[2]+hi[2])/2:+.3f})"
        )
