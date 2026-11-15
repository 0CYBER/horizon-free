# FML loader APIs and their Horizon routes

NeoForge's mod loader (FML) hands a mod its own files and paths: the list of mod files, a config's full path, the
game directory, an update URL. Horizon answers the same needs with different shapes on purpose. This page says which
FML APIs have no counterpart of the same shape, what a Horizon mod uses instead, and why, so a porter does not search
the vocabulary for a name that will never exist.

A row marked **by design** is not a gap. A row that links an issue is a real gap that is tracked, not by design.

| FML API | Horizon route | Why it differs |
| --- | --- | --- |
| [`ModList`](https://aldak0.ru/javadoc/26.3.x/net/neoforged/fml/ModList.html) file reads: `getModFiles`, `getModFileById`, `forEachModFile`, `applyForEachModFile`, `applyForEachModFileAlphabetical` | `LoadedMods` from `RegistrationContext::mods()`: `ids()`, `iter()` and `get(id)` over the loaded packages, each a `PackageInfo` (see [Load phases and access](../concepts/load-phases.md)) | **By design.** A package is one mod, so there is one `PackageInfo` per mod rather than a file with several mods in it. It carries the id, version, kind, side, namespaces, dependencies and the manifest's details (display name, description, authors, license, links, the icon's path inside the package, runtime and properties), and no path to the package on disk. Iteration is in dispatch order; there is no alphabetical variant, so sort the ids yourself. |
| [`ModConfig.getFullPath`](https://aldak0.ru/javadoc/26.3.x/net/neoforged/fml/config/ModConfig.html) | `ConfigFile.file`, a name relative to `config/` (or to a world's `serverconfig/`), and `Host::config_files()` for the calling mod's own files (see [Config files and settings](../how-to/mod-settings.md)) | **By design.** `config_files()` is bound to the calling mod and never lists another mod's files. A wasm mod has no file path at all: the game reads the file and hands over its text. |
| [`FMLLoader.getGameDir`](https://aldak0.ru/javadoc/26.3.x/net/neoforged/fml/loading/FMLLoader.html) and [`FMLPaths.GAMEDIR`](https://aldak0.ru/javadoc/26.3.x/net/neoforged/fml/loading/FMLPaths.html) | `Host::instance_dir()` on native; `RegistrationContext::instance_dir()` on both targets (see [Load phases and access](../concepts/load-phases.md)) | **By design.** `Host::instance_dir()` is native only: a wasm mod has no file system to point at, and reads its own data and assets through reload listeners. `RegistrationContext::instance_dir()` gives the path on both targets, but as data, not as permission to open it. |
| [`IModInfo.getUpdateURL`](https://aldak0.ru/javadoc/26.3.x/net/neoforged/neoforgespi/language/IModInfo.html) | None in a mod. The mod browser compares the installed version with the newest one in Horizon's registry (see [Installing and using mods](../start/installing.md)) | **By design for registry mods:** updates come from the registry, not from a URL a mod declares. A mod installed from outside the registry has no update check yet: [arismoko/horizon#3413](https://github.com/arismoko/horizon/issues/3413). |

A mod that needs to know who else is loaded asks `LoadedMods`; one that needs a file reads it through the services
above. If neither answers, file an issue naming the FML API and what the mod does with it.
