# Publishing your mod

Horizon's registry lists mods, modpacks, resource packs and shader packs made for Horizon. Players find them at
[playhorizon.dev/mods](https://playhorizon.dev/mods) and in the launcher. Each file is checked before installation.

1. Run `cargo horizon-mod check target/horizon-mod/<id>.hmod` on your package and put its source in a public repository under a suitable licence.
2. Open [playhorizon.dev/mods/submit](https://playhorizon.dev/mods/submit), sign in with GitHub, and submit the page and version file. The service opens a pull request for review.
3. Submit each new version's file through the same form. An update that changes only the version file merges once its checks pass; a change to the page waits for review. Published versions are pinned and never replaced: submit a new version instead.

Submit explicit version files; the registry refuses entries that follow repositories. The full rules are in [the registry index](../reference/registry-index.md).

Once it is published, its page and its card in the launcher show its rating: players signed in with GitHub on its
page give it 1 to 5 stars, one rating per account, which they can change. Its owners can't rate it, and nobody sees
who rated.

## Integrity and signing

- **Integrity.** Every download is checked against the SHA-256 the index lists before the file reaches the game
  folder.
- **Signing.** Authors sign files with an Ed25519 key (minisign format: a `.minisig` beside the file); the index pins
  each project's public key, so an update signed by another key is refused. Unsigned local files stay allowed, with an
  "unsigned" mark.
- **Takedowns.** A file the registry takes down (`revoked.toml`) is refused by the launcher and turned off where
  installed.
