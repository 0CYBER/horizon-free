# The manifest and the package

A `.hmod` is one mod: content, native code, or both. `horizon_mod_package::Package` reads the shared manifest and
validates the package; `horizon_mod_package::Content` reads its resources, and `horizon_mod_package::index::native::Bundle`
reads its code.

## The zip

- A zip archive. Entries are stored or deflate-compressed, the only methods the readers support.
- A path is relative and `/`-separated: no `\`, no `:`, no empty, `.` or `..` part, and it doesn't start with `/`.
  Files must be regular files, not symlinks, devices or executables. Executable payloads are refused even under
  resource paths.
- Each path appears once. Directory entries carry no payload.

## Project and archive layout

```text
my_mod.hmod
├── horizon-mod.toml
├── icon.png                # optional
├── assets/my_mod/
├── data/my_mod/
├── Cargo.toml              # code mods only
├── src/lib.rs              # code mods only
└── mod.wasm                # wasm mods only
```

- **Content only:** no `Cargo.toml` or `[native]`. Nothing compiles.
- **Native code:** root `Cargo.toml` and `src/`, with `[native]` in the same manifest. The launcher builds it
  against the pinned prebuilt game; mixins compile into a separate function-replacement patch library. The game itself does not compile. Its resources load through the same content reader as a content mod's.
- **Wasm code:** `mod.wasm` at the top, declared by `[wasm]`, with the `Cargo.toml` and `src/` it was built from
  (a converted mod records `built_with` instead and carries no source). The game runs it with no rebuild.
- Inconsistent code declarations and files refuse the package. `mod.wasm` anywhere else, undeclared, or
  declared but missing, refuses it; so does any other executable file.

There is no mod-id directory level inside the archive. `horizon-mod.toml` is the only manifest and the only
source of the mod's id and version; Cargo's package name and version are not mod identity. IDs are plain
`[a-z0-9_]{1,64}` names, also used as resource namespaces. The launcher writes `horizon-mixin.toml` in its
staging area when needed. Hand-written tool manifests, `horizon-native.json`, and input `native/` or `rust/`
folders are unsupported.

## `horizon-mod.toml`

The shared [manifest fields](../how-to/content-mods.md#the-manifest) include `id`, `name`, `version`, `api = "0.2"`
and `capabilities = ["registries"]`. The content API is checked for exact equality. Unknown fields
refuse the manifest. Native mods may declare top-level `internals`, an array of tables with
`path`, optional `hash`, and `kind` when a path names several member kinds. Without `hash`, the build uses the fingerprint from the pinned game’s catalog. An explicitly empty `hash` is refused. Native code adds `[native]`:

| Field | Type | Rule |
| --- | --- | --- |
| `game` | string | not empty, the Minecraft version (`"26.3"`) |
| `horizon` | string | the exact Horizon commit, 40 lowercase hex characters |
| `mixins` | string | the exact horizon-mixins commit, 40 lowercase hex characters |
| `module` | array of tables | optional support modules, written as `[[native.module]]` |

API points use top-level declarations.
API function listeners use top-level `[[inject]]`: `id`, `at`, lowercase `kind`, `with`, and optional `hash`,
with optional `priority` (default 0). `head`, `return` and `replace` are supported.
The `access` key is refused by name. Use `[[native.mixin]]` for game internals.
Unknown fields are refused by the schema. API function declarations use only `[[inject]]`.
See [Injections](../concepts/injections.md) for the fields and point injections, and
[Named hook points](../how-to/entities/named-hook-points.md) for the named hook contracts.

A module has a unique, nonempty `id` (one id space with the injections), a `file` naming the game source
file that receives it, a `name` that is a Rust module identifier, and a `source` payload file under
`src/`, holding the module's items without an outer wrapper. An optional `visibility` is exactly
`"pub"` or `"pub(crate)"`, the default. See [Support modules](../concepts/native-route/5-support-modules.md).

`[[inject]]` only fills compiled slots and never compiles game code. Native source edits
use `[[native.mixin]]` with `id`, `target`, uppercase `kind`, `helper` and the other
horizon-mixins options ([Mixins](../concepts/native-route/2-mixins.md)). `[[native.loop]]`, `[[native.bridge]]` and `[[native.module]]` complete the route; `[[native.state]]` is refused. See [The native route](../concepts/native-route.md) for every table, its fields and its refusals.

A wasm mod adds `[wasm]` instead, written by `cargo horizon-mod build --wasm`
([Building to `mod.wasm`](../how-to/wasm-mods.md)):

| Field | Type | Rule |
| --- | --- | --- |
| `sha512` | string | `mod.wasm`'s SHA-512, 128 lowercase hex characters |
| `toolchain` | string | not empty, the Rust toolchain it was built with |
| `linked` | table | optional, dependency id → the SHA-512 of the `mod.wasm` it was linked against |
| `built_with` | string | optional, not empty: the converter that made it, for a package with no source |

At load `mod.wasm` must start with `\0asm`, be at most 64 MiB and match `sha512`; each failure refuses the
package with its reason.

A wasm mod or library may declare the memory it expects its dependency group to have, as top-level
`wasm_memory_mib`: an integer from 1 to 4096 (wasm32's 4 GiB), in MiB. The group gets the largest value any
member declares; with none declared it gets 1024 (1 GiB). The instance's cap (`wasm.memory_cap_mib` in
`horizon-mods.txt`, 4096 unless the player lowers it) bounds it: a declaration above the cap refuses the whole
group at load, naming the group and both numbers. Native mods are not wasm groups: they share the game's memory,
and the key means nothing to them. See [Memory](../how-to/wasm-mods.md#memory).

Top-level `[dependencies]` declares other mods by ID: a semver requirement string, or a table:

```toml
[dependencies]
base = { version = "^1", optional = false, side = "both", order = "after", reason = "Provides the shared API", url = "https://example.org/base" }
```

Versions refer to `horizon-mod.toml`, not Cargo metadata. `optional` defaults to false;
`side` defaults to `"both"`; `order` is `"after"` (default), `"before"` or `"none"`.
After means the dependency's listeners/content precede the owner's; before reverses that
**dispatch** edge; none removes it. Reason is at most 512 bytes and URL must start with
`https://`. Unknown fields and malformed values refuse the manifest; there is no `kind`
key or alternate optional spelling.

