//! Energy in a block's buffer for native server mods (docs/modding/src/how-to/native-mods.md): the buffer a mod
//! block declared through `native_block_entity_energy`, as the game's own `ENERGY` capability
//! reaches it through a side. Vanilla has no energy; the contract is NeoForge's
//! `SimpleEnergyHandler`. They act where the world is simulated, never on a client's copy, and
//! only on the level in place; their writes are kept by the transaction they are given.
use super::Game;
use horizon_client::containers::{BlockEntity, ModEnergyBuffer};
use horizon_core::nbt::{Compound, JavaString, Tag};
use mod_api::{BlockSide, Transaction};
use std::sync::Arc;

/// The amount in `nbt`: the int `energy`, as `SimpleEnergyHandler` saves it;
/// anything else, or a negative, reads as empty.
fn read_energy(nbt: &Compound) -> i32 {
    match nbt.get(&JavaString::from("energy")) {
        Some(Tag::Int(energy)) => (*energy).max(0),
        _ => 0,
    }
}

/// `SimpleEnergyHandler.insert`: the room left (none when the amount is
/// over the capacity), the call's limit and what was offered, whichever is
/// least.
fn insert_amount(buffer: &ModEnergyBuffer, energy: i32, offered: i32) -> i32 {
    (buffer.capacity.saturating_sub(energy))
        .min(offered)
        .min(buffer.max_insert)
        .max(0)
}

/// `SimpleEnergyHandler.extract`: what is stored, the call's limit and what
/// was asked, whichever is least.
fn extract_amount(buffer: &ModEnergyBuffer, energy: i32, asked: i32) -> i32 {
    energy.min(asked).min(buffer.max_extract).max(0)
}

