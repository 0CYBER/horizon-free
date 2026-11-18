# Publishing your mod

Horizon's registry lists mods, modpacks, resource packs and shader packs made for Horizon. Players find them at
[playhorizon.dev/mods](https://playhorizon.dev/mods) and in the launcher. Each file is checked before installation.

1. Run `cargo horizon-mod check target/horizon-mod/<id>.hmod` on your package and put its source in a public repository under a suitable licence.
2. Attach the file to a GitHub release of that repository and open a pull request on [horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods) that adds your page and version record, as its README's [Publishing without the form](https://github.com/horizon-rust/horizon-mods#publishing-without-the-form) describes. A maintainer reviews it and stages the file. The form at [playhorizon.dev/mods/submit](https://playhorizon.dev/mods/submit) can't take `.hmod` files made with this SDK yet.
3. Publish each new version the same way, with a new `[[versions]]` record. Published versions are pinned and never replaced: submit a new version instead.

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
