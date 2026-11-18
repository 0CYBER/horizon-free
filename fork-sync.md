# Syncing Horizon-Free with upstream

The fork follows `horizon-rust/horizon` and keeps a few launcher patches of its own. An upstream
rebase replaces `.github/workflows/` wholesale, so every sync must restore the fork's workflow
again.

## Steps

1. `git fetch upstream`
2. `git checkout -b new-upstreamN upstream/main`
3. Cherry-pick the README commit (`Man that ai really doesnt love saving readme`).
4. Reapply the launcher patches (see the list below). Carrying the fork's own diff over is quicker
   and keeps the patches together: `git diff <previous fork base> <fork main> -- .
   ':(exclude).github/*' ':(exclude)launcher/lang/*'` then `git apply -3` it on the new branch.
   Upstream rewrites `launcher/lang/*.json` wholesale, so merge those on their own: never
   re-serialise a file (that reorders keys and adds noise), only append the fork's keys at the end
   as upstream does for its own, and keep every value upstream changed.
5. Bump `launcher/Cargo.toml` (`version = "0.2.X"`) and the `horizon-launcher` entry in
   `launcher/Cargo.lock`. The workflow tags `launcher@X.Y.Z` from that version.
6. Delete every upstream workflow and restore the fork's:
   ```sh
   git rm -q .github/workflows/*.yml
   git checkout <fork-workflow-commit> -- .github/workflows/launcher.yml
   ```
   `launcher.yml` builds and releases the launcher for Windows (x64 + ARM64), Linux (x86_64) and
   macOS (arm64 + x64) on every push to `main`, and skips a version whose release already exists.
   The ARM64 Windows job is optional: if its runner fails, the other platforms still publish.
   macOS: `macos-26-intel` builds both assets, the arm64 one cross-compiled, because GitHub's
   Apple Silicon runners are regularly unavailable for hours. That is why
   `packaging/macos/package.sh` only starts the app when the host has the binary's processor
   (`uname -m` against `lipo -archs`), and the workflow's "The version is the release's" step
   checks a cross-built binary's arch and embedded version string instead of running it.
   The Linux step installs GPUI's libraries from the first apt mirror that resolves
   (`azure.archive.ubuntu.com` → `archive.ubuntu.com` → `mirrors.edge.kernel.org`), skipping
   hosts whose name does not resolve: the runner's own mirror fails now and then and apt then
   sits on it for minutes.
7. `cargo fmt`, `cargo check`, `cargo test --lib --bins` in `launcher/`.
8. Commit, fast-forward `main`, force-push, and let the workflow publish.
9. If a job is cancelled while queued ("The job was not acquired by Runner of type hosted even
   after multiple attempts", GitHub's macOS capacity), a queued job cannot be rerun on its own:
   rerun the whole run, which is safe while the version is unreleased:
   ```sh
   gh run cancel <run-id> -R 0CYBER/horizon-free
   gh run rerun <run-id> -R 0CYBER/horizon-free
   ```

## Launcher patches the fork keeps

- `backend::LAUNCHER_REPO = "0CYBER/horizon-free"`: self-update reads the fork's releases.
- The launcher's release files download from the fork too (`parse_releases_from(LAUNCHER_REPO, ...)`,
  `signed::asset_url_in`): Horizon publishes `launcher@X.Y.Z` under the same numbering, so a list
  read from the fork but an address built from `REPO` installed Horizon's launcher of the same
  version, and that build updates itself to the newest official one. `stage_update` refuses any
  launcher file whose URL is not in the fork's own releases.
