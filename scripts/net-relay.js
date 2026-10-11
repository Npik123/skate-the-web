// Multiplayer for the browser client: the part Steam plays on desktop.
//
// Browsers cannot open UDP sockets or run the Steam relay helper, so the page
// keeps one WebSocket to this server at /net and the server does both jobs the
// helper did:
//
//   * Lobby directory. Text frames carry the same `skate_net::directory`
//     Request/Response JSON the helper exchanged: `CMD <request>` from the
//     game, `EVENT <response>` back. Owner events repeat every 250 ms, exactly as
//     the helper re-announced Steam's owner, which is how joins and host
//     migration reach the session.
//   * Kicks. `KICK <peer>` from a lobby's owner removes that player and bans
//     them from the lobby for as long as it lasts: their peer id and their
//     address (hashed, never logged), so reconnecting does not get them back
//     in. The kicked player is sent `KICKED`.
//   * Packet relay. Binary frames are datagrams: the game sends
//     `[u64 LE target peer][packet]` and receives `[u64 LE source peer][packet]`.
//     A packet is forwarded only between members of the same lobby, and dropped
//     (never queued) when a client falls behind, so the session sees the same
//     loss it was built to tolerate on UDP.
//
// Peer and lobby ids are random 48-bit integers: well inside the range a
// JavaScript number holds exactly, and opaque to the game like a SteamID.
const crypto = require('crypto');

const PATH = '/net';
const GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';
const CAPACITY = 10; // skate_net::lobby::DEFAULT_PLAYERS
const MAX_CAPACITY = 20; // skate_net::lobby::MAX_PLAYERS
// A lobby's size as its owner asked (0 or nonsense: the default).
function capacityOf(value) {
  const n = Math.floor(Number(value) || 0);
  return n >= 2 ? Math.min(n, MAX_CAPACITY) : CAPACITY;
}
const PAGE_SIZE = 5; // skate_net::directory::PAGE_SIZE
const MTU = 1200; // skate_net::packed::MTU
const MAX_MESSAGE = 4096;
// A client this far behind is not reading; its datagrams are dropped, as UDP would.
const BACKLOG = 1 << 20;

const peers = new Map(); // id -> Peer
// Bytes sent since the server last asked (its traffic log).
let sent = { bytes: 0, packets: 0 };
function takeTraffic() {
  const out = sent;
  sent = { bytes: 0, packets: 0 };
  return out;
}
const lobbies = new Map(); // id -> { id, owner, members:Set, map, physics, label, bannedPeers:Set, bannedAddresses:Set }

// A connection's address, hashed: the proxy's client header when there is one.
function addressOf(request) {
  const forwarded = request.headers['cf-connecting-ip'] || String(request.headers['x-forwarded-for'] || '').split(',')[0].trim();
  const raw = forwarded || request.socket.remoteAddress || '';
  return crypto.createHash('sha256').update(`skate-relay:${raw}`).digest('hex');
}

function randomId(taken) {
  for (;;) {
    const id = crypto.randomBytes(6).readUIntLE(0, 6);
    if (id !== 0 && !taken.has(id)) return id;
  }
}

// --- WebSocket framing (RFC 6455, server side) -------------------------------

function frame(opcode, payload) {
  const length = payload.length;
  let header;
  if (length < 126) {
    header = Buffer.from([0x80 | opcode, length]);
  } else if (length < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x80 | opcode;
    header[1] = 126;
    header.writeUInt16BE(length, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x80 | opcode;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(length), 2);
  }
  return Buffer.concat([header, payload]);
}

class Peer {
  constructor(socket, address) {
    this.socket = socket;
    this.address = address;
    this.id = randomId(peers);
    this.lobby = null;
    this.buffer = Buffer.alloc(0);
    this.fragments = null;
    this.fragmentOpcode = 0;
    peers.set(this.id, this);
    socket.setNoDelay(true);
    socket.on('data', (chunk) => this.data(chunk));
    socket.on('close', () => this.closed());
    socket.on('error', () => socket.destroy());
    this.text(`READY ${this.id}`);
  }

  write(opcode, payload) {
    if (this.socket.destroyed) return;
    const data = frame(opcode, payload);
    sent.bytes += data.length;
    sent.packets += 1;
    this.socket.write(data);
  }

  text(message) {
    this.write(0x1, Buffer.from(message, 'utf8'));
  }

  event(request, event) {
    this.text(`EVENT ${JSON.stringify({ request, event })}`);
  }

