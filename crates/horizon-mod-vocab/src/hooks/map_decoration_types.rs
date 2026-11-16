//! The `map_decoration_types` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The map decoration types a mod adds (NeoForge's `RegisterEvent` for
    /// `Registries.MAP_DECORATION_TYPE`): push a `MapDecorationType` per type. Asked once at
    /// start, on every side.
    #[notes = r#"## Map decoration types

`Hooks::map_decoration_types` is the registration-phase counterpart of NeoForge 26.3's
`RegisterEvent` for `Registries.MAP_DECORATION_TYPE`. It receives a `RegistrationContext`, once at
startup on every side, and collects a `MapDecorationType` per type: an id `<mod id>:<path>`, the
GUI atlas sprite that draws it (`assetId`) and `show_on_item_frame`.

A registered type is a name a `minecraft:map_decorations` entry can use, exactly as a vanilla
one: an item's entry `{type: "mymod:treasure", x, z, rotation}` (what `/give` writes and
`MapItemSavedData.addTargetDecoration` adds) is drawn with the type's sprite on a held map, in an
item frame and on the cartography table's screen, and a type with `show_on_item_frame` false is
left out of the last two as vanilla's player arrows and markers are. The sprite is looked up in
the GUI atlas by name, so a pack's texture or a mod's runtime sprite
(`Hooks::register_runtime_sprites`, `mymod:gui/treasure`) works; a sprite the atlas lacks is left
out of the drawing. A mod type never counts toward a map's decoration limit (`trackCount`).

An id in vanilla's namespace, not of that form, registered already, or with an empty sprite is
logged and left out. Types are frozen once at startup; a type a
server's map data names that a client did not register is not drawn. Map data a Horizon server
sends to a client (`ClientboundMapItemData`) numbers vanilla's types only, so a mod type reaches a
client through the item's component, not through that packet."#]
    #[hook(
        side = "both",
        contract = "collect",
        out = "types",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn map_decoration_types();
}
