// One local server for the browser client and the user's own converted data.
//
//   node scripts/serve-web.js [dist-root] [port] [asset-root] [--local]
//
// dist/ is served at the root. Anything under assets/, maps/ or settings/ is
// read from the converted folder instead — the one your own disc produced — and
// /manifest.json lists what is there. The client streams those files on demand
// and keeps nothing, which is why dist/ never contains game data and no second
// server is needed.
//
// `/boot-package` hands the whole boot set (assets/ and settings/, a few
// thousand small files) over as one response, so the engine's startup is a
// single transfer instead of one request per file. Everything carries an ETag,
// so a reload revalidates instead of re-sending.
//
// Binds every interface so other machines on your network can use it; pass
// --local to keep it on this one. Either way this is your own machine's data.
const http = require('http');
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');
const os = require('os');
const crypto = require('crypto');
const { Readable, Transform } = require('stream');
const { pipeline } = require('stream/promises');
const relay = require('./net-relay');
const gameDataPack = require('./game-data-pack');
const { acquirePack, parsePack, entryStream, entryGuard } = gameDataPack;

// Conversion leftovers the game never reads: the customiser's raw disc files,
// roster work area and database dumps are about a gigabyte on their own.
const SKIP = /(^|\/)(_conversion|_load|\.cache)(\/|$)|^assets\/private\/customisation\/sets\/[^/]+\/(source|roster-work|database)\//;
const ASSET = /^(assets|maps|settings)\//;
const MOUNT = /^(assets|settings)\//;
// The customiser's part models and textures (about 1 GB) download when worn or
// browsed, through the engine's asset reader; only its catalogs load up front.
// Game audio (.ogg) streams in the same way: only its sound table mounts.
const LAZY = /^assets\/private\/(customisation\/(?!.*\.json$)|audio\/.*\.ogg$|dropper\/(objects|textures)\/)/;
// Converted files the engine never reads (see web/host.js): kept out of the
// boot package so nobody downloads them.
// Also the stock data no part of the engine opens (measured with ?trace-reads
// over full sessions, and nothing in the code names them): the PS3 ragdoll
// banks (ropa2, 86 MB), the original .rx2 models (content, 30 MB), the
// editor and pedestrian animation banks (only OnBoard/OffBoard are read),
// recipes, stream lists, the minimap and the default skater's textures:
// about 150 MB every player downloaded and held in memory for nothing.
const UNUSED = /^assets\/private\/(customisation\/sets\/[^/]+\/catalog\.json|native-props\/|native-backdrops\/|default_skater\/|stock\/data\/(ropa2|content|cacrecipes|stream)\/|stock\/data\/maps\/minimap\.|stock\/data\/anim\/(?!(OnBoard|OffBoard)\.abin$))/;
const mounted = (file) => MOUNT.test(file) && !LAZY.test(file) && !UNUSED.test(file);
const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.wasm': 'application/wasm',
  '.skate': 'application/octet-stream',
  '.glb': 'model/gltf-binary',
  '.png': 'image/png',
};
// Game data is mostly already-compressed payloads, so only the client is worth
// squeezing: the wasm module alone loses about two thirds of its size.
// Maps only shrink about 15%, but a city is 370 MB, and every friend who
// loads one pulls it through this machine's upload while others are playing.
const COMPRESSIBLE = new Set(['.wasm', '.js', '.json', '.html', '.css', '.skate', '.irradiance']);

const argv = process.argv.slice(2);
const local = argv.includes('--local');
const args = argv.filter((arg) => !arg.startsWith('--'));
const distRoot = path.resolve(args[0] || 'dist');
const port = Number(args[1] || 80);

// --download-limit=<megabits per second>: the most all game-data downloads
// together may use. The multiplayer relay shares this machine's upload; when a
// friend downloads a city at full speed, everyone's packets queue behind it,
// players time out, and the session stutters. Off by default; set it a little
// below your upload speed (or SKATE_DOWNLOAD_MBPS) if loading in lags others.
const downloadLimit = Number((argv.find((a) => a.startsWith('--download-limit=')) || '').split('=')[1]
  || process.env.SKATE_DOWNLOAD_MBPS || 0) * 1e6 / 8;