  data(chunk) {
    this.buffer = this.buffer.length ? Buffer.concat([this.buffer, chunk]) : chunk;
    for (;;) {
      const b = this.buffer;
      if (b.length < 2) return;
      const fin = (b[0] & 0x80) !== 0;
      const opcode = b[0] & 0x0f;
      const masked = (b[1] & 0x80) !== 0;
      let length = b[1] & 0x7f;
      let offset = 2;
      if (length === 126) {
        if (b.length < 4) return;
        length = b.readUInt16BE(2);
        offset = 4;
      } else if (length === 127) {
        if (b.length < 10) return;
        length = Number(b.readBigUInt64BE(2));
        offset = 10;
      }
      // Clients must mask; anything larger than a control message is not ours.
      if (!masked || length > MAX_MESSAGE) {
        this.socket.destroy();
        return;
      }
      if (b.length < offset + 4 + length) return;
      const mask = b.subarray(offset, offset + 4);
      const payload = Buffer.from(b.subarray(offset + 4, offset + 4 + length));
      for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i & 3];
      this.buffer = b.subarray(offset + 4 + length);
      if (opcode === 0x8) {
        this.write(0x8, Buffer.alloc(0));
        this.socket.end();
        return;
      }
      if (opcode === 0x9) {
        this.write(0xa, payload);
        continue;
      }
      if (opcode === 0xa) continue;
      if (opcode === 0x0) {
        if (!this.fragments) continue;
        this.fragments.push(payload);
        if (!fin) continue;
        const whole = Buffer.concat(this.fragments);
        this.fragments = null;
        this.message(this.fragmentOpcode, whole);
        continue;
      }
      if (!fin) {
        this.fragmentOpcode = opcode;
        this.fragments = [payload];
        continue;
      }
      this.message(opcode, payload);
    }
  }

  message(opcode, payload) {
    if (opcode === 0x2) this.packet(payload);
    else if (opcode === 0x1) this.control(payload.toString('utf8'));
  }

  // [u64 LE target][datagram] -> target receives [u64 LE source][datagram].
  packet(payload) {
    if (payload.length < 8 || payload.length > 8 + MTU || !this.lobby) return;
    const target = Number(payload.readBigUInt64LE(0));
    const lobby = lobbies.get(this.lobby);
    const to = peers.get(target);
    if (!lobby || !to || target === this.id || !lobby.members.has(target)) return;
    if (to.socket.writableLength > BACKLOG) return;
    payload.writeBigUInt64LE(BigInt(this.id), 0);
    to.write(0x2, payload);
  }

  control(text) {
    if (text.startsWith('KICK ')) {
      this.kick(Number(text.slice(5)));
      return;
    }
    if (!text.startsWith('CMD ') || text.length > 1400) return;
    let request;
    try {
      request = JSON.parse(text.slice(4));
    } catch {
      return;
    }
    const id = Number(request && request.id) || 0;
    const command = request && request.command;
    if (!command || typeof command !== 'object') return;
    const [kind] = Object.keys(command);
    const args = command[kind] || {};
    if (kind === 'Browse') this.browse(id, args);
    else if (kind === 'Host') this.host(id, args);
    else if (kind === 'Join') this.join(id, args);
    else if (kind === 'Capacity') this.resize(id, args);
  }

  browse(request, { page = 0, map, physics }) {
    const list = [...lobbies.values()].sort((a, b) => a.id - b.id);
    const total = list.length;
    const last = Math.max(0, Math.floor((total - 1) / PAGE_SIZE));
    const shown = Math.min(Math.max(0, Number(page) || 0), last);
    const rows = list.slice(shown * PAGE_SIZE, shown * PAGE_SIZE + PAGE_SIZE).map((lobby) => ({
      id: lobby.id,
      map: lobby.label,
      players: Math.min(lobby.members.size, lobby.capacity),
      capacity: lobby.capacity,
      compatible: lobby.map === map && lobby.physics === physics,
    }));
    this.event(request, { Rows: { page: shown, total, rows } });
  }

  host(request, { map, physics, label, capacity }) {
    this.leave();
    const lobby = {
      id: randomId(lobbies),
      owner: this.id,
      members: new Set([this.id]),
      map,
      physics,
      label: clean(label),
      capacity: capacityOf(capacity),
      bannedPeers: new Set(),
      bannedAddresses: new Set(),
    };
    lobbies.set(lobby.id, lobby);
    this.lobby = lobby.id;
    console.log(`relay: ${this.id} hosts lobby ${lobby.id} (${lobby.label})`);
    this.event(request, ownerEvent(lobby, this.id));
  }

  join(request, { lobby: wanted }) {
    this.leave();
    const lobby = lobbies.get(Number(wanted));
    if (!lobby) {
      this.event(request, { Error: 'Lobby unavailable. It may have closed; refresh and try again.' });
      return;
    }
    if (lobby.bannedPeers.has(this.id) || lobby.bannedAddresses.has(this.address)) {
      this.event(request, { Error: "You were kicked from this session and can't rejoin it." });
      return;
    }
    if (lobby.members.size >= lobby.capacity) {
      this.event(request, { Error: 'Lobby is full. Refresh to check for a free slot.' });
      return;
    }
    lobby.members.add(this.id);
    this.lobby = lobby.id;
    console.log(`relay: ${this.id} joins lobby ${lobby.id} (${lobby.members.size} players)`);
    this.event(request, ownerEvent(lobby, this.id));
  }

  // The lobby owner changes how many players it holds.
  resize(request, { capacity }) {
    const lobby = this.lobby && lobbies.get(this.lobby);
    if (!lobby || lobby.owner !== this.id) {
      this.event(request, { Error: 'Only the host can change the lobby size.' });
      return;
    }
    lobby.capacity = capacityOf(capacity);
    this.event(request, 'Done');
  }

  // The lobby owner removes a player and bans them from this lobby.
  kick(target) {
    const lobby = this.lobby && lobbies.get(this.lobby);
    if (!lobby || lobby.owner !== this.id || target === this.id || !lobby.members.has(target)) return;
    const peer = peers.get(target);
    lobby.bannedPeers.add(target);
    // Not the host's own network: that would lock out the host's household.
    if (peer && peer.address !== this.address) lobby.bannedAddresses.add(peer.address);
    console.log(`relay: ${target} kicked from lobby ${lobby.id}`);
    if (peer) {
      peer.leave();
      peer.text('KICKED');
    } else {
      lobby.members.delete(target);
    }
  }

  leave() {
    const lobby = this.lobby && lobbies.get(this.lobby);
    this.lobby = null;
    if (!lobby) return;
    lobby.members.delete(this.id);
    if (lobby.members.size === 0) {
      lobbies.delete(lobby.id);
      console.log(`relay: lobby ${lobby.id} closed`);
    } else if (lobby.owner === this.id) {
      // Steam elects a new owner when the owner leaves; the repeated owner
      // event below is what tells the session to migrate.
      [lobby.owner] = [...lobby.members].sort((a, b) => a - b);
      console.log(`relay: lobby ${lobby.id} host moved to ${lobby.owner}`);
    }
  }

  closed() {
    this.leave();
    peers.delete(this.id);
  }
}

