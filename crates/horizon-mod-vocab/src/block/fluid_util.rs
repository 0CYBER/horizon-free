//! Fluid interaction helpers, as NeoForge 26.3's `FluidUtil`: the common
//! case of a player right-clicking a tank with a bucket or any fluid container, in one call, with
//! the bucket's sound and game event, and a container picking up and placing fluid in the world
//! as a bucket does. Vocabulary over [`FLUIDS`], [`ITEM_FLUIDS`] at an [`ItemPlace`], the move
//! step of [`move_resources`](crate::move_resources) and the level's
//! [`pickup_fluid`](LevelRef::pickup_fluid) and [`place_fluid`](LevelRef::place_fluid), and a
//! dispenser's answer for a fluid container ([`dispense_container`]): nothing changes until a mod
//! calls them.
use crate::content::resource::{Ends, move_once};
use crate::{
    Capability, Direction, DispenseItemEvent, Dispensed, FLUIDS, FluidResource, FluidStack,
    GameEventContext, Hand, HostTransaction, ITEM_FLUIDS, ItemPlace, LevelRef, PlayerRef,
    ResourceHandler, ServerHost, SoundSource,
};

/// `FluidType.BUCKET_VOLUME`: what a source block holds, and what a placement takes.
const BUCKET: u32 = 1000;

/// `FluidUtil.interactWithFluidHandler` for a block: `player`'s `hand` against the tanks of the
/// block at `pos` in `level` as `side` reaches them ([`FLUIDS`]), as
/// [`interact_with_handler`] does. False where the block has no tanks.
///
/// A tank block calls it from its `use_item_on` part and answers a success when it is true:
///
/// ```no_run
/// use mod_api::{BlockPart, Direction, InteractionResult, ServerHost, UseItemOnEvent, fluid_util};
/// struct Tank;
/// #[mod_api::horizon_part]
/// impl BlockPart for Tank {
///     fn use_item_on(
///         &self,
///         host: &mut dyn ServerHost,
///         event: &mut UseItemOnEvent<'_>,
///     ) -> Option<InteractionResult> {
///         let click = event.click;
///         let player = host.player(click.player)?;
///         let level = player.level(host)?;
///         let side = Direction::ALL.into_iter().find(|d| d.normal() == click.hit.face);
///         fluid_util::interact_with_fluid_handler(host, player, click.hand, level, click.pos, side)
///             .then_some(InteractionResult::Success)
///     }
/// }
/// ```
pub fn interact_with_fluid_handler(
    host: &mut dyn ServerHost,
    player: PlayerRef,
    hand: Hand,
    level: LevelRef,
    pos: [i32; 3],
    side: Option<Direction>,
) -> bool {
    if player.level(host) != Some(level) {
        return false;
    }
    let Some(handler) = level.capability(host, &FLUIDS, pos, side) else {
        return false;
    };
    interact_with_handler(host, player, hand, Some(pos), &handler)
}

