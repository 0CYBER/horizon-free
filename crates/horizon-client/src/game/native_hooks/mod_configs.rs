use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The config files native mods keep (NeoForge's `registerConfig`):
    /// push yours, each of a kind, with the text written to `config/` when
    /// the file is missing. Asked by the window when the game starts (its
    /// `Client` and `Common` configs), by a dedicated server when it starts
    /// (`Common` and `Server`) and by a server before each world it opens
    /// (`Server`); a game reads only its own kinds, so push them all every
    /// time. A file outside `config/`, or one already registered, is skipped.
    /// API 397.
    pub(in crate::game) fn native_register_configs(&mut self, configs: &mut Vec<mod_api::Config>) {
        self.fire_register_configs(configs);
    }
    /// A registered config was read (`ModConfigEvent.Loading`): `config`
    /// says whose, which file and its text, which the mod parses (TOML by
    /// convention). Every mod hears every config, so check `mod_id`. Fired
    /// where the config's kind loads (`native_register_configs`), a `Server`
    /// one before `native_server_about_to_start`. API 397.
    pub(in crate::game) fn native_config_loading(&mut self, config: &ConfigFile) {
        self.fire_config_loading(config);
    }
    /// A loaded config's file changed on disk (`ModConfigEvent.Reloading`):
    /// its new text. The game looks at the files' modification times once a
    /// second, on the window and on the server's tick; a file saved with the
    /// same text, removed or unreadable fires nothing and keeps its last
    /// text. API 397.
    pub(in crate::game) fn native_config_reloading(&mut self, config: &ConfigFile) {
        self.fire_config_reloading(config);
    }
    /// A `Server` config is dropped as its server stops
    /// (`ModConfigEvent.Unloading`), after `native_server_stopping`; the next
    /// world reads it again. `Client` and `Common` configs stay until the
    /// game quits. API 397.
    pub(in crate::game) fn native_config_unloading(&mut self, config: &ConfigFile) {
        self.fire_config_unloading(config);
    }
}
