"""Turn a .glb into a Custom Models entry: manifest.json + preview.png + character.glb.

    python tools/make_custom_character.py MyModel.glb
    python tools/make_custom_character.py MyModel.glb --name "My Skater" --out /opt/skate3-web/custom-characters/entries

Writes <out>/<id>/ with the three files the game expects, where <id> is the
SHA-256 of the GLB (so the same model always gets the same folder). Copy that
folder into the server's custom-characters/entries/ and it appears for everyone
under CREATE > Custom Models > Show: Imported.

The preview is a front view rendered here in software (numpy + Pillow), at the
size and on the background of the game's own previews.

The game animates the model with the Skate 3 skeleton, so the GLB must be skinned
to it (bones named HIPS, SPINE, ... like the Windows importer's output). The
script warns when it is not; such a model will not load in game.
"""
import argparse
import hashlib
import io
import json
import math
import shutil
import struct
import sys
from pathlib import Path

import numpy as np
from PIL import Image

WIDTH, HEIGHT = 256, 320          # the game's preview size
BACKGROUND = (14, 19, 28)         # and its background colour
SUPERSAMPLE = 3                   # render larger, then downscale for smooth edges
SKATE_BONES = {
    'HIPS', 'SPINE', 'SPINE1', 'SPINE2', 'SPINE3', 'NECK', 'NECK1', 'HEAD',
    'RIGHTSHOULDER', 'RIGHTARM', 'RIGHTFOREARM', 'RIGHTHAND',
    'LEFTSHOULDER', 'LEFTARM', 'LEFTFOREARM', 'LEFTHAND',
    'RIGHTUPLEG', 'RIGHTLEG', 'RIGHTFOOT', 'RIGHTTOEBASE',
    'LEFTUPLEG', 'LEFTLEG', 'LEFTFOOT', 'LEFTTOEBASE',
    'SKATEBOARD_ROOT', 'TRUCK_FRONT', 'RIGHT_WHEELFRONT', 'LEFT_WHEELFRONT',
    'TRUCK_BACK', 'LEFT_WHEELBACK', 'RIGHT_WHEELBACK',
    'RIGHTTOEBASE_REPARENTED', 'LEFTTOEBASE_REPARENTED', 'LEFTHAND_REPARENTED', 'RIGHTHAND_REPARENTED',
}
COMPONENTS = {5120: np.int8, 5121: np.uint8, 5122: np.int16, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}
WIDTHS = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT4': 16}


# --- GLB reading ---------------------------------------------------------------

class Glb:
    def __init__(self, data: bytes):
        magic, version, _ = struct.unpack_from('<III', data, 0)
        if magic != 0x46546C67 or version != 2:
            raise SystemExit('Not a binary glTF 2.0 (.glb) file.')
        offset, self.json, self.bin = 12, None, b''
        while offset < len(data):
            length, kind = struct.unpack_from('<II', data, offset)
            chunk = data[offset + 8: offset + 8 + length]
            if kind == 0x4E4F534A:
                self.json = json.loads(chunk)
            elif kind == 0x004E4942:
                self.bin = chunk
            offset += 8 + length
        if self.json is None:
            raise SystemExit('The .glb has no JSON chunk.')
        self.j = self.json

    def view(self, index):
        view = self.j['bufferViews'][index]
        if view.get('buffer', 0) != 0:
            raise SystemExit('Only self-contained .glb files are supported (no external buffers).')
        start = view.get('byteOffset', 0)
        return self.bin[start: start + view['byteLength']], view.get('byteStride')

    def accessor(self, index):
        a = self.j['accessors'][index]
        dtype = np.dtype(COMPONENTS[a['componentType']])
        width = WIDTHS[a['type']]
        count = a['count']
        if 'bufferView' not in a:
            return np.zeros((count, width), dtype=np.float32)
        raw, stride = self.view(a['bufferView'])
        start = a.get('byteOffset', 0)
        item = dtype.itemsize * width
        if stride and stride != item:
            rows = np.frombuffer(raw, dtype=np.uint8, count=stride * (count - 1) + item, offset=start)
            rows = np.lib.stride_tricks.as_strided(rows, (count, item), (stride, 1))
            out = np.frombuffer(rows.copy().tobytes(), dtype=dtype).reshape(count, width)
        else:
            out = np.frombuffer(raw, dtype=dtype, count=count * width, offset=start).reshape(count, width)
        out = out.astype(np.float32 if a['componentType'] == 5126 or a.get('normalized') else np.int64)
        if a.get('normalized') and a['componentType'] != 5126:
            out = out / float(np.iinfo(dtype).max)
        return out

    def image(self, index):
        source = self.j['images'][index]
        if 'bufferView' in source:
            raw, _ = self.view(source['bufferView'])
            return Image.open(io.BytesIO(raw)).convert('RGBA')
        uri = source.get('uri', '')
        if uri.startswith('data:'):
            import base64
            return Image.open(io.BytesIO(base64.b64decode(uri.split(',', 1)[1]))).convert('RGBA')
        return None


