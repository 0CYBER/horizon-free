# horizon-mod-registry

The service behind the registry's submission form (`playhorizon.dev/mods/submit`). A creator signs in with GitHub and
**lists a project by its GitHub repository and a page** (`repo` plus name, summary, licence...): the service checks that the
repository is public and opens a pull request adding `mods/<id>.toml` to
[horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods), where a reviewer reads the repository once.
**It takes no versions for such a project**: they are the repository's GitHub releases, read and checked by that
repository's CI with no pull request ("publish a GitHub release in <repo>", a 409). Projects listed before that
(no `repo`) can still be sent as a file: the service runs the file's own checks
(`horizon_mod_package::index::inspect`, the signature, the entry's fields), uploads it to the `staging` release as
`<sha256>.<extension>` and opens a pull request adding or updating the entry. Formats and rules:
[docs/modding/src/reference/registry-index.md](../../docs/modding/src/reference/registry-index.md).

Signed-in players also rate published projects, 1 to 5 stars: one rating per GitHub account and project, which they can
change, never their own project's. That is the only data it keeps, in one SQLite file (`REGISTRY_DB`); it only ever
shows totals, never who rated. A session is a signed cookie holding the GitHub login and id, and the user's OAuth token
is used once, to read them, and dropped. The sign-in asks for no scope.

| Route | |
| --- | --- |
| `GET /api/mods/auth/login` | Starts the GitHub sign-in; `?back=/mods/<id>` (or `/mods`) comes back to that page |
| `GET /api/mods/auth/callback` | GitHub's redirect: sets the session, back to `/mods/submit` or the `back` page |
| `POST /api/mods/auth/logout` | Ends it |
| `GET /api/mods/me` | `{login, csrf}`, or 401 |
| `POST /api/mods/submit` | Multipart: `entry` (JSON: `id kind name summary description authors license icon key`, and either `repo` (no file: lists the project) or `source signature version game changelog` with the `file` (a project with no `repo`)); `{pull_request}` or `{error}` |
| `GET /api/mods/ratings` | `{projects: {<id>: {average, count}}}` for every rated project; cached a minute |
| `GET /api/mods/<id>/rating` | The signed-in account's own `{stars}` (null when it hasn't rated), or 401 |
| `POST /api/mods/<id>/rating` | JSON `{stars}`, 1 to 5: sets or changes it; `{stars, average, count}` or `{error}` |
| `GET /healthz` | `ok` |

A submission or a rating must come from `REGISTRY_PUBLIC_URL`'s origin with the session's `X-CSRF-Token`. Six
submissions per account and sixty in all per hour; thirty ratings per account and 600 in all. A project's owners can't
rate it, and only projects on the registry's `main` can be rated. Only an entry's `owners` publish its updates; a key change, an icon in the repository or anything else
is a pull request by hand.

## Owner updates

Only for projects with no `repo` (a project listed by its repository has no update to send). An *owner update*: the
submitter's GitHub ID is in the entry's `owner_ids` on the registry's `main`, and the only change to the entry is one
new version (key, owners, icon, kind, id, name, summary, description, authors, licence and source as they were; a
signature when the entry has a key). Then, and only when `REGISTRY_UPDATE_KEY` is set, the pull request's body ends
with a trailer and it gets the label `owner-update`:

```
<!-- owner-update
format: v2
id: coords_hud
version: 0.1.0
sha256: <hex>
file: coords_hud-0.1.0.hmod
submitter: alice
submitter_id: 7
hmac: <hex HMAC-SHA256 with REGISTRY_UPDATE_KEY of "owner-update-v2\n<id>\n<version>\n<sha256>\n<file>\n<submitter>\n<submitter_id>">
-->
```

horizon-mods' `owner-update` workflow doesn't read the trailer: it merges an owner's update without a reviewer only
when the pull request was opened by an account in the entry's `owner_ids` (its README says which kinds). The form's
pull requests are opened by the bot, so every one of them, owner update or not, waits for a reviewer; the trailer and
the label tell the reviewer what the form checked.

A refused submission is always `{error}` JSON, sent once the upload is over (a browser still sending when the answer
comes reports a network error instead): 413 for a file over 64 MiB, 504 when GitHub doesn't answer in time (30 s a
call, 5 minutes for the file's upload), 502 for GitHub's other failures.

## Running it

```sh
cargo test                     # against a mock GitHub (tests/submit.rs)
REGISTRY_PUBLIC_URL=http://localhost:4173 REGISTRY_SESSION_KEY=... GITHUB_CLIENT_ID=... GITHUB_CLIENT_SECRET=... \
  GITHUB_BOT_TOKEN=... REGISTRY_REPO=you/horizon-mods-test REGISTRY_DB=/tmp/registry.sqlite3 cargo run
# tests/app-key.pem is a throwaway key made for tests/app_auth.rs, nobody's App.
```

