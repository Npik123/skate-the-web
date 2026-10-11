// Game data as a single zip, for machines that have no converted folder.
//
//   node scripts/serve-web.js --pack=https://example/skate3-game-data.zip
//   node scripts/serve-web.js --pack=D:\packs\skate3-game-data.zip
//
// The archive is downloaded once into data/packs/ and then read in place: the
// central directory gives the file list, and each file is inflated straight out
// of the archive on demand. Nothing is extracted, so a 1.4 GB pack costs 1.4 GB
// on disk, not 2.8 GB, and the server's boot package and ETag handling work the
// same as they do for a converted folder.
//
// The data inside is Skate 3's content. Whatever this points at has to be
// something you are allowed to serve — your own converted disc, or a copy you
// make for machines on your own network.
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');
const { Readable, Transform } = require('stream');
const { pipeline } = require('stream/promises');

// Used when no converted folder is installed on this machine.
const DEFAULT_PACK_URL = 'https://gamedata.xposed.wtf/skate3-game-data.zip';

const PACK_DIR = path.resolve('data', 'packs');
const EOCD = 0x06054b50;
const EOCD64 = 0x06064b50;
const EOCD64_LOCATOR = 0x07064b50;
const CENTRAL = 0x02014b50;
const LOCAL = 0x04034b50;

// A zip of a converted folder usually wraps everything in one directory
// (skate3-web/assets/private/game.json). The engine's paths start at assets/,
// so the wrapper is found once and stripped from every entry.
function wrapperPrefix(names, marker = 'assets/private/game.json') {
  if (names.includes(marker)) return '';
  // Everything before the marker is the wrapper, however deep it goes:
  // `skate3-web/data/installations/<id>/assets/private/game.json` is a whole
  // converted output zipped up, and it unwraps to the same assets/, maps/,
  // settings/ the server expects.
  for (const name of names) {
    if (!name.endsWith(marker)) continue;
    const at = name.length - marker.length;
    if (at > 0) return name.slice(0, at);
  }
  // No marker to anchor on: a zip of a single folder still has to be unwrapped.
  const roots = new Set(names.map((name) => name.slice(0, name.indexOf('/') + 1)));
  if (roots.size === 1) {
    const [only] = roots;
    if (only && only !== 'assets/' && only !== 'maps/' && only !== 'settings/') return only;
  }
  return '';
}

// --pack=<url|file>, SKATE_PACK, or the default above. --no-pack turns it off.
function packSource(argv, env) {
  if (argv.includes('--no-pack')) return null;
  const flag = argv.find((arg) => arg.startsWith('--pack'));
  // Trimmed: `set SKATE_PACK=path && node …` in cmd.exe leaves the space before
  // the && on the value, which is otherwise a path that cannot be opened.
  if (flag) return flag.includes('=') ? flag.slice(flag.indexOf('=') + 1).trim() : '';
  return env.SKATE_PACK !== undefined ? String(env.SKATE_PACK).trim() : DEFAULT_PACK_URL;
}

function packCachePath(source) {
  const named = source.startsWith('http') || source.startsWith('file:') ? new URL(source).pathname : source;
  const name = path.basename(named) || 'skate3-game-data.zip';
  return path.join(PACK_DIR, name.endsWith('.zip') ? name : `${name}.zip`);
}

// Downloads once and reuses what is already there. A half written file never
// takes the place of a good one: the download lands on .part first.
async function acquirePack(source, log = console.log) {
  if (!source) return null;
  source = String(source).trim();
  const local = !source.startsWith('http://') && !source.startsWith('https://');
  if (local) {
    const file = path.resolve(source);
    if (!fs.existsSync(file)) throw new Error(`${file} does not exist`);
    return file;
  }
  const target = packCachePath(source);
  if (fs.existsSync(target) && fs.statSync(target).size > 0) {
    log(`game data  : reusing ${target}`);
    return target;
  }
  fs.mkdirSync(PACK_DIR, { recursive: true });
  log(`game data  : downloading ${source}`);
  const response = await fetch(source);
  if (!response.ok) throw new Error(`${source}: HTTP ${response.status}`);
  const expected = Number(response.headers.get('content-length')) || 0;
  const partial = `${target}.part`;
  let at = 0;
  let reported = 0;
  const body = Readable.fromWeb(response.body);
  body.on('data', (chunk) => {
    at += chunk.length;
    if (at - reported < 32 * 1048576) return;
    reported = at;
    const mb = (n) => (n / 1048576).toFixed(0);
    log(`             ${mb(at)}${expected ? ` / ${mb(expected)}` : ''} MB`);
  });
  await pipeline(body, fs.createWriteStream(partial));
  if (expected && at !== expected) throw new Error(`${source}: got ${at} of ${expected} bytes`);
  fs.renameSync(partial, target);
  log(`             ${(at / 1048576).toFixed(0)} MB saved to ${target}`);
  return target;
}

