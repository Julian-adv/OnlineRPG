# /// script
# dependencies = ["pillow", "numpy"]
# ///
"""Shrink embedded GLB textures that carry no visual payload for their cost.

Meshy/Tripo exports ship normal and metallicRoughness maps as 2048² RGBA PNG,
which is 9-10 MB of noise per character that PNG cannot compress and the GPU
expands to ~21 MB of VRAM anyway. Per material this script:

  - normal map: flat -> drop the texture; otherwise JPEG q92 4:4:4, max 1024²
  - metallicRoughness: constant -> fold into the factors and drop; otherwise
    JPEG q90 4:4:4, max 1024²
  - KHR_materials_specular specularTexture: the spec reads strength from the
    alpha channel only, so an alpha-constant map is a no-op -> fold and drop
  - base color: JPEG q92 4:4:4, kept at its authored size unless --base-max
    says otherwise, and only when the material is opaque and the alpha channel
    is constant (JPEG has no alpha to give back)
  - textures/images/samplers/bufferViews left unreferenced are pruned

Re-encoding is skipped whenever it would not shrink the image. Verified against
the Khronos validator and A/B renders: at the scale the game draws characters
the change is not visible (>=50 dB PSNR).

Run:  uv run tools/repack-glb-textures.py client/public/models/characters
      uv run tools/repack-glb-textures.py <src-dir> --out <dir>   (dry run)
"""
import argparse
import io
import sys
from pathlib import Path

import numpy as np
from PIL import Image

from lib.glb import read_glb, view_bytes, write_glb

NORMAL_MAX = 1024
MR_MAX = 1024
BASE_MAX = 4096  # authored size wins; --base-max lowers it
NORMAL_QUALITY = 92
MR_QUALITY = 90
BASE_QUALITY = 92
FLAT_TILT_DEG = 0.5
CONST_STD = 0.5

def to_jpeg(image, max_size, quality, original):
    """Returns the encoded bytes, or None when it would not be an improvement.
    4:4:4 is mandatory: normal XY and roughness/metalness live in separate
    channels, so chroma subsampling would bleed them into each other."""
    if image.format == 'JPEG' and image.width <= max_size:
        return None  # already done; re-encoding would only add generation loss
    if image.width > max_size:
        image = image.resize((max_size, max_size), Image.LANCZOS)
    buf = io.BytesIO()
    image.convert('RGB').save(buf, 'JPEG', quality=quality, subsampling=0, optimize=True)
    data = buf.getvalue()
    return None if len(data) >= len(original) else (data, image.width)


