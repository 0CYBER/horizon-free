# horizon-mod-registry

The service behind the registry's submission form (`playhorizon.dev/mods/submit`). A creator signs in with GitHub and
sends a file with its page; the service runs the file's own checks (`horizon_mods::index::inspect`, the signature, the
entry's fields), uploads it to the `staging` release of
[ludovic111/horizon-mods](https://github.com/ludovic111/horizon-mods) as `<sha256>.<extension>`, and opens a pull
request adding or updating `mods/<id>.toml`. That repository's CI checks it again, with a trial run of a mod in the
sandbox; reviewers decide, and merging publishes. Formats and rules: [docs/api/mod-index.md](../../docs/api/mod-index.md).

It keeps no data: a session is a signed cookie holding the GitHub login, and the user's OAuth token is used once, to
read that login, and dropped. The sign-in asks for no scope.

| Route | |
| --- | --- |
| `GET /api/mods/auth/login` | Starts the GitHub sign-in |
| `GET /api/mods/auth/callback` | GitHub's redirect: sets the session, back to `/mods/submit` |
| `POST /api/mods/auth/logout` | Ends it |
| `GET /api/mods/me` | `{login, csrf}`, or 401 |
| `POST /api/mods/submit` | Multipart: `entry` (JSON: `id kind name summary description authors license source icon key signature version game changelog`) and `file`; `{pull_request}` or `{error}` |
| `GET /healthz` | `ok` |

A submission must come from `REGISTRY_PUBLIC_URL`'s origin with the session's `X-CSRF-Token`. Six per account and sixty
in all per hour. Only an entry's `owners` publish its updates; a key change, an icon in the repository or anything else
is a pull request by hand.

## Running it

```sh
cargo test                     # against a mock GitHub (tests/submit.rs)
REGISTRY_PUBLIC_URL=http://localhost:4173 REGISTRY_SESSION_KEY=... GITHUB_CLIENT_ID=... GITHUB_CLIENT_SECRET=... \
  GITHUB_BOT_TOKEN=... REGISTRY_REPO=you/horizon-mods-test cargo run
```

| Variable | |
| --- | --- |
| `REGISTRY_PUBLIC_URL` | The site, `https://playhorizon.dev`: the OAuth callback is `<it>/api/mods/auth/callback` |
| `REGISTRY_SESSION_KEY` | 32 characters or more, random: signs sessions |
| `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET` | The OAuth app |
| `GITHUB_BOT_TOKEN` | Fine-grained, the registry repository only: Contents and Pull requests read and write |
| `REGISTRY_REPO` | Default `ludovic111/horizon-mods` |
| `PORT` | Default 8080 |

On Railway it is the service `horizon-mod-registry` of the project `horizon-site`, next to the website: built from
`main` with this `Dockerfile` from the repository root, health check `/healthz`, no public domain and no volume. The
website's Caddy sends `/api/mods/*` to it over Railway's private network (`REGISTRY_UPSTREAM` on the website service:
`http://horizon-mod-registry.railway.internal:8080`). `railway.json` lists the same settings; Railway reads them from
the service, not from the file.