Side is the **installation**, not the logical side executing a hook: an integrated server
belongs to the client installation, a dedicated server to the server installation.
Inactive declarations are excluded before missing/version/order checks and code dependency
selection. A missing required dependency, incompatible installed version or dispatch cycle
refuses loading; an absent optional dependency is ignored. Content, native and wasm manifests
share the installed-set rule resolver, including mixed content/code relationships.

**Horizon addition: physical linking is separate from dispatch policy.** A code package's
`[dependencies]` also declares real API library requirements. `before`, `none`, and instance
overrides do not remove Cargo/wasm imports, reverse compiler staging, bypass ABI/hash/toolchain
checks, or manufacture a missing library. Physical libraries still resolve dependency-first
or refuse by name. Content constraints do not require an API package.
See [Dependencies between mods](../concepts/dependencies.md).

Top-level `[properties]` holds free-form values other mods read: strings, integers, floats and booleans only,
at most 64 keys and 8 KiB (keys and strings by length, 8 bytes per number, 1 per boolean); a table, an array
or a larger table refuses the package. A mod reads any loaded package's manifest through
`context.mods().get(id)`: `PackageInfo` gives `name`, `description`, `authors`, `license`, `homepage`,
`issues`, `icon_path`, `runtime` (`Native`, `Wasm`, or `None` for content), `dependencies` and
`property(key)`. Each readonly `PackageDependency` exposes `id()`, `version()`, `kind()`,
`order()`, `side()`, `reason()` and `url()`. `DependencyKind` is `Required`, `Optional`,
`Incompatible`, `Discouraged` or `LoadBefore`; `DependencyOrder` is `After`, `Before` or `None`.
These are all original declarations, including inactive sides, not the effective override-rewritten
graph. `LoadBefore` is ordering-only: its version `*` is a marker, order is `Before` and side
is `Both`, not an API library declaration. The metadata is the same on client and server (NeoForge's
`ModContainer.getModInfo`, `IModInfo` and `getModProperties`).

Load rules name other mods without linking against them (NeoForge's `INCOMPATIBLE`, `DISCOURAGED` and
`ordering = "BEFORE"`). The game checks them alongside ordinary dependencies for the selected content and loaded code:

```toml
load_before = ["minimap"]            # their hooks run after ours when they are loaded

[incompatible]
old_lib = "<2"                       # refuse to start beside old_lib below 2.0

[discouraged]
shaders = { version = "*", reason = "Flickers with the water shader", url = "https://example.org/w", side = "client" }
```

