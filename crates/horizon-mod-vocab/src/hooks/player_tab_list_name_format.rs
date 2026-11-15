//! The `player_tab_list_name_format` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// ServerPlayer.getTabListDisplayName: the name the Tab list shows for
    /// `player`, None to let the client choose it. Replace `display_name` to
    /// change it. It runs while the Tab list is built. Answer from the arguments and `context` alone, and fast: `context` holds the side, the registries and the settings of the resource generation, and a `/reload` builds the next one.
    #[hook(
        side = "server",
        contract = "collect",
        out = "display_name",
        context = "loop",
        neoforge = "PlayerEvent.TabListNameFormat",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::lan::hub::native_player_tab_list_name_format"
    )]
    fn player_tab_list_name_format();
}
