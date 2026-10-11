// Packs a converted install into the zip the web server consumes.
//
//   node scripts/make-game-data-pack.js [install] [--out=<zip>] [--dist] [--trim]
//
// The layout inside matches the published pack — `<prefix>/data/installations/<id>`
// holding assets/, maps/ and settings/ — so a pack built here and one downloaded
// from a URL are interchangeable, and `scripts/serve-web.js` reads it in place
// without extracting anything.
//
// Written to data/packs/skate3-game-data.zip, which is exactly the cache path the
// default pack URL would download to: build it once and SERVE.bat finds it and
// stops reaching for the network. `--dist` also copies it into dist/, where the
// server hands it out as a normal file for another machine to grab.
//
// Left out by default: the conversion's leftovers (_conversion, _load, .cache
// and the customiser's set staging), about a gigabyte the server never serves.
// The published pack is that same trimmed set. `--all` keeps everything.
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const argv = process.argv.slice(2);
const flag = (name) => argv.some((arg) => arg === `--${name}`);
const option = (name) => {
  const found = argv.find((arg) => arg.startsWith(`--${name}=`));
  return found ? found.slice(found.indexOf('=') + 1).trim() : null;
};

// The leftovers the conversion leaves behind are about a gigabyte of files the
// game never reads; `serve-web.js` skips them too.
const SKIP = /(^|\/)(_conversion|_load|\.cache)(\/|$)|^assets\/private\/customisation\/sets\/[^/]+\/(source|roster-work|database)\//;
const PREFIX = option('inner') || 'skate3-web';

function looksLikeInstall(dir) {
  return fs.existsSync(path.join(dir, 'assets', 'private', 'game.json'));
}

function findInstall(explicit) {
  if (explicit) {
    const dir = path.resolve(explicit);
    if (!looksLikeInstall(dir)) throw new Error(`${dir} is not a converted install: no assets/private/game.json`);
    return dir;
  }
  for (const root of [process.env.SKATE_ASSETS, path.resolve('data', 'installations')]) {
    if (!root || !fs.existsSync(root)) continue;
    if (looksLikeInstall(root)) return path.resolve(root);
    const found = fs.readdirSync(root)
      .map((name) => path.join(root, name))
      .filter(looksLikeInstall)
      .map((dir) => ({ dir, at: fs.statSync(dir).mtimeMs }))
      .sort((a, b) => b.at - a.at);
    if (found.length) return path.resolve(found[0].dir);
  }
  throw new Error('No converted install found. Point this at one, or convert your disc first (scripts\\prepare-web-assets.ps1).');
}

function walk(dir, base, out) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    const rel = base ? `${base}/${entry.name}` : entry.name;
    if (entry.isDirectory()) walk(full, rel, out);
    else out.push({ full, rel });
  }
  return out;
}

// `--split[=MB]`: the pack as several zips of at most MB each (480 by
// default), which a CDN can cache whole: Cloudflare caches files up to 512 MB,
// and larger ones are read from storage on every request, many times slower.
// The first zip keeps the usual name and lists the others in
// assets/private/pack-parts.json; each part holds assets/private/pack-part-N.json
// so readers can find its root.
const splitOption = argv.find((arg) => arg === '--split' || arg.startsWith('--split='));
const splitMB = splitOption ? Number(splitOption.split('=')[1] || 480) : 0;
// Startup files first, so the boot reads come from the first parts.
const BOOT = /^(assets|settings)\//;
const LAZY = /^assets\/private\/(customisation\/(?!.*\.json$)|audio\/.*\.ogg$)/;

const install = findInstall(argv.find((arg) => !arg.startsWith('--')));
const id = path.basename(install);
const out = path.resolve(option('out') || path.join('data', 'packs', 'skate3-game-data.zip'));
const stage = path.resolve('data', 'packs', `.stage-${process.pid}`);

fs.rmSync(stage, { recursive: true, force: true });
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.rmSync(`${out}.part`, { force: true });