impl Game<'_> {
    /// The buffer of the mod block at `at` in the level in place and the energy it holds (its
    /// block entity's, and not the placeholder standing in while it ticks), if `at`'s side
    /// reaches a fill or a drain (none: every side does).
    fn energy_buffer(&self, at: BlockSide) -> Option<(Arc<ModEnergyBuffer>, i32)> {
        if !self.level_in_place(at.level) {
            return None;
        }
        let [x, y, z] = at.pos;
        let buffer = self
            .world
            .block_entities
            .mod_energy(self.world.block(x, y, z).0)?;
        let reached = at.side.is_none_or(|face| {
            buffer.insert_faces[face as usize] || buffer.extract_faces[face as usize]
        });
        // The block entity is out of the world while it ticks.
        match self.world.block_entities.get(&(x, y, z)) {
            Some(BlockEntity::Mod(m)) if reached && !m.id().is_empty() => {
                Some((Arc::clone(buffer), read_energy(&m.nbt)))
            }
            _ => None,
        }
    }
    /// What the buffer at `at` takes (`insert`) or gives of up to `amount` through its side,
    /// written into its block entity, saved with its chunk and kept by the transaction.
    fn energy_moved(&mut self, at: BlockSide, amount: u32, insert: bool) -> u32 {
        let Some((buffer, energy)) = self.energy_buffer(at) else {
            return 0;
        };
        let faces = match insert {
            true => &buffer.insert_faces,
            false => &buffer.extract_faces,
        };
        if at.side.is_some_and(|face| !faces[face as usize]) {
            return 0;
        }
        let amount = i32::try_from(amount).unwrap_or(i32::MAX);
        let (moved, after) = match insert {
            true => {
                let moved = insert_amount(&buffer, energy, amount);
                (moved, energy.saturating_add(moved))
            }
            false => {
                let moved = extract_amount(&buffer, energy, amount);
                (moved, energy - moved)
            }
        };
        if moved > 0 {
            let [x, y, z] = at.pos;
            self.journal_block_entity((x, y, z));
            if let Some(BlockEntity::Mod(m)) = self.world.block_entities.get_mut(&(x, y, z)) {
                m.nbt.insert(JavaString::from("energy"), Tag::Int(after));
                self.world.modified.insert((x >> 4, z >> 4));
            }
        }
        moved.unsigned_abs()
    }
    /// The energy the buffer of the mod block at `at` in the level in place holds, for the
    /// game's own energy capability: none where no buffer is, in another level, a chunk that
    /// isn't loaded, or the block entity ticking now, or where `at`'s side reaches neither a
    /// fill nor a drain.
    pub(in crate::game) fn energy_amount(&self, at: BlockSide) -> Option<u32> {
        Some(self.energy_buffer(at)?.1.unsigned_abs())
    }
    /// The capacity of the buffer at `at` (`energy_amount`), 0 where none is.
    pub(in crate::game) fn energy_capacity(&self, at: BlockSide) -> u32 {
        self.energy_buffer(at)
            .map_or(0, |(buffer, _)| buffer.capacity.max(0).unsigned_abs())
    }
    /// `SimpleEnergyHandler.insert`: puts up to `amount` into the buffer at `at`
    /// (`energy_amount`) through its side, no more than its room and its limit per call,
    /// kept by transaction `tx`. How much went in.
    pub(in crate::game) fn energy_insert(
        &mut self,
        at: BlockSide,
        amount: u32,
        tx: &Transaction,
    ) -> u32 {
        if !self.transaction_holds(tx) {
            return 0;
        }
        self.energy_moved(at, amount, true)
    }
    /// `SimpleEnergyHandler.extract`: takes up to `amount` out of the buffer at `at`
    /// (`energy_amount`) by its side, no more than it holds and its limit per call, kept
    /// by transaction `tx`. How much came out.
    pub(in crate::game) fn energy_extract(
        &mut self,
        at: BlockSide,
        amount: u32,
        tx: &Transaction,
    ) -> u32 {
        if !self.transaction_holds(tx) {
            return 0;
        }
        self.energy_moved(at, amount, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::HostTransaction;
    use mod_api::ServerHost;

    fn buffer(capacity: i32, max_insert: i32, max_extract: i32) -> ModEnergyBuffer {
        ModEnergyBuffer {
            capacity,
            max_insert,
            max_extract,
            insert_faces: [true; 6],
            extract_faces: [true; 6],
        }
    }

    #[test]
    #[ignore = "daily"]
    fn entity_energy_and_fluids_move_and_roll_back() {
        use super::super::super::native_entity_refs::tests::with_game;
        use horizon_client::chunk::Chunk;
        use horizon_client::containers::{ModBlockEntity, ModBlockEntityFields};
        use horizon_client::entities::ArmorStand;
        use horizon_client::fast_hash::FxHashMap;
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::{Direction, Vec3};
        use mod_api::{
            BlockUpdateFlags, ENERGY, ENTITY_ENERGY, ENTITY_FLUIDS, Holder, move_energy,
            move_resources,
        };
        use mod_platform::__private::part_tests::{ROBOT_FLUID, game_thread};
        with_game(|game| {
            let _mods = game_thread();
            // Installing the capability fixture replaces the registry snapshot.
            crate::game::mods::register_iterated_registries(&game.options.resources).unwrap();
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let block = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world
                .block_entities
                .set_mod_energy(FxHashMap::from_iter([(
                    block.0,
                    Arc::new(buffer(1000, 1000, 1000)),
                )]));
            let pos = [0, 64, 0];
            assert_eq!(
                game.level_set_block(level, pos, block, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            let mut fields = ModBlockEntityFields::default();
            fields.nbt.insert("energy".into(), Tag::Int(800));
            game.world.block_entities.insert(
                (0, 64, 0),
                BlockEntity::Mod(Box::new(ModBlockEntity::new("test:battery".into(), fields))),
            );
            for id in [13, 14] {
                game.armor_stands.push(ArmorStand::new(id, Vec3::ZERO, 0.0));
            }
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(15, pig, Vec3::ZERO, 0.0));
            let (stand, other, pig) = (
                game.entity(13).unwrap(),
                game.entity(14).unwrap(),
                game.entity(15).unwrap(),
            );
            let host: &mut dyn ServerHost = game;
            assert!(pig.capability(host, &ENTITY_ENERGY, None).is_none());
            assert!(pig.capability(host, &ENTITY_FLUIDS, None).is_none());
            let source = level.capability(host, &ENERGY, pos, None).unwrap();
            // The side reaches the provider: this one answers from every side but below.
            assert!(
                stand
                    .capability(host, &ENTITY_ENERGY, Some(Direction::Down))
                    .is_none()
            );
            assert!(
                stand
                    .capability(host, &ENTITY_ENERGY, Some(Direction::Up))
                    .is_some()
            );
            let target = stand.capability(host, &ENTITY_ENERGY, None).unwrap();
            assert_eq!(
                host.transaction(|host, tx| move_energy(host, &source, &target, 100, tx)),
                Ok(100)
            );
            assert_eq!((source.amount(host), target.amount(host)), (700, 100));
            let abort: Result<(), ()> = host.transaction(|host, tx| {
                assert_eq!(move_energy(host, &source, &target, 100, tx), Ok(100));
                Err(())
            });
            assert!(abort.is_err());
            assert_eq!((source.amount(host), target.amount(host)), (700, 100));
            ROBOT_FLUID.set(host, Holder::Entity(14), 600).unwrap();
            let tank = stand.capability(host, &ENTITY_FLUIDS, None).unwrap();
            let from = other.capability(host, &ENTITY_FLUIDS, None).unwrap();
            assert_eq!(
                host.transaction(|host, tx| move_resources(host, &from, &tank, |_| true, 100, tx)),
                Ok(100)
            );
            assert_eq!((from.amount(host, 0), tank.amount(host, 0)), (500, 100));
            let abort: Result<(), ()> = host.transaction(|host, tx| {
                assert_eq!(
                    move_resources(host, &from, &tank, |_| true, 100, tx),
                    Ok(100)
                );
                Err(())
            });
            assert!(abort.is_err());
            assert_eq!((from.amount(host, 0), tank.amount(host, 0)), (500, 100));
        });
    }

    #[test]
    fn an_insert_stops_at_the_room_the_limit_and_the_offer() {
        let b = buffer(1000, 100, 50);
        assert_eq!(insert_amount(&b, 0, 40), 40);
        assert_eq!(insert_amount(&b, 0, 500), 100);
        assert_eq!(insert_amount(&b, 950, 500), 50);
        assert_eq!(insert_amount(&b, 1000, 500), 0);
        // Over the capacity after a mod update shrank it: nothing goes in.
        assert_eq!(insert_amount(&b, 1500, 500), 0);
        assert_eq!(insert_amount(&b, 0, i32::MAX), 100);
        assert_eq!(
            insert_amount(&buffer(i32::MAX, i32::MAX, 0), 5, i32::MAX),
            i32::MAX - 5
        );
    }

    #[test]
    fn an_extract_stops_at_what_is_stored_the_limit_and_the_ask() {
        let b = buffer(1000, 100, 50);
        assert_eq!(extract_amount(&b, 800, 30), 30);
        assert_eq!(extract_amount(&b, 800, 500), 50);
        assert_eq!(extract_amount(&b, 20, 500), 20);
        assert_eq!(extract_amount(&b, 0, 500), 0);
        assert_eq!(extract_amount(&buffer(1000, 100, 0), 800, 500), 0);
    }

    #[test]
    fn the_amount_reads_as_the_saved_int_and_anything_else_as_empty() {
        let mut nbt = Compound::new();
        assert_eq!(read_energy(&nbt), 0);
        nbt.insert(JavaString::from("energy"), Tag::Int(750));
        assert_eq!(read_energy(&nbt), 750);
        nbt.insert(JavaString::from("energy"), Tag::Int(-5));
        assert_eq!(read_energy(&nbt), 0);
        nbt.insert(
            JavaString::from("energy"),
            Tag::String(JavaString::from("lots")),
        );
        assert_eq!(read_energy(&nbt), 0);
    }

    #[test]
    #[ignore = "daily"]
    fn a_buffers_energy_moves_by_its_sides_and_limits_and_an_abort_puts_it_back() {
        use super::super::super::native_entity_refs::tests::with_game;
        use horizon_client::chunk::Chunk;
        use horizon_client::containers::{ModBlockEntity, ModBlockEntityFields};
        use horizon_client::fast_hash::FxHashMap;
        use horizon_core::geometry::Direction;
        use mod_api::{BlockUpdateFlags, ENERGY, move_energy};
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let block = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            // A buffer of 1000 taking at most 300 an insert, from every side but its top, and
            // giving at most 500 an extract, by every side.
            let mut fills = [true; 6];
            fills[Direction::Up as usize] = false;
            let buffer = ModEnergyBuffer {
                insert_faces: fills,
                ..buffer(1000, 300, 500)
            };
            let tables = FxHashMap::from_iter([(block.0, Arc::new(buffer))]);
            game.world.block_entities.set_mod_energy(tables);
            let (from, to) = ([0, 64, 0], [2, 64, 0]);
            for (pos, energy) in [(from, 800), (to, 0)] {
                let flags = BlockUpdateFlags::NEIGHBORS;
                assert_eq!(game.level_set_block(level, pos, block, flags), Ok(true));
                let mut fields = ModBlockEntityFields::default();
                fields
                    .nbt
                    .insert(JavaString::from("energy"), Tag::Int(energy));
                let entity =
                    BlockEntity::Mod(Box::new(ModBlockEntity::new("test:battery".into(), fields)));
                game.world
                    .block_entities
                    .insert((pos[0], pos[1], pos[2]), entity);
            }
            let moved = |game: &mut Game<'_>, side, max, commit: bool| {
                let host: &mut dyn ServerHost = game;
                let source = level.capability(host, &ENERGY, from, None).unwrap();
                let target = level.capability(host, &ENERGY, to, side).unwrap();
                host.transaction(|host, tx| {
                    let moved = move_energy(host, &source, &target, max, tx).unwrap();
                    if commit { Ok(moved) } else { Err(moved) }
                })
            };
            let held = |game: &mut Game<'_>| {
                let host: &mut dyn ServerHost = game;
                [from, to].map(|pos| {
                    let buffer = level.capability(host, &ENERGY, pos, None).unwrap();
                    buffer.amount(host)
                })
            };
            // The target's limit per insert: the move is tried again with the 300 it took.
            assert_eq!(moved(game, None, 1000, true), Ok(300));
            assert_eq!(held(game), [500, 300]);
            // Its top takes no fill, and nothing moves.
            assert_eq!(moved(game, Some(Direction::Up), 1000, true), Ok(0));
            assert_eq!(held(game), [500, 300]);
            // An aborted move leaves both as they were.
            assert_eq!(moved(game, None, 200, false), Err(200));
            assert_eq!(held(game), [500, 300]);
        });
    }
}
