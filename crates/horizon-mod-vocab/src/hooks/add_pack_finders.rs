//! Repository discovery, separately from resource files and reload listeners.
pub trait Hooks {
    /// Add discoverable resource or data packs before a repository selects its
    /// packs (`AddPackFindersEvent`). The builder reports the repository's pack
    /// type. Runs at startup and whenever that repository is rediscovered; no
    /// world state is available during startup registration.
    #[notes = r##"## Discoverable bundled packs

`Hooks::add_pack_finders` receives a typed `HostSide` and an
`AddPackFindersEvent`. Use `event.packs.pack_type()` to distinguish
`PackType::ClientResources` from `PackType::ServerData`. Register a
`PackRegistration` with `event.packs.add_pack_finders(...)`; registrations for
the other repository type are ignored. This hook contributes whole packs, while
resource-file hooks and reload listeners retain their separate roles.

A location such as `example:assets/example/resourcepacks/high_contrast` names a
loaded mod and a directory inside its package, containing `pack.mcmeta` and the
usual `assets/` or `data/` tree. Folder packages and `.hmod` archives use the same
logical paths, without extraction. Invalid locations, missing metadata and
duplicate repository ids return `PackFinderError` without adding a partial pack.

The repository id is `mod/<location>`. Packs are optional by default: resource
packs appear in Resource Packs, and data packs appear in the world's Data Packs
screen and `/datapack` repository. Selecting a resource pack applies it through
client reload; data-pack selection applies through the world's data reload.
Discovery runs again for reload, so replacing an archive opens its new contents.
Use translated `Text` for the registration's title and pack metadata description.

`always_active` makes a pack required and prevents deselection. `position`
chooses its default insertion priority (`Top` or `Bottom`); later resource
layers win. Required data packs also enter a new world's initial selection when
the player does not open the Data Packs screen. `PackSource::Feature` exposes
packs with declared feature flags in Experiments. Their flags must be enabled
for the world before `/datapack enable` accepts them, and feature packs that
request flags cannot be disabled through that command.

`hidden` keeps a pack out of the Resource Packs, Data Packs and Experiments
screens, for a compatibility pack or texture override players should not see.
A hidden pack still loads when selected, so pair it with `always_active` for one
that is always on. `/datapack list` and `/datapack enable|disable` still name
hidden data packs for operators."##]
    #[hook(side = "both", contract = "listen", neoforge = "AddPackFindersEvent")]
    fn add_pack_finders();
}