/// `FluidUtil.interactWithFluidHandler` for any handler: fills the fluid container `player`
/// holds in `hand` ([`ITEM_FLUIDS`] at [`ItemPlace::PlayerHand`]) from `handler` and, when
/// nothing goes in, empties it into `handler`. Either way it moves the first fluid that moves,
/// as much of it as fits, in a transaction of its own: a stack of more than one container has
/// one split off, changed, where the place puts it, and a creative player keeps the hand's
/// stack, as [`ItemPlace::PlayerHand`] writes. Once something moved it plays the fill or empty
/// sound and game event at the centre of `pos`, or half a block above the player's feet
/// where `pos` is none, as
/// [`trigger_sound_and_game_event`]. True when something moved; false, with nothing changed,
/// where the hand holds no fluid container or nothing moves either way.
///
/// The sound and game event go out as it returns: call it outside a transaction the caller
/// may still abort.
pub fn interact_with_handler(
    host: &mut dyn ServerHost,
    player: PlayerRef,
    hand: Hand,
    pos: Option<[i32; 3]>,
    handler: &Capability<dyn ResourceHandler<FluidResource>>,
) -> bool {
    let Some(level) = player.level(host) else {
        return false;
    };
    let place = ItemPlace::PlayerHand {
        player: player.uuid(),
        hand,
    };
    let Some(item) = place.capability(host, &ITEM_FLUIDS) else {
        return false;
    };
    let moved = match move_first(host, handler, &item) {
        Some(moved) => (moved, true),
        None => match move_first(host, &item, handler) {
            Some(moved) => (moved, false),
            None => return false,
        },
    };
    let (moved, pickup) = moved;
    let fluid = FluidResource::of(&moved);
    let at = match pos {
        Some([x, y, z]) => Some([x, y, z].map(|c| f64::from(c) + 0.5)),
        None => player
            .entity()
            .position(host)
            .map(|[x, y, z]| [x, y + 0.5, z]),
    };
    if let Some(at) = at {
        trigger_sound_and_game_event(host, &fluid, level, at, Some(player), pickup);
    }
    true
}

/// `FluidUtil.triggerSoundAndGameEvent`: what a bucket plays as `fluid` goes in or out of a
/// container at `at` in `level`. `pickup` is true when the fluid came out of the handler into the
/// container (`minecraft:fluid_pickup` and the fluid's
/// [`fluid_pickup_sound`](crate::prelude::TagsHost::fluid_pickup_sound)) and false when it went
/// in (`minecraft:fluid_place` and its
/// [`fluid_empty_sound`](crate::prelude::TagsHost::fluid_empty_sound)). The sound plays under
/// the block slider at full volume and pitch for every player near, the acting one included,
/// since the server decides the move; the game event names `player` as its source when given.
/// A fluid with no world form, a tank-only one, has no sound, and only its game event goes out.
pub fn trigger_sound_and_game_event(
    host: &mut dyn ServerHost,
    fluid: &FluidResource,
    level: LevelRef,
    at: [f64; 3],
    player: Option<PlayerRef>,
    pickup: bool,
) {
    let id = fluid
        .fluid()
        .key()
        .and_then(crate::content::registry_keys::fluid_id);
    let sound = id.and_then(|id| match pickup {
        true => host.fluid_pickup_sound(id),
        false => host.fluid_empty_sound(id),
    });
    if let Some(sound) = sound {
        level.play_sound(host, sound, at, SoundSource::Blocks, 1.0, 1.0);
    }
    let event = match pickup {
        true => "minecraft:fluid_pickup",
        false => "minecraft:fluid_place",
    };
    let context = GameEventContext {
        source_entity: player.map(PlayerRef::entity),
        affected_state: None,
    };
    level.game_event(host, event, at, context);
}

/// `FluidUtil.tryPickupFluid` with a side: [`try_pickup_world_fluid`] first, and where the
/// block at `pos` has no fluid to give that way, one move of the first fluid its tanks give at
/// `side` ([`FLUIDS`]) into `destination`, as much as fits in one go, in a transaction of its own
/// (nested in one the caller has open, which then decides). What moved, or None with nothing
/// changed. Either way the bucket's fill sound and `minecraft:fluid_pickup` go out at the
/// block's centre at the outermost commit ([`LevelRef::play_fluid_effects`]), and not at all if
/// it aborts.
pub fn try_pickup_fluid(
    host: &mut dyn ServerHost,
    destination: &Capability<dyn ResourceHandler<FluidResource>>,
    player: Option<PlayerRef>,
    level: LevelRef,
    pos: [i32; 3],
    side: Option<Direction>,
) -> Option<FluidStack> {
    if let Some(picked) = try_pickup_world_fluid(host, destination, player, level, pos) {
        return Some(picked);
    }
    let handler = level.capability(host, &FLUIDS, pos, side)?;
    host.transaction(|host, tx| {
        let moved = move_first(host, &handler, destination).ok_or(())?;
        if let Some(fluid) = moved.key() {
            level.play_fluid_effects(host, pos, fluid, true, player, tx);
        }
        Ok::<_, ()>(moved)
    })
    .ok()
}

