"""Convert Skate 3's audio from the player's own disc into Ogg Vorbis for the engine.

    python -m tools.asset_pipeline.audio [--disc <extracted disc>/data/audio] [--out <install>/assets/private/audio]

Skate 3 (Xbox 360) stores its audio in EA's formats with XMA2 inside:
  audiofiles.big  sound banks (.abk ABKC, .bnk SPLC), each holding numbered clips
  grains.big      per-surface rolling grains (.grain: a small grain table, then SNR)
  wheels.big      wheel spin loops (.snr)
  ambience.big    area ambience streams (.sns); their SNR headers are in
                  ambienceresident.big
  music/          EA music player streams (.mus) and playlists (.mpf)

vgmstream (data/tools/vgmstream) decodes them; soundfile writes Ogg Vorbis. The
clips inside a bank are numbered, not named, so each gets measured (length,
loudness, attack, brightness) into index.json, which the engine's sound table
(assets/private/audio/sounds.json) draws on.

Nothing here is shipped: it is built from the disc on the player's machine.
"""
import argparse
import json
import shutil
import struct
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
import soundfile

from tools.asset_pipeline import splc
from tools.asset_pipeline.vlt import hash64
from tools.owned_game.big import BigArchive

ROOT = Path(__file__).resolve().parents[2]
VGMSTREAM = ROOT / 'data' / 'tools' / 'vgmstream' / 'vgmstream-cli.exe'

# The banks the skateboard, the skater and the menus draw on.
SFX_BANKS = [
    'GRINDS.abk', 'board_scrapes.abk', 'Brd_Squeaks.abk', 'Bodyslide.abk', 'WHEEL_SKID_BANK.abk',
    'Seams_Bank.abk', 'Rolling_Rattles.abk', 'Sk8_Air_Flip_Tricks.abk', 'Skate_Collisions.bnk',
    'Skate_Metal.bnk', 'sk8_foley.bnk', 'sk8_menu.bnk', 'FOOT_DRAG.abk', 'fstep_skateshoe1_sm.abk',
    'sense_of_speed.abk', 'PatchBank_Rolling_Surfaces.abk', 'Foley_Cloth.abk', 'Treatments.abk',
    'killed_it.abk', 'owned_it.abk', 'hom_slo_mo.abk', 'fade_to_white.abk',
    'HOM_Set_1.bnk', 'HOM_Set_2.bnk', 'HOM_Set_3.bnk', 'HOM_Set_4.bnk', 'HOM_Set_5.bnk',
    'crowds_tens_a.abk', 'crowds_hundreds_a.abk', 'crowds_sweeteners_a.abk',
]
QUALITY = {'sfx': 0.45, 'loop': 0.4, 'ambience': 0.25, 'music': 0.35}


def vgmstream(source: Path, output: Path, subsong: int | None = None, loops: bool = False):
    args = [str(VGMSTREAM), '-o', str(output)]
    if subsong is not None:
        args += ['-s', str(subsong)]
    if not loops:
        args += ['-i']  # play the clip once: ignore authored loop points
    args.append(str(source))
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode != 0 or not output.exists():
        raise RuntimeError(f'vgmstream could not decode {source.name}#{subsong}: {result.stdout[-300:]}{result.stderr[-300:]}')


def subsongs(source: Path) -> int:
    result = subprocess.run([str(VGMSTREAM), '-m', str(source)], capture_output=True, text=True)
    for line in result.stdout.splitlines():
        if line.startswith('stream count:'):
            return int(line.split(':')[1])
    return 1 if 'sample rate' in result.stdout else 0