const bucket = { tokens: 0, at: Date.now() };
function throttle() {
  return new Transform({
    transform(chunk, _encoding, done) {
      const pass = () => {
        const now = Date.now();
        bucket.tokens = Math.min(downloadLimit / 4, bucket.tokens + (now - bucket.at) / 1000 * downloadLimit);
        bucket.at = now;
        if (bucket.tokens >= Math.min(chunk.length, downloadLimit / 4)) {
          bucket.tokens -= chunk.length;
          done(null, chunk);
        } else {
          setTimeout(pass, 20);
        }
      };
      pass();
    },
  });
}
// Streams a body to a download, through the shared limit when one is set.
function stream(source, response) {
  return downloadLimit > 0 ? pipeline(source, throttle(), response) : pipeline(source, response);
}

function looksLikeInstall(dir) {
  return fs.existsSync(path.join(dir, 'assets', 'private', 'game.json'));
}

// Newest converted install wins, so adding a second map to a newer folder is
// picked up without editing a script.
function findAssets(explicit) {
  const named = [explicit, process.env.SKATE_ASSETS].filter(Boolean);
  for (const candidate of named) {
    const dir = path.resolve(candidate);
    if (!looksLikeInstall(dir)) {
      console.error(`${dir} does not look like a converted install: no assets/private/game.json`);
      process.exit(1);
    }
    return dir;
  }
  const base = path.resolve('data', 'installations');
  if (!fs.existsSync(base)) return null;
  const found = fs.readdirSync(base)
    .map((name) => path.resolve(base, name))
    .filter(looksLikeInstall)
    .map((dir) => ({ dir, at: fs.statSync(dir).mtimeMs }))
    .sort((a, b) => b.at - a.at);
  return found.length ? found[0].dir : null;
}

// A converted folder on this machine is always the fastest source, so it wins.
// `--pack=<url|file>` (or SKATE_PACK) is the fallback when there is none, and
// asking for it explicitly means the folder on disk is left alone; `--no-pack`
// turns the fallback off entirely.
//
// `--client-pack[=<url>]` (or SKATE_CLIENT_PACK) is different: this server holds
// no game data at all. Browsers are told the zip's address (/config.json) and
// read files straight out of it with range requests (web/pack.js), so the data
// never passes through this machine. The zip's host must allow cross-origin
// GET/HEAD with Range (see DEPLOY.md). Without a URL it is the default pack.
const clientPackFlag = argv.find((arg) => arg === '--client-pack' || arg.startsWith('--client-pack='));
const clientPack = clientPackFlag
  ? (clientPackFlag.includes('=') ? clientPackFlag.slice(clientPackFlag.indexOf('=') + 1).trim() : gameDataPack.DEFAULT_PACK_URL)
  : (process.env.SKATE_CLIENT_PACK || '').trim() || null;
const explicitAssets = Boolean(args[2] || process.env.SKATE_ASSETS);
const explicitPack = argv.some((arg) => arg.startsWith('--pack')) && !argv.includes('--no-pack');
const assetRoot = clientPack || (explicitPack && !explicitAssets) ? null : findAssets(args[2]);
const packSource = assetRoot || clientPack ? null : gameDataPack.packSource(argv, process.env);
let packFile = null;
let pack = null;
// Matches web/host.js: zips published next to the game data pack.
const ADD_ON_PACKS = [
  { file: 'skate3-audio.zip', marker: 'assets/private/audio/sounds.json', what: 'game audio', override: true },
  // The original UI screens replace the game data's older copies.
  { file: 'skate3-ui.zip', marker: 'assets/private/frontend/buttons.json', what: 'original UI screens', override: true },
  { file: 'skate3-dropper.zip', marker: 'assets/private/dropper/catalog.json', what: 'Object Dropper props', override: true },
];

// The converted folder holds a few thousand files. Walking it is done once at
// startup and then refreshed in the background, never on a request: Node
// serves every player and the multiplayer relay from one thread, and a blocking
// walk per page load froze all of them for its duration.
async function walk(dir, base, out) {
  for (const entry of await fs.promises.readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    const rel = base ? `${base}/${entry.name}` : entry.name;
    if (entry.isDirectory()) await walk(full, rel, out);
    else if (!SKIP.test(rel)) {
      const stat = await fs.promises.stat(full);
      out.push({ path: rel, size: stat.size, at: stat.mtimeMs });
    }
  }
  return out;
}

