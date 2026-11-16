# Install from GitHub

Install a native mod from the Mods page with **Add from GitHub**, or use:

```sh
horizon-launcher mods add-github URL --trust-native-code
```

`--trust-native-code` records your consent to run native code. You can omit it after the launcher
has saved your consent. Native mods run with full access to your computer; read the source before
installing. Before downloading the package or source, the launcher presents the
exact resolved source commit link with its warning so you can review that revision.
GitHub installs are not reviewed by the Horizon registry.

## Choose a source

| URL | Selection |
| --- | --- |
| `https://github.com/OWNER/REPO` | Latest release, or the default branch if there is no release |
| `https://github.com/OWNER/REPO/releases/tag/TAG` | Named release |
| `https://github.com/OWNER/REPO/tree/REF` | Branch or tag resolved to a commit |
| `https://github.com/OWNER/REPO/commit/SHA` | Exact commit |

The launcher resolves links through GitHub's public API without sign-in. It reports rate-limit
errors; set `HORIZON_GITHUB_TOKEN` to use an authenticated rate limit.

A release with a `.hmod` asset installs that package. The launcher compares its SHA-256 with
GitHub's digest **when GitHub supplies one**. Without a package asset, it downloads the source
archive at the resolved commit from `codeload.github.com`.

Source discovery follows `native-build FOLDER`: each mod is a flat project with
`horizon-mod.toml`, `[native]`, `Cargo.toml` and `src/`. Each project pins its Horizon commit
with `[native] horizon`. If the archive contains several mods, select one with `--mod ID` or
install all with `--all`. Use `--instance NAME` to choose the destination instance.

The launcher validates packages and packs source projects into `.hmod` files before installation.
Validation checks flat source and content layouts and size limits without executing the mod.
This installer rejects content-only release packages.

## Build and update

Build the installed mods with `horizon-launcher native-build`. Installing a native mod enables
Native mods for the destination.

The launcher saves `horizon-native-mods/<id>.origin.json` beside the package, recording the
repository, release tag when applicable, commit, asset name and installed file's SHA-256.
`horizon-launcher mods list` displays the GitHub origin with its tag or abbreviated commit.

- `horizon-launcher mods update-github [ID] --check` checks for updates without installing them.
- `horizon-launcher mods update-github [ID]` installs updates with the native-code warning and
  saved consent requirement. Release installs follow the latest release; commit installs follow
  the default branch.
- `horizon-launcher mods remove ID` removes a mod.

A GitHub install does not overwrite a mod installed from the registry or by hand.