// Hard links, not copies: the staged tree costs no extra disk and is quick to
// build, and tar only reads it.
const entries = [];
let skipped = 0;
for (const entry of walk(install, '', [])) {
  if (!flag('all') && SKIP.test(entry.rel)) {
    skipped += 1;
    continue;
  }
  entries.push({ ...entry, size: fs.statSync(entry.full).size });
}
const bytes = entries.reduce((sum, entry) => sum + entry.size, 0);
console.log(`install : ${install}`);
console.log(`files   : ${entries.length} (${(bytes / 1048576).toFixed(0)} MB)${skipped ? `, ${skipped} conversion leftovers left out (--all keeps them)` : ''}`);

// Which part each file goes in (one part unless splitting).
const parts = [[]];
if (splitMB) {
  const limit = splitMB * 1048576;
  const rank = (e) => (BOOT.test(e.rel) && !LAZY.test(e.rel) ? 0 : 1);
  entries.sort((a, b) => rank(a) - rank(b) || a.rel.localeCompare(b.rel));
  let filled = 0;
  for (const entry of entries) {
    if (entry.size > limit) throw new Error(`${entry.rel} is ${(entry.size / 1048576).toFixed(0)} MB, over the ${splitMB} MB part size`);
    if (filled + entry.size > limit && parts[parts.length - 1].length) {
      parts.push([]);
      filled = 0;
    }
    parts[parts.length - 1].push(entry);
    filled += entry.size;
  }
} else {
  parts[0] = entries;
}
const partName = (n) => (n === 0 ? path.basename(out) : path.basename(out).replace(/\.zip$/i, `-${n + 1}.zip`));

const zip = (n, list, extra) => {
  const stage = path.resolve('data', 'packs', `.stage-${process.pid}-${n}`);
  fs.rmSync(stage, { recursive: true, force: true });
  const inner = path.join(stage, PREFIX, 'data', 'installations', id);
  for (const entry of list) {
    const target = path.join(inner, entry.rel);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    try {
      fs.linkSync(entry.full, target);
    } catch {
      fs.copyFileSync(entry.full, target);
    }
  }
  for (const [rel, text] of extra) {
    fs.mkdirSync(path.dirname(path.join(inner, rel)), { recursive: true });
    fs.writeFileSync(path.join(inner, rel), text);
  }
  const file = path.join(path.dirname(out), partName(n));
  fs.rmSync(file, { force: true });
  execFileSync('tar', ['-a', '-c', '-f', file, PREFIX], { cwd: stage, stdio: ['ignore', 'ignore', 'inherit'] });
  fs.rmSync(stage, { recursive: true, force: true });
  const size = fs.statSync(file).size;
  console.log(`pack    : ${file}  (${(size / 1048576).toFixed(0)} MB, ${list.length} files)`);
  if (splitMB && size > 512 * 1048576) console.warn(`          over 512 MB: a CDN will not cache it whole`);
  return file;
};

console.log('zipping…');
const written = parts.map((list, n) => {
  const extra = [];
  if (splitMB) extra.push([`assets/private/pack-part-${n + 1}.json`, JSON.stringify({ part: n + 1, of: parts.length }) + '\n']);
  if (splitMB && n === 0) extra.push(['assets/private/pack-parts.json', JSON.stringify(parts.slice(1).map((_, k) => ({ file: partName(k + 1), marker: `assets/private/pack-part-${k + 2}.json` })), null, 2) + '\n']);
  return zip(n, list, extra);
});
const size = written.reduce((sum, file) => sum + fs.statSync(file).size, 0);
if (splitMB) console.log(`parts   : ${written.length} zips, ${(size / 1048576).toFixed(0)} MB in all — upload every one of them together`);

const parsed = require('./game-data-pack').parsePack(out);
const maps = [...parsed.entries.keys()].filter((name) => /^maps\/[^/]+\.skate$/.test(name)).length;
console.log(`contents: ${parsed.entries.size} files, ${maps} maps, game.json=${parsed.entries.has('assets/private/game.json')}, root ${parsed.wrapper || '(none)'}`);

if (flag('dist')) {
  for (const file of written) {
    const target = path.join('dist', path.basename(file));
    fs.copyFileSync(file, target);
    console.log(`dist    : ${target} — served at /${path.basename(file)}`);
  }
}
console.log('');
console.log('SERVE.bat now uses this instead of downloading the default pack URL.');