def plan(gltf, raw, images, log):
    """Rewrites materials in place. Returns (replacement bytes by image index,
    texture indices to unlink)."""
    replaced, dropped = {}, set()

    def source_of(texinfo):
        return gltf['textures'][texinfo['index']]['source']

    for material in gltf.get('materials', []):
        pbr = material.setdefault('pbrMetallicRoughness', {})

        base = pbr.get('baseColorTexture')
        if base:
            i = source_of(base)
            alpha = np.asarray(images[i].convert('RGBA'))[:, :, 3]
            opaque = material.get('alphaMode', 'OPAQUE') == 'OPAQUE'
            if not opaque or alpha.std() >= CONST_STD or alpha.mean() < 255:
                log.append('  base: has usable alpha -> kept')
            else:
                enc = to_jpeg(images[i], BASE_MAX, BASE_QUALITY, raw[i])
                if enc is None:
                    log.append(f'  base: {images[i].width}px {images[i].format} kept')
                else:
                    replaced[i] = enc[0]
                    log.append(f'  base: {images[i].width}px {images[i].format}'
                               f' -> {enc[1]}px JPEG q{BASE_QUALITY}')

        normal = material.get('normalTexture')
        if normal:
            i = source_of(normal)
            vec = np.asarray(images[i].convert('RGB'), dtype=np.float32) / 127.5 - 1.0
            vec /= np.maximum(np.linalg.norm(vec, axis=2, keepdims=True), 1e-6)
            tilt = np.degrees(np.arccos(np.clip(vec[:, :, 2], -1, 1)))
            if float(np.percentile(tilt, 99.9)) < FLAT_TILT_DEG:
                dropped.add(normal['index'])
                material.pop('normalTexture')
                log.append(f'  normal: flat ({tilt.mean():.2f}deg) -> dropped')
            else:
                enc = to_jpeg(images[i], NORMAL_MAX, NORMAL_QUALITY, raw[i])
                if enc is None:
                    log.append(f'  normal: {images[i].width}px {images[i].format} kept')
                else:
                    replaced[i] = enc[0]
                    log.append(f'  normal: {images[i].width}px {images[i].format}'
                               f' -> {enc[1]}px JPEG q{NORMAL_QUALITY}')

        mr = pbr.get('metallicRoughnessTexture')
        if mr:
            i = source_of(mr)
            px = np.asarray(images[i].convert('RGB'), dtype=np.float32)
            if px[:, :, 1].std() < CONST_STD and px[:, :, 2].std() < CONST_STD:
                pbr['roughnessFactor'] = round(
                    pbr.get('roughnessFactor', 1.0) * float(px[:, :, 1].mean()) / 255, 6)
                pbr['metallicFactor'] = round(
                    pbr.get('metallicFactor', 1.0) * float(px[:, :, 2].mean()) / 255, 6)
                dropped.add(mr['index'])
                pbr.pop('metallicRoughnessTexture')
                log.append(f"  mr: constant -> rough={pbr['roughnessFactor']}"
                           f" metal={pbr['metallicFactor']}")
            else:
                enc = to_jpeg(images[i], MR_MAX, MR_QUALITY, raw[i])
                if enc is None:
                    log.append(f'  mr: {images[i].width}px {images[i].format} kept')
                else:
                    replaced[i] = enc[0]
                    log.append(f'  mr: {images[i].width}px {images[i].format}'
                               f' -> {enc[1]}px JPEG q{MR_QUALITY}')

        specular = material.get('extensions', {}).get('KHR_materials_specular', {})
        spec_tex = specular.get('specularTexture')
        if spec_tex:
            i = source_of(spec_tex)
            alpha = np.asarray(images[i].convert('RGBA'))[:, :, 3]
            if alpha.std() < CONST_STD:
                specular['specularFactor'] = round(
                    specular.get('specularFactor', 1.0) * float(alpha.mean()) / 255, 6)
                dropped.add(spec_tex['index'])
                specular.pop('specularTexture')
                log.append(f"  specular: alpha constant -> "
                           f"specularFactor={specular['specularFactor']}, dropped")
            else:
                log.append('  specular: alpha varies -> kept')

    return replaced, dropped