let inventory = [];
const RESCAN_MS = 30_000;

// Custom characters everyone on this server can wear (CREATE > Custom Models).
// Copy whole model folders into <this folder>/entries/: each is named by its
// 64-character id and holds manifest.json, character.glb and preview.png,
// exactly as the Windows build's importer writes them to
// %LOCALAPPDATA%\Skate3RustEngine\custom-characters\entries. The client sees
// them under userdata/custom-characters/, where the web build looks.
const CHARACTERS = 'userdata/custom-characters/';
const charactersRoot = path.resolve((argv.find((a) => a.startsWith('--custom-characters=')) || '').split('=').slice(1).join('=')
  || process.env.SKATE_CUSTOM_CHARACTERS || path.join(distRoot, '..', 'custom-characters'));
let characters = [];
let characterNotes = '';

async function refreshCharacters() {
  const entries = path.join(charactersRoot, 'entries');
  try {
    await fs.promises.mkdir(entries, { recursive: true });
  } catch {}
  let folders = [];
  try {
    folders = await fs.promises.readdir(entries, { withFileTypes: true });
  } catch {
    characters = [];
    return;
  }
  const found = [];
  const skipped = [];
  for (const folder of folders) {
    if (!folder.isDirectory()) continue;
    if (!/^[0-9a-f]{64}$/.test(folder.name)) {
      skipped.push(`${folder.name} (folder name must be the model's 64-character id)`);
      continue;
    }
    const files = [];
    for (const name of ['manifest.json', 'character.glb', 'preview.png']) {
      try {
        const stat = await fs.promises.stat(path.join(entries, folder.name, name));
        if (stat.isFile()) files.push({ path: `${CHARACTERS}entries/${folder.name}/${name}`, size: stat.size, at: stat.mtimeMs });
      } catch {}
    }
    if (files.length === 3) found.push(...files);
    else skipped.push(`${folder.name.slice(0, 12)}… (needs manifest.json, character.glb and preview.png)`);
  }
  characters = found;
  const notes = skipped.length ? `skipped ${skipped.join(', ')}` : '';
  if (notes && notes !== characterNotes) console.log(`characters : ${notes}`);
  characterNotes = notes;
}

async function refreshInventory() {
  await refreshCharacters();
  if (!assetRoot && packSource && !pack) {
    try {
      packFile = await acquirePack(packSource);
      pack = parsePack(packFile);
      // A split pack (make-game-data-pack.js --split): the first zip lists the
      // rest, fetched and read beside it.
      const partsEntry = pack.entries.get('assets/private/pack-parts.json');
      if (partsEntry) {
        const chunks = [];
        for await (const chunk of entryStream(packFile, 'pack-parts.json', partsEntry)) chunks.push(chunk);
        for (const { file: name, marker } of JSON.parse(Buffer.concat(chunks).toString('utf8'))) {
          const source = /^https?:/.test(packSource) ? new URL(name, packSource).href
            : path.join(path.dirname(path.resolve(packFile)), name);
          const file = await acquirePack(source);
          for (const [entryName, entry] of parsePack(file, marker).entries) pack.entries.set(entryName, { ...entry, file });
          console.log(`game data  : part ${name}`);
        }
      }
      // Add-on packs published beside the game data (the converted audio):
      // their entries join the same table, each remembering its own zip.
      for (const addOn of ADD_ON_PACKS) {
        const source = /^https?:/.test(packSource) ? new URL(addOn.file, packSource).href
          : path.join(path.dirname(path.resolve(packFile)), addOn.file);
        try {
          const file = await acquirePack(source);
          for (const [name, entry] of parsePack(file, addOn.marker).entries) {
            if (addOn.override || !pack.entries.has(name)) pack.entries.set(name, { ...entry, file });
          }
          console.log(`add-on     : ${addOn.what} from ${file}`);
        } catch (error) {
          console.log(`add-on     : no ${addOn.what} pack (${error.message}); continuing without it`);
        }
      }
    } catch (error) {
      console.error(`game data  : ${error.message}`);
      console.error('             Continuing without game data; pass --pack=<url|file> or --no-pack.');
    }
  }
  if (!assetRoot && !pack) return;
  try {
    inventory = assetRoot
      ? (await walk(assetRoot, '', [])).filter((file) => ASSET.test(file.path))
      : [...pack.entries]
        .filter(([name]) => ASSET.test(name) && !SKIP.test(name))
        .map(([name, entry]) => ({ path: name, size: entry.size, at: entry.at }));
  } catch (error) {
    console.error('could not list game data:', error.message);
  }
}

