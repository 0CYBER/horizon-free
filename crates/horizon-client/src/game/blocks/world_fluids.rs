//! A mod's fluid container picking up and placing fluid in the world, as NeoForge's
//! `FluidUtil.tryPickupFluid` and `tryPlaceFluid` do through a bucket's own
//! `BucketPickup.pickupBlock` and `BucketItem.emptyContents`. Everything is decided at the call,
//! against the world as the transaction has it; the block is written then without its updates
//! and kept by the journal, and the rest is done at the outermost commit. An abort leaves the
//! world as it was, which the reference doesn't promise.
use super::{Game, spill::Spill, transactions::After};
use horizon_core::position::BlockPos;
use horizon_world::{
    fluid::FluidKind,
    level::{FluidWrite, fluid_empty_sound, fluid_pickup_sound},
};
use mod_api::{
    FluidKey, FluidStack, GameEventContext, LevelRef, PlayerRef, SoundSource, Transaction,
};

/// `FluidType.BUCKET_VOLUME`: what one source block holds.
const BUCKET: u32 = 1000;

/// A bucket's sound and fluid game event waiting for the outermost commit.
pub(in crate::game) struct FluidEffects {
    level: LevelRef,
    pos: [i32; 3],
    /// The fluid's world form, whose bucket sounds play; none for a tank-only one.
    fluid: Option<FluidKind>,
    /// A fill (`fluid_pickup`), else an empty (`fluid_place`).
    pickup: bool,
    player: Option<PlayerRef>,
}

/// A world pickup or placement decided at the call, its block already set in the transaction:
/// what the outermost commit still does.
pub(in crate::game) struct WrittenFluid {
    level: LevelRef,
    write: FluidWrite,
    /// The broken block's drops, made from the block as it was at the call.
    drops: Spill,
    /// A pickup (the bucket's fill sound and `fluid_pickup`), else a placement.
    pickup: bool,
    player: Option<PlayerRef>,
}

