//! The `friction` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The friction of a block state this part is attached to (`IBlockExtension.getFriction`) is being read for the entity on it; vanilla's values are 0.6 for most blocks, 0.98 for ice, packed ice and frosted ice, 0.989 for blue ice and 0.8 for slime. `entity` and `kind` are the entity's id and type (`minecraft:player`, `minecraft:item`), `pos` the block and `state` its state. A part may change `friction` (`event.friction`), which starts at the block's own value; an answer that is not finite or is outside `0..=2` (the range a content block's declared friction takes) is dropped and the block's own value is used. This deliberately limits NeoForge's uncapped answer to protect movement from invalid mod input. It is read where vanilla reads the block's friction: a living entity's ground travel (the player, mobs and armor stands), an item's and an experience orb's ground drag, a boat's ground friction and a lead's angular friction on the ground. It runs for each read, on both sides and on the client's replayed ticks, so register the mod on both and keep the listener a pure function of its inputs. It is given the loop `context` of the side that ticks the entity, and takes no `Host`, so a part answers from its inputs alone and cannot read the neighbouring blocks. Diverges from upstream: new hook.
    #[notes = r#"## Block friction

- **Block Part method `friction(context, event)`**: edit `event.friction`, the friction of the block state `event.state` at `event.pos` the part is attached to (NeoForge's `IBlockExtension.getFriction`), as the entity `event.entity` of type `event.kind` on it reads it. `friction` starts at the block's own value (0.6 for most blocks, 0.98 for ice, packed ice and frosted ice, 0.989 for blue ice, 0.8 for slime). The edited value flows on as vanilla's does: through the `friction_modifier` attribute, into a living entity's ground speed and into the drag (× 0.91). An answer that is not finite or is outside `0..=2` is dropped for the block's own value; nothing panics. Unlike NeoForge's uncapped raw answer at item, orb, boat and leash sites, this safety boundary protects movement from invalid mod input. Read for the player, mobs, armor stands, items, experience orbs, boats (`AbstractBoat.getGroundFriction`) and leads (`Leashable.angularFriction`). It fires on both sides and on the client's replayed ticks, so a mod registers on both and keeps the listener a pure function of its inputs. `context` is the loop context of the side that ticks the entity; there is no `Host`, so the part cannot read neighbouring blocks. With no part attached to the state, the owning game checks the hook subscription and prepared API block key before any event conversion; with none installed the movement result is bit-identical to vanilla's. Diverges from upstream: new hook."#]
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        scope = "block",
        key = "state",
        cancellable = false,
        target = "horizon_client_lib::motion::surface::native_friction",
        neoforge = "IBlockExtension.getFriction"
    )]
    fn friction();
}