| Variable | |
| --- | --- |
| `REGISTRY_PUBLIC_URL` | The site, `https://playhorizon.dev`: the OAuth callback is `<it>/api/mods/auth/callback` |
| `REGISTRY_SESSION_KEY` | 32 characters or more, random: signs sessions |
| `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET` | The OAuth app |
| `GITHUB_BOT_TOKEN` | Fine-grained, the registry repository only: the permissions below. Not needed while the App is set |
| `REGISTRY_APP_ID`, `REGISTRY_APP_INSTALLATION_ID`, `REGISTRY_APP_KEY_FILE` | The service's GitHub App (below): its App ID or client ID, its installation's number, and the path of its private key (`.pem`). All three or none: only some of them stops the service at start |
| `REGISTRY_UPDATE_KEY` | Optional, random, 32 characters or more: signs owner updates' trailers (below). Unset: every update waits for a reviewer. The same value is the secret `REGISTRY_UPDATE_KEY` of horizon-mods |
| `REGISTRY_REPO` | Default `horizon-rust/horizon-mods` |
| `REGISTRY_DB` | The ratings' SQLite file: default `/data/registry.sqlite3`, the volume |
| `PORT` | Default 8080 |
| `HOST` | Default `0.0.0.0` |
| `REGISTRY_TRUST_PROXY` | `1` behind a proxy that overwrites `CF-Connecting-IP` (the Cloudflare Tunnel): the per-address limits then count that header (else the last `X-Forwarded-For` hop). Unset, the TCP peer counts and the headers are ignored, so a service reachable directly can't have its limits dodged. Behind a proxy without it, every client shares the proxy's address |

### The bot: a GitHub App or a token

The service writes to horizon-mods as its bot: with `GITHUB_BOT_TOKEN` its pull requests are opened by the token's
owner, and as a GitHub App by `<app-slug>[bot]`. The App is the horizon-rust organization's, installed on
horizon-mods only, and is not the App horizon-mods' `owner-update` workflow merges with (`vars.REGISTRY_APP_ID`
there): that one may merge past the review rule, and the website's service needs no such right. Both, App and
token, need these repository permissions:

| Permission | For |
| --- | --- |
| Contents: read and write | the entry and `revoked.toml` on `main`, a submission's branch (refs, compare) and its commits, the `staging` release and its uploads, native-build releases |
| Pull requests: read and write | finding a branch's open pull request, opening it, its `owner-update` or `owner-edit` label |
| Actions: read and write | starting `native-build.yml` for a prebuilt request |
| Metadata: read | every App has it |

With all three `REGISTRY_APP_*` set, the service signs a JWT with the key (ten minutes), exchanges it for an
installation token (an hour) and replaces that token ten minutes before it ends, or right away when GitHub refuses
it (one retry a call). The log at start names the bot (`as the GitHub App <id>` or `as GITHUB_BOT_TOKEN`), never a secret. To switch,
put the key file where the service user can read it (mode 0400), add the three variables to the environment file and
restart: `GITHUB_BOT_TOKEN` may stay. To switch back, remove the three variables and restart. A pull request opened
before the switch keeps its author.

Production runs on Ludovic's home server **ludothinkpad** (NixOS) since 2026-11-22, next to the website, declared in
`nixos/ludothinkpad/horizon.nix` of ludovic111/LifeOS: the system service `horizon-registry` (user `horizon`, a
low-priority slice with capped CPU and memory) on `127.0.0.1:8183`, no public domain; the website sends `/api/mods/*`
to it locally (`REGISTRY_UPSTREAM=http://127.0.0.1:8183` on the website). The ratings are in
`/var/lib/horizon/registry/registry.sqlite3`, copied nightly to the Mac mini by `horizon-backup`. **A push to `main`
goes live** when it changes a path in `deploy.json` (`watchPatterns`): the host's `horizon-deploy` timer builds
`cargo build --release --locked --manifest-path crates/horizon-mod-registry/Cargo.toml` (the build reads all of
`crates/`, `tools/layering/mod_vocab_state.json` and `tools/mixins/narrow-reference-rule.patch`: `horizon-mod-vocab`'s
build script reads the game's hook sources and that inventory, and `horizon-mod-package` includes that patch),
restarts the service, checks `/healthz` and rolls back if it fails. The `Dockerfile` builds the same thing as an image
(the `mod-registry` workflow checks it). Secrets are one environment file in 1Password (Agents, `Horizon host env
registry`), written to the host by `horizon-secrets registry`. The service sets `REGISTRY_TRUST_PROXY=1`, so the client
address for limits is Cloudflare's `CF-Connecting-IP`.