An `[incompatible]` or `[discouraged]` entry is a semver requirement on the other mod's version, or a
table with `version` (every version when left out), `reason` (at most 512 bytes), `url` (`https://`
only) and `side` (`"both"`, the default, `"client"` or `"server"`, the dedicated server). A rule
about a mod that isn't loaded holds nothing. A matching incompatible mod refuses startup with the
owner's reason and link; a matching discouraged mod loads with a warning in the log and on
both affected installed rows in the Mods page. Every notice remains visible in the scrollable
panel, even when a row is off or failed; registration notices are retained too.

A player can override a rule in `horizon-mods/dependency-overrides.toml`: a table per mod that owns
the rule, mapping the other mod's ID to `"drop"` (remove its logical validation/dispatch edge)
or `"discourage"` (warn instead of refusing). This also applies to ordinary required/optional
manifest dependencies; the physical code-library requirements above remain enforced. Every
applied override is logged and shown on both affected installed Mods rows. Missing files mean
no overrides; malformed files refuse loading with their path in the diagnostic. Owner and target
IDs use the same validated ID syntax as manifests.

The game reads this file from its game directory. Native builds read the policy and selected
content from `horizon-mods` beside the source folder (the normal source folder is
`horizon-native-mods`), and apply the target installation side. A standalone server export
has a separate runtime instance: its operator supplies that instance's policy if overrides
are needed at runtime; source-instance policy is not an implicit global setting.

```toml
[my_mod]
old_lib = "discourage"
```

## Original Java jar layers

Content and code mods can list ordered `[[jar]]` references. Use
[`cargo horizon-mod use-jar`](../how-to/content-mods.md#java-jars-as-external-assets) to record the
original `id`, `version` and exact file's `sha512`, without copying it into the mod.

**Pack order.** From bottom to top, assets use vanilla, then each mod's jars in
`[[jar]]` order, then that mod's own files, with later mods above earlier ones and
player resource packs above all mods. The mod's own files win over its jars for
the same path (the log lists each duplicate asset path once at load). Files every
resource pack stacks (atlases, lang, sounds, blockstates) merge in that same order
through the normal pack merger.

Data uses vanilla, then each mod's jars in `[[jar]]` order and that mod's own
`data/`, then the world's enabled datapacks. Later mods override earlier ones,
tags merge with their normal replacement rules, and a later recipe replaces an
earlier recipe at the same id. A jar may use any namespace. Only its passive
`assets/` and `data/` participate: Java classes and the jar's own pack metadata are
not loaded.

Original jars stay in `<game home>/jars/<sha512>.jar`; no bytes are copied into a
mod. The launcher checks and supplies exact files, and the existing ZIP readers
write only the decoded resource cache. The complete ordered reference list,
including version, contributes to the composed root's stamp, so changing it
rebuilds on the next resource load. Server `/reload` uses the accepted inputs of
that composed root, not another package scan.

**Unavailable originals.** Launcher preparation refuses before a native build or
game launch and names all requiring mods, the original jar and its expected
cache path. Direct game resource or world loading with a missing native
requirement fails as a whole; it never publishes code without the matching data.
A missing input during `/reload` reports the typed failure to the command source
and keeps the previous data and composed root. Content-only mods have no code to
strand: each whole mod can instead be refused, with the reason in its Mods screen panel, including
every owner of a shared missing file. The game never downloads jars.

## Source paths

A code package contains root `Cargo.toml`, optional `Cargo.lock` and `build.rs`, `.rs` and `.toml` files under `src/`,
and root text files named `README*`, `LICENSE*`, `LICENCE*`, `COPYING*` or `NOTICE*`.
Your root `impl Hooks` uses `#[mod_api::horizon_mod]`; the attribute generates the factory
([Writing hooks](../start/writing-hooks.md)).

Folder packaging selects source and resource files, excluding `.git/` and `target/`. It validates selected
files before copying them. Whole archives are checked, not treated as source folders with ignored extra entries.

## Content paths and icon

`assets/` or `data/`, then at least two parts of `[a-z0-9_.-]` (vanilla's resource-location characters).
Code and content use the same top-level resource trees; there is no separate native data folder.

`icon.png` is optional for both kinds. It has no manifest key. The package reader fully validates it once:
a PNG, nonzero square dimensions, at most 512 × 512 pixels and 256 KiB encoded. The launcher shows it in
the mod list. The scaffold does not create a default icon.

## Limits

| What | Limit |
| --- | --- |
| A registry file, as downloaded | 64 MiB |
| Selected native source files, excluding the manifest, icon and resources | 511 files, 64 MiB unpacked in total |
| Content files | 65,536 files, 16 MiB each, 512 MiB in total |
| `horizon-mod.toml` | 32 MiB |
| `icon.png` | 256 KiB encoded, 512 × 512 pixels |
