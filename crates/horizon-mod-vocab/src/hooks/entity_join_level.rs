//! The `entity_join_level` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0059

pub trait Hooks {
    /// Before a player's tick on both logical sides (NeoForge 26.3
    /// `PlayerTickEvent.Pre`). Non-cancellable, including death-screen ticks.
    /// `player` identifies the ticked player; on the server their state is
    /// acting. Fires before sleep, cooldowns and body work. Players still tick
    /// under `/tick freeze`, but not while the level is paused.
    #[hook(side = "both", contract = "listen", neoforge = "PlayerTickEvent.Pre")]
    fn player_tick_pre(player: PlayerRef);
    /// After a player's tick on both logical sides (NeoForge 26.3
    /// `PlayerTickEvent.Post`). Non-cancellable, including death-screen ticks.
    /// `player` is the same identity as in `player_tick_pre`; on the server
    /// their state is acting. Runs after body work and server-side item,
    /// arrow and XP pickups, so reads see the completed tick. Client reads
    /// see only the state available to that client, not future server updates.
    #[hook(side = "both", contract = "listen", neoforge = "PlayerTickEvent.Post")]
    fn player_tick_post(player: PlayerRef);
    /// An entity joins the server level `dimension` (NeoForge's
    /// `EntityJoinLevelEvent`): vanilla's `PersistentEntitySectionManager.addEntity`.
    /// `loaded_from_disk` is true for an entity coming back with its chunk,
    /// false for one added fresh (`addFreshEntity`: a drop, an orb, `/summon`,
    /// an arrival through a portal). `entity` is its ID and `kind` its type
    /// (`minecraft:item`). It is already in the level's lists, so helpers that
    /// take an entity ID find it. Set `canceled` to keep it out: it is taken
    /// back out as if never added, and a `/summon` of it fails. Tells every
    /// kind in the game's entity lists (items, orbs, mobs, boats, minecarts,
    /// armor stands, primed TNT, falling blocks, projectiles, rockets,
    /// tridents, end crystals, the dragon, cushions, paintings, item frames),
    /// once each, in the order they were created; check `kind`. A drop, an
    /// orb, a mob, a `/summon` and a chunk's entities are told at once, any
    /// other spawn between the tick's phases (it is checked after the blocks,
    /// the vehicles, the projectiles, the items and the mobs, and at the
    /// tick's end). A mod that cancels one of the others sees it removed
    /// before it ticks again. Inside a transaction (`Host::transaction`) opened
    /// in the entity's own level the join is told at that transaction's root
    /// commit, not at the spawn call (an entity spawned in another level, as by
    /// a command run `in` it, is told at once): before the
    /// commit the entity exists and `entity_constructing` has run, but no one
    /// is told it joined, and a listener reads the entity when the commit
    /// runs. An aborted transaction tells nothing, and the entity it spawned
    /// is gone. A join cancelled at the commit leaves the spawn's handle
    /// stale. Joins at one commit come in the order the entities were added,
    /// before the removals the transaction asked for take effect.
    /// Not told: the entities Horizon keeps without
    /// simulating (displays, markers, interactions). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "EntityJoinLevelEvent",
        version = 2
    )]
    fn entity_join_level();
}