def prune(gltf, binary, replaced, dropped):
    """Drops unlinked textures/images/samplers/bufferViews, reindexes every
    reference and rebuilds the binary chunk."""
    keep_tex = [i for i in range(len(gltf.get('textures', []))) if i not in dropped]
    tex_map = {old: new for new, old in enumerate(keep_tex)}
    gltf['textures'] = [gltf['textures'][i] for i in keep_tex]

    def remap(owner, key):
        info = owner.get(key)
        if info is not None:
            info['index'] = tex_map[info['index']]

    for material in gltf.get('materials', []):
        pbr = material.get('pbrMetallicRoughness', {})
        remap(pbr, 'baseColorTexture')
        remap(pbr, 'metallicRoughnessTexture')
        for key in ('normalTexture', 'occlusionTexture', 'emissiveTexture'):
            remap(material, key)
        for ext in material.get('extensions', {}).values():
            for key in [k for k in ext if k.endswith('Texture')]:
                remap(ext, key)

    used_images = {t['source'] for t in gltf['textures'] if 'source' in t}
    keep_img = [i for i in range(len(gltf['images'])) if i in used_images]
    img_map = {old: new for new, old in enumerate(keep_img)}
    for t in gltf['textures']:
        t['source'] = img_map[t['source']]
    gltf['images'] = [gltf['images'][i] for i in keep_img]
    for old, new in img_map.items():
        if old in replaced:
            gltf['images'][new]['mimeType'] = 'image/jpeg'

    used_samplers = {t['sampler'] for t in gltf['textures'] if 'sampler' in t}
    keep_samp = [i for i in range(len(gltf.get('samplers', []))) if i in used_samplers]
    samp_map = {old: new for new, old in enumerate(keep_samp)}
    for t in gltf['textures']:
        if 'sampler' in t:
            t['sampler'] = samp_map[t['sampler']]
    if 'samplers' in gltf:
        gltf['samplers'] = [gltf['samplers'][i] for i in keep_samp]

    payload = {}
    for new, old in enumerate(keep_img):
        bv = gltf['images'][new]['bufferView']
        payload[bv] = replaced.get(old) or view_bytes(gltf, binary, bv)
    for accessor in gltf.get('accessors', []):
        if 'bufferView' in accessor:
            payload.setdefault(accessor['bufferView'],
                               view_bytes(gltf, binary, accessor['bufferView']))
        for part in (accessor.get('sparse') or {}).values():
            if isinstance(part, dict) and 'bufferView' in part:
                payload.setdefault(part['bufferView'],
                                   view_bytes(gltf, binary, part['bufferView']))

    old_views = gltf['bufferViews']
    keep_bv = sorted(payload)
    bv_map = {old: new for new, old in enumerate(keep_bv)}
    out, views = bytearray(), []
    for old in keep_bv:
        bv = dict(old_views[old])
        data = payload[old]
        out += b'\x00' * ((-len(out)) % max(4, bv.get('byteStride', 4)))
        bv['buffer'], bv['byteOffset'], bv['byteLength'] = 0, len(out), len(data)
        out += data
        views.append(bv)
    out += b'\x00' * ((-len(out)) % 4)
    gltf['bufferViews'] = views
    gltf['buffers'] = [{'byteLength': len(out)}]

    for image in gltf['images']:
        image['bufferView'] = bv_map[image['bufferView']]
    for accessor in gltf.get('accessors', []):
        if 'bufferView' in accessor:
            accessor['bufferView'] = bv_map[accessor['bufferView']]
        for part in (accessor.get('sparse') or {}).values():
            if isinstance(part, dict) and 'bufferView' in part:
                part['bufferView'] = bv_map[part['bufferView']]

    return bytes(out)


def repack(src, dst, log):
    gltf, binary = read_glb(src)
    raw = [view_bytes(gltf, binary, im['bufferView']) for im in gltf.get('images', [])]
    images = [Image.open(io.BytesIO(r)) for r in raw]
    replaced, dropped = plan(gltf, raw, images, log)
    write_glb(dst, gltf, prune(gltf, binary, replaced, dropped))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('src', type=Path, help='GLB file or directory of GLBs')
    ap.add_argument('--out', type=Path,
                    help='write here instead of overwriting the sources')
    ap.add_argument('--base-max', type=int,
                    help='downscale base color to this size (default: keep authored)')
    args = ap.parse_args()
    if args.base_max:
        global BASE_MAX
        BASE_MAX = args.base_max

    files = sorted(args.src.glob('*.glb')) if args.src.is_dir() else [args.src]
    if not files:
        sys.exit(f'no .glb under {args.src}')
    if args.out:
        args.out.mkdir(parents=True, exist_ok=True)

    before = after = 0
    for path in files:
        dst = (args.out / path.name) if args.out else path
        size_before = path.stat().st_size
        log = [f'== {path.name}']
        repack(path, dst, log)
        size_after = dst.stat().st_size
        before, after = before + size_before, after + size_after
        log.append(f'  {size_before / 1e6:.2f}MB -> {size_after / 1e6:.2f}MB')
        print('\n'.join(log))
    print(f'TOTAL {before / 1e6:.2f}MB -> {after / 1e6:.2f}MB '
          f'({100 * (1 - after / before):.1f}% smaller)')


if __name__ == '__main__':
    main()
