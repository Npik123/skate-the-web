"""Export Skate 3's online challenges (Spot Battle, Hall of Meat, 1-Up).

Everything comes from the user's own disc, as the game itself reads it:

* data/big/db.big: challengebanks/main lists the challenges of each online
  mode (own_the_spots_battles, hall_of_meats_battles, 1ups); each has a
  challenge_local_data record (time limit, rounds, cumulative scoring, its
  locators and trigger volumes by name), inheriting defaults from
  challenge_local_data_framework (default -> main -> the mode's group).
  Field names are hashed; the state graph scripts (Lua 5.1 chunks in
  challenge_stategraph/*.bin) name every field they read, which names them.
* data/content/missions.big: each mission's cSim_Global.xsf holds its
  locators (EB0009, a matrix per name) and trigger volumes (aggregate volumes
  of one oriented box each, in the order of the EB0019 name table).
* data/big/miscload.big: data/boundary/<challenge>.xml, the area's outline.

usage: challenges.py --game <disc root> --assets <installation assets dir>
"""
from pathlib import Path
import argparse
import json
import re
import struct
import sys
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / 'tools/vendor/university/tools/vanilla_map_extraction/tools'))

from tools.owned_game.big import BigArchive  # noqa: E402
from tools.asset_pipeline import vlt  # noqa: E402
from tools.asset_pipeline.dynamic_props import sections  # noqa: E402
from tools.asset_pipeline.teleports import location_records  # noqa: E402

MODES = {'own_the_spots_battles': 'spot_battle', 'hall_of_meats_battles': 'hall_of_meat', '1ups': 'one_up',
         's_k_a_t_e': 'skate', 'dominations': 'domination', 'races': 'deathrace'}
LOCAL_DATA = 0x46A56ADDF284C381  # class challenge_local_data
DISTRICTS = {'dwtn': 'DownTown', 'dtwn': 'DownTown', 'indu': 'Industrial', 'ind': 'Industrial', 'univ': 'University',
             'bbpk': 'BlackBoxPark', 'bbx': 'BlackBoxPark', 'sksc': 'SkateSchool'}


# ---- Lua 5.1 constants (identifiers only; nothing is executed) ------------

def lua_identifiers(data):
    """Every identifier-like string constant of every chunk in `data`."""
    found = set()
    at = 0
    while (at := data.find(b'\x1bLuaQ', at)) >= 0:
        try:
            pos = [at + 12]

            def u8():
                pos[0] += 1
                return data[pos[0] - 1]

            def u32():
                pos[0] += 4
                return struct.unpack_from('<I', data, pos[0] - 4)[0]

            def string():
                n = u32()
                pos[0] += n
                return data[pos[0] - n:pos[0] - 1].decode('latin-1') if n else None

            def function():
                string()
                u32(); u32(); u8(); u8(); u8(); u8()
                skip = 4 * u32()  # read the count before moving past the code
                pos[0] += skip
                for _ in range(u32()):
                    t = u8()
                    if t == 1:
                        u8()
                    elif t == 3:
                        u32()
                    elif t == 4:
                        s = string()
                        if s and re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', s):
                            found.add(s)
                    elif t != 0:
                        raise ValueError('constant type')
                for _ in range(u32()):
                    function()
                skip = 4 * u32()  # line info
                pos[0] += skip
                for _ in range(u32()):
                    string(); u32(); u32()
                for _ in range(u32()):
                    string()

            function()
            at = pos[0]
        except (ValueError, struct.error, IndexError):
            at += 5
    return found


# ---- values ---------------------------------------------------------------

def cstring(raw, offset):
    end = raw.find(b'\0', offset)
    return raw[offset:end].decode('latin-1') if 0 <= offset < len(raw) and end > offset else None


def scalar(field):
    t, d = field['type'].split('::')[-1], field['data']
    if t == 'Float':
        return struct.unpack('>f', bytes.fromhex(d[:8]))[0]
    width = {'Int8': 2, 'UInt8': 2, 'Int16': 4, 'UInt16': 4, 'Int32': 8, 'UInt32': 8}.get(t)
    if width:
        v = int(d[:width], 16)
        bits = width * 4
        return v - (1 << bits) if t.startswith('Int') and v >= 1 << (bits - 1) else v
    if t == 'Bool':
        return d[:2] != '00'
    if t == 'Text':
        return d
    return None


def names_in(field, binary, width):
    """The names a location / trigger volume field points at (one or an array)."""
    items = field.get('array', {}).get('items') if field.get('array') else [field['data']]
    out = []
    for item in items or []:
        raw = bytes.fromhex(item)
        if len(raw) < 4:
            continue
        name = cstring(binary, struct.unpack_from('>I', raw, 0)[0])
        if name:
            out.append(name.split('|')[1] if '|' in name else name)
    return out if width == 'many' else (out[0] if out else None)


