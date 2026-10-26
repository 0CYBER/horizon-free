# Publish a mod

Horizon's registry lists mods, modpacks and resource packs made for Horizon. Players find them at [playhorizon.dev/mods](/mods) and on the launcher's Mods page, which checks every file before it installs it. Each project is reviewed by a person before it is published. This page takes your mod from a file on your computer to the registry.

## Before you submit

1. **Choose a version.** Set `version` in `horizon-mod.toml` (such as `1.0.0`). Every version you publish needs a new one, and a published file never changes.
2. **Build it for release.** `cargo horizon-mod build` (release is the default; `--debug` is for quick local builds) writes `target/horizon-mod/<id>.hmod`. That file is what you submit.
3. **Check it.** `cargo horizon-mod check target/horizon-mod/<id>.hmod` must pass ([Test a mod](/docs/test-a-mod#check-it-as-the-registry-does)).
4. **Publish the source.** Put the mod's source in a public repository, under a licence that lets players use it (MIT, Apache-2.0, GPL and so on, as an [SPDX](https://spdx.org/licenses/) name). Reviewers read it, and the file must be built from it.

## Sign it

Signing is optional and recommended. When your project has a public key, every later version must be signed with the matching secret key, so nobody else can publish an update in your name, even with access to your GitHub account.

Horizon uses [minisign](https://jedisct1.github.io/minisign/) signatures. Install minisign (`brew install minisign`, `scoop install minisign`, or your Linux distribution's package), then make a key pair once:

```sh
minisign -G -p horizon-mods.pub -s horizon-mods.key
```

Keep `horizon-mods.key` and its password private and backed up: losing it means changing the project's key by hand, through the registry's maintainers. The public key is the last line of `horizon-mods.pub` (it starts with `RW`).

Sign each file you publish, and check the signature as the registry will:

```sh
minisign -S -s horizon-mods.key -m target/horizon-mod/playtime.hmod
cargo horizon-mod check target/horizon-mod/playtime.hmod --key "RW…your public key…" --signature target/horizon-mod/playtime.hmod.minisig
```

The report then says "signature verified".

## Submit it

Open [playhorizon.dev/mods/submit](/mods/submit) and sign in with GitHub. The page reads only your public profile. Then fill in:

- **The file**: the kind (mod), the `.hmod`, its id (the manifest's `id`), its version, the Minecraft version (26.3) and what changed.
- **Its page**: the name, a one-line summary for the list, a description (Markdown), the authors, the licence, the source address and, optionally, a square PNG icon (at most 256 KiB).
- **Signature**: your public key (for a new project) and the file's `.minisig`.

**Submit for review** checks the file again and opens a pull request in [ludovic111/horizon-mods](https://github.com/ludovic111/horizon-mods). It adds `mods/<id>.toml`, with you as the project's owner. The repository's checks run the file in the sandbox and post their report on the pull request.

## Review and publication

A reviewer reads the pull request, your source and the checks' report. Questions and requests for changes come as comments on the pull request: answer them there, and submit a new version from the form when the file must change.

When the reviewer merges it, the file becomes a release of horizon-mods within minutes, and the project gets its page at `playhorizon.dev/mods/<id>` and shows in the launcher. The page lists what the mod may do, as the game's Mods screen does, with your source, licence and the file's SHA-256.

## Updates

An update is the same form with a new version: it adds the version to your project's entry. The project's owners publish updates without a reviewer's extra look, and if the project has a key, the new file must be signed with it.

Without the form, you can open the pull request by hand: the [horizon-mods README](https://github.com/ludovic111/horizon-mods) says how. The registry's file formats are in [the mod index reference](/docs/api/mod-index).

## Reports and takedowns

Anyone can report a project from its page (**Report it**). A project that does harm is taken down: the launcher then refuses its files and turns installed copies off, telling players why.
