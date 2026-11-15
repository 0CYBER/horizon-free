//! The `data_map_types` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0150

pub trait Hooks {
    /// The server is assembling its data tables, as for
    /// `data_pack_files` (a world's first open and every `/reload`):
    /// push the data map types your mod registers (NeoForge's
    /// `RegisterDataMapTypesEvent`). A type is an `id` (`mod_id:name`, not in
    /// `minecraft`) for a `registry`: any the game names. Its values come
    /// from the world's data packs'
    /// `data/<mod_id>/data_maps/<registry folder>/<name>.json` (the registry's
    /// path for a `minecraft` registry, `<namespace>/<path>` otherwise), and
    /// from files you push through `native_data_pack_files` under the key
    /// `data_maps/<mod_id>:<registry folder>/<name>`: `replace`, `values`
    /// (entry ids and `#tags` to JSON values, each optionally guarded by its
    /// own `neoforge:conditions`) and `remove`, a newer value overriding an
    /// older. Push a typed map's `DataMap::ty` and read its values with the
    /// key's `data` (`ItemKey::data`, `EntityTypeKey::data`...), or by name
    /// with `data_map_value` and `data_map_entries`. Server-side, not synced
    /// to clients.
    #[hook(
        side = "server",
        contract = "collect",
        out = "types",
        neoforge = "RegisterDataMapTypesEvent"
    )]
    fn data_map_types();
}