// One file of the game data, wherever it lives: a stream either way, so the boot
// package and a single map request do not care which source is behind it.
function openAsset(name) {
  if (assetRoot) return fs.createReadStream(path.join(assetRoot, name));
  const entry = pack.entries.get(name);
  if (!entry) throw new Error(`${name} is not in ${packFile}`);
  return entryStream(entry.file || packFile, name, entry).pipe(entryGuard(name, entry));
}

function scan() {
  return inventory;
}

// What the client asks for by name.
function manifest() {
  return [...scan(), ...characters].map(({ path: file, size }) => ({ path: file, size }));
}

// The boot set plus the framing the client splits it with: a 4-byte
// little-endian header length, the header, then every file back to back.
function bootPackage() {
  const files = manifest().filter((file) => mounted(file.path));
  const total = files.reduce((sum, file) => sum + file.size, 0);
  const newest = scan().reduce((at, file) => Math.max(at, file.at), 0);
  const header = Buffer.from(JSON.stringify({ files, total }), 'utf8');
  const prefix = Buffer.alloc(4);
  prefix.writeUInt32LE(header.length);
  return {
    files,
    header,
    prefix,
    etag: `W/"boot-${files.length}-${total}-${Math.round(newest)}"`,
    length: 4 + header.length + total,
  };
}

// Resolve `rel` under `root`, refusing anything that escapes it.
function under(root, rel) {
  if (!root) return null;
  const file = path.resolve(root, rel);
  const prefix = root.endsWith(path.sep) ? root : root + path.sep;
  if (!file.startsWith(prefix)) return null;
  return fs.existsSync(file) && fs.statSync(file).isFile() ? file : null;
}

// A file's ETag is a hash of its contents (computed once per size and mtime).
// Size and mtime alone are not enough: a rebuilt file can keep its size, and
// unzipping a release can restore an old mtime, and then a browser keeps its
// old copy (an old skate3rust.js beside a new wasm stops the engine).
const etags = new Map();
async function etagOf(file, stat) {
  const key = `${file}|${stat.size}|${stat.mtimeMs}`;
  if (!etags.has(key)) {
    etags.set(key, new Promise((resolve, reject) => {
      const hash = crypto.createHash('sha1');
      fs.createReadStream(file)
        .on('data', (chunk) => hash.update(chunk))
        .on('error', reject)
        .on('end', () => resolve(`"${hash.digest('hex').slice(0, 20)}"`));
    }));
  }
  try {
    return await etags.get(key);
  } catch (error) {
    etags.delete(key);
    throw error;
  }
}

function fresh(request, etag) {
  return request.headers['if-none-match'] === etag;
}

// Compressed copies of the client files, made once per version. Compressing
// the 56 MB wasm module costs seconds of CPU; doing it for every visitor put
// that load on the machine everyone was playing through.
const STATIC_CACHE = path.join(os.tmpdir(), 'skate-web-static');
const staticJobs = new Map();

function compressedStatic(file, etag, kind = 'gz') {
  const name = `${crypto.createHash('sha1').update(file + etag).digest('hex').slice(0, 20)}.${kind}`;
  const target = path.join(STATIC_CACHE, name);
  if (fs.existsSync(target)) return Promise.resolve(target);
  if (staticJobs.has(name)) return staticJobs.get(name);
  const size = fs.statSync(file).size;
  const encoder = kind === 'br'
    ? zlib.createBrotliCompress({ params: {
      [zlib.constants.BROTLI_PARAM_QUALITY]: 11,
      [zlib.constants.BROTLI_PARAM_LGWIN]: 24,
      [zlib.constants.BROTLI_PARAM_SIZE_HINT]: size,
    } })
    : zlib.createGzip({ level: 6 });
  const job = (async () => {
    fs.mkdirSync(STATIC_CACHE, { recursive: true });
    const partial = `${target}.${process.pid}.part`;
    await pipeline(fs.createReadStream(file), encoder, fs.createWriteStream(partial));
    fs.renameSync(partial, target);
    return target;
  })().finally(() => staticJobs.delete(name));
  staticJobs.set(name, job);
  return job;
}

