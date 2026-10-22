# Opt-in data collection

Horizon collects nothing unless the player says yes. The launcher asks once, with four separate switches that are all off until the player turns one on, and the same switches stay in Settings. The game and the launcher obey the same answer. The code is in `crates/horizon-telemetry`.

The one exception is the anonymous ["I'm playing" ping](#the-im-playing-ping) behind the launcher's "N playing now": it holds nothing about the player, and the launcher says so next to the switches.

## Consent

`telemetry/consent.json` in the game folder (`HORIZON_HOME`, else `horizon` in the platform data folder):

```json
{ "version": 1, "answered": 1792060800, "choices": { "performance": true, "crashes": true, "bugs": false, "benchmarks": false }, "install_id": "3f2c…" }
```

- `version` is the consent version the player answered (`CONSENT_VERSION`). It goes up when what is collected changes. The launcher then asks again, and until the player answers nothing new is kept or sent.
- `answered` is when they answered, in seconds since 1970 (UTC).
- `install_id` is 128 random bits, made the first time the player says yes to anything. It ties their items together so they can be deleted, and nothing else. "Delete my data" asks the server to delete everything under it, then replaces it.

Nothing leaves the machine without consent:

- `outbox::push` keeps an item only if its kind is allowed.
- `upload::flush` reads the consent again before sending and deletes any waiting item whose kind the player has turned off since, so with everything off it never opens a connection.

`tests/consent_gating.rs` checks this against a local mock server that counts connections.

## What is sent

Every item has `day` (days since 1970, UTC: no finer time), `source` (`launcher` or `game`), `version` (the release), `kind` and `data`.

| Kind | Switch | `data` | Largest | A day |
|---|---|---|---|---|
| `perf` | performance | `system`, `settings` (render and simulation distance, max FPS, vsync, fullscreen, graphics mode), `seconds` played, `frames` and `ticks` summaries, `peak_ram_mb`, `to_title_ms`, `to_world_ms` | 4 KiB | 12 |
| `crash` | crash reports | `system`, `error`, `stack` (one frame a line), `thread`, the graphics and settings details, all scrubbed. No log, no world, no server | 32 KiB | 5 |
| `bug` | bug reports | `system`, the player's `text` (up to 4000 characters), and the `log` only if they add it after reading it | 80 KiB | 5 |
| `benchmark` | benchmark uploads | the benchmark report as `docs/benchmark-report.md` defines it, as it is, with its own schema field | 128 KiB | 10 |

`system` is the OS and its version, the architecture, the CPU model and thread count, installed memory rounded to the GiB, and the GPU, driver and graphics backend.

A summary (`frames`, `ticks`) is the count, the 50th, 95th and 99th percentile and the maximum in milliseconds (to the tenth), and 14 bucket counts: under 4 ms, 4–6, 6–8, 8–10, 10–12, 12–14, 14–16.7, 16.7–20, 20–25, 25–33.3, 33.3–50, 50–100, 100–250, and 250 ms and more. Raw frame times never leave: `Timings` records them in tenths of a millisecond (20 KB however long the session) and only its summary is kept. A perf item is under 1 KiB.

Never sent: account names or UUIDs, world names, server or LAN addresses, chat, file paths with the user's name, IP addresses. `scrub::Scrubber` replaces the user folder in paths (`/Users/<user>/…`, `C:\Users\<user>\…`), the user name, e-mail and IP addresses and UUIDs. It also replaces the terms the caller passes (the world's name, the server's address, account names). The server does not store the sender's IP address.

## Storage and sending

- `telemetry/outbox/` holds one JSON file per item, `<day>-<kind>-<random>.json`. It never holds more than 200 items or 2 MiB, and the oldest go first. `kept.json` counts today's items per kind against the caps above.
- The launcher sends the outbox when it starts and when it closes. Items go as one JSON batch (`{"schema":1,"install_id":…,"items":[…]}`, up to 64 items and 1 MiB), compressed with zstd: `POST /v1/batch` with `Content-Encoding: zstd`. A day's perf summaries are under 2 KB on the wire.
- After a failure the launcher waits a minute, doubling up to a day, and sends at most 8 batches a day. A batch the server refuses with a 4xx (not 429) is dropped.
- `HORIZON_TELEMETRY_URL` sets the server. There is none by default: until there is, nothing is sent and the outbox stays within its caps.
- `SCHEMA` versions the format. New optional fields keep it. A field that changes meaning or goes bumps it, and the server keeps reading the older ones.

## The "I'm playing" ping

Ludovic decided on 2026-10-22 to count every player, with or without consent, and to tell them. The launcher's first-run question and Settings → Data collection show it as an "Always on" row next to the switches. Players who answered the question before the ping existed see the same text once on the Play page. The privacy page on playhorizon.dev says the same.

While the game runs (not a dedicated server, a benchmark or a run that quits by itself), a thread of its own (`crates/horizon-client/src/heartbeat.rs`) sends `POST /v1/live` once a minute. It also sends one at once when a world is opened to or closed from LAN or friends:

```json
{ "session": "9b1c…", "version": "v2026.10.22", "platform": "macos", "lan": false }
```

- `session` is 128 random bits made when the game starts. It is kept only in the game's memory and is not the install ID, so two sessions of one player can't be linked.
- `version` is the release, `platform` is `linux`, `macos`, `windows` or `other`, and `lan` says whether a LAN or play-with-friends world is open.
- Nothing else: no account, install ID, world, server, settings or hardware. The server checks each field (`live::Ping::check`) and refuses anything more.

The server keeps sessions in memory only, never on disk, and never with the sender's address. A session counts while its last ping is under 2 minutes old. `GET /v1/live` gives the count, rounded (`live::round`: exact under 10, then to 5, 10 and 100):

```json
{ "playing": 1240, "lan": 85, "versions": { "v2026.10.22": 1100, "other": 140 }, "platforms": { "linux": 300, "macos": 240, "windows": 700 } }
```

The launcher reads it when it opens and every minute after, and shows "N playing now" on the Play page. It shows nothing when the server can't be reached. Like the rest, there is no server by default (`HORIZON_TELEMETRY_URL`): until there is one, the game sends no ping and the launcher shows no count. `HORIZON_NO_HEARTBEAT=1` keeps automated runs out of the count.

## Server

- `POST /v1/batch`
- `DELETE /v1/installs/<install_id>`: deletes everything sent under that ID.
- `GET /v1/aggregate`: the anonymous totals the website's results page reads.
- `POST /v1/live` and `GET /v1/live`: the "I'm playing" ping and the live count.

`crates/horizon-telemetry-server` is the server. Its [README](../crates/horizon-telemetry-server/README.md) covers storage (about 190 bytes per play session), retention (180 days of raw rows, anonymous daily totals kept), rate limits and hosting.