function ownerEvent(lobby, own) {
  return { Owner: { lobby: lobby.id, owner: lobby.owner, own } };
}

// skate_net::directory::label: no control characters, 48 chars, 64 bytes
// (room for "Map | host name").
function clean(text) {
  let out = '';
  for (const c of String(text || '')) {
    if (/\p{Cc}/u.test(c)) continue;
    if ([...out].length >= 48 || Buffer.byteLength(out + c) > 64) break;
    out += c;
  }
  return out || 'Unknown map';
}

setInterval(() => {
  for (const lobby of lobbies.values()) {
    for (const id of lobby.members) {
      const peer = peers.get(id);
      if (peer) peer.event(0, ownerEvent(lobby, id));
    }
  }
}, 250).unref();

/// `server.on('upgrade', upgrade)`: accepts WebSockets at /net, refuses the rest.
function upgrade(request, socket) {
  const key = request.headers['sec-websocket-key'];
  if (request.url.split('?')[0] !== PATH || !key || !/websocket/i.test(request.headers.upgrade || '')) {
    socket.end('HTTP/1.1 400 Bad Request\r\n\r\n');
    return;
  }
  const accept = crypto.createHash('sha1').update(key + GUID).digest('base64');
  socket.write(
    'HTTP/1.1 101 Switching Protocols\r\n' +
    'Upgrade: websocket\r\n' +
    'Connection: Upgrade\r\n' +
    `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
  );
  new Peer(socket, addressOf(request));
}

module.exports = { upgrade, PATH, takeTraffic };