// Brotli at its strongest makes the wasm module about 40% smaller than gzip,
// but takes a minute or two to make: until it exists, gzip goes out instead.
function brotliReady(file, etag) {
  const name = `${crypto.createHash('sha1').update(file + etag).digest('hex').slice(0, 20)}.br`;
  const target = path.join(STATIC_CACHE, name);
  if (fs.existsSync(target)) return target;
  if (!staticJobs.has(name)) compressedStatic(file, etag, 'br').catch((error) => console.error('brotli failed:', error.message));
  return null;
}

// What this machine uploads, logged every 15 minutes, so a bandwidth bill can
// be traced to the files (or the relay) behind it.
const traffic = new Map();
function count(kind, bytes) {
  const entry = traffic.get(kind) || { bytes: 0, requests: 0 };
  entry.bytes += bytes;
  entry.requests += 1;
  traffic.set(kind, entry);
}
setInterval(() => {
  const relayed = relay.takeTraffic();
  if (relayed.bytes) traffic.set('relay', { bytes: relayed.bytes, requests: relayed.packets });
  if (!traffic.size) return;
  const rows = [...traffic].sort((a, b) => b[1].bytes - a[1].bytes)
    .map(([kind, { bytes, requests }]) => `${kind} ${(bytes / 1048576).toFixed(0)} MB/${requests}`);
  console.log(`traffic 15m: ${rows.join(', ')}`);
  traffic.clear();
}, 15 * 60_000).unref();

async function send(request, response, file, status = 200) {
  const stat = await fs.promises.stat(file);
  const extension = path.extname(file).toLowerCase();
  const etag = await etagOf(file, stat);
  // Script and wasm URLs carry their build (?v=<hash>, scripts/build-web.ps1):
  // one never changes, so browsers keep it without asking again.
  const versioned = /[?&]v=[0-9a-f]+/.test(request.url) && (extension === '.js' || extension === '.wasm');
  const headers = {
    'Cache-Control': versioned ? 'public, max-age=31536000, immutable' : 'private, no-cache',
    'ETag': etag,
    'Vary': 'Accept-Encoding',
  };
  if (fresh(request, etag)) {
    response.writeHead(304, headers);
    response.end();
    count('304', 0);
    return;
  }
  const accepts = request.headers['accept-encoding'] || '';
  const compressible = COMPRESSIBLE.has(extension);
  const brotli = compressible && /\bbr\b/.test(accepts) && brotliReady(file, etag);
  const gzip = !brotli && compressible && /\bgzip\b/.test(accepts);
  const body = brotli || (gzip ? await compressedStatic(file, etag) : file);
  const length = brotli || gzip ? (await fs.promises.stat(body)).size : stat.size;
  response.writeHead(status, {
    ...headers,
    'Content-Type': MIME[extension] || 'application/octet-stream',
    ...(brotli ? { 'Content-Encoding': 'br' } : gzip ? { 'Content-Encoding': 'gzip' } : {}),
    // The uncompressed size, for the page's download progress.
    ...(brotli || gzip ? { 'X-Raw-Length': String(stat.size) } : {}),
    'Content-Length': length,
  });
  if (request.method === 'HEAD') {
    response.end();
    return;
  }
  count(extension === '.wasm' ? 'wasm' : extension.slice(1) || 'other', length);
  await stream(fs.createReadStream(body), response);
}

async function* bootBody(pkg) {
  yield pkg.prefix;
  yield pkg.header;
  for (const file of pkg.files) {
    const source = openAsset(file.path);
    for await (const chunk of source) yield chunk;
  }
}

// A pack entry cannot be handed to `send`: it is an offset inside one big file,
// not a path. Recompression is left to the boot package's cached artifact, so a
// single map is gzipped as it goes.
async function sendPackEntry(request, response, name, entry) {
  const extension = path.extname(name).toLowerCase();
  const etag = `W/"pack-${entry.size}-${Math.round(entry.at)}"`;
  const headers = { 'Cache-Control': 'private, no-cache', 'ETag': etag, 'Vary': 'Accept-Encoding' };
  if (fresh(request, etag)) {
    response.writeHead(304, headers);
    response.end();
    return;
  }
  const gzip = COMPRESSIBLE.has(extension) && /\bgzip\b/.test(request.headers['accept-encoding'] || '');
  response.writeHead(200, {
    ...headers,
    'Content-Type': MIME[extension] || 'application/octet-stream',
    ...(gzip ? { 'Content-Encoding': 'gzip' } : { 'Content-Length': entry.size }),
  });
  if (request.method === 'HEAD') {
    response.end();
    return;
  }
  const source = openAsset(name);
  await stream(gzip ? source.pipe(zlib.createGzip({ level: 6 })) : source, response);
}