impl Game<'_> {
    /// `BucketPickup.pickupBlock` for a mod's fluid container at `pos` in the level in place,
    /// kept by transaction `tx`: the source taken, a bucket of it. None, with nothing done,
    /// where the block gives no fluid, off the level in place, or for a transaction that is not
    /// open.
    pub(in crate::game) fn level_pickup_fluid(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        player: Option<PlayerRef>,
        tx: &Transaction,
    ) -> Option<FluidStack> {
        if !self.fluid_write_allowed(level, pos, tx) {
            return None;
        }
        let creative = self.creative(player);
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let write =
            self.with_level(|b, level, ticks| b.plan_fluid_pickup(level, ticks, at, creative))?;
        let key = mod_api::__private::fluid_key(write.fluid.name())?;
        self.journal_fluid_write(level, write, true, player);
        Some(FluidStack::new(key, BUCKET))
    }
    /// `BucketItem.emptyContents` of a bucket of `fluid` at `pos` in the level in place, for
    /// a mod's fluid container, kept by transaction `tx`. Whether it was placed, or hissed away
    /// where water evaporates. False, with nothing done, for a fluid without a world form,
    /// where it can't be placed, off the level in place, or for a transaction that is not
    /// open.
    pub(in crate::game) fn level_place_fluid(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: FluidKey,
        player: Option<PlayerRef>,
        validate: bool,
        tx: &Transaction,
    ) -> bool {
        if !self.fluid_write_allowed(level, pos, tx) {
            return false;
        }
        let Some(kind) = mod_api::__private::fluid_id(fluid)
            .and_then(FluidKind::from_name)
            .filter(|kind| *kind != FluidKind::Empty)
        else {
            return false;
        };
        let creative = self.creative(player);
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let write = self.with_level(|b, level, ticks| {
            b.plan_fluid_place(level, ticks, at, kind, creative, validate)
        });
        let Some(write) = write else {
            return false;
        };
        self.journal_fluid_write(level, write, false, player);
        true
    }
    fn fluid_write_allowed(&self, level: LevelRef, pos: [i32; 3], tx: &Transaction) -> bool {
        self.transaction_holds(tx)
            && self.level_in_place(level)
            && self.native_level_write_allowed(level, pos)
    }
    fn creative(&self, player: Option<PlayerRef>) -> bool {
        player.is_some_and(|player| self.player_is_creative(player) == Some(true))
    }
    /// What `write` does at the call. The block entity a new block of another kind removes is
    /// kept whole by the journal before any mod code runs; then the broken block's drops are
    /// made from the block as it is, with its block entity still there (the `block_drops`
    /// listeners, which only read, see the old block, and whatever unpacking its loot changes
    /// in the entity an abort puts back; a cancelled drop keeps the replacement); then the new
    /// block is stored, without its updates, and that block entity goes. The rest waits for
    /// the commit.
    fn journal_fluid_write(
        &mut self,
        level: LevelRef,
        write: FluidWrite,
        pickup: bool,
        player: Option<PlayerRef>,
    ) {
        let pos = [write.pos.x, write.pos.y, write.pos.z];
        let key = (pos[0], pos[1], pos[2]);
        let b = self.res.behaviours();
        let to = write.to(b);
        let removes = to != write.from
            && b.block(to) != b.block(write.from)
            && self.world.block_entities.get(&key).is_some();
        if removes {
            self.journal_block_entity(key);
        }
        let drops = match write.broken() {
            Some(broken) => self.with_level(|b, level, ticks| {
                b.drop_resources(level, ticks, write.pos, broken);
                std::mem::take(level.spill)
            }),
            None => Spill::default(),
        };
        if to != write.from {
            self.journal_set_block(pos, to);
        }
        if removes {
            self.journal_remove_block_entity(key);
        }
        self.after_commit(After::FluidWrite(Box::new(WrittenFluid {
            level,
            write,
            drops,
            pickup,
            player,
        })));
    }
    /// `FluidUtil.triggerSoundAndGameEvent` for `fluid` at the centre of `pos` in the level in
    /// place, at the outermost commit of transaction `tx`.
    pub(in crate::game) fn level_play_fluid_effects(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: FluidKey,
        pickup: bool,
        player: Option<PlayerRef>,
        tx: &Transaction,
    ) -> bool {
        if !self.transaction_holds(tx) || !self.level_in_place(level) {
            return false;
        }
        let Some(id) = mod_api::__private::fluid_id(fluid) else {
            return false;
        };
        self.after_commit(After::FluidEffects(Box::new(FluidEffects {
            level,
            pos,
            fluid: FluidKind::from_name(id),
            pickup,
            player,
        })));
        true
    }
    /// The rest of a world pickup or placement at the outermost commit, its block's updates
    /// done already: the fluid tick, the broken block's drops, its particles and
    /// `block_destroy`, a doused block's or a dried ghast's sound, then the bucket's sound and
    /// fluid game event (the hiss alone where water evaporates).
    pub(in crate::game) fn commit_fluid_write(&mut self, written: WrittenFluid) {
        let WrittenFluid {
            level,
            write,
            drops,
            pickup,
            player,
        } = written;
        self.with_level(|b, level, ticks| b.fluid_write_ticks(level, ticks, &write));
        debug_assert!(self.spill.is_empty(), "a spill left between level calls");
        self.spill = drops;
        self.flush_spill();
        self.with_level(|b, level, ticks| b.fluid_write_effects(level, ticks, &write));
        let _ = self.process_level_events();
        if write.sounds() {
            let pos = [write.pos.x, write.pos.y, write.pos.z];
            self.commit_fluid_effects(FluidEffects {
                level,
                pos,
                fluid: Some(write.fluid),
                pickup,
                player,
            });
        }
    }
    /// The bucket's sound (none for a fluid with no world form) and fluid game event, from the
    /// player when one is given, at the outermost commit.
    pub(in crate::game) fn commit_fluid_effects(&mut self, effects: FluidEffects) {
        let FluidEffects {
            level,
            pos,
            fluid,
            pickup,
            player,
        } = effects;
        let at = pos.map(|c| f64::from(c) + 0.5);
        let (sound, event) = match pickup {
            true => (fluid.and_then(fluid_pickup_sound), "minecraft:fluid_pickup"),
            false => (fluid.and_then(fluid_empty_sound), "minecraft:fluid_place"),
        };
        if let Some(sound) = sound {
            self.level_play_sound(level, sound, at, SoundSource::Blocks, 1.0, 1.0);
        }
        let context = GameEventContext {
            source_entity: player.map(PlayerRef::entity),
            affected_state: None,
        };
        self.level_game_event(level, event, at, context);
    }
}
