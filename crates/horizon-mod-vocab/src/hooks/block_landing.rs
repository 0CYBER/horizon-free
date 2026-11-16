//! The `block_landing` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity landed on a block state this part is attached to (`Block.fallOn`), after falling `event.distance` blocks. `event.damage` is the fall damage the entity calculated, before hurt handling and the fall sounds: change it, and zero prevents both. An entity that cannot be hurt by the fall (creative, fall immune) is told with zero damage and keeps its immunity, and a cancelled fall still lands. It runs inside the entity's move, on the level random host: it reads blocks and draws the level's random in vanilla's order, and cannot write. A write the override makes (farmland turning to dirt, a turtle egg breaking) is asked with `event.request(LandingWrite::SetBlock(state))` or `LandingWrite::DestroyBlock { drop }` and lands at the block the entity landed on once the entity's batch has ticked, in the order asked, if the block is still the state it landed on. SetBlock takes an API BlockState. The complete requested batch is checked before queuing: an invalid key or ordinal refuses all writes and sibling outputs from that callback, including damage. The original landed-state guard is checked once per batch, not between its ordered writes. Players, mobs and dropped items land here (an item as an `EntityRef` of kind `minecraft:item`, with zero damage: items take none); projectiles and the other entities do not.
    #[hook(
        side = "server",
        contract = "collect",
        out = "damage",
        scope = "block",
        key = "state",
        host = "level",
        target = "horizon_client::game::native_hooks::block_landing::HitLevel<'_>::native_fall_on",
        neoforge = "LivingFallEvent"
    )]
    fn fall_on(
        dimension: &str,
        entity: i32,
        kind: &str,
        state: BlockState,
        pos: BlockPos,
        distance: f64,
        damage: &mut f32,
        writes: &mut Vec<LandingWrite>,
    );
    /// An entity bounced off a block state this part is attached to (`Block.bounceOn`), after falling `event.fall_distance` blocks. It fires only when the entity bounced, just before vanilla sets its movement. `event.movement` is the movement the bounce leaves it (`x`, `y`, `z`): change it to change what the entity keeps. It runs inside the entity's move, on the level random host: it reads blocks and draws the level's random in vanilla's order, but writes nothing. Players and mobs bounce here; dropped items and the other entities do not.
    #[hook(
        side = "server",
        contract = "collect",
        out = "movement",
        scope = "block",
        key = "state",
        host = "level",
        target = "horizon_client::game::native_hooks::block_landing::HitLevel<'_>::native_bounce_on",
        neoforge = "none (vanilla Block.bounceOn; IBlockExtension.getBounceRestitution only returns a factor)"
    )]
    fn bounce_on(
        dimension: &str,
        entity: i32,
        kind: &str,
        state: BlockState,
        pos: BlockPos,
        fall_distance: f64,
        movement: &mut [f64; 3],
    );
}