# ---- world objects from mission streams ------------------------------------

def mission_objects(missions_dir):
    import skate3_streams as st
    locators, volumes = {}, {}
    for folder in sorted(p for p in missions_dir.iterdir() if p.is_dir()):
        xst = next(folder.glob('*_Sim.xst'), None)
        xsf = folder / 'cSim_Global.xsf'
        if not xst or not xsf.exists():
            continue
        try:
            assets = st.read_sfil(xsf, st.read_atoc(xst), require_all_records=False)
        except Exception:  # noqa: BLE001 - a mission without a readable sim stream adds nothing
            continue
        for asset in assets:
            raw = asset.data
            try:
                for record in location_records(raw):
                    m = record['matrix']
                    locators.setdefault(record['locator'], [round(v, 4) for row in m for v in row])
            except ValueError:
                pass
            order, boxes = [], []
            for base, _, size, _, _, kind in sections(raw):
                blob = raw[base:base + size]
                if kind == 0xEB0019:
                    order = [re.sub(rb'_0x[0-9a-f]{16}:.*', b'', n).decode('latin-1')
                             for n in re.findall(rb'([A-Za-z0-9_]+_0x[0-9a-f]{16}:[^\0]*)', blob)]
                elif kind == 0x80001 and size >= 96:
                    f = struct.unpack('>24f', blob[:96])
                    if struct.unpack_from('>I', blob, 64)[0] == 4:  # box
                        boxes.append(dict(basis=[round(v, 5) for v in f[0:3] + f[4:7] + f[8:11]],
                                          center=[round(v, 4) for v in f[12:15]],
                                          half=[round(v, 4) for v in f[17:20]]))
            if order and len(order) == len(boxes):
                for name, box in zip(order, boxes):
                    volumes.setdefault(name, box)
    return locators, volumes


def hashed(name):
    """A record's class or key as its 64-bit hash (names come back resolved or as Hash_...)."""
    return int(name[5:], 16) if name.startswith('Hash_') else vlt.hash64(name)


def follow(field, by_ref):
    """The records a RefSpec (or an array of them) points at, in order."""
    if not field:
        return []
    items = field['array']['items'] if field.get('array') else [field['data']]
    out = []
    for item in items:
        raw = bytes.fromhex(item)
        if len(raw) >= 16:
            cls, key = struct.unpack_from('>QQ', raw, 0)
            if (cls, key) in by_ref:
                out.append(by_ref[(cls, key)])
    return out


# scoring_wipeouts' fields, by the hash of their names (those recovered).
HOM_CURVES = {
    0xB2A997F8579B1998: 'BailSpeedScore',
    0x32B1A3C2961F4E6E: 'DropHeightScore',
    0x96CDCC590988982E: 'DropTimeScore',
    0x2451424AD330B08F: 'BailDurationScore',
    0x6CD9F0CD7675949C: 'BailRotationScore',
}
HOM_BONES = 0x1F9B5EA147CBEEE9
HOM_ENTITIES = {0x9E620977A93D37BA: 'PedestrianData', 0xFF1CED9B60B28CDA: 'VehicleData', 0xD0E1C97AF413A1D3: 'DMOData'}


