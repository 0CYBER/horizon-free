//! The `debug_entry_lines` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The lines of one enabled F3 debug entry a mod registered (`register_debug_entries`), asked each
    /// frame the overlay draws it. `id` is the entry's id; push its lines to `lines`, legacy colour
    /// codes (§) work. A disabled entry is never asked. Client-side.
    #[hook(
        side = "client",
        contract = "collect",
        out = "lines",
        cancellable = false,
        neoforge = "RegisterDebugEntriesEvent"
    )]
    fn debug_entry_lines();
}
