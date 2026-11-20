//! The `dragon_fight_won` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The Ender Dragon fight is won (`EnderDragonFight.setDragonKilled`), on every win, not
    /// only the first. `pos` is where its dragon egg forms, `won_before` whether the fight was
    /// won before, and `forms` whether an egg forms unless a mod answers: vanilla's, on the
    /// first win only, after `dragon_egg_form`. Return `Some(true)` to place an egg there on
    /// this win, `Some(false)` to leave the block, `None` to keep `forms`. Server-side.
    #[hook(
        side = "server",
        contract = "replace",
        out = "egg",
        neoforge = "none (NeoForge has no such event)",
        paper = "none (DragonEggFormEvent is dragon_egg_form's counterpart)",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_dragon_fight_won"
    )]
    fn dragon_fight_won();
}
