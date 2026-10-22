# horizon-telemetry-server

This server receives what players choose to send (performance summaries, crash and bug reports, benchmark uploads: [docs/telemetry.md](../../docs/telemetry.md)) and stores it in one SQLite file. It serves anonymous totals to the website. It also keeps the live player count from the games' anonymous "I'm playing" pings, in memory only. It is one small Rust process, about 7 MB of memory when idle.

    cargo run --release                     # in this folder: its own workspace, like the launcher's
    PORT=8080 HORIZON_TELEMETRY_DB=/data/telemetry.sqlite3 horizon-telemetry-server

The launcher sends to it when `HORIZON_TELEMETRY_URL` is set (and to `DEFAULT_ENDPOINT` in `crates/horizon-telemetry` once there is one).

## Endpoints

| | |
|---|---|
| `POST /v1/batch` | A zstd batch (`Content-Encoding: zstd`, at most 256 KiB, 1 MiB inflated, 64 items). Returns 204. Returns 400 for a bad batch, 413 for one too large, 415 if it is not zstd, 429 when over a limit. |
| `DELETE /v1/installs/<id>` | Deletes everything that install sent. Its share of the daily totals is rebuilt at once. |
| `GET /v1/aggregate?days=30` | The totals the website reads. Cached 10 minutes, with `Access-Control-Allow-Origin` set to the website. |
| `POST /v1/live` | A game's "I'm playing" ping (JSON, at most 512 bytes). Returns 204. Returns 400 for a bad ping, 413 for one too large, 429 over the address's limit, 503 when it holds `LIVE_MAX_SESSIONS` already. |
| `GET /v1/live` | How many play now, rounded: `playing`, `lan`, `versions`, `platforms`. Made again at most every 15 seconds, with the same `Access-Control-Allow-Origin`. |
| `GET /healthz` | `ok` |

The totals are grouped per version, OS, GPU vendor and render distance. Each group has its sessions, install-days, hours played, the 14 frame-time buckets summed, and the mean p50 and p99 frame times. Groups with fewer than 5 install-days are left out. The totals also list crash counts per version and the newest 50 benchmark uploads.

## Storage

- `systems`: each distinct machine description once. Sessions point at it.
- `perf`: one row per session, in columns, its frame and tick summaries as 80-byte blobs.
- `reports`: crash and bug reports and benchmarks, their data compressed with zstd.
- `perf_daily`: the totals per day, rebuilt from `perf` when a day changes (new sessions, a deletion).

Measured: **about 190 bytes per play session** on disk, indexes included (`tests/server.rs`, 10 000 sessions). 1 000 players playing twice a day for 180 days is about 70 MB.

The live count is a table in memory: a session ID, when it last pinged, its version, platform and LAN flag, about 100 bytes a session. A session counts while its last ping is under 2 minutes old and is forgotten after that. Nothing of it reaches the disk, so a restart empties it; it is whole again a minute later.

The day stored is the day the server received an item. The sender's IP address is never written down. The per-address rate limit keeps addresses in memory for an hour at most.

Raw rows are deleted after `HORIZON_TELEMETRY_RETENTION_DAYS` (180). The daily totals stay; they hold no install ID.

## Settings

All are environment variables:

- `PORT`
- `HORIZON_TELEMETRY_DB`
- `HORIZON_TELEMETRY_RETENTION_DAYS` (180)
- `HORIZON_TELEMETRY_MIN_INSTALL_DAYS` (5)
- `HORIZON_TELEMETRY_REQUESTS_PER_HOUR` per address (60, bursts of 6)
- `HORIZON_TELEMETRY_ITEMS_PER_DAY` per install (100)
- `HORIZON_TELEMETRY_TRUST_PROXY=1` behind a proxy (the client's address is then the first of `X-Forwarded-For`)
- `HORIZON_TELEMETRY_ALLOW_ORIGIN` (`https://playhorizon.dev`)
- `HORIZON_TELEMETRY_LIVE_PER_HOUR` per address, pings and reads (600, bursts of 60: ten players behind one address)
- `HORIZON_TELEMETRY_LIVE_WINDOW_SECONDS` (120)
- `HORIZON_TELEMETRY_LIVE_MAX_SESSIONS` (200 000)

## Hosting

`Dockerfile` builds it from the repository root (`docker build -f crates/horizon-telemetry-server/Dockerfile .`). The database goes on a volume mounted at `/data`.

At Railway's prices (RAM $10/GB-month, CPU $20/vCPU-month, volumes $0.15/GB-month, egress $0.05/GB), one more service with a 1 GB volume uses about $0.35 a month:

- RAM about 15 MB: $0.15
- CPU nearly idle: under $0.05
- the volume: $0.15
- egress, the cached totals: cents

That fits inside the Hobby plan's included $5 of usage. Railway Postgres would cost a few dollars a month for its memory alone and buys nothing at this size. A bucket isn't needed: crash reports are a few KB compressed and live in the same file.