// The boot set is mostly text, animation and raw colour data — 252 MB of it
// compresses to about 97 MB — so it goes out as one gzip stream the browser
// decodes for us. The unpacked length rides in X-Boot-Bytes, because with
// Content-Encoding set the client cannot read it off Content-Length.
const BOOT_CACHE = path.join(os.tmpdir(), 'skate-web-boot');
const bootJobs = new Map();

// Compressing 252 MB takes a few seconds of CPU, so the result is written next
// to the temporary files once per version of the game data and streamed from
// there afterwards.
function bootArtifact(pkg) {
  const name = `${crypto.createHash('sha1').update(pkg.etag).digest('hex').slice(0, 16)}.gz`;
  return { name, file: path.join(BOOT_CACHE, name) };
}

async function compressedPackage(pkg) {
  const artifact = bootArtifact(pkg);
  if (fs.existsSync(artifact.file) && fs.statSync(artifact.file).size > 0) return artifact.file;
  if (bootJobs.has(artifact.name)) return bootJobs.get(artifact.name);
  const job = (async () => {
    fs.mkdirSync(BOOT_CACHE, { recursive: true });
    const partial = `${artifact.file}.${process.pid}.part`;
    await pipeline(Readable.from(bootBody(pkg)), zlib.createGzip({ level: 6 }), fs.createWriteStream(partial));
    fs.renameSync(partial, artifact.file);
    return artifact.file;
  })().finally(() => bootJobs.delete(artifact.name));
  bootJobs.set(artifact.name, job);
  return job;
}

function dropStaleArtifacts() {
  if (!assetRoot && !pack) return;
  const keep = bootPackage();
  const current = bootArtifact(keep).name;
  try {
    for (const name of fs.readdirSync(BOOT_CACHE)) {
      if (/^[0-9a-f]{16}\.gz$/.test(name) && name !== current) fs.rmSync(path.join(BOOT_CACHE, name), { force: true });
    }
  } catch {
    // No cache directory yet; the first request creates it.
  }
}

async function sendBootPackage(request, response) {
  const pkg = bootPackage();
  const gzip = /\bgzip\b/.test(request.headers['accept-encoding'] || '');
  const headers = {
    'Content-Type': 'application/octet-stream',
    'Cache-Control': 'private, no-cache',
    'ETag': pkg.etag,
    'Vary': 'Accept-Encoding',
    'X-Boot-Bytes': String(pkg.length),
  };
  if (fresh(request, pkg.etag)) {
    response.writeHead(304, headers);
    response.end();
    return;
  }
  // `?raw=1`: the same gzip stream as a plain file (no Content-Encoding), so
  // the page can keep it compressed in its saved downloads and skip the whole
  // transfer next time with an ETag check (web/host.js). X-Boot-Raw marks it.
  if (/[?&]raw=1(&|$)/.test(request.url)) {
    const file = await compressedPackage(pkg);
    response.writeHead(200, { ...headers, 'Content-Type': 'application/gzip', 'X-Boot-Raw': '1', 'Content-Length': fs.statSync(file).size });
    if (request.method === 'HEAD') response.end();
    else await stream(fs.createReadStream(file), response);
    return;
  }
  if (!gzip) {
    response.writeHead(200, { ...headers, 'Content-Length': pkg.length });
    if (request.method !== 'HEAD') await stream(Readable.from(bootBody(pkg)), response);
    else response.end();
    return;
  }
  const file = await compressedPackage(pkg);
  response.writeHead(200, {
    ...headers,
    'Content-Encoding': 'gzip',
    'Content-Length': fs.statSync(file).size,
  });
  if (request.method === 'HEAD') response.end();
  else await stream(fs.createReadStream(file), response);
}

