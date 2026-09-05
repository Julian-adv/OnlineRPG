# /// script
# dependencies = ["pillow"]
# ///
"""Rebuild client/public/textures/**/*.glb from assets/textures-src.

The client reads only the first material's base/normal/MR/AO maps
(splatLayerLoader.ts), so each GLB becomes those maps as WebP q90 plus one
placeholder triangle: the loader finds the material by traversing the scene
for a Mesh, and viewers such as Blender import nothing without one.
"""
import argparse
import io
import json
import re
import struct
from pathlib import Path

from PIL import Image

from lib.glb import read_glb, view_bytes, write_glb

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / 'assets/textures-src'
OUT = ROOT / 'client/public/textures'
QUALITY = 90
SIZE = 1024
# The terrain atlas packs palette layers at 512 (doc/SPLATMAP_V2.md); layers
# housing also draws at full size stay 1k.
PALETTE_SIZE = 512
MAP_KEYS = ('baseColorTexture', 'metallicRoughnessTexture', 'normalTexture', 'occlusionTexture')


def palette_only():
    palette = {l['texture'] for l in json.loads((ROOT / 'shared/palette.json').read_text())['layers']}
    housing = re.findall(r"glb: '([^']+)'", (ROOT / 'client/src/lib/utils/housing-textures.ts').read_text())
    return palette - set(housing)


def encode(image, max_size, alpha):
    longest = max(image.size)
    if longest > max_size:
        scale = max_size / longest
        image = image.resize((round(image.width * scale), round(image.height * scale)), Image.LANCZOS)
    buf = io.BytesIO()
    image.convert('RGBA' if alpha else 'RGB').save(buf, 'WEBP', quality=QUALITY, method=6)
    return buf.getvalue(), max(image.size)


def repack(gltf, binary, max_size):
    material = gltf['materials'][0]
    pbr = material.get('pbrMetallicRoughness', {})
    # Texture-info extensions (KHR_texture_transform) reach the game: the
    # loader hands the parsed texture through unchanged.
    used = ['EXT_texture_webp'] + [e for e in gltf.get('extensionsUsed', []) if e == 'KHR_texture_transform']
    keep_alpha = material.get('alphaMode', 'OPAQUE') != 'OPAQUE'
    blob, views, images, textures = bytearray(), [], [], []
    out_pbr, out_material = {}, {}

    def add_view(data):
        blob.extend(b'\x00' * (-len(blob) % 4))
        views.append({'buffer': 0, 'byteOffset': len(blob), 'byteLength': len(data)})
        blob.extend(data)
        return len(views) - 1

    for src, dst in ((pbr, out_pbr), (material, out_material)):
        for key, info in src.items():
            if key not in MAP_KEYS:
                continue
            image_index = gltf['textures'][info['index']]['source']
            raw = view_bytes(gltf, binary, gltf['images'][image_index]['bufferView'])
            image = Image.open(io.BytesIO(raw))
            data, size = encode(image, max_size, keep_alpha and key == 'baseColorTexture')
            images.append({'mimeType': 'image/webp', 'bufferView': add_view(data)})
            textures.append({'extensions': {'EXT_texture_webp': {'source': len(images) - 1}}})
            dst[key] = dict(info, index=len(textures) - 1)
            print(f'  {key}: {image.width}px {image.format} -> {size}px {len(data) / 1e3:.0f}KB')
    out_material['pbrMetallicRoughness'] = out_pbr

    tri = add_view(struct.pack('<9f', 0, 0, 0, 1, 0, 0, 0, 1, 0))
    gltf_out = {
        'asset': {'version': '2.0', 'generator': 'repack-material-glbs.py'},
        'extensionsUsed': used,
        'extensionsRequired': ['EXT_texture_webp'],
        'scene': 0,
        'scenes': [{'nodes': [0]}],
        'nodes': [{'mesh': 0}],
        'meshes': [{'primitives': [{'attributes': {'POSITION': 0}, 'material': 0}]}],
        'accessors': [{'bufferView': tri, 'componentType': 5126, 'count': 3,
                       'type': 'VEC3', 'min': [0, 0, 0], 'max': [1, 1, 0]}],
        'materials': [out_material],
        'textures': textures,
        'images': images,
        'bufferViews': views,
        'buffers': [{'byteLength': len(blob)}],
    }
    return gltf_out, bytes(blob)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--out', type=Path, default=OUT, help='write here instead (dry run)')
    args = ap.parse_args()
    small = palette_only()
    before = after = 0
    for path in sorted(SRC.rglob('*.glb')):
        rel = path.relative_to(SRC)
        dst = args.out / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        max_size = PALETTE_SIZE if rel.stem in small else SIZE
        print(f'== {rel} (max {max_size})')
        gltf, binary = read_glb(path)
        write_glb(dst, *repack(gltf, binary, max_size))
        a, b = path.stat().st_size, dst.stat().st_size
        before, after = before + a, after + b
        print(f'  {a / 1e6:.2f}MB -> {b / 1e6:.2f}MB')
    print(f'TOTAL {before / 1e6:.1f}MB -> {after / 1e6:.1f}MB ({100 * (1 - after / before):.1f}% smaller)')


if __name__ == '__main__':
    main()
