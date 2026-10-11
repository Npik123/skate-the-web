"""Skate 3's SPLC sound patches (.bnk), read and rendered the way the game plays them.

An SPLC bank holds samples plus *patches*: the sounds the game actually asks
for. A patch layers one or more nodes; each node plays a slice of one sample
(or a random pick among a few), delayed, faded, panned, pitched and scaled.
Playing the bank's raw samples instead gives the wrong sound: the front end's
A-button click, for one, is 65 ms out of a longer sample sped up by a tritone.

Layout (big-endian), as found in data/audio/audiofiles.big sk8_menu.bnk:
  0x00  'SPLC', u32 version (3), u32 table end, u32 patch count (twice),
        u32 0, u32 sample count, char[36] bank name
  0x40  patch entries, 36 bytes each:
          u16 id, u16 node count, f32 pitch, f32 volume, f32 0, f32 1,
          f32 length (ms), 12 bytes 0
  then  the cues, 72 bytes each (u32 cue count at 0x10): u16 patch count,
        u16 0, then u16 patch id + u16 pad per patch; the game asks for cues
  then  the nodes of every patch in patch order, each:
          8 bytes, u16 1, u16 0, u8 variant count, u8 mode, u16 0,
          then per variant 68 bytes (72 apart): u16 sample, u16 0,
          f32 gain, f32 pitch, f32 1, f32 pan (degrees), f32 delay (s),
          f32 slice start, f32 slice end, f32 fade-in end, f32 fade-out start
          (seconds into the sample; 0 end = to the end), then curve/flags
  table end: the samples (EA SNR), as `audio.splc_samples` splits them.

The game names sounds through VLT fields typed by the bank's name (the "fe"
rows' sk8_menu field, a surface's Skate_Collisions fields...). Such a value is
a cue: value - patch count = cue index (0 = no sound).
"""
import struct

import numpy as np


def patches(raw: bytes) -> list[dict]:
    """Every patch of an SPLC bank: its pitch, length and nodes."""
    if raw[:4] != b'SPLC':
        raise ValueError('not an SPLC bank')
    table_end, count, cue_count = struct.unpack_from('>III', raw, 8)
    entries = []
    for i in range(count):
        at = 0x40 + 36 * i
        pid, nodes = struct.unpack_from('>HH', raw, at)
        pitch, volume, _, _, length = struct.unpack_from('>5f', raw, at + 4)
        entries.append({'id': pid, 'count': nodes, 'pitch': pitch, 'volume': volume, 'ms': length, 'nodes': []})
    at = 0x40 + 36 * count + 72 * cue_count - 8
    for entry in entries:
        for _ in range(entry['count']):
            start = at
            while raw[at + 8:at + 12] != b'\x00\x01\x00\x00':
                at += 4
                if at - start > 64 or at >= table_end:
                    raise ValueError(f'SPLC node not found near {start:#x}')
            variants = []
            for v in range(raw[at + 12]):
                base = at + 16 + 72 * v
                fields = [0.0] * 9
                for k in range(9):
                    # The last node runs into the sample data: its trailing
                    # fields are absent (0 = whole sample, no fades).
                    if base + 8 + 4 * k <= table_end:
                        fields[k] = struct.unpack_from('>f', raw, base + 4 + 4 * k)[0]
                gain, pitch, _, pan, delay, begin, end, fade_in, fade_out = fields
                variants.append({'sample': struct.unpack_from('>H', raw, base)[0], 'gain': gain, 'pitch': pitch or 1.0,
                                 'pan': pan, 'delay': delay, 'begin': begin, 'end': end,
                                 'fade_in': fade_in, 'fade_out': fade_out})
            entry['nodes'].append(variants)
            at = at + 16 + 72 * len(variants) - 4
    return entries


