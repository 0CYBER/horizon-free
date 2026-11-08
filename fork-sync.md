# Syncing Horizon-Free with upstream

The fork follows `horizon-rust/horizon` and keeps a few launcher patches of its own. An upstream
rebase replaces `.github/workflows/` wholesale, so every sync must restore the fork's workflow
again.

## Steps

1. `git fetch upstream`
2. `git checkout -b new-upstreamN upstream/main`
3. Cherry-pick the README commit (`Man that ai really doesnt love saving readme`).
4. Reapply the launcher patches (see the list below).
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
7. `cargo fmt`, `cargo check`, `cargo test --lib --bins` in `launcher/`.
8. Commit, fast-forward `main`, force-push, and let the workflow publish.
9. If a build fails with "The job was not acquired by Runner of type hosted even after multiple
   attempts" (GitHub's macOS capacity), rerun just that job; the version is not released yet, so
   nothing gets published twice:
   ```sh
   gh run rerun <run-id> -R 0CYBER/horizon-free --failed
   ```

## Launcher patches the fork keeps

- `backend::LAUNCHER_REPO = "0CYBER/horizon-free"`: self-update reads the fork's releases.
- `launcher_releases()` falls back to the fork's `releases.atom` when GitHub's API refuses
  (60 anonymous reads an hour; Horizon's mirror does not carry the fork), and a check that fails
  is reported (`Updates::launcher_error`) instead of reading as "up to date".
- `backend::OFFLINE_PLAY = true`: offline play in the window and the CLI, no Microsoft account
  that owns the game.
- `Settings::load()` and `client_as()` force `dev_offline` / `session_for_game(.., true)`.
- CLI `account offline NAME`, `account select NAME` and `play` bypass the owner gate.
- Signature check disabled for fork releases (`signed_digest` returns nothing, `download()`
  skips the check when the digest is empty). `fill_stage` then stores the archive's own SHA-256,
  so `place_checked` still refuses bytes that changed after the download; keep that.
- No Microsoft plates: `frame.rs` top-bar sign-in, `glass_account.rs` Microsoft card,
  `main.rs` `account_rows` and prepare sign-in.
- `Horizon-Free` link in the glass sidebar, above the Discord button (`frame.rs`), and the legacy
  sidebar link (`main.rs`).
- The README warning block.

`launcher/tests/owner_gate.rs` is upstream's: it asserts that `play` and the MCP server refuse
without an owner, which the fork removes on purpose, so `cargo test` fails those two and nothing
else. `cargo test --lib --bins` is the suite the fork checks.

The game itself still updates from `horizon-rust/horizon` (`backend::REPO`): the fork builds and
releases the launcher only.
