//! The `register_debug_entries` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The F3 debug entries a mod adds (NeoForge's `RegisterDebugEntriesEvent`): push a `DebugEntry`
    /// per entry, its id `<mod id>:<path>` and the vanilla group its lines join. Asked once at start,
    /// on the client. An id in vanilla's namespace, not of that form, or registered already is logged
    /// and left out. Each entry is listed and switched on or off with vanilla's, and `debug_entry_lines`
    /// fills it while it is on.
    #[notes = r#"## Debug screen entries

`Hooks::register_debug_entries` is the client registration-phase counterpart of NeoForge
26.3's `RegisterDebugEntriesEvent`. It receives a `RegistrationContext`, once at client
startup, and collects a `DebugEntry` per entry: an id `<mod id>:<path>` and the `DebugGroup`
whose vanilla group its lines join (`Position`, `Light`, `Memory`, `LookingAtBlock`,
`LookingAtEntity`, `LookingAtFluid` or `ChunkGeneration`). An id in vanilla's namespace, not
of that form, or registered already is logged and left out.

An entry is listed in the debug options' text section, sorted by its id with vanilla's, and
is switched like them (`alwaysOn`, `inOverlay`, `never`), saved in `debug-profile.json` under
its full id. On the overlay, its group comes after vanilla's groups.
Its lines are asked for each frame the overlay draws it, and only while it is on:
`Hooks::debug_entry_lines` receives the entry's id and a `&mut Vec<String>` to push lines to,
with legacy colour codes (§) working. A disabled entry, or one with no lines, costs nothing.
Nothing is uploaded or recorded: the lines are drawn on the local screen only.

An entry in `DebugGroup::Renderer` is vanilla's `DebugEntryNoop`: it has no lines, and is listed
in the renderer section instead. It is the switch of the `DebugRenderer` with its id, see
[debug renderers](#debug-renderers)."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "entries",
        context = "registration",
        neoforge = "RegisterDebugEntriesEvent"
    )]
    fn register_debug_entries();
}
