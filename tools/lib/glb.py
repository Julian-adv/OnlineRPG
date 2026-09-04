"""GLB container framing shared by the tools/ GLB scripts."""
import json
import struct

GLB_MAGIC = 0x46546C67
CHUNK_JSON = 0x4E4F534A
CHUNK_BIN = 0x004E4942


def read_glb(path):
    data = path.read_bytes()
    magic, _version, length = struct.unpack_from('<III', data, 0)
    if magic != GLB_MAGIC:
        raise ValueError(f'not a glb: {path}')
    off, gltf, binary = 12, None, b''
    while off < length:
        clen, ctype = struct.unpack_from('<II', data, off)
        chunk = data[off + 8:off + 8 + clen]
        if ctype == CHUNK_JSON:
            gltf = json.loads(chunk.decode('utf-8'))
        elif ctype == CHUNK_BIN:
            binary = chunk
        off += 8 + clen
        off += (-off) % 4
    return gltf, binary


def write_glb(path, gltf, binary):
    js = json.dumps(gltf, separators=(',', ':')).encode('utf-8')
    js += b' ' * ((-len(js)) % 4)
    binary += b'\x00' * ((-len(binary)) % 4)
    chunks = [struct.pack('<II', len(js), CHUNK_JSON), js]
    if binary:
        chunks += [struct.pack('<II', len(binary), CHUNK_BIN), binary]
    total = 12 + sum(len(c) for c in chunks)
    path.write_bytes(b''.join([struct.pack('<III', GLB_MAGIC, 2, total), *chunks]))


def view_bytes(gltf, binary, index):
    bv = gltf['bufferViews'][index]
    start = bv.get('byteOffset', 0)
    return binary[start:start + bv['byteLength']]