def features(samples: np.ndarray, rate: int) -> dict:
    mono = samples.mean(axis=1) if samples.ndim > 1 else samples
    if not len(mono):
        return {'seconds': 0, 'peak': 0, 'rms': 0, 'attack': 0, 'centroid': 0, 'steady': 0}
    peak = float(np.abs(mono).max())
    rms = float(np.sqrt(np.mean(mono ** 2)))
    window = max(1, rate // 100)
    envelope = np.sqrt(np.convolve(mono ** 2, np.ones(window) / window, mode='same'))
    attack = float(np.argmax(envelope > envelope.max() * 0.7) / rate) if envelope.max() > 0 else 0.0
    # Steady loops keep their level; impacts fall away after the hit.
    halves = np.array_split(envelope, 4)
    steady = float(min(h.mean() for h in halves) / max(max(h.mean() for h in halves), 1e-9))
    spectrum = np.abs(np.fft.rfft(mono[: rate * 4] * np.hanning(min(len(mono), rate * 4))))
    freqs = np.fft.rfftfreq(min(len(mono), rate * 4), 1 / rate)
    centroid = float((spectrum * freqs).sum() / max(spectrum.sum(), 1e-9))
    return {'seconds': round(len(mono) / rate, 3), 'peak': round(peak, 4), 'rms': round(rms, 4),
            'attack': round(attack, 3), 'centroid': round(centroid), 'steady': round(steady, 3)}


def to_ogg(wav: Path, ogg: Path, kind: str, stereo: bool = False) -> dict:
    samples, rate = soundfile.read(wav, dtype='float32', always_2d=True)
    if samples.shape[1] > 2 or (samples.shape[1] == 2 and not stereo):
        # Fold surround (ambience is 5.0) down to stereo, or to mono for effects.
        if stereo:
            left = samples[:, [0] + list(range(2, samples.shape[1], 2))].mean(axis=1)
            right = samples[:, [1] + list(range(3, samples.shape[1], 2))].mean(axis=1)
            samples = np.stack([left, right], axis=1)
        else:
            samples = samples.mean(axis=1, keepdims=True)
    info = features(samples, rate)
    ogg.parent.mkdir(parents=True, exist_ok=True)
    # Written in small blocks: libsndfile's Vorbis encoder overflows the stack
    # when handed a long clip in one call.
    with soundfile.SoundFile(ogg, 'w', rate, samples.shape[1], format='OGG', subtype='VORBIS',
                             compression_level=1 - QUALITY[kind]) as handle:
        for start in range(0, len(samples), 4096):
            handle.write(samples[start:start + 4096])
    info['rate'] = rate
    info['channels'] = samples.shape[1]
    return info


class Disc:
    def __init__(self, audio: Path):
        self.audio = audio
        self.archives = {}

    def entries(self, archive: str) -> dict:
        if archive not in self.archives:
            big = BigArchive(self.audio / archive)
            self.archives[archive] = (big, {Path(e.path).name: e for e in (getattr(big, 'entries', None) or big._parse())})
        return self.archives[archive][1]

    def read(self, archive: str, name: str) -> bytes:
        self.entries(archive)
        big, entries = self.archives[archive]
        return big.read(entries[name])


def splc_samples(raw: bytes) -> list[bytes]:
    """Split an SPLC bank (.bnk) into standalone SNR sounds.

    Header: 'SPLC', version 3, u32 table end, u32 patch count (x2), u32 0,
    u32 sample count. Past the patch table the samples sit back to back, each
    an EA-XMA SNR header (codec 3, mono or stereo, 32-48 kHz) and its data.
    """
    import re
    if raw[:4] != b'SPLC':
        raise ValueError('not an SPLC bank')
    table_end, = struct.unpack_from('>I', raw, 8)
    expected, = struct.unpack_from('>I', raw, 0x18)
    header = re.compile(rb'[\x03\x13][\x00\x04](?:\xbb\x80|\xac\x44|\x7d\x00|\x5d\xc0|\x56\x22)')
    starts = [m.start() for m in header.finditer(raw, table_end)]
    if len(starts) != expected:
        raise ValueError(f'SPLC bank lists {expected} samples but {len(starts)} were found')
    return [raw[a:b] for a, b in zip(starts, starts[1:] + [len(raw)])]


def convert_bank(disc: Disc, name: str, work: Path, out: Path) -> dict:
    stem = Path(name).stem
    raw = disc.read('audiofiles.big', name)
    clips = {}
    if raw[:4] == b'SPLC':
        pieces = splc_samples(raw)
        for index, piece in enumerate(pieces, 1):
            (work / f'{stem}_{index}.snr').write_bytes(piece)
        count = len(pieces)
        source = None
    else:
        source = work / name
        source.write_bytes(raw)
        count = subsongs(source)

    def one(index):
        wav = work / f'{stem}_{index}.wav'
        if source is None:
            vgmstream(work / f'{stem}_{index}.snr', wav)
            (work / f'{stem}_{index}.snr').unlink()
        else:
            vgmstream(source, wav, index)
        info = to_ogg(wav, out / 'banks' / stem / f'{index:03d}.ogg', 'sfx')
        wav.unlink()
        return index, info

    with ThreadPoolExecutor(max_workers=6) as pool:
        for index, info in pool.map(one, range(1, count + 1)):
            clips[f'{index:03d}'] = info
    if source is not None:
        source.unlink()
    return clips


class Bank:
    """An SPLC bank decoded for rendering: its patches, cues and samples."""

    def __init__(self, disc: Disc, name: str, work: Path):
        self.raw = disc.read('audiofiles.big', name)
        self.samples = []
        for index, piece in enumerate(splc_samples(self.raw)):
            snr = work / f'{Path(name).stem}_{index}.snr'
            wav = snr.with_suffix('.wav')
            snr.write_bytes(piece)
            vgmstream(snr, wav)
            data, rate = soundfile.read(wav, dtype='float32', always_2d=True)
            self.samples.append((data.mean(axis=1), rate))
            snr.unlink()
            wav.unlink()
        self.patches = {p['id']: p for p in splc.patches(self.raw)}

    def render(self, value: int, clip: str, work: Path, out: Path, volume: float = 1.0) -> dict | None:
        """Render the cue a VLT value of this bank names to `out/<clip>.ogg`
        (and `<clip>_2.ogg`... for its random variants). None for no sound."""
        chosen = [self.patches[p] for p in splc.cue_patches(self.raw, value) if p in self.patches]
        if not chosen:
            return None
        clips = []
        # A cue with several patches plays one of them; a patch with random
        # variants plays one of those.
        renders = [splc.render(patch, self.samples, choice) for patch in chosen for choice in range(splc.variants(patch))]
        peak = max((float(np.abs(mixed).max()) for mixed, _ in renders if len(mixed)), default=0.0)
        # Rendered at full patch gain; louder-than-full-scale layers are
        # scaled into range and the difference carried by the table volume.
        scale = 1 / peak if peak > 1 else 1.0
        pan = any(v['pan'] for patch in chosen for node in patch['nodes'] for v in node)
        for choice, (mixed, rate) in enumerate(renders):
            if not len(mixed) or not mixed.any():
                continue
            data = mixed * scale if pan else (mixed.mean(axis=1, keepdims=True) * np.sqrt(2) * scale).clip(-1, 1)
            wav = work / f'{Path(clip).name}_{choice}.wav'
            soundfile.write(wav, data, rate)
            name = f'{clip}.ogg' if choice == 0 else f'{clip}_{choice + 1}.ogg'
            to_ogg(wav, out / name, 'sfx', stereo=pan)
            wav.unlink()
            clips.append(name)
        return {'clips': clips, 'volume': round(volume / scale, 4), 'patches': [p['id'] for p in chosen]}


def stock_collections(out: Path) -> list[dict] | None:
    stock = out.parent / 'stock' / 'skater-collections.json'
    return json.loads(stock.read_text())['collections'] if stock.exists() else None


def convert_fe(disc: Disc, work: Path, out: Path) -> dict:
    """The front end's named sounds, rendered from their sk8_menu cues the
    way the game layers them (see splc.py), one clip per random variant.
    Names and volumes come from the VLT "fe" rows in the converted stock
    collections; without them the cues are numbered instead."""
    bank = Bank(disc, 'sk8_menu.bnk', work)
    shutil.rmtree(out / 'fe', ignore_errors=True)
    collections = stock_collections(out)
    if collections:
        named = splc.fe_sounds(collections)
    else:
        count = struct.unpack_from('>I', bank.raw, 12)[0]
        named = {f'cue_{cue:03d}': {'value': count + cue, 'volume': 1.0} for cue in range(len(splc.cues(bank.raw)))}
    result = {}
    for name, sound in sorted(named.items()):
        rendered = bank.render(sound['value'], f'fe/{name}', work, out, sound['volume'])
        if rendered is not None:
            result[name] = rendered
    return result


# Skate 3's audio materials (Sk8::Audio::eAudioMaterialTypes) in enum order:
# the hashed name of each, as the game's own table lists them. A collision
# triangle's audio surface code (the low seven bits of its surface id) is an
# index here, and the name keys the material's row in the VLT material class.
SURFACE_MATERIALS = [
    0x84EF7A70636ED529, 0x9182CCAD63479434, 0x49A5B1F124E53FDE, 0x49F167E3FC3A008B,
    0xD7EFFA5849BB7B15, 0x72B4ECCDC6CEE6E8, 0xAF8EAB3B0227FD48, 0x54D22F1336F8B796,
    0x42CD19124AE613EB, 0x0613243A490D11EA, 0xA4B6334D87D72CC7, 0x3738E9708EB6C776,
    0x1BFF6E5CA982683A, 0x5BBDF4492AC7FB47, 0x22132549704E8733, 0x1F2ACA29DEBE0729,
    0x500227DC11D198AC, 0xB7DBFE813D021524, 0x6EAACF19DDD07B5A, 0x60EC16F48C0BE620,
    0x791C508A6E1205EA, 0x42337BA2D862B622, 0x10ED41289D47F052, 0xBD556D5B135BBF8E,
    0xB74F55A0A96516A0, 0x3B214D295DB514F7, 0xA7AB65A3C044B3DA, 0xE96D23CB66372699,
    0xB062804C25EDD8A7, 0xA8EF0E67A39D62D9, 0xF7141297FEB29B5D, 0x0DEB3D61CAED9567,
    0x7AC77169773C0CE4, 0x0193A08E3EDA269B, 0xDBF3EC4C65A69F5D, 0x3A43FEE3639BA301,
    0x309033B5E5872675, 0xEF0C2293B7888864, 0xD807FA29ADE65957, 0x10017676688AA504,
    0xD8F132C02494531F, 0x5D4D49C598C7C2E3, 0xD3184F6BD8E629CC, 0xA227D8D1EB22C467,
    0x7D5F7C90590CD76C, 0xE351230372B8AA43, 0xC65BAF14769DFCEB, 0x13D19F33ABDEFDA4,
    0x6B170B796C325537, 0x8640A18245382F66, 0xC53FA0D15249C949, 0x9988214E2D3F25C2,
    0x6A0D63DA93EF3459, 0x99433514E821A363, 0xF002CCA1F7FD6978, 0x947A2EEC879CF0C4,
    0xC5C3FCD994414A11, 0x45EF058BC984C7E8, 0x03745EEB6C08A0B7, 0xF3D78078D942C1F5,
    0x3E93846A0C0904EF, 0x71D3FF58545A7240, 0x36F1FB399593A42E, 0xCA39C5BF26B5A8CE,
    0x98CD99EC1E257D5A, 0x94BFC99C9D77FBE9, 0x3F4E33C9A0B9B05E, 0xDC7B09876FB19FFF,
    0x8BD87DE72E1A3157, 0x0613243A490D11EA, 0x0613243A490D11EA, 0x0613243A490D11EA,
    0xBFBDAC657253E431, 0x73235D7F8A764EA3, 0x070B3A354E4B91E8, 0x6C322F848110963C,
    0x07FB54E5A776E0DD, 0x4EE7754C40081FFB, 0x59BA34C2182DFAED, 0x761C67A1D52C27D8,
    0x5AAA7F375D3FAFD7, 0x7CAF97EA114D54A9, 0xA029E9DED863C78E, 0xB13FD71EA972B2E7,
    0x061A5C9D1EB44C60, 0x0526D0B41AA7E7F3, 0xD7A4B6C19268884E, 0x0023C43AFE5425C2,
    0x0B56CDEC643B3C5C, 0xA5C4BE35A6484D2A, 0x44C45991DF83F528, 0x954AB2622DDDCDAE,
    0xB2BD1F28DDE601B0, 0x09A0A000A6C314BE,
]
MATERIAL_CLASS = 'Hash_D40CB4C0FFE45676'
# A material's impact layers: by landing strength, the first and third layer,
# and a fourth layer hard landings add. Surfaces that play the Skate_Metal
# bank keep the same shape in fields whose names are not known.
IMPACT_FIELDS = {
    'Skate_Collisions': {'hard': ['Hard_L1', 'Hard_L3'], 'medium': ['Medium_L1', 'Medium_L3'],
                         'soft': ['Soft_L1', 'Soft_L3'], 'extra': ['L4']},
    'Skate_Metal': {'hard': ['Hash_1411108A7E9CC74A', 'Hash_79DD0E6659793D0E'],
                    'medium': ['Hash_50796F92F3DE449B'], 'soft': ['Hash_595537EBA0196BE7'],
                    'extra': ['Hash_F54277A83E0170FD']},
}


def field_key(name: str) -> str:
    return name if name.startswith('Hash_') else f'Hash_{hash64(name):016X}'


def material_rows(collections: list[dict]) -> dict[str, dict]:
    """Material rows by key, each with its parents' fields filled in."""
    rows = {r['key']: r for r in collections if r['class'] == MATERIAL_CLASS}
    resolved = {}
    for key in rows:
        chain, at = [], key
        while at in rows and at not in chain:
            chain.append(at)
            at = rows[at].get('parent') or None
        fields = {}
        for k in reversed(chain):
            fields.update(rows[k]['fields'])
        resolved[key] = fields
    return resolved


def row_for(rows: dict[str, dict], name_hash: int) -> dict | None:
    for key, fields in rows.items():
        if (int(key[5:], 16) if key.startswith('Hash_') else hash64(key)) == name_hash:
            return fields
    return None


# Skate 3's grind types (the game's own list: grind_roughconcrete,
# grind_smoothconcrete, grind_roughwood, grind_smoothwood, grind_rail1..4,
# grind_coping, grind_car, grind_plastic, grind_braidedbnch,
# grind_perforatedbnch, grind_rail5), as the hashed key of each type's row in
# the VLT grind class, in enum order (the table the executable holds). The
# AudioSurfaceMap's grind column indexes this list.
GRIND_NAMES = ['roughconcrete', 'smoothconcrete', 'roughwood', 'smoothwood', 'rail1', 'rail2', 'rail3', 'rail4',
               'coping', 'car', 'plastic', 'braidedbnch', 'perforatedbnch', 'rail5']
GRIND_ROWS = [
    0x72766EB54205429A, 0x849358CD45882044, 0x8DB0436F6B3405D5, 0xA36240FD856EEBA2,
    0x8212F939B27CB441, 0x8710D7DAA28B90E7, 0x80AD71A0341B4AEB, 0x29FC132BB19BE7C9,
    0xFD4F9F04AE61311B, 0x7CBD549861AE837E, 0xBDDCF1000271E0F7, 0x0C4C7B1701C3EA1B,
    0x3696D249613957DD, 0xEDB51E711C25AA58,
]
GRIND_CLASS = 'Hash_049861E8F9A8D16B'
# The grind row's cues: three enter and three exit hits (tiers 1-3) in each
# bank, and a spare pair that defaults to tier 3 (used here for board slides).
GRIND_CUES = {
    'Skate_Collisions': {'enter': ['Hash_71F2DBB8CA385137', 'Hash_F9FED84DD3E93919', 'Hash_013C2E46521A9080'], 'exit': ['Hash_9086C01E46C97B05', 'Hash_B2DB39923B3A2C3F', 'Hash_FDDA1C015F450486'], 'slide': ['Hash_2162F0EFBDDE72C6', 'Hash_FFD9CE934742D506']},
    'Skate_Metal': {'enter': ['Hash_4A76108CDB52FDD3', 'Hash_8375C9A4AD144F7D', 'Hash_F344EC2BC1BD4B6E'], 'exit': ['Hash_F314474F29EB8D64', 'Hash_60470BE7D8CA45C4', 'Hash_A7639B2F6ABFF8E8'], 'slide': ['Hash_991C563B70C40DED', 'Hash_92258B0B80C10301']},
}
# The GRINDS bank's loop sets (each a few layers of three random variants),
# in bank order, and the grind type each is taken to belong to. The bank's
# patch graph is not decoded, so this pairing is by the loops' character
# against each type's own enter hits; sounds.json carries it as data.
GRIND_LOOPS = {
    'roughconcrete': [[1, 2, 3], [7, 8, 9], [10, 11, 12], [4, 5, 6]],
    'rail3': [[19, 20, 21], [13, 14, 15], [16, 17, 18], [22, 23, 24], [118, 119, 120]],
    'smoothconcrete': [[31, 32, 33], [25, 26, 27], [28, 29, 30], [34, 35, 36]],
    'braidedbnch': [[43, 44, 45], [37, 38, 39], [40, 41, 42], [46, 47, 48]],
    'rail1': [[49, 50, 51], [52, 53, 54], [61, 62, 63], [55, 56, 57], [58, 59, 60]],
    'coping': [[64, 65, 66], [67, 68, 69], [73, 74, 75], [70, 71, 72]],
    'roughwood': [[76, 77, 78], [79, 80, 81], [82, 83, 84], [88, 89, 90]],
    'plastic': [[85, 86, 87]],
    'smoothwood': [[91, 92, 93], [94, 95, 96], [100, 101, 102], [97, 98, 99], [115, 116, 117]],
    'rail5': [[103, 104, 105], [106, 107, 108], [112, 113, 114], [109, 110, 111]],
    'car': [[121, 122, 123]],
}
GRIND_LOOPS['rail2'] = GRIND_LOOPS['rail1']
GRIND_LOOPS['rail4'] = GRIND_LOOPS['rail1']
GRIND_LOOPS['perforatedbnch'] = GRIND_LOOPS['braidedbnch']


def hashed(name: str) -> str:
    return f'Hash_{hash64(name):016X}'


def convert_materials(disc: Disc, work: Path, out: Path) -> dict | None:
    """Impacts and grinds as the game builds them, from its material and
    grind tables, each cue rendered from the Skate_Collisions or Skate_Metal
    bank. Board parts (trucks landing, the tail popping), body parts (a
    bail's thumps, bone cracks), each audio surface's own layers, and each
    grind type's enter and exit hits with its volumes."""
    collections = stock_collections(out)
    if not collections:
        return None
    rows = material_rows(collections)
    banks = {name: Bank(disc, f'{name}.bnk', work) for name in IMPACT_FIELDS}
    shutil.rmtree(out / 'impacts', ignore_errors=True)
    rendered = {}

    def cue(bank, value):
        if (bank, value) not in rendered:
            rendered[bank, value] = banks[bank].render(value, f'impacts/{bank}/{value}', work, out)
        return rendered[bank, value]

    def level(fields, name, default=1.0):
        field = fields.get(field_key(name))
        return round(int(field['data'][:8], 16) / 32767, 4) if field else default

    def layers(fields: dict) -> dict | None:
        for bank, tiers in IMPACT_FIELDS.items():
            result = {}
            for tier, names in tiers.items():
                result[tier] = []
                for name in names:
                    field = fields.get(field_key(name))
                    if not field or field['type'] != bank:
                        continue
                    one = cue(bank, int(field['data'][:8], 16))
                    if one:
                        result[tier].append(one)
            if any(result.values()):
                result['volume'] = level(fields, 'Volume')
                result['jump'] = level(fields, 'Jump_Volume')
                return result
        return None

    parts = {}
    for part in ('board', 'truck', 'arm', 'leg', 'torso', 'head', 'foot', 'skin', 'cloth', 'bonecrack', 'bonesnap'):
        fields = row_for(rows, hash64(part))
        parts[part] = layers(fields) if fields else None
    surfaces = []
    for name_hash in SURFACE_MATERIALS:
        fields = row_for(rows, name_hash)
        surfaces.append(layers(fields) if fields else None)

    # Hall of Meat: each body part's bone sounds (HOM_Set_1 cues).
    hom = {}
    hom_bank = Bank(disc, 'HOM_Set_1.bnk', work)
    for part in ('hom_arm', 'hom_leg', 'hom_torso', 'hom_head', 'hom_foot'):
        fields = row_for(rows, hash64(part))
        if not fields:
            continue
        clips = []
        for field in fields.values():
            if field['type'] == 'HOM_Set_1' and int(field['data'][:8], 16):
                value = int(field['data'][:8], 16)
                if ('HOM_Set_1', value) not in rendered:
                    rendered['HOM_Set_1', value] = hom_bank.render(value, f'impacts/HOM_Set_1/{value}', work, out)
                if rendered['HOM_Set_1', value]:
                    clips.append(rendered['HOM_Set_1', value])
        hom[part[4:]] = {'cues': clips, 'volume': level(fields, 'Volume')}

    # Grinds: the grind class rows, resolved through their parents.
    grind_rows = {r['key']: r for r in collections if r['class'] == GRIND_CLASS}
    def grind_fields(key):
        chain, at = [], key
        while at in grind_rows and at not in chain:
            chain.append(at)
            at = grind_rows[at].get('parent') or None
        fields = {}
        for k in reversed(chain):
            fields.update(grind_rows[k]['fields'])
        return fields
    def grind_type(key, name):
        fields = grind_fields(key)
        if not fields:
            return None
        entry = {'name': name, 'enter': [], 'exit': [], 'slide_enter': None, 'slide_exit': None}
        # A row inherits the default row's concrete cues; a metal row's own
        # Skate_Metal cues replace them.
        chosen = [bank for bank, names in GRIND_CUES.items()
                  if any((f := fields.get(n)) and f['type'] == bank and int(f['data'][:8], 16) for n in names['enter'])]
        preferred = 'Skate_Metal' if 'Skate_Metal' in chosen else 'Skate_Collisions'
        for bank, names in GRIND_CUES.items():
            if bank != preferred:
                continue
            for kind in ('enter', 'exit'):
                for field_name in names[kind]:
                    field = fields.get(field_name)
                    if field and field['type'] == bank and int(field['data'][:8], 16):
                        entry[kind].append(cue(bank, int(field['data'][:8], 16)))
            for kind, field_name in zip(('slide_enter', 'slide_exit'), names['slide']):
                field = fields.get(field_name)
                if field and field['type'] == bank and int(field['data'][:8], 16):
                    entry[kind] = cue(bank, int(field['data'][:8], 16))
        def floats(prefix):
            vals = []
            for i in (1, 2, 3):
                field = fields.get(field_key(f'{prefix}{i}'))
                vals.append(round(struct.unpack('>f', bytes.fromhex(field['data']))[0], 3) if field else 1.0)
            return vals
        entry['enter_volume'] = floats('EnterVol')
        entry['exit_volume'] = floats('ExitVol')
        entry['rail_volume'] = floats('RailVol')
        field = fields.get(field_key('MaxSpeed'))
        entry['max_speed'] = round(struct.unpack('>f', bytes.fromhex(field['data']))[0], 2) if field else 45.0
        entry['loops'] = [[f'banks/GRINDS/{i:03d}.ogg' for i in layer] for layer in GRIND_LOOPS.get(name, [])]
        return entry
    types = []
    for name, key_hash in zip(GRIND_NAMES, GRIND_ROWS):
        key = next((k for k in grind_rows if (int(k[5:], 16) if k.startswith('Hash_') else hash64(k)) == key_hash), None)
        types.append(grind_type(key, name) if key else None)
    default = grind_type('default', 'default')
    return {'parts': parts, 'surfaces': surfaces, 'hom': hom, 'grinds': {'types': types, 'default': default}}


# Rolling speed (m/s) at the first and the last grain of a recording.
GRAIN_SPEED = (0.5, 11.0)
ROLL_BANDS = 6


def grain_table(raw: bytes) -> tuple[int, list[int]]:
    """A .grain file: u32 header size, f32 seconds, a 0x1c-byte table header,
    then one byte per grain (its length in 512-sample units), then the SNR.
    The grains are consecutive slices of one take of a board speeding up, so
    a grain's index stands for a rolling speed: the game's grain player picks
    grains by speed and overlaps them."""
    header = struct.unpack('>I', raw[:4])[0]
    return header, [n * 512 for n in raw[8 + 0x1c:header]]


def grain_bands(mono: np.ndarray, rate: int, lengths: list[int], seed: int) -> list[np.ndarray]:
    """Loops for ROLL_BANDS speeds along the take, each built like the grain
    player builds the sound: grains from around that speed, in random order,
    overlapped with equal-power crossfades. Each loop wraps seamlessly."""
    starts = np.concatenate([[0], np.cumsum(lengths)[:-1]]).astype(int)
    grains = [mono[a:a + n] for a, n in zip(starts, lengths) if a + n <= len(mono) and n > 0]
    count = len(grains)
    rng = np.random.default_rng(seed)
    bands = []
    for band in range(ROLL_BANDS):
        centre = band / (ROLL_BANDS - 1) * (count - 1)
        spread = max(2.0, count / ROLL_BANDS / 2)
        pool = [i for i in range(count) if abs(i - centre) <= spread]
        target = int(rate * 3.0)
        out = np.zeros(target + rate, np.float32)
        at = 0
        while at < target:
            grain = grains[int(rng.choice(pool))].astype(np.float32)
            fade = len(grain) // 4
            ramp = np.sin(np.linspace(0, np.pi / 2, fade, dtype=np.float32))
            shaped = grain.copy()
            shaped[:fade] *= ramp
            shaped[-fade:] *= ramp[::-1]
            end = min(len(out), at + len(shaped))
            out[at:end] += shaped[:end - at]
            at += len(grain) - fade
        # Wrap the overhang onto the start so the loop has no seam.
        loop = out[:at].copy()
        tail = out[at:at + len(grains[0])]
        loop[:len(tail)] += tail
        bands.append(loop)
    return bands


def convert_grains(disc: Disc, work: Path, out: Path) -> dict:
    result = {}
    for seed, name in enumerate(sorted(disc.entries('grains.big'))):
        raw = disc.read('grains.big', name)
        header, lengths = grain_table(raw)
        stem = Path(name).stem
        snr = work / (stem + '.snr')
        snr.write_bytes(raw[header:])
        wav = snr.with_suffix('.wav')
        vgmstream(snr, wav)
        data, rate = soundfile.read(wav, dtype='float32', always_2d=True)
        result[stem] = to_ogg(wav, out / 'grains' / f'{stem}.ogg', 'loop')
        result[stem]['grains'] = len(lengths)
        bands = []
        for band, loop in enumerate(grain_bands(data.mean(axis=1), rate, lengths, seed)):
            band_wav = work / f'{stem}_{band}.wav'
            soundfile.write(band_wav, loop, rate)
            clip = f'grains/{stem}/{band}.ogg'
            to_ogg(band_wav, out / clip, 'loop')
            band_wav.unlink()
            speed = GRAIN_SPEED[0] + (GRAIN_SPEED[1] - GRAIN_SPEED[0]) * band / (ROLL_BANDS - 1)
            bands.append({'clip': clip, 'speed': round(speed, 2)})
        result[stem]['bands'] = bands
        snr.unlink()
        wav.unlink()
    return result


# Skate 3's rolling sound types, in the order of the AudioSurfaceMap's second
# column (the game's own list: audio_plywood, audio_roughasphalt, ...), and the
# grain take each one rolls on. Grass and dirt have no take of their own; the
# game plays rough asphalt under them, quieter (rolling.csi GrassDirtVolume).
ROLLING_TYPES = [
    ('plywood', 'wood_ramp_hard', 1.0), ('roughasphalt', 'asphalt_rough_hard', 1.0),
    ('roughconcrete', 'concrete_rough_hard', 1.0), ('smoothasphalt', 'asphalt_smooth_hard', 1.0),
    ('smoothconcrete', 'concrete_smooth_hard', 1.0), ('woodramp', 'wood_ramp_hard', 1.0),
    ('aggregate', 'concrete_aggregate_hard', 1.0), ('grass', 'asphalt_rough_soft', 0.7),
    ('dirt', 'asphalt_rough_soft', 0.7), ('metal', 'metal_smooth_hard', 1.0),
    ('manhole', 'metal_smooth_hard', 1.0), ('drain', 'metal_smooth_hard', 1.0),
    ('grate', 'metal_smooth_hard', 1.0),
]


# Seam patterns, in the order of the AudioSurfaceMap's seventh column (the
# game's list: default, spidercrack, square2x2 ... special2): the distance in
# metres rolled between seam clacks, as a [min, max] range. Squares are feet.
FOOT = 0.3048
SEAM_SPACING = [
    (1.8, 3.4), (0.9, 3.0), (2 * FOOT, 2 * FOOT), (4 * FOOT, 4 * FOOT), (8 * FOOT, 8 * FOOT),
    (12 * FOOT, 12 * FOOT), (24 * FOOT, 24 * FOOT), (0.4, 0.9), (0.9, 1.8), (1.8, 3.6),
    (0.15, 0.2), (1.5, 1.5), (0.3, 0.6), (0.15, 0.15), (1.0, 2.0), (1.0, 2.0),
]


# The wheel-skid bank's four sets (its four patches, in bank order): smooth,
# rough, wood and dirt or grass, each with soft and hard scrubs.
SKID_CLASS = {'smoothasphalt': 0, 'smoothconcrete': 0, 'metal': 0, 'manhole': 0, 'drain': 0, 'grate': 0,
              'roughasphalt': 1, 'roughconcrete': 1, 'aggregate': 1, 'plywood': 2, 'woodramp': 2, 'grass': 3, 'dirt': 3}
SKID_SETS = [list(range(1, 33)), list(range(33, 65)) + [73, 74, 75, 76], [65, 66, 67, 68] + list(range(77, 85)) + [93, 94, 95, 96],
             list(range(85, 93))]


def audio_surfaces(out: Path) -> list[dict] | None:
    """The game's AudioSurfaceMap: for each audio surface code (the low seven
    bits of a collision triangle's surface id), its rolling type."""
    stock = out.parent / 'stock' / 'skater-collections.json'
    if not stock.exists():
        return None
    for row in json.loads(stock.read_text())['collections']:
        for field in row['fields'].values():
            if field.get('type', '').endswith('AudioSurfaceMap') and field.get('array'):
                result = []
                for item in field['array']['items']:
                    columns = struct.unpack('>18I', bytes.fromhex(item))
                    kind, grind, seams = columns[1], columns[4], columns[6]
                    name, grain, gain = ROLLING_TYPES[kind] if kind < len(ROLLING_TYPES) else ('smoothconcrete', 'concrete_smooth_hard', 1.0)
                    spacing = SEAM_SPACING[seams] if seams < len(SEAM_SPACING) else SEAM_SPACING[0]
                    result.append({'rolling': name, 'grain': grain, 'gain': gain, 'seams': [round(v, 3) for v in spacing],
                                   'grind': grind, 'skid': SKID_CLASS.get(name, 0)})
                return result
    return None


def convert_wheels(disc: Disc, work: Path, out: Path) -> dict:
    result = {}
    for name in disc.entries('wheels.big'):
        if not name.endswith('.snr'):
            continue
        snr = work / name
        snr.write_bytes(disc.read('wheels.big', name))
        wav = snr.with_suffix('.wav')
        vgmstream(snr, wav)
        result[Path(name).stem] = to_ogg(wav, out / 'wheels' / f'{Path(name).stem}.ogg', 'loop')
        snr.unlink()
        wav.unlink()
    return result


def convert_streams(disc: Disc, archive: str, resident: str, folder: str, kind: str, work: Path, out: Path) -> dict:
    headers = disc.entries(resident)
    result = {}
    for name in disc.entries(archive):
        if not name.endswith('.sns'):
            continue
        stem = Path(name).stem
        if f'{stem}.snr' not in headers:
            continue
        (work / f'{stem}.snr').write_bytes(disc.read(resident, f'{stem}.snr'))
        (work / f'{stem}.sns').write_bytes(disc.read(archive, name))
        wav = work / f'{stem}.wav'
        vgmstream(work / f'{stem}.snr', wav)
        result[stem] = to_ogg(wav, out / folder / f'{stem}.ogg', kind, stereo=True)
        for suffix in ('.snr', '.sns', '.wav'):
            (work / f'{stem}{suffix}').unlink(missing_ok=True)
    return result


def build_table(index: dict) -> dict:
    """The engine's sound table: gameplay events to clips, picked by measurement.

    The banks number their clips without names, so these are best guesses from
    each clip's shape (one-shot hit or steady loop), loudness and brightness.
    The table is plain data: the sound picker (tools/sound_picker.html) lets a
    player audition every clip and reassign any event by ear.
    """
    banks = index.get('banks', {})

    def clips(bank, keep, order=lambda c: -c['peak'], limit=8):
        chosen = [(k, v) for k, v in banks.get(bank, {}).items() if keep(v)]
        chosen.sort(key=lambda kv: order(kv[1]))
        return [f'banks/{bank}/{k}.ogg' for k, _ in chosen[:limit]]

    def hit(v):
        return v['attack'] < 0.03 and v['seconds'] < 0.9

    def loop(v):
        return v['steady'] > 0.35 and v['seconds'] > 1.2

    grain = 'concrete_smooth_hard' if 'concrete_smooth_hard' in index.get('grains', {}) else next(iter(index.get('grains', {})), None)
    menu = banks.get('sk8_menu', {})
    ticks = sorted((k for k, v in menu.items() if v['seconds'] < 0.15 and v['centroid'] > 5000), key=int)
    events = {
        # Board pops off its tail: a short, bright crack.
        'pop': {'clips': clips('Skate_Collisions', lambda v: hit(v) and v['seconds'] < 0.45 and 1500 < v['centroid'] < 4000), 'volume': [0.8, 1.0], 'pitch': [0.94, 1.06]},
        # Wheels and deck coming down: longer and lower.
        'land': {'clips': clips('Skate_Collisions', lambda v: hit(v) and 0.35 <= v['seconds'] <= 0.9 and v['centroid'] < 2500, order=lambda c: -c['rms']), 'volume': [0.8, 1.0], 'pitch': [0.95, 1.05]},
        'flip': {'clips': clips('Sk8_Air_Flip_Tricks', lambda v: v['seconds'] < 0.5, limit=14), 'volume': [0.6, 0.85], 'pitch': [0.95, 1.08]},
        'grind_start': {'clips': clips('Skate_Metal', lambda v: hit(v) and v['seconds'] < 0.7 and v['centroid'] > 2500), 'volume': [0.8, 1.0], 'pitch': [0.95, 1.05]},
        'grind_stop': {'clips': clips('board_scrapes', lambda v: True, limit=12), 'volume': [0.6, 0.8], 'pitch': [0.95, 1.05]},
        'skid': {'clips': clips('WHEEL_SKID_BANK', lambda v: True, limit=40), 'volume': [0.5, 0.75], 'pitch': [0.9, 1.1]},
        'seam': {'clips': clips('Seams_Bank', lambda v: v['seconds'] < 0.4, limit=40), 'volume': [0.25, 0.4], 'pitch': [0.95, 1.05]},
        'step': {'clips': clips('fstep_skateshoe1_sm', lambda v: True, limit=40), 'volume': [0.5, 0.7], 'pitch': [0.95, 1.05]},
        'bail': {'clips': [c for n in range(1, 6) for c in clips(f'HOM_Set_{n}', lambda v: hit(v), order=lambda c: -c['rms'], limit=6)], 'volume': [0.8, 1.0], 'pitch': [0.95, 1.05]},
        'body': {'clips': clips('Skate_Collisions', lambda v: hit(v) and v['centroid'] < 1200, order=lambda c: -c['rms'], limit=16), 'volume': [0.5, 0.8], 'pitch': [0.9, 1.05]},
        # Game-mode stingers (Skate 3's "killed it" / "owned it" moments).
        'mode_start': {'clips': clips('killed_it', lambda v: v['seconds'] < 1.0, limit=2), 'volume': [0.8, 0.8], 'pitch': [1, 1]},
        'one_up': {'clips': clips('killed_it', lambda v: 1.5 < v['seconds'] < 2.0, limit=4), 'volume': [0.8, 0.8], 'pitch': [1, 1]},
        'mode_win': {'clips': clips('owned_it', lambda v: True, limit=2), 'volume': [0.9, 0.9], 'pitch': [1, 1]},
        'mode_end': {'clips': clips('killed_it', lambda v: v['seconds'] > 2.0, limit=2), 'volume': [0.7, 0.7], 'pitch': [1, 1]},
        'menu_move': {'clips': [f'banks/sk8_menu/{k}.ogg' for k in ticks[:4]], 'volume': [0.6, 0.6], 'pitch': [1, 1]},
        'menu_select': {'clips': ['banks/sk8_menu/013.ogg'] if '013' in menu else [], 'volume': [0.7, 0.7], 'pitch': [1, 1]},
        'menu_back': {'clips': ['banks/sk8_menu/021.ogg'] if '021' in menu else [], 'volume': [0.7, 0.7], 'pitch': [1, 1]},
    }
    loops = {
        # Rolling: level and pitch follow board speed (m/s).
        'roll': {'clip': f'grains/{grain}.ogg' if grain else None, 'volume': [0.0, 0.75], 'pitch': [0.8, 1.25], 'speed': [0.3, 12.0]},
        'grind': {'clip': (clips('GRINDS', lambda v: loop(v) and v['centroid'] > 3500, limit=1) or [None])[0], 'volume': [0.6, 0.85], 'pitch': [0.9, 1.15], 'speed': [1.0, 10.0]},
        'slide': {'clip': (clips('GRINDS', lambda v: loop(v) and v['centroid'] < 2500, limit=1) or [None])[0], 'volume': [0.6, 0.85], 'pitch': [0.9, 1.15], 'speed': [1.0, 10.0]},
        'air': {'clip': 'wheels/Whls_spins_Jump_1.ogg' if 'Whls_spins_Jump_1' in index.get('wheels', {}) else None, 'volume': [0.0, 0.3], 'pitch': [0.9, 1.1], 'speed': [2.0, 12.0]},
        'wind': {'clip': (clips('sense_of_speed', lambda v: loop(v), limit=1) or [None])[0], 'volume': [0.0, 0.45], 'pitch': [0.9, 1.15], 'speed': [6.0, 16.0]},
    }
    # The front end's own sounds, by the names its screens ask for
    # (FE_SOUND_CORE_A_BUTTON...). A patch Skate 3 left empty gets no clips.
    fe = index.get('fe', {})
    for name, sound in fe.items():
        events[f'FE_SOUND_{name.upper()}'] = {'clips': sound['clips'], 'volume': [sound['volume']] * 2, 'pitch': [1, 1]}
    # The engine's own menus click like the game's core menus.
    for event, name in (('menu_select', 'core_a_button'), ('menu_back', 'core_b_button'), ('menu_move', 'core_nav_down')):
        if fe.get(name, {}).get('clips'):
            events[event] = {'clips': fe[name]['clips'], 'volume': [fe[name]['volume']] * 2, 'pitch': [1, 1]}
    ambience = {name: f'ambience/{name}.ogg' for name in index.get('ambience', {})}
    # Impacts and grinds from the game's material tables (convert_materials).
    impacts = index.get('impacts')
    surfaces = index.get('surfaces') or []

    def bank_clips(bank, numbers):
        return [f'banks/{bank}/{n:03d}.ogg' for n in numbers if f'{n:03d}' in banks.get(bank, {})]
    gameplay = {
        # Powerslide scrubs by surface set (soft ones first: quieter, the
        # lighter scrubs; the louder half for hard slides).
        'skids': {'sets': [bank_clips('WHEEL_SKID_BANK', numbers) for numbers in SKID_SETS],
                  'surface': [s['skid'] for s in surfaces]},
        # Sense of speed: seven wind loops from a breeze to a gale, and
        # gusts for the fastest runs.
        'wind': {'loops': bank_clips('sense_of_speed', range(1, 8)), 'gusts': bank_clips('sense_of_speed', range(8, 18)),
                 'speed': [4.0, 16.0]},
        # Board rattles over rough ground, three sets of two layers.
        'rattles': [bank_clips('Rolling_Rattles', range(1 + 6 * k, 7 + 6 * k)) for k in range(3)],
        'flips': {'flip': bank_clips('Sk8_Air_Flip_Tricks', [11, 12, 13, 14]), 'catch': bank_clips('Sk8_Air_Flip_Tricks', [15, 16]),
                  'spin': bank_clips('Sk8_Air_Flip_Tricks', [17, 18, 19, 20]), 'whoosh': bank_clips('Sk8_Air_Flip_Tricks', [21, 22, 23, 24]),
                  'wheels': bank_clips('Sk8_Air_Flip_Tricks', range(25, 31))},
        # Tail scrapes (reverts), concrete and wood.
        'scrapes': {'concrete': bank_clips('board_scrapes', range(1, 16)), 'wood': bank_clips('board_scrapes', range(16, 31))},
        'squeaks': [bank_clips('Brd_Squeaks', range(1 + 6 * k, 7 + 6 * k)) for k in range(3)],
        'steps': [bank_clips('fstep_skateshoe1_sm', range(1 + 48 * k, 49 + 48 * k)) for k in range(4)],
        'bodyslide': [bank_clips('Bodyslide', range(1 + 5 * k, 6 + 5 * k)) for k in range(4)],
        'cloth': {'jump': bank_clips('Foley_Cloth', range(17, 25)), 'flip': bank_clips('Foley_Cloth', range(9, 17)),
                  'land': bank_clips('Foley_Cloth', range(25, 29))},
    }
    if impacts and impacts.get('grinds'):
        gameplay['grinds'] = dict(impacts['grinds'], surface=[s['grind'] for s in surfaces])
    # Rolling, as the grain player plays it: loops of the take at a few speeds,
    # crossfaded by board speed (not one recording pitched up and down), from
    # the recording of the surface under the wheels.
    bands = index.get('grains', {}).get(grain, {}).get('bands', []) if grain else []
    rolling = {'bands': bands, 'volume': [0.5, 0.8], 'speed': [0.3, 12.0]} if bands else None
    surfaces = index.get('surfaces')
    if rolling and surfaces:
        takes = {stem: info['bands'] for stem, info in index['grains'].items() if info.get('bands')}
        rolling['surfaces'] = {stem: takes[stem] for stem in sorted({s['grain'] for s in surfaces}) if stem in takes}
        rolling['map'] = [s['grain'] if s['grain'] in takes else grain for s in surfaces]
        # The same surfaces on soft wheels (the recording's `_soft` take).
        soft = {stem: stem.replace('_hard', '_soft') for stem in takes if stem.replace('_hard', '_soft') in takes}
        rolling['surfaces'].update({soft[stem]: takes[soft[stem]] for stem in soft})
        rolling['map_soft'] = [soft.get(take, take) for take in rolling['map']]
        rolling['gain'] = [s['gain'] for s in surfaces]
        rolling['seams'] = [s['seams'] for s in surfaces]
    # Each map's background bed (the retail game crossfades area beds; one per
    # map is the starting point).
    beds = {
        'University': '09_univ_campus', 'DownTown': '04_dt_main', 'Industrial': '11_indu_shipyard',
        'SkateSchool': '18_skate_school', 'MaloofMoneyCup': '21_interior_arena_amb',
        'BlackBoxPark': '22_interior_tunnel_amb', 'DownTownSkatePark': '05_dt_parks',
        'IndustrialSkatePark': '20_indu_old_factory', 'MegaPark': '08_univ_mt_low', 'StartPark': '05_dt_parks',
        'title': '23_Press_start_screen',
    }
    maps = {name: ambience[bed] for name, bed in beds.items() if bed in ambience}
    table = {'version': 1, 'events': events, 'loops': loops, 'ambience': ambience, 'maps': maps}
    if rolling:
        table['rolling'] = rolling
    if impacts:
        table['impacts'] = {'parts': impacts.get('parts', {}), 'surfaces': impacts.get('surfaces', []), 'hom': impacts.get('hom', {})}
    if surfaces:
        table['gameplay'] = gameplay
    return table


def default_disc() -> Path | None:
    record = ROOT / 'data' / 'installation.json'
    if not record.exists():
        return None
    iso = Path(json.loads(record.read_text())['source'])
    for candidate in [iso.parent / iso.parent.name / 'data' / 'audio', iso.parent / 'data' / 'audio']:
        if candidate.is_dir():
            return candidate
    return None


def default_out() -> Path:
    record = json.loads((ROOT / 'data' / 'installation.json').read_text())
    return ROOT / 'data' / record['directory'] / 'assets' / 'private' / 'audio'


def convert(disc_audio: Path, out: Path, tool: Path = None, parts=('banks', 'fe', 'impacts', 'grains', 'wheels', 'ambience'),
            rebuild_table: bool = False, report=print) -> None:
    """Convert the disc's audio folder into `out`. Setup calls this for the
    'audio' group with the vgmstream it downloaded."""
    global VGMSTREAM
    if tool is not None:
        VGMSTREAM = Path(tool)
    if not (disc_audio / 'audiofiles.big').exists():
        raise FileNotFoundError(f'{disc_audio}: no audiofiles.big')
    if not VGMSTREAM.exists():
        raise FileNotFoundError(f'vgmstream is missing: {VGMSTREAM}')
    out.mkdir(parents=True, exist_ok=True)
    parts = set(parts)
    disc = Disc(disc_audio)
    index_path = out / 'index.json'
    index = json.loads(index_path.read_text()) if index_path.exists() else {}
    with tempfile.TemporaryDirectory(prefix='skate-audio-') as temporary:
        work = Path(temporary)
        if 'banks' in parts:
            index.setdefault('banks', {})
            for name in SFX_BANKS:
                report(f'Converting game audio: {name}')
                index['banks'][Path(name).stem] = convert_bank(disc, name, work, out)
        if 'fe' in parts:
            report('Converting game audio: front-end sounds')
            index['fe'] = convert_fe(disc, work, out)
        if 'impacts' in parts:
            report('Converting game audio: impacts and grinds')
            impacts = convert_materials(disc, work, out)
            if impacts:
                index['impacts'] = impacts
        if 'grains' in parts:
            report('Converting game audio: rolling grains')
            index['grains'] = convert_grains(disc, work, out)
        surfaces = audio_surfaces(out)
        if surfaces:
            index['surfaces'] = surfaces
        if 'wheels' in parts:
            report('Converting game audio: wheels')
            index['wheels'] = convert_wheels(disc, work, out)
        if 'ambience' in parts:
            report('Converting game audio: ambience')
            index['ambience'] = convert_streams(disc, 'ambience.big', 'ambienceresident.big', 'ambience', 'ambience', work, out)
    index_path.write_text(json.dumps(index, indent=1))
    table = out / 'sounds.json'
    # A table tuned by ear in the sound picker is kept unless asked otherwise.
    if not table.exists() or rebuild_table:
        table.write_text(json.dumps(build_table(index), indent=1))
    else:
        # Keep picks made by ear; only add events this version introduced.
        current = json.loads(table.read_text())
        fresh = build_table(index)
        added = False
        for section in ('events', 'loops', 'maps'):
            for name, value in fresh.get(section, {}).items():
                # Sounds decoded from the game's own patches replace earlier
                # guesses; everything else keeps what was picked by ear.
                exact = section == 'events' and (name.startswith('FE_SOUND_') or name.startswith('menu_')) and 'fe' in index
                if exact and current.get(section, {}).get(name) != value:
                    current.setdefault(section, {})[name] = value
                    added = True
                elif name not in current.setdefault(section, {}):
                    current[section][name] = value
                    added = True
        for section in ('rolling', 'impacts', 'gameplay'):
            if fresh.get(section) and current.get(section) != fresh[section]:
                current[section] = fresh[section]
                added = True
        if added:
            table.write_text(json.dumps(current, indent=1))
    total = sum(f.stat().st_size for f in out.rglob('*.ogg'))
    report(f'Game audio: {total / 1048576:.0f} MB of Ogg Vorbis')


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--disc', type=Path, default=default_disc(), help="the extracted disc data/audio folder")
    parser.add_argument('--out', type=Path, default=None, help="where to write (default: the active install assets/private/audio)")
    parser.add_argument('--only', nargs='*', help='only these parts: banks fe impacts grains wheels ambience')
    parser.add_argument('--rebuild-table', action='store_true', help='regenerate sounds.json even if one exists (discards picks made by ear)')
    args = parser.parse_args()
    if not args.disc:
        sys.exit('Pass --disc <extracted disc>/data/audio (the folder holding audiofiles.big).')
    convert(args.disc, args.out or default_out(), parts=args.only if args.only is not None else ('banks', 'fe', 'impacts', 'grains', 'wheels', 'ambience'),
            rebuild_table=args.rebuild_table)


if __name__ == '__main__':
    main()
