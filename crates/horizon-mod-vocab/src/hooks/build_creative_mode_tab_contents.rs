//! The `build_creative_mode_tab_contents` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The stacks a mod adds to the creative tab `tab` (`minecraft:ingredients`, or another mod's
    /// `<mod id>:<name>`) while the tabs' contents are built (NeoForge's
    /// `BuildCreativeModeTabContentsEvent`, during vanilla's `CreativeModeTabs.buildAllTabContents`):
    /// push each stack to `items`, in order (`accept`). They come after the tab's own items and
    /// those of the mods before, and join the end of the search tab. A stack the tab or the search
    /// tab already lists (the same item and components) is not listed twice there, and an empty
    /// stack is left out. `has_permissions` is the build's `hasPermissions`: whether the operator
    /// items tab is allowed (the player can use game master blocks and the Operator Items Tab
    /// option is on). Asked for every item tab, the operator items tab and the mods' tabs among
    /// them, not for the search, saved toolbars or survival inventory tabs. Asked when the tabs
    /// are built: the first time the creative inventory opens after the game's resources load,
    /// and again whenever `has_permissions` changes. Client-side.
    #[hook(
        side = "client",
        contract = "collect",
        out = "items",
        neoforge = "BuildCreativeModeTabContentsEvent"
    )]
    fn build_creative_mode_tab_contents();
}