/// `FluidUtil.tryPickupFluid` without a side: the fluid source at `pos` in `level`, a
/// waterlogged block's water or a bubble column's ([`LevelRef::pickup_fluid`]), a bucket of it,
/// into `destination`, all of it or none, in a transaction of its own (nested in one the caller
/// has open, which then decides). What was picked up, or None with nothing changed: no source
/// there, or `destination` doesn't take the whole bucket. The block's updates, the bucket's
/// fill sound and `minecraft:fluid_pickup` follow at the outermost commit.
pub fn try_pickup_world_fluid(
    host: &mut dyn ServerHost,
    destination: &Capability<dyn ResourceHandler<FluidResource>>,
    player: Option<PlayerRef>,
    level: LevelRef,
    pos: [i32; 3],
) -> Option<FluidStack> {
    host.transaction(|host, tx| {
        let picked = level.pickup_fluid(host, pos, player, tx).ok_or(())?;
        let fluid = FluidResource::of(&picked);
        match destination.insert_any(host, tx, &fluid, picked.amount) == picked.amount {
            true => Ok(picked),
            false => Err(()),
        }
    })
    .ok()
}

/// `FluidUtil.tryPlaceFluid` with a handler: a bucket of the first fluid of `source`, index by
/// index, that comes out whole and places at `pos` in `level` ([`LevelRef::place_fluid`]):
/// into air or a block it replaces, water into a waterloggable block, or hissing away where
/// water evaporates. `validate` refuses a waterloggable block that won't take it, which a
/// bucket empties into all the same (NeoForge's `validatePlaced`). Each try is a transaction
/// of its own (nested in one the caller has open, which then decides). What was placed, or
/// None with nothing changed. The block's updates, the fluid tick, a replaced block's drops
/// and the bucket's empty sound and `minecraft:fluid_place` follow at the outermost commit.
pub fn try_place_fluid(
    host: &mut dyn ServerHost,
    source: &Capability<dyn ResourceHandler<FluidResource>>,
    player: Option<PlayerRef>,
    level: LevelRef,
    pos: [i32; 3],
    validate: bool,
) -> Option<FluidStack> {
    for index in 0..source.size(host) {
        let Some(fluid) = source.resource(host, index) else {
            continue;
        };
        let Some(key) = fluid.fluid().key() else {
            continue;
        };
        let placed = host.transaction(|host, tx| {
            if source.extract(host, tx, index, &fluid, BUCKET) != BUCKET {
                return Err(());
            }
            match level.place_fluid(host, pos, key, player, validate, tx) {
                true => Ok(fluid.to_stack(BUCKET)),
                false => Err(()),
            }
        });
        if let Ok(placed) = placed {
            return Some(placed);
        }
    }
    None
}