// Reads the end-of-central-directory record, walking back over any archive
// comment, then the directory itself. Zip64 fields are followed when the 32-bit
// slots are saturated, so a pack over 4 GB still resolves.
// `marker` is the file that anchors the unwrapping: the game data's
// game.json by default, or an add-on pack's own (the audio's sounds.json).
function parsePack(file, marker) {
  const fd = fs.openSync(file, 'r');
  try {
    const size = fs.fstatSync(fd).size;
    const tailLength = Math.min(size, 65557);
    const tail = Buffer.alloc(tailLength);
    fs.readSync(fd, tail, 0, tailLength, size - tailLength);
    let eocd = -1;
    for (let i = tail.length - 22; i >= 0; i--) {
      if (tail.readUInt32LE(i) === EOCD) {
        eocd = i;
        break;
      }
    }
    if (eocd < 0) throw new Error(`${file} is not a zip: no end-of-central-directory record`);

    let count = tail.readUInt16LE(eocd + 10);
    let directorySize = tail.readUInt32LE(eocd + 12);
    let directoryOffset = tail.readUInt32LE(eocd + 16);
    if (eocd >= 20 && tail.readUInt32LE(eocd - 20) === EOCD64_LOCATOR) {
      const record = Number(tail.readBigUInt64LE(eocd - 20 + 8));
      const header = Buffer.alloc(56);
      fs.readSync(fd, header, 0, 56, record);
      if (header.readUInt32LE(0) !== EOCD64) throw new Error(`${file} has a broken zip64 record`);
      count = Number(header.readBigUInt64LE(32));
      directorySize = Number(header.readBigUInt64LE(40));
      directoryOffset = Number(header.readBigUInt64LE(48));
    }

    const directory = Buffer.alloc(directorySize);
    let read = 0;
    while (read < directorySize) {
      read += fs.readSync(fd, directory, read, directorySize - read, directoryOffset + read);
    }

    const modified = fs.statSync(file).mtimeMs;
    const found = [];
    let at = 0;
    while (at + 46 <= directory.length && directory.readUInt32LE(at) === CENTRAL) {
      const method = directory.readUInt16LE(at + 10);
      const compressed = directory.readUInt32LE(at + 20);
      const uncompressed = directory.readUInt32LE(at + 24);
      const nameLength = directory.readUInt16LE(at + 28);
      const extraLength = directory.readUInt16LE(at + 30);
      const commentLength = directory.readUInt16LE(at + 32);
      const localOffset = directory.readUInt32LE(at + 42);
      const name = directory.toString('utf8', at + 46, at + 46 + nameLength);
      const entry = { compressed, size: uncompressed, localOffset };
      if (compressed === 0xffffffff || uncompressed === 0xffffffff || localOffset === 0xffffffff) {
        let extra = at + 46 + nameLength;
        const end = extra + extraLength;
        while (extra + 4 <= end && directory.readUInt16LE(extra + 2) > 0) {
          const id = directory.readUInt16LE(extra);
          const length = directory.readUInt16LE(extra + 2);
          if (id === 0x0001) {
            let slot = extra + 4;
            if (uncompressed === 0xffffffff) {
              entry.size = Number(directory.readBigUInt64LE(slot));
              slot += 8;
            }
            if (compressed === 0xffffffff) {
              entry.compressed = Number(directory.readBigUInt64LE(slot));
              slot += 8;
            }
            if (localOffset === 0xffffffff) {
              entry.localOffset = Number(directory.readBigUInt64LE(slot));
              slot += 8;
            }
            break;
          }
          extra += 4 + length;
        }
      }
      if (!name.endsWith('/')) found.push({ ...entry, name, method });
      at += 46 + nameLength + extraLength + commentLength;
    }

    const wrapper = wrapperPrefix(found.map((entry) => entry.name), marker);
    const entries = new Map();
    for (const entry of found) {
      if (wrapper && !entry.name.startsWith(wrapper)) continue;
      const name = wrapper ? entry.name.slice(wrapper.length) : entry.name;
      if (name) entries.set(name, { ...entry, name, at: modified });
    }
    if (entries.size === 0 && count !== 0) throw new Error(`${file} lists no files`);
    return { file, entries, wrapper };
  } finally {
    fs.closeSync(fd);
  }
}

// The local header repeats the name and extra field with its own lengths, so the
// data starts after those, not after the central directory's copy.
function entryStream(file, name, entry) {
  if (entry.compressed === 0) return Readable.from([]);
  const fd = fs.openSync(file, 'r');
  let start;
  try {
    const header = Buffer.alloc(30);
    fs.readSync(fd, header, 0, 30, entry.localOffset);
    if (header.readUInt32LE(0) !== LOCAL) throw new Error(`${name}: broken local header`);
    start = entry.localOffset + 30 + header.readUInt16LE(26) + header.readUInt16LE(28);
  } finally {
    fs.closeSync(fd);
  }
  const raw = fs.createReadStream(file, { start, end: start + entry.compressed - 1 });
  if (entry.method === 0) return raw;
  if (entry.method !== 8) {
    throw new Error(`${name}: zip compression method ${entry.method} is not supported (store or deflate only)`);
  }
  return raw.pipe(zlib.createInflateRaw());
}

// A truncated or corrupt entry would otherwise reach the engine as a short file.
function entryGuard(name, entry) {
  let seen = 0;
  return new Transform({
    transform(chunk, _encoding, done) {
      seen += chunk.length;
      if (seen > entry.size) done(new Error(`${name}: unpacked larger than the archive says`));
      else done(null, chunk);
    },
    flush(done) {
      done(seen === entry.size ? null : new Error(`${name}: unpacked ${seen} of ${entry.size} bytes`));
    },
  });
}

module.exports = { DEFAULT_PACK_URL, acquirePack, packSource, parsePack, entryStream, entryGuard };
