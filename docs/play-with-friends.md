# Play with friends (invite codes)

A player hosts their single-player world with one click and gets a short code such as
`K7QM-3XPD`. Friends paste the code (on the multiplayer screen in game, or on the launcher's
Play page) and join. No port forwarding, no accounts.

It is built on what Open to LAN already does: the integrated server (`crates/horizon-client/src/lan`)
listens on a local TCP port and speaks the vanilla protocol. Play with friends adds a tunnel in front
of that port and a small relay service (`crates/horizon-relay`) that introduces the two ends. Neither
the integrated server nor the guest's client changes: the tunnel carries exactly the bytes vanilla's
protocol would send, between a loopback port on each side.

```
 guest's game                                                         host's game
 ┌──────────────┐  127.0.0.1:A  ┌────────────┐     direct TCP       ┌────────────┐  127.0.0.1:LAN  ┌──────────────┐
 │ client       │──────────────▶│ guest      │◀═══ (preferred) ════▶│ host       │────────────────▶│ integrated   │
 │ (unchanged)  │               │ tunnel     │                      │ tunnel     │                 │ server (LAN) │
 └──────────────┘               └─────┬──────┘                      └─────┬──────┘                 └──────────────┘
                                      │      ┌──────────────────┐         │
                                      └─────▶│  horizon-relay   │◀────────┘
                                 join, PAKE, │ codes, rendezvous│ register, control,
                                 or the whole│ byte relay       │ attach per guest
                                 stream      └──────────────────┘
```

## What vanilla has

Vanilla 26.3 (read with `tools/rea_vanilla.py`) has two related things, and neither is this:

- **Open to LAN** (`IntegratedServer.publishServer`, `ShareToLanScreen`, `LanServerPinger`,
  `PublishCommand`): the integrated server listens on a port and announces itself by multicast on the
  local network. This is the baseline Horizon follows exactly: Play with friends uses the very same
  server, the same `IntegratedServer.MAX_PLAYERS` (8), the same offline-mode login, game mode and
  command settings as Open to LAN. Hosting through a code *is* opening to LAN, plus the tunnel.
- **Realms invite codes** (`com.mojang.realmsclient.dto.InviteCode`, `RealmsInviteCodesScreen`):
  codes that add a player to a Realm, a server Mojang hosts. They need accounts and Mojang's
  service; nothing in the client tunnels a local world.

So the code, the relay and the screens are Horizon's own, in the launcher's design language (the
existing `ui_theme` widgets, so the redesign in flight restyles them with everything else).

## Identity

Like LAN today: offline mode. A guest joins with the name their game uses; the server gives them the
offline UUID for it; the host's own profile is reserved so nobody can take the host's name. Online-mode
identity (the host checks guests against Mojang's session servers when both are signed in to a
Microsoft account) can come later on top of the same tunnel, since the tunnel carries the login
packets unchanged. Because every tunnelled guest reaches the integrated server from `127.0.0.1`,
`/ban-ip` has nothing useful to act on for them (the LAN host has no such command anyway).

## Codes

A code is 8 symbols of Crockford base32 (`0-9 A-Z` without `I L O U`), shown as two groups of four:
`K7QM-3XPD`. Input ignores case, spaces and dashes, and reads `O` as `0` and `I`/`L` as `1`.

- The first four symbols are the **nameplate** (20 bits). The relay allocates it at random among the
  free ones when the host registers, and uses it to find the host. It is the only part the relay sees.
- The last four are the **secret** (20 bits). The host's game draws it from the OS random generator
  and never sends it anywhere. It is the password of the key exchange below.

A code lives as long as the host keeps sharing: when the host stops sharing, closes the world or loses
its connection to the relay, the nameplate is freed (the relay keeps a bare "ended" mark for 15 minutes
so a guest gets "this invite has ended" rather than "unknown code").

Guessing: to join a world uninvited, someone needs an active nameplate (the relay limits resolution
attempts per address and in total, see below) and then the secret, for which each attempt is one online run of the
key exchange (20 bits: one chance in a million). After 5 failed exchanges the host's game retires the
code and shows a new one, so a nameplate found by luck is worth at most 5 tries.

## Encryption: SPAKE2, then ChaCha20-Poly1305