- `launcher_releases()` falls back to the fork's `releases.atom` when GitHub's API refuses
  (60 anonymous reads an hour; Horizon's mirror does not carry the fork), and a check that fails
  is reported (`Updates::launcher_error`) instead of reading as "up to date".
- `backend::OFFLINE_PLAY = true`: offline play in the window and the CLI, no Microsoft account
  that owns the game. `Settings::load()` forces `dev_offline`, the window's owner gate asks
  `owner_shown()` (no gate), and `backend/developer_play.rs`'s `client_as(tag, Option<&account::Player>)`
  asks `session_for_game(&game_home(), OFFLINE_PLAY)`: no owner is no refusal. `play_press` counts
  the offline build as an owner's, so Play's first press is "Prepare & play" (from the player's own
  Minecraft install) instead of a "Sign in to play" that goes nowhere (`widgets.rs`, its callers
  `glass_home.rs`, `home.rs`, and `blocks_play.rs`'s prepare section). Upstream's
  `developer_offline` opt-in (`HORIZON_LAUNCHER_DEVELOPER_OFFLINE`) stays as upstream wrote it; the
  fork does not need it. Two of upstream's tests pin the gate the fork removes: `client_as` must
  return [`SIGN_IN_HINT`](launcher/src/backend.rs) without an owner (`developer_offline.rs`), and
  launcher releases must come from `horizon-rust/horizon` (`release_loading_tests.rs`). Keep them
  green by testing the fork's truth: the same calls stop at the missing build and read
  `LAUNCHER_REPO` (list and file address), never at a sign-in.
- CLI `account offline NAME`, `account select NAME` and `play` bypass the owner gate.
- Offline names: upstream removed them in `1e789800af` ("launcher: remove offline names"), so
  every sync restores that commit (`git revert -n 1e789800af`) and resolves the conflicts in
  `backend.rs`, `cli.rs`, `frame.rs`, `glass_account.rs` and the one lang file whose translations
  drifted. Upstream's split of `main.rs` moved the settings rows into `blocks_settings.rs`, so
  `offline_rows` is wired there (after `account_rows`). Keep `OFFLINE_PLAY` doing the job
  `dev_offline` did upstream: the chips in Settings > Account, the top bar's quick switch,
  `account offline|select|forget-offline|skin` and the `--name`/`HORIZON_OFFLINE_SKIN*` the game
  gets for them.
- Signature check disabled for fork releases (`signed_digest` returns nothing, `download()`
  skips the check when the digest is empty). `fill_stage` then stores the archive's own SHA-256,
  so `place_checked` still refuses bytes that changed after the download; keep that.
- No Microsoft plates: `frame.rs` top-bar sign-in, `glass_account.rs` Microsoft card,
  `blocks_settings.rs` `account_rows` and prepare sign-in.
- `Horizon-Free` link in the glass sidebar, above the Discord button (`frame.rs`).

- The README warning block.
- `backend::PARTNER_BANNERS = false`: the partner/sponsor plate (Sculk's sidebar card and Servers
  page banner) never draws (`frame.rs`'s `rail_update_card`, `blocks_settings.rs`'s settings page),
  and its "Hide after 180 s" timer never runs.
- `Settings::game_dir`, honoured by `backend::game_home()` through a cached read that `Settings::save`
  forgets: Settings > Game's "Game folder" row (Open, Change, Use the default) and the
  "Minecraft folder" row beside it (`blocks_settings.rs`: `game_folder_row`, `minecraft_folder_row`,
  `choose_game_folder`; the glass page hangs them in `glass_settings.rs`'s `section(4)`). The Horizon
  instance plays in that folder, so its saves, `options.txt` and packs come from there.
- Skins for the game's own offline name: `skins::GAME_LOOK` (`@game`) is the reserved key its look is
  kept under, so "New name" doesn't lose it. `Accounts::look_key`/`look_env` give the game the skin,
  the game-name chip in `glass_account.rs` has the pencil, and `switch_page.rs` its "Skin:" button
  (issue #14).
- `crates/horizon-threads/src/allocator_stats.rs`: the fork used to cast mimalloc's JSON pointer to
  `c_char` (`raw.cast::<std::ffi::c_char>()`) because `libmimalloc-sys` returns `*mut c_char` and
  `c_char` is `u8` on aarch64, so the Windows ARM64 launcher job failed to compile the engine crate
  with E0308. Upstream fixed the same line on its own (`raw.cast()`), so the fork no longer patches
  the file.
- `wardrobe_shown()` (`wardrobe_page.rs`) also asks for a signed-in Microsoft account when
  `OFFLINE_PLAY` is set: cosmetics belong to an account on Horizon's service, and the fork has no
  sign-in, so without one the Wardrobe and Shop tabs only led to a "Sign in with Microsoft" button
  that went nowhere.

`launcher/tests/owner_gate.rs` is upstream's: it asserts that `play` and the MCP server refuse
without an owner, which the fork removes on purpose, so `cargo test` fails those two and nothing
else. `cargo test --lib --bins` is the suite the fork checks.

The game itself still updates from `horizon-rust/horizon` (`backend::REPO`): the fork builds and
releases the launcher only.