const server = http.createServer((request, response) => {
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    response.writeHead(405);
    response.end('method not allowed');
    return;
  }
  const url = decodeURIComponent(request.url.split('?')[0]);
  // Where the page should read game data from: null means this server.
  if (url === '/config.json') {
    // A server that fetched its data from a public zip points browsers at the
    // same zip: they read it from there (Cloudflare R2 bandwidth) and fall back
    // to this server only when they cannot (the zip's host refuses CORS).
    const offload = !clientPack && pack && /^https:/.test(packSource || '') && !process.env.SKATE_NO_OFFLOAD;
    const body = JSON.stringify(offload ? { pack: packSource, fallback: true } : { pack: clientPack });
    response.writeHead(200, {
      'Content-Type': 'application/json',
      'Content-Length': Buffer.byteLength(body),
      'Cache-Control': 'private, no-cache',
    });
    response.end(body);
    return;
  }
  if (url === '/manifest.json') {
    const body = JSON.stringify(manifest());
    response.writeHead(200, {
      'Content-Type': 'application/json',
      'Content-Length': Buffer.byteLength(body),
      'Cache-Control': 'private, no-cache',
    });
    response.end(body);
    return;
  }
  if (url === '/boot-package') {
    sendBootPackage(request, response).catch((error) => {
      console.error('boot package failed:', error.message);
      response.destroy();
    });
    return;
  }
  const rel = url === '/' ? 'index.html' : url.replace(/^\/+/, '');
  // The client's own fixture map lives in dist/, so fall back to it when the
  // converted folder has no such file.
  if (pack && ASSET.test(rel)) {
    const entry = pack.entries.get(rel);
    if (entry) {
      sendPackEntry(request, response, rel, entry).catch((error) => {
        if (!response.headersSent) response.writeHead(500);
        response.destroy(error);
      });
      return;
    }
  }
  const file = rel.startsWith(CHARACTERS) ? under(charactersRoot, rel.slice(CHARACTERS.length))
    : (ASSET.test(rel) ? under(assetRoot, rel) : null) || under(distRoot, rel);
  if (!file) {
    response.writeHead(404);
    response.end('not found');
    return;
  }
  send(request, response, file).catch((error) => {
    if (!response.headersSent) response.writeHead(500);
    response.destroy(error);
  });
});
// Multiplayer: WebSocket lobby directory and packet relay (scripts/net-relay.js).
server.on('upgrade', relay.upgrade);
// List the game data before accepting players, then keep the list fresh in
// the background so newly converted maps appear without a restart.
setInterval(refreshInventory, RESCAN_MS).unref();
refreshInventory().then(() => server.listen(port, local ? '127.0.0.1' : '0.0.0.0', () => {
  const files = manifest();
  dropStaleArtifacts();
  const bytes = files.reduce((sum, file) => sum + file.size, 0);
  const boot = bootPackage();
  console.log(`dist/      : ${distRoot}`);
  const packNote = pack && pack.wrapper ? ` (pack root ${pack.wrapper})` : '';
  console.log(`game data  : ${clientPack ? `browsers read it directly from ${clientPack}` : assetRoot || (packFile ? `${packFile}${packNote}` : '(none — client fixtures only)')}`);
  if (assetRoot || pack) {
    const maps = files.filter((file) => /^maps\/[^/]+\.skate$/i.test(file.path)).length;
    console.log(`             ${files.length} files, ${(bytes / 1048576).toFixed(0)} MB, ${maps} maps`);
    console.log(`boot set   : ${boot.files.length} files, ${(boot.length / 1048576).toFixed(0)} MB in one gzip transfer (browser decodes it)`);
  }
  console.log(`characters : ${characters.length / 3} custom model(s) in ${path.join(charactersRoot, 'entries')}`);
  if (downloadLimit > 0) console.log(`downloads  : limited to ${(downloadLimit * 8 / 1e6).toFixed(0)} Mbit/s in total, leaving the rest for multiplayer`);
  console.log(`listening  : http://127.0.0.1${port === 80 ? '' : `:${port}`}/${local ? '' : '  (and on this machine\'s LAN address)'}`);
  console.log(`multiplayer: WebSocket relay at ${relay.PATH} (host or join a session from the game's menu)`);
  console.log('Open that address. The client reads these files on demand; it stores nothing.');
  if ((assetRoot || pack) && !local) console.log('This hands out a copy of the game data. Keep it on a network you control.');
}));
