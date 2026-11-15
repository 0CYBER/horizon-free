//! Load-phase declarative profession registration.
pub trait Hooks {
    /// Declare villager professions once per mod load, before world admission.
    /// The engine resolves every reference atomically with the owning resource tables.
    /// Vanilla replacement and overlapping custom acquisition POIs are refused.
    /// NeoForge adds a profession through `RegisterEvent` on `Registries.VILLAGER_PROFESSION`;
    /// this hook declares it as data, so only the registration moment corresponds.
    #[hook(
        side = "server",
        contract = "collect",
        out = "professions",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn register_villager_professions();
}