def hall_of_meat(d):
    """Hall of Meat's slam scoring (skatercollections scoring_wipeouts): the
    bonus curves (8 points each), every bone's damage levels (points at an
    impact) and the share of an impact passed to its neighbours, and the
    points for hitting pedestrians, vehicles and objects by class."""
    rows = vlt.convert(d / 'skaterschema', d / 'skatercollections', ['scoring_wipeouts', 'default'])['collections']
    row = next((r for r in rows if r['class'] == 'scoring_wipeouts'), None)
    if not row:
        return None
    fields = {hashed(k): f for k, f in row['fields'].items()}
    out = dict(curves={}, bones=[], entities={})
    for h, name in HOM_CURVES.items():
        if h in fields:
            v = struct.unpack('>20f', bytes.fromhex(fields[h]['data']))
            out['curves'][name] = dict(x=list(v[4:12]), y=list(v[12:20]))
    raw = bytes.fromhex(fields[HOM_BONES]['data']) if HOM_BONES in fields else b''
    for e in range(len(raw) // 84):
        w = struct.unpack_from('>21I', raw, e * 84)
        f = struct.unpack_from('>21f', raw, e * 84)
        out['bones'].append(dict(
            spread=[[w[1 + 2 * k], f[2 + 2 * k]] for k in range(4) if w[1 + 2 * k] != 0xFFFFFFFF],
            levels=[[w[9 + 2 * k], f[10 + 2 * k]] for k in range(6)]))
    for h, name in HOM_ENTITIES.items():
        if h in fields:
            items = fields[h]['array']['items'] if fields[h].get('array') else [fields[h]['data']]
            out['entities'][name] = [list(struct.unpack('>II', bytes.fromhex(i))) for i in items]
    return out


def boundary(miscload_dir, name):
    for candidate in (name, name.removesuffix('_ol'), name.lstrip('_')):
        path = miscload_dir / 'data/boundary' / f'{candidate}.xml'
        if path.exists():
            root = ET.fromstring(path.read_text(encoding='ascii'))
            return [[float(v) for v in p.text.split(',')] for p in root.iter('Point')]
    return None


def title(language, name):
    base = name.lstrip('_').upper()
    # The online versions have their own names (Brute Chute, not Meat Chute).
    stem = base.removesuffix('_OL')
    for key in (f'ID_CHALLENGE_{stem}_OL_TITLE', f'ID_CHALLENGE_{base}_TITLE', f'ID_CHALLENGE_{stem}_TITLE'):
        if key in language:
            return key
    return None


def export(game: Path, assets: Path):
    with tempfile.TemporaryDirectory(prefix='skate-challenges-') as tmp:
        tmp = Path(tmp)
        db = BigArchive(game / 'data/big/db.big')
        db.extract_entries([e for e in db.entries if re.match(
            r'data/db/(skaterschema|skatercollections|challenge_local_data_framework|challengebanks/main|challenge_stategraph/|challenge_local_data/)', e.path)], tmp)
        missions = BigArchive(game / 'data/content/missions.big')
        missions.extract_entries([e for e in missions.entries if e.path.endswith(('_Sim.xst', 'cSim_Global.xsf'))], tmp)
        misc = BigArchive(game / 'data/big/miscload.big')
        misc.extract_entries([e for e in misc.entries if e.path.startswith('data/boundary/')], tmp)
        d = tmp / 'data/db'
        idents = set()
        for b in (d / 'challenge_stategraph').glob('*.bin'):
            idents |= lua_identifiers(b.read_bytes())
        # The banks' own record names, from the export's summary report.
        report = (d / 'challengebanks/main_summaryreport.txt').read_text(encoding='latin-1').split()
        idents |= {Path(line).stem for line in report if line.endswith('.xml')}
        idents |= {line.strip('/').split('/')[0].removesuffix('.class') for line in report if '.class/' in line}
        banks = vlt.convert(d / 'skaterschema', d / 'challengebanks/main', sorted(idents))['collections']
        bank_binary = (d / 'challengebanks/main.bin').read_bytes()
        challenges = {r['key']: r for r in banks if r['class'] == 'challenges'}
        members = [(k, MODES[r['parent']]) for k, r in challenges.items() if r['parent'] in MODES]
        # Deathrace is the online races only (the rest are career races).
        members = [(k, m) for k, m in members if m != 'deathrace' or k.endswith('_ol')]
        global_data = {r['key']: r for r in banks if r['class'] == 'challenge_global_data'}
        objectives = {}
        for r in banks:
            if r['class'] == 'challenge_objective' and r['parent'] in challenges:
                objectives.setdefault(r['parent'], []).append(r)
        names = sorted(idents | set(challenges))
        framework = {r['key']: r for r in vlt.convert(d / 'skaterschema', d / 'challenge_local_data_framework', names)['collections']}
        locators, volumes = mission_objects(tmp / 'data/content/missions')
        # Challenge locators that live with the world (Domination's, race starts).
        for path in sorted((game / 'data/content/global_locators').rglob('*.rx2')):
            try:
                for record in location_records(path.read_bytes()):
                    locators.setdefault(record['locator'], [round(v, 4) for row in record['matrix'] for v in row])
            except ValueError:
                pass
        language = json.loads((assets / 'private/frontend/language.json').read_text(encoding='utf-8'))
        place = lambda n: dict(name=n, matrix=locators.get(n)) if n else None  # noqa: E731
        vol = lambda n: dict(name=n, **volumes[n]) if n in volumes else dict(name=n)  # noqa: E731
        out = []
        for key, mode in sorted(members):
            stem = d / 'challenge_local_data' / key
            if not stem.with_suffix('.vlt').exists():
                continue
            rows = vlt.convert(d / 'skaterschema', stem, names)['collections']
            by_ref = {(hashed(r['class']), hashed(r['key'])): r for r in rows}
            main = next((r for r in rows if hashed(r['class']) == LOCAL_DATA and r['parent'] in framework), rows[0] if rows else None)
            binary = stem.with_suffix('.bin').read_bytes()
            fields, chain, parent = {}, [], main['parent'] if main else ''
            while parent in framework:
                chain.append(framework[parent]['fields'])
                parent = framework[parent]['parent']
            for layer in reversed(chain):
                fields.update(layer)
            local = main['fields'] if main else {}
            fields.update(local)
            settings = {k: v for k, f in fields.items() if not k.startswith('Hash_') and (v := scalar(f)) is not None}
            loc = lambda f, w='one', src=local, raw=binary: names_in(src[f], raw, w) if f in src else None  # noqa: E731
            short = key.lstrip('_').split('_')
            district = DISTRICTS.get(short[1] if short[0] in ('1up', 'ots', 'hom', 'skate', 'dom', 'race') else short[0])
            visual = [s for s in re.findall(rb'[A-Za-z0-9_]+_vis?_\d+', binary)]
            item = dict(
                id=key, mode=mode, district=district, title=title(language, key),
                settings=settings,
                start=place(loc('StartLocation')),
                spawn=place(loc('TurnBasedAttemptSpawnPoint')),
                wait=place(loc('TurnBasedWaitingLocation')),
                start_rotation=[place(n) for n in (loc('StartLocationRotation', 'many') or [])],
                wait_rotation=[place(n) for n in (loc('WaitLocationRotation', 'many') or [])],
                scoring_boundary=vol(loc('ScoringBoundary')) if loc('ScoringBoundary') else None,
                spot_volumes=[vol(n) for n in (loc('SpotVolumes', 'many') or [])],
                indicators=[place(n.decode()) for n in dict.fromkeys(visual)],
                boundary=boundary(tmp, key),
            )
            # Domination: its objects are the challenge's objectives, each with
            # its scoring boundary; players start at the challenge's location.
            if mode == 'domination':
                item['objects'] = [vol(names_in(o['fields']['ScoringBoundary'], bank_binary, 'one'))
                                   for o in sorted(objectives.get(key, []), key=lambda o: o['key'])
                                   if 'ScoringBoundary' in o['fields']]
                g = global_data.get(key, {}).get('fields', {})
                if not item['start'] and 'Location' in g:
                    item['start'] = place(names_in(g['Location'], bank_binary, 'one'))
            # Deathrace: heats of legs of ordered gates (trigger volumes), each
            # heat with its start, time limit and wipeout limit.
            if mode == 'deathrace' and main:
                heats = []
                for heat in follow(main['fields'].get('RaceHeats'), by_ref):
                    hf = heat['fields']
                    legs = []
                    for leg in follow(hf.get('Legs'), by_ref):
                        gates = [names_in(g['fields']['GateVolume'], binary, 'one')
                                 for g in follow(leg['fields'].get('Gates'), by_ref) if 'GateVolume' in g['fields']]
                        finish = names_in(leg['fields']['SplitTimeTriggers'], binary, 'many') if 'SplitTimeTriggers' in leg['fields'] else []
                        legs.append(dict(gates=[vol(n) for n in gates if n], finish=[vol(n) for n in finish if n]))
                    heats.append(dict(
                        start=place(names_in(hf['StartLocation'], binary, 'one')) if 'StartLocation' in hf else item['start'],
                        time_limit=scalar(hf['TimeLimit']) if 'TimeLimit' in hf else None,
                        wipeout_limit=scalar(hf['WipeOutLimit']) if 'WipeOutLimit' in hf else 0,
                        legs=legs))
                item['heats'] = heats
                item['gate_skippable'] = bool(scalar(local['RaceGateSkipable'])) if 'RaceGateSkipable' in local else False
                if not item['start'] and heats:
                    item['start'] = heats[0]['start']
            out.append(item)
        hom = hall_of_meat(d)
    # Beside the game's string table: it ships with the UI add-on pack.
    target = assets / 'private/frontend/challenges.json'
    target.write_text(json.dumps(dict(version=1, challenges=out, hall_of_meat=hom), indent=1), encoding='utf-8')
    missing = [c['id'] for c in out if not any(
        p and p.get('matrix') for p in [c['spawn'], c['start'], *c['start_rotation']])]
    print(f'Challenges: {len(out)} ({", ".join(f"{m}={sum(c["mode"] == m for c in out)}" for m in MODES.values())}); '
          f'{len(locators)} locators, {len(volumes)} volumes; without a placed start: {missing}')
    return out


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--game', type=Path, required=True)
    p.add_argument('--assets', type=Path, required=True)
    a = p.parse_args()
    export(a.game, a.assets)
