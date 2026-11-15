//! The `setting_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0020

pub trait Hooks {
    /// The player changed setting `key` of mod `mod_id` on its page in Options > Mods
    /// (`Registrar::register_setting`); the new value is kept already, so read it with
    /// `setting_bool` and its siblings. Only mod `mod_id` hears it: a mod is told of its own
    /// settings, never another's.
    #[hook(
        side = "client",
        contract = "listen",
        owner = "mod_id",
        neoforge = "ModConfigEvent.Reloading (a ModConfigSpec value set on the config screen)"
    )]
    fn setting_changed();
}