def local_matrix(node):
    if 'matrix' in node:
        return np.array(node['matrix'], dtype=np.float64).reshape(4, 4).T
    t = np.array(node.get('translation', [0, 0, 0]), dtype=np.float64)
    x, y, z, w = node.get('rotation', [0, 0, 0, 1])
    s = np.array(node.get('scale', [1, 1, 1]), dtype=np.float64)
    r = np.array([
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ])
    m = np.eye(4)
    m[:3, :3] = r * s
    m[:3, 3] = t
    return m


def world_matrices(j):
    nodes = j.get('nodes', [])
    world = [None] * len(nodes)
    scene = j.get('scenes', [{}])[j.get('scene', 0)] if j.get('scenes') else {'nodes': list(range(len(nodes)))}

    def visit(index, parent):
        world[index] = parent @ local_matrix(nodes[index])
        for child in nodes[index].get('children', []):
            visit(child, world[index])

    for root in scene.get('nodes', []):
        visit(root, np.eye(4))
    return world


# --- Geometry in its rest pose ----------------------------------------------------

def triangles(glb):
    """Yields (positions, normals, uvs, indices, material) per primitive, in world space."""
    j = glb.j
    world = world_matrices(j)
    for node_index, node in enumerate(j.get('nodes', [])):
        if 'mesh' not in node or world[node_index] is None:
            continue
        skin = j['skins'][node['skin']] if 'skin' in node and j.get('skins') else None
        joint_matrices = None
        if skin:
            joints = skin['joints']
            if 'inverseBindMatrices' in skin:
                inverse = glb.accessor(skin['inverseBindMatrices']).reshape(-1, 4, 4).transpose(0, 2, 1).astype(np.float64)
            else:
                inverse = np.repeat(np.eye(4)[None], len(joints), axis=0)
            joint_matrices = np.stack([(world[jn] if world[jn] is not None else np.eye(4)) @ inverse[k] for k, jn in enumerate(joints)])
        for primitive in j['meshes'][node['mesh']]['primitives']:
            if primitive.get('mode', 4) != 4:
                continue
            attributes = primitive['attributes']
            positions = glb.accessor(attributes['POSITION']).astype(np.float64)
            normals = glb.accessor(attributes['NORMAL']).astype(np.float64) if 'NORMAL' in attributes else None
            uvs = glb.accessor(attributes['TEXCOORD_0']).astype(np.float64) if 'TEXCOORD_0' in attributes else None
            homogeneous = np.c_[positions, np.ones(len(positions))]
            if joint_matrices is not None and 'JOINTS_0' in attributes and 'WEIGHTS_0' in attributes:
                # Skin in the rest pose: the sum of weighted joint matrices per vertex.
                jidx = glb.accessor(attributes['JOINTS_0']).astype(np.int64)
                weights = glb.accessor(attributes['WEIGHTS_0']).astype(np.float64)
                weights = weights / np.maximum(weights.sum(1, keepdims=True), 1e-8)
                blend = np.einsum('vk,vkij->vij', weights, joint_matrices[np.clip(jidx, 0, len(joint_matrices) - 1)])
            else:
                blend = np.repeat(world[node_index][None], len(positions), axis=0)
            positions = np.einsum('vij,vj->vi', blend, homogeneous)[:, :3]
            if normals is not None:
                normals = np.einsum('vij,vj->vi', blend[:, :3, :3], normals)
                normals /= np.maximum(np.linalg.norm(normals, axis=1, keepdims=True), 1e-8)
            if 'indices' in primitive:
                indices = glb.accessor(primitive['indices']).reshape(-1, 3).astype(np.int64)
            else:
                indices = np.arange(len(positions)).reshape(-1, 3)
            yield positions, normals, uvs, indices, primitive.get('material')