The relay must not be able to read or change game traffic, but all the two ends share is a short
code, and the relay sees part of it. A key derived directly from the code would fall to an offline
search in seconds, so the ends run a **password-authenticated key exchange** (SPAKE2 over Ed25519,
the RustCrypto `spake2` crate, the construction magic-wormhole uses for the same problem):

1. Over the relayed connection, each end sends one SPAKE2 message built from the secret and bound to
   the nameplate and the session number. Each derives a 32-byte key `K`. With different secrets the
   keys differ, and nothing in the messages helps an offline guess: a dishonest relay gets exactly one
   guess per session, and a wrong guess shows up as a failed join.
2. Every stream that carries the session (the relayed one, and each direct connection tried) then
   exchanges two fresh 16-byte randoms in the clear and derives its own keys with HKDF-SHA256 from `K`,
   the randoms and the direction. Frames are ChaCha20-Poly1305 with a per-direction counter nonce, at
   most 16 KiB of payload each. The first frame each way is a key confirmation: a wrong code or a
   tampered byte ends the stream.

What the relay **can** see: the nameplate, the IP addresses and ports of the host and of each guest
(it is their TCP peer), when sessions start and end, and the size and timing of the encrypted frames
of relayed sessions. What it **can't**: the secret half of the code, the key, the game bytes (names,
chat, positions, the world), or the candidate addresses the ends exchange (they travel encrypted).

The relay connection itself is plain TCP (a TLS front would hide the nameplate from the network
path, but adds certificates to a service that holds nothing worth hiding).

## Connecting: direct first, then the relay

Every session starts through the relay, so the key exchange always has a path. Then:

1. Over the encrypted relayed stream, the ends swap their **candidates**: the local addresses they can
   be reached on (found by asking the OS which source address it would use toward the relay, toward
   the public IPv4 internet and toward the public IPv6 internet; this finds the LAN address, a global
   IPv6 address and a tailnet address when there is one) plus the address the relay saw them come
   from (their public, reflexive address), all with the port of their **direct socket**. The guest's
   join connection and the host's attach connection leave from that port, so the address the relay
   saw is the NAT's mapping of it. (The host's long-lived control connection uses another port, so
   it never holds the direct port's connection to the relay.)
2. Both ends then dial every candidate of the other at once, for up to 3 seconds, retrying every 250
   ms, from their direct socket's port (`SO_REUSEADDR`/`SO_REUSEPORT`, so the same port also listens and
   carried the relay connection). Dialling from both sides at once is **TCP simultaneous open**: each
   side's outgoing SYN opens its own NAT for the other's, which gets through NATs that map a local port
   to the same public port for every destination (most home routers) and stateful firewalls (IPv6).
   On the same LAN, on a tailnet, with IPv6 or with a forwarded port the plain dial wins first.
3. Each direct stream that connects is authenticated with the session key as above. The guest keeps
   the first one that passes, says so over the relayed stream, and both ends close the rest. If none
   passes in time, the session stays on the relayed stream: the relay copies the encrypted bytes.

Why TCP and not UDP hole punching with QUIC: punching UDP succeeds more often across NATs (published
measurements put it around 80% against 60–65% for TCP), but it needs a reliable transport over UDP,
which means QUIC (an async runtime and a QUIC/TLS stack in the game) for a stream that then still needs
the same end-to-end layer. The relay fallback makes every session connect either way; the direct path
only saves latency and relay bandwidth. The tunnel takes any byte stream (`Read + Write`), so a QUIC
transport can be added later beside the TCP one without touching the rest.

The reflexive candidate is only as good as the relay's view of addresses: if the hosting platform's
TCP proxy hides client addresses (Railway's documentation doesn't say), the relay sees the proxy's
and that candidate simply never connects; the LAN, IPv6, tailnet and forwarded-port paths still work.
`GET /whoami` on the relay shows what it sees, to check after deploying.

`HORIZON_FRIENDS_PATH=relay` skips the direct attempts (players who don't want to share their address
with the other end, and the tests); `=direct` refuses the relay fallback (tests).

## The relay protocol

