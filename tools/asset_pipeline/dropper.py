"""Export Skate 3's Object Dropper catalog from the user's parkassets.big.

The catalog is the game's own dynamic-object recipe (recipe/dynamic/
dynamic_db_*.xml): components (categories, e.g. DMO_Ramps) of models, each
naming its geometry by arena ID (dynamic/model/dynamic/<category>/<id>.rx2).
Every model RX2 carries its render meshes, RW ClusteredMesh collision (with
native edge codes) and native grind splines; all three are exported as they
are. Textures come from the recipe's material table (mat id -> diffuse) and
are downscaled for the browser. Display names come from the VLT dynamic-object
class (DA7A8F0EF63BE539) where the game has one.

usage: dropper.py --game <disc root> --assets <installation assets dir>
"""
from pathlib import Path
import argparse
import json
import re
import struct
import sys
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / 'tools/vendor/university/tools/vanilla_map_extraction/tools'))
sys.path.insert(0, str(ROOT / 'tools/vendor/utt'))

from tools.owned_game.big import BigArchive  # noqa: E402

TEXTURE_SIZE = 128
CATALOG_CLASS = 'Hash_DA7A8F0EF63BE539'
TEMPLATE_FIELD = 'Hash_73061AB91FDAE615'
NAME_FIELD = 'Hash_AA753A536859233D'
HEX_ID = re.compile(r'(0x[0-9a-fA-F]{16})$')


def display_name(model_name):
    """'ab_DMO_GLBL_ConstructionBarrier' -> 'Construction Barrier'."""
    name = re.sub(r'^[a-z]{1,3}_', '', model_name)
    name = re.sub(r'(?i)^dmo_(glbl_|global_)?', '', name)
    name = re.sub(r'(?i)^(glbl|dt|un|in|ot|ur|ad|wf|pj|svm|soho)_', '', name)
    name = name.replace('_', ' ')
    name = re.sub(r'(?<=[a-z])(?=[A-Z])', ' ', name)
    return ' '.join(w if w.isupper() else w.capitalize() for w in name.split()) or model_name


def vlt_names(collections):
    names = {}
    for row in json.loads(collections.read_text(encoding='utf-8'))['collections']:
        if row['class'] != CATALOG_CLASS:
            continue
        template = row['fields'].get(TEMPLATE_FIELD, {}).get('data', '')
        label = row['fields'].get(NAME_FIELD, {}).get('data', '')
        if template and label and not label.startswith('ID_STRING_MISSING'):
            names[template.upper()] = label.lstrip('#') if label.startswith('#') else label
    return names


def mesh_arrays(raw, source):
    from mdl_parser import parse_rx2
    model = parse_rx2(raw, source_path=source, strict=False)
    meshes = []
    for mesh in model.meshes:
        if mesh.uvs is None or not mesh.vertex_count:
            continue
        found = HEX_ID.search(mesh.name or '')
        meshes.append(dict(
            material=found.group(1).lower() if found else None,
            positions=[round(float(v), 4) for p in mesh.vertices for v in p],
            normals=[round(float(v), 3) for p in mesh.normals for v in p] if mesh.normals is not None else [],
            uvs=[round(float(v), 4) for p in mesh.uvs for v in p],
            indices=[int(i) for f in mesh.faces for i in f],
        ))
    return meshes


def collision(raw):
    from retail_collision_mesh import decode_rx2_clustered_meshes
    out = []
    for mesh in decode_rx2_clustered_meshes(raw):
        for t in mesh.triangles:
            out.append(dict(points=[list(t.a), list(t.b), list(t.c)], surface=int(t.surface),
                            edges=list(t.edge_codes) if t.edge_codes is not None else None))
    return out


def splines(raw):
    from retail_grind_splines import decode_grind_splines
    try:
        return decode_grind_splines(raw)
    except Exception:  # noqa: BLE001 - a model without splines is fine
        return []


def save_texture(raw, target):
    import rx2_parser
    from PIL import Image
    parsed = rx2_parser.parse_rx2(raw)
    if not parsed.textures or not parsed.textures[0].rgba:
        return None
    texture = parsed.textures[0]
    height = texture.height // max(texture.cube_faces, 1)
    image = Image.frombytes('RGBA', (texture.width, height), bytes(texture.rgba[:texture.width * height * 4]))
    image.thumbnail((TEXTURE_SIZE, TEXTURE_SIZE))
    image.save(target, optimize=True)
    return dict(width=image.width, height=image.height)


def export(game: Path, assets: Path):
    archive = BigArchive(game / 'data/content/parkassets.big')
    by_path = {e.path.lower(): e for e in archive.entries}
    recipe = next(e for e in archive.entries if re.search(r'recipe/dynamic/dynamic_db_.*\.xml$', e.path))
    tree = ET.fromstring(archive.read(recipe))
    diffuse = {}
    for mat in tree.iter('mat'):
        for sp in mat.iter('sp'):
            if sp.get('chn') == 'diffuse':
                diffuse[mat.get('id').lower()] = sp.get('id').lower()
    names = vlt_names(assets / 'private/stock/skater-collections.json')
    out = assets / 'private/dropper'
    (out / 'objects').mkdir(parents=True, exist_ok=True)
    (out / 'textures').mkdir(parents=True, exist_ok=True)
    textures = {}
    categories = []
    for comp in tree.iter('comp'):
        items = []
        for mod in comp.iter('mod'):
            lod = mod.find('lod')
            if lod is None or not lod.get('arenaid'):
                continue
            arena = lod.get('arenaid').lower()
            entry = next((e for p, e in by_path.items() if p.endswith(f'/{arena}.rx2') and '/model/' in p), None)
            if entry is None:
                continue
            raw = archive.read(entry)
            meshes = mesh_arrays(raw, entry.path)
            triangles = collision(raw)
            if not meshes or not triangles:
                continue
            for mesh in meshes:
                texture = diffuse.get(mesh['material'] or '')
                mesh['texture'] = None
                if texture and texture not in textures:
                    source = by_path.get(f'data/content/dynamic/texture/{texture}.rx2') \
                        or by_path.get(f'data/content/static/texture/{texture}.rx2')
                    textures[texture] = save_texture(archive.read(source), out / 'textures' / f'{texture}.png') if source else None
                if texture and textures.get(texture):
                    mesh['texture'] = f'{texture}.png'
                mesh.pop('material')
            key = arena[2:].upper()
            name = names.get(key) or display_name(mod.get('n', key))
            (out / 'objects' / f'{key}.json').write_text(json.dumps(dict(
                id=key, name=name, model=mod.get('n'), meshes=meshes, collision=triangles,
                grind_splines=splines(raw))), encoding='utf-8')
            items.append(dict(id=key, name=name))
        if items:
            label = comp.get('n', 'Objects').replace('DMO_', '').replace('_', ' ')
            categories.append(dict(name=label, items=items))
    (out / 'catalog.json').write_text(json.dumps(dict(version=1, categories=categories), indent=1), encoding='utf-8')
    count = sum(len(c['items']) for c in categories)
    print(f'Object Dropper: {count} objects in {len(categories)} categories, {sum(1 for t in textures.values() if t)} textures')
    return count


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--game', type=Path, required=True)
    p.add_argument('--assets', type=Path, required=True)
    a = p.parse_args()
    export(a.game, a.assets)