/// NeoForge's `DispenseFluidContainer`: a dispenser's answer for a fluid container, which a
/// container's [`dispense_item`](crate::ItemPart::dispense_item) part returns (it is no default:
/// vanilla's buckets keep their own behaviours). `event.stack` runs in a two-slot copy whose
/// item may change ([`ItemPlace::ScratchSlots`]), reached one item at a time
/// ([`ItemPlace::OneByOne`]): the container's handler sees one container, and an empty one may
/// become a full one of another item. It picks up from the block in front first
/// ([`try_pickup_fluid`], its `FLUIDS` reached from the dispenser's side), and places a bucket
/// of what it holds there ([`try_place_fluid`], unvalidated, as a bucket) only where nothing
/// was picked up; the fluid's sound and game event go out as the transfer commits, before the
/// dispenser's click and smoke. A single container is replaced in place; from a larger stack
/// the slot keeps the rest and the changed container goes into the dispenser, else out of its
/// front with a second click ([`dispense_with_remainder`](crate::prelude::ItemsServerHost::dispense_with_remainder)).
/// Where neither moves anything, nothing is changed and one container is dispensed as it is
/// ([`dispense_default`](crate::prelude::ItemsServerHost::dispense_default)). None, for vanilla
/// to decide, only where the dispenser's level isn't open.
pub fn dispense_container(
    host: &mut dyn ServerHost,
    event: &DispenseItemEvent<'_>,
) -> Option<Dispensed> {
    let level = host.level(event.dimension)?;
    let [x, y, z] = event.pos;
    let [dx, dy, dz] = event.facing.normal();
    let front = [x + dx, y + dy, z + dz];
    let side = Some(event.facing.opposite());
    let stack = event.stack.clone();
    let moved = host.transaction(|host, tx| {
        let copy = host
            .scratch_slots(tx, stack.clone())
            .ok()
            .flatten()
            .ok_or(())?;
        // The container's handler sees one container at a time, whatever the stack.
        let one = ItemPlace::OneByOne(Box::new(copy.clone()));
        let item = one.capability(host, &ITEM_FLUIDS).ok_or(())?;
        let transferred = try_pickup_fluid(host, &item, None, level, front, side).is_some()
            || try_place_fluid(host, &item, None, level, front, false).is_some();
        match transferred {
            true => host.scratch_take_slots(tx, &copy).ok_or(()),
            false => Err(()),
        }
    });
    let left = match moved {
        // The changed container went into the first slot: a single one, replaced in place.
        Ok((first, None)) => Ok(first),
        Ok((_, Some(changed))) => {
            host.dispense_with_remainder(event.dimension, event.pos, event.facing, stack, changed)
        }
        Err(()) => host.dispense_default(event.dimension, event.pos, event.facing, stack),
    };
    Some((left.ok()?, 1000))
}

/// `FluidUtil.getFirstStackContained`: what the fluid container at `place` holds at its first
/// index that holds any ([`ITEM_FLUIDS`]), whatever the count of the stack there; it may not
/// come out, and the container may hold more. None where nothing there holds a fluid.
pub fn first_stack_contained(host: &mut dyn ServerHost, place: &ItemPlace) -> Option<FluidStack> {
    let item = place.capability(host, &ITEM_FLUIDS)?;
    (0..item.size(host)).find_map(|index| {
        let fluid = item.resource(host, index)?;
        let amount = item.amount(host, index);
        (amount > 0).then(|| fluid.to_stack(amount))
    })
}

/// One move of the first fluid of `from`, index by index, that `to` takes any of, as much of it
/// as `to` takes in one go (NeoForge's `ResourceHandlerUtil.moveFirst`), all or nothing in a
/// transaction of its own: what moved, or none, with nothing changed, where none moves or a
/// handler answers more than it was asked. One go, not [`move_resources`](crate::move_resources)' repeats: a stack of
/// buckets has one filled, as vanilla's click fills one.
fn move_first(
    host: &mut dyn ServerHost,
    from: &Capability<dyn ResourceHandler<FluidResource>>,
    to: &Capability<dyn ResourceHandler<FluidResource>>,
) -> Option<FluidStack> {
    for index in 0..from.size(host) {
        let Some(fluid) = from.resource(host, index) else {
            continue;
        };
        let want = from.amount(host, index);
        let ends = Ends {
            from,
            to,
            at: &index,
        };
        let moved = move_once(
            host,
            ends,
            want,
            |host, tx, want| from.extract(host, tx, index, &fluid, want),
            |host, tx, out| to.insert_any(host, tx, &fluid, out),
        );
        if let Ok(moved) = moved
            && moved > 0
        {
            return Some(fluid.to_stack(moved));
        }
    }
    None
}
