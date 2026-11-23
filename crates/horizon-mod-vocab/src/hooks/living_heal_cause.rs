//! The `living_heal_cause` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0353

pub trait Hooks {
    /// A living entity healed by `amount` after `living_heal` let the heal
    /// through, and `cause` says why: `regeneration` (peaceful natural
    /// regeneration), `satiated` (food saturation), `magic` (an instant
    /// health effect) or `other` (a mob's heal from any other source). It
    /// only hears: the amount is the one applied (or about to be applied for
    /// an instant health effect, whose heal lands in the next effect step).
    /// `entity` is the affected entity's network id. Canceled heals are not
    /// told. Server and client loops, as `living_heal`. NeoForge's
    /// `LivingHealEvent` carries no cause; Paper's `EntityRegainHealthEvent`
    /// carries a regain reason, which this hook mirrors.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        neoforge = "none (LivingHealEvent has no cause)",
        target = "horizon_client_lib::hunger::native_living_heal_cause"
    )]
    fn living_heal_cause();
}