def cues(raw: bytes) -> list[list[int]]:
    """Each cue's patch ids, by cue index."""
    count, cue_count = struct.unpack_from('>II', raw, 12)
    base = 0x40 + 36 * count - 4
    out = []
    for cue in range(cue_count):
        at = base + 72 * cue
        n = min(struct.unpack_from('>H', raw, at)[0], 16)
        out.append([struct.unpack_from('>H', raw, at + 4 + 4 * i)[0] for i in range(n)])
    return out


def cue_patches(raw: bytes, value: int) -> list[int]:
    """The patches a VLT sound value of this bank plays ([] for none)."""
    count = struct.unpack_from('>I', raw, 12)[0]
    table = cues(raw)
    return table[value - count] if count <= value < count + len(table) else []


def _resample(mono: np.ndarray, ratio: float) -> np.ndarray:
    if abs(ratio - 1) < 1e-4 or len(mono) < 2:
        return mono
    positions = np.arange(0, len(mono) - 1, ratio)
    return np.interp(positions, np.arange(len(mono)), mono).astype(np.float32)


def _variant(sample: np.ndarray, rate: int, v: dict, pitch: float) -> np.ndarray:
    """One node variant, as mono samples at `rate`: slice, fades, gain, pitch."""
    begin = max(0, int(v['begin'] * rate))
    end = int(v['end'] * rate) if v['end'] > v['begin'] else len(sample)
    clip = sample[begin:min(end, len(sample))].astype(np.float32).copy()
    n = len(clip)
    if n == 0:
        return clip
    if v['fade_in'] > v['begin']:
        k = min(n, int((v['fade_in'] - v['begin']) * rate))
        clip[:k] *= np.linspace(0, 1, k, endpoint=False, dtype=np.float32)
    if v['begin'] <= v['fade_out'] < (v['end'] if v['end'] > v['begin'] else len(sample) / rate):
        k0 = int((v['fade_out'] - v['begin']) * rate)
        if k0 < n:
            clip[k0:] *= np.linspace(1, 0, n - k0, dtype=np.float32)
    return _resample(clip * v['gain'], pitch * v['pitch'])


def render(entry: dict, samples: list[tuple[np.ndarray, int]], choice: int = 0) -> tuple[np.ndarray, int]:
    """Mix a patch to stereo (unclipped: layers can sum past full scale).
    Nodes with several variants (the game picks one at random) take variant
    `choice` (wrapping)."""
    rate = max((r for _, r in samples), default=48000)
    layers = []
    for node in entry['nodes']:
        if not node:
            continue
        v = node[choice % len(node)]
        if v['gain'] <= 0 or v['sample'] >= len(samples):
            continue
        sample, sample_rate = samples[v['sample']]
        mono = _variant(sample, sample_rate, v, entry['pitch'])
        if sample_rate != rate:
            mono = _resample(mono, sample_rate / rate)
        # Constant-power pan from the authored angle.
        angle = (np.clip(v['pan'], -90, 90) / 90 + 1) * np.pi / 4
        offset = int(max(0.0, v['delay']) * rate)
        layers.append((offset, np.stack([mono * np.cos(angle), mono * np.sin(angle)], axis=1) * np.sqrt(2)))
    length = max((o + len(m) for o, m in layers), default=0)
    out = np.zeros((length, 2), np.float32)
    for offset, layer in layers:
        out[offset:offset + len(layer)] += layer
    return out * entry['volume'], rate


def variants(entry: dict) -> int:
    return max((len(node) for node in entry['nodes']), default=1)


def fe_sounds(collections: list[dict]) -> dict[str, dict]:
    """The front end's named sounds from the VLT "fe" rows: name -> sk8_menu
    value (a cue, see cue_patches), volume."""
    sounds = {}
    for row in collections:
        if row.get('class') != 'fe':
            continue
        patch = volume = None
        for field in row['fields'].values():
            if field['type'] == 'sk8_menu':
                patch = int(field['data'], 16)
            elif field['type'] == 'EA::Reflection::Float' and volume is None:
                volume = struct.unpack('>f', bytes.fromhex(field['data']))[0]
        if patch:
            sounds[row['key']] = {'value': patch, 'volume': 1.0 if volume is None else volume}
    return sounds
