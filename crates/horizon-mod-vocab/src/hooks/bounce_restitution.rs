//! The `bounce_restitution` hook and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity landed on a block state this part is attached to and asks its restitution
    /// (`IBlockExtension.getBounceRestitution`, which `Entity.getBlockBounciness` asks of the block
    /// an entity lands on): the factor its vertical speed is multiplied by when it hits the block
    /// from above. Set `factor` (`event.factor`) to change it; it starts at the block's own value,
    /// 1.0 for slime, 0.75 for a bed or a shelf mushroom and zero for the rest. `event.entity` and
    /// `event.kind` are the entity that landed and its type, and `event.pos` is the block it
    /// reads (`Entity.getOnPosLegacy`). The entity takes the larger of the factor and its own
    /// bounciness and, unless it is a living one, multiplies it by 0.8 as vanilla does, so 0.5
    /// bounces an entity half as high as slime. A sneaking player, a block tagged
    /// `#minecraft:suppresses_bounce` and a fall too slow to matter still do not bounce, and
    /// `bounce_on` still hears the bounce afterwards. A non-finite answer is ignored.
    ///
    /// Asked for the player and the mobs on both sides, so a client predicts its own bounce with
    /// the answer: register the mod on both and answer the same on each. The level host reads
    /// the entity's level (match `LevelSide` for its side) and cannot write; the question can be
    /// asked more than once for one tick: a client predicting its own move and a server
    /// replaying a client's tick ask again. The answer must be a pure function of the host
    /// and the event: no random draws, no part state that changes per call, and no counting
    /// calls. The same inputs must give the same factor.
    /// Dropped items keep their own slime rule. With no part attached to the state, the
    /// owning game checks the hook subscription and prepared API block key before any event
    /// conversion, so the cost is one table lookup. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "collect",
        out = "factor",
        scope = "block",
        key = "state",
        host = "level",
        target = "horizon_client::game::native_hooks::bounce_restitution::HitLevel<'_>::native_bounce_restitution",
        neoforge = "IBlockExtension.getBounceRestitution"
    )]
    fn bounce_restitution();
}