def material_info(glb, index, cache):
    if index is None:
        return None, np.array([0.8, 0.8, 0.8, 1.0]), 'OPAQUE', 0.5
    if index in cache:
        return cache[index]
    material = glb.j['materials'][index]
    pbr = material.get('pbrMetallicRoughness', {})
    factor = np.array(pbr.get('baseColorFactor', [1, 1, 1, 1]), dtype=np.float64)
    texture = None
    if 'baseColorTexture' in pbr:
        source = glb.j['textures'][pbr['baseColorTexture']['index']].get('source')
        if source is not None:
            image = glb.image(source)
            if image is not None:
                texture = np.asarray(image, dtype=np.float64) / 255.0
    result = (texture, factor, material.get('alphaMode', 'OPAQUE'), material.get('alphaCutoff', 0.5))
    cache[index] = result
    return result


# --- Software rendering -----------------------------------------------------------

def render(glb, yaw_degrees):
    parts = list(triangles(glb))
    if not parts:
        raise SystemExit('The .glb has no triangle meshes to draw.')
    yaw = math.radians(yaw_degrees)
    turn = np.array([[math.cos(yaw), 0, math.sin(yaw)], [0, 1, 0], [-math.sin(yaw), 0, math.cos(yaw)]])
    everything = np.concatenate([p[0] for p in parts]) @ turn.T
    low, high = everything.min(0), everything.max(0)
    width, height = WIDTH * SUPERSAMPLE, HEIGHT * SUPERSAMPLE
    # Fit the whole body with a margin, as the game's previews do.
    scale = min(width * 0.86 / max(high[0] - low[0], 1e-6), height * 0.9 / max(high[1] - low[1], 1e-6))
    centre = (low + high) / 2
    colour = np.zeros((height, width, 3))
    colour[:] = np.array(BACKGROUND) / 255.0
    depth = np.full((height, width), -np.inf)
    light = np.array([-0.35, 0.45, 0.82])
    light /= np.linalg.norm(light)
    cache = {}
    for positions, normals, uvs, indices, material in parts:
        texture, factor, alpha_mode, cutoff = material_info(glb, material, cache)
        p = positions @ turn.T
        n = normals @ turn.T if normals is not None else None
        sx = (p[:, 0] - centre[0]) * scale + width / 2
        sy = height / 2 - (p[:, 1] - centre[1]) * scale
        sz = p[:, 2]
        for a, b, c in indices:
            x0, x1, x2 = sx[a], sx[b], sx[c]
            y0, y1, y2 = sy[a], sy[b], sy[c]
            area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)
            if abs(area) < 1e-9:
                continue
            left, right = int(max(min(x0, x1, x2), 0)), int(min(max(x0, x1, x2) + 1, width))
            top, bottom = int(max(min(y0, y1, y2), 0)), int(min(max(y0, y1, y2) + 1, height))
            if left >= right or top >= bottom:
                continue
            ys, xs = np.mgrid[top:bottom, left:right] + 0.5
            w0 = ((x1 - xs) * (y2 - ys) - (x2 - xs) * (y1 - ys)) / area
            w1 = ((x2 - xs) * (y0 - ys) - (x0 - xs) * (y2 - ys)) / area
            w2 = 1 - w0 - w1
            inside = (w0 >= 0) & (w1 >= 0) & (w2 >= 0)
            if not inside.any():
                continue
            z = w0 * sz[a] + w1 * sz[b] + w2 * sz[c]
            region = depth[top:bottom, left:right]
            nearer = inside & (z > region)
            if not nearer.any():
                continue
            rgba = np.broadcast_to(factor, nearer.shape + (4,)).copy()
            if texture is not None and uvs is not None:
                u = (w0 * uvs[a, 0] + w1 * uvs[b, 0] + w2 * uvs[c, 0]) % 1.0
                v = (w0 * uvs[a, 1] + w1 * uvs[b, 1] + w2 * uvs[c, 1]) % 1.0
                th, tw = texture.shape[:2]
                rgba = rgba * texture[(v * th).astype(int).clip(0, th - 1), (u * tw).astype(int).clip(0, tw - 1)]
            if alpha_mode != 'OPAQUE':
                nearer &= rgba[..., 3] >= (cutoff if alpha_mode == 'MASK' else 0.5)
                if not nearer.any():
                    continue
            if n is not None:
                normal = w0[..., None] * n[a] + w1[..., None] * n[b] + w2[..., None] * n[c]
                normal /= np.maximum(np.linalg.norm(normal, axis=-1, keepdims=True), 1e-8)
            else:
                face = np.cross(p[b] - p[a], p[c] - p[a])
                normal = np.broadcast_to(face / max(np.linalg.norm(face), 1e-8), nearer.shape + (3,))
            shade = 0.38 + 0.72 * np.abs(normal @ light)
            region[nearer] = z[nearer]
            colour[top:bottom, left:right][nearer] = np.clip(rgba[..., :3] * shade[..., None], 0, 1)[nearer]
    image = Image.fromarray((colour * 255).astype(np.uint8), 'RGB')
    return image.resize((WIDTH, HEIGHT), Image.LANCZOS)