One TCP port. The first line of each connection is a JSON object (at most 4 KiB, within 10 seconds);
a connection whose first bytes are `GET ` gets an HTTP answer instead (`GET /health`, for the hosting
platform's health check), so one port serves both.

| From | Line | Meaning |
| --- | --- | --- |
| host | `{"op":"host","v":1,"protocol":P,"game":"26.3"}` | register; the relay answers `{"op":"registered","nameplate":"K7QM","token":T,"observed":"ip:port"}` |
| guest | `{"op":"join","v":1,"nameplate":"K7QM","protocol":P,"game":"26.3"}` | resolve; the relay announces the session to the host, waits for its attach, then answers `{"op":"bridged","session":S,"observed":…}` or `{"op":"error","code":…}` |
| relay → host | `{"op":"join","session":S}` | a guest wants in: open a new connection and attach |
| host | `{"op":"attach","v":1,"session":S,"token":T}` (new connection) | the relay pairs it with the guest's connection, answers `{"op":"bridged","session":S,"observed":…}` to both, then copies bytes |
| relay ↔ host | `{"op":"ping"}` / `{"op":"pong"}` | every 20 s; a host silent for 60 s is dropped and its code ends |

Error codes the guest can get: `unknown_code`, `ended` (the host stopped sharing), `version_mismatch`
(with the host's game version: the relay compares protocol numbers, so a guest on another Minecraft
version is told before anything else happens), `host_unreachable` (the host didn't attach within 10
seconds), `full` (the host has 8 guests connecting or the relay is at capacity), `rate_limited`,
`bad_request`.

After `bridged`, the stream is the session: the SPAKE2 messages, then the encrypted frames.

## Limits

All set by environment variables on the relay (defaults in brackets):

- `RELAY_MAX_HOSTS` [1000] codes at once; `RELAY_MAX_BRIDGES` [200] relayed streams at once;
  `RELAY_MAX_PENDING_PER_HOST` [8] guests joining one host at the same time.
- Per client address (IPv6 grouped by /64): 6 host registrations a minute (burst 3); 12 joins a minute
  (burst 6); 5 unknown codes in 10 minutes block joins from that address for 10 minutes.
  `RELAY_PER_ADDRESS_LIMITS=0` turns these off for a proxy that shows every client as one address;
  `RELAY_PROXY_PROTOCOL=1` reads the client's address from a PROXY protocol v1 line instead.
- For everyone together: `RELAY_GLOBAL_UNKNOWN_PER_MIN` [120] unknown codes a minute (burst 30); past
  it, joins are refused as rate limited before the code is looked up, so a refusal says nothing about
  whether a code exists. Even if one attacker had the whole budget: with 1000 worlds shared, an active
  nameplate turns up about every 9 minutes, and each is worth 5 one-in-a-million tries before its
  code is retired, so a break-in takes on the order of thousands of years.
- `RELAY_SESSION_KBPS` [1024] KiB/s each way per relayed stream; `RELAY_TOTAL_MBPS` [100] Mbit/s for
  the whole relay; a relayed stream with no byte either way for 120 s is closed (the game's keep-alive
  sends one every 15 s, so only dead streams idle).
- A first line must arrive within 10 s; hosts answer pings within 60 s.

## Privacy

No accounts. The relay keeps only what a live session needs, in memory: active nameplates and the
host connection behind each, pending sessions, rate-limit counters per address (forgotten after 10
minutes). It logs **counts** every 5 minutes (hosts, relayed streams, joins, unknown codes,
rate-limited requests, relayed bytes) and nothing else: no addresses, no codes, no names.
`/health` answers counts too.

A direct connection shows each player's IP address to the other, as a LAN game or a server address
would. `HORIZON_FRIENDS_PATH=relay` avoids it at the cost of relay bandwidth.

## What players see

Host (pause menu, Horizon style): **Invite friends** beside Open to LAN. It opens the world to the
network if it isn't (same settings as Open to LAN), registers with the relay and shows the code with a
**Copy** button, the friends connected, and **Stop sharing**. Stopping sharing ends the code; it leaves
Open to LAN as it was before sharing started.

Guest: **Join a friend** on the multiplayer screen (a code field) and a **Join a friend** row on the
launcher's Play page, which starts the game straight into the join. Errors are plain: "No world uses
this code", "This invite has ended", "The code doesn't match (check the last four characters)",
"Your friend's game didn't answer", "Your friend plays Minecraft X; you have Y".

Configuration: `HORIZON_RELAY_URL=host:port` at run time, else the address compiled in from the same
variable at build time. **Empty by default**: until a relay is deployed, the buttons say "Not available
yet" and do nothing else.

## Failure modes

| Failure | What happens |
| --- | --- |
| Relay down or not configured | Host: "Couldn't reach the invite service" (the world stays open to LAN if it was). Guest: same message. |
| Host closes the world / quits / loses the network | The relay drops the code ("ended"); connected guests see the usual disconnect. |
| Relay restarts | Codes end; hosts reconnect and get a **new** code (shown in the panel). Direct sessions keep running; relayed ones drop. |
| Direct attempt half works (one SYN through) | Only authenticated streams count; the rest are closed after 3 s. |
| A guest's NAT blocks everything but the relay | Relayed, at most `RELAY_SESSION_KBPS`. |
| Wrong secret | That guest gets "The code doesn't match"; after 5, the host gets a new code. |
| Guest on another version | Refused by the relay before any key exchange, with both versions in the message. |

## Deploying the relay

**Needs Ludovic**: nothing is deployed yet, and the game's default relay address is empty.

The relay is one static binary, `horizon-relay`, with no disk state; it fits Railway's smallest
instance. Files: `crates/horizon-relay/Dockerfile` (build context: the repository root) and
`crates/horizon-relay/railway.toml`.

1. New Railway service from the `ludovic111/horizon` repository; in its settings set the config file
   path to `crates/horizon-relay/railway.toml` (it selects the Dockerfile and the health check).
2. Variables: `PORT=7000` (the relay listens on `$PORT`); optionally the limits above. **One
   replica only** (`numReplicas = 1` in `railway.toml`): codes live in the relay's memory, and the
   TCP proxy spreads connections over replicas at random.
3. Networking: add a **TCP proxy** to port 7000. Railway gives an address like
   `example.proxy.rlwy.net:12345`. (An HTTP domain is not needed; the health check reaches the service
   directly on `$PORT`.)
4. Health check: `GET /health` on `$PORT` (in `railway.toml`), answers `200` with counts.
5. Check addresses: `curl http://<tcp proxy address>/whoami` from a home connection. If it shows your
   public address, keep the defaults. If it shows a Railway address, set `RELAY_PER_ADDRESS_LIMITS=0`
   (the global limits remain) and expect fewer direct connections across NATs.
6. Point the game at it: set the repository variable `HORIZON_RELAY_URL` to the TCP proxy address so
   release builds compile it in, or test first with `HORIZON_RELAY_URL=… horizon-client`.

Cost and bandwidth: the relay idles at a few MB of memory and no CPU. Sessions that connect directly
cost a few kilobytes of relay traffic each. A relayed player costs what the game sends: joining (the
chunks around the spawn) is a few MB, playing is roughly 10–50 KB/s, flying over new terrain up to a
few hundred KB/s (capped at `RELAY_SESSION_KBPS`). A relayed player-hour is therefore around 50–200 MB
of egress; 100 relayed player-hours a month is 5–20 GB. Check Railway's current egress, memory and
CPU prices against that; `RELAY_TOTAL_MBPS` bounds the worst case.

## Testing

- Rust unit tests: codes (alphabet, normalisation, parsing), frames and the key exchange (wrong secret,
  tampering, nonce order), rate limiters and bandwidth buckets.
- `crates/horizon-relay/tests/tunnel.rs`: a relay, a host tunnel in front of an echo server standing in
  for the integrated server, and guest tunnels, in one process: direct path, forced relay path, wrong
  secret (and the 5-strike new code), unknown and ended codes, version mismatch, rate limits.
- `tools/test_friends.py`: a relay, a host Horizon with an integrated world and a guest Horizon, through
  the direct path and the forced relay path; chat, movement and a placed block seen by both.
- Two machines over the tailnet (relay on one, host and guest on two others).

## Pull requests

1. `horizon-friends` (codes, wire, key exchange, tunnel ends) and `horizon-relay` (the service, its
   Dockerfile and Railway config) with their tests. Nothing player-facing.
2. Host: Invite friends in the pause menu, `horizon-ctl friends …`.
3. Guest: Join a friend in game and in the launcher, `tools/test_friends.py`.