# --- Entry ---------------------------------------------------------------------------

def check_rig(glb):
    skins = glb.j.get('skins', [])
    if not skins:
        return 'it has no skin (skeleton), so the game cannot animate it'
    names = {glb.j['nodes'][i].get('name', '') for skin in skins for i in skin['joints']}
    unknown = sorted(n for n in names if n not in SKATE_BONES)
    if 'HIPS' not in names:
        return 'its skeleton is not the Skate 3 one (no HIPS bone); run it through the Windows importer first'
    if unknown:
        return f'it has bones the game does not know: {", ".join(unknown[:8])}{"..." if len(unknown) > 8 else ""}'
    return None


def main():
    parser = argparse.ArgumentParser(description='Make a Custom Models entry (manifest.json + preview.png) from a .glb.')
    parser.add_argument('glb', type=Path, help='the character .glb')
    parser.add_argument('--name', help='name shown in game (default: the file name)')
    parser.add_argument('--out', type=Path, default=Path(__file__).resolve().parent.parent / 'custom-characters' / 'entries',
                        help='folder to write the entry into (default: custom-characters/entries next to dist/)')
    parser.add_argument('--yaw', type=float, default=0.0, help='turn the model this many degrees for the preview (180 if it faces away)')
    args = parser.parse_args()

    data = args.glb.read_bytes()
    if len(data) >= 64 * 1024 * 1024:
        sys.exit('The .glb is 64 MB or more; the game will not load a model that large.')
    glb = Glb(data)
    problem = check_rig(glb)
    if problem:
        print(f'warning: {problem}.', file=sys.stderr)

    identity = hashlib.sha256(data).hexdigest()
    name = ''.join(c for c in (args.name or args.glb.stem.replace('_', ' ')) if c.isprintable())[:64]
    folder = args.out / identity
    folder.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(args.glb, folder / 'character.glb')
    (folder / 'manifest.json').write_text(json.dumps({'version': 1, 'id': identity, 'name': name}, indent=2) + '\n', encoding='utf-8')
    print('rendering preview...')
    render(glb, args.yaw).save(folder / 'preview.png', optimize=True)
    print(f'done: {folder}')
    print('Copy that folder into the server\'s custom-characters/entries/ (it shows up within 30 seconds).')


if __name__ == '__main__':
    main()
