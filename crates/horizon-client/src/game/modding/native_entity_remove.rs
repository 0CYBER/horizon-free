//! Removing an entity for native server mods, through an `EntityRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): `Entity.discard`
//! and `Entity.remove` with a reason. Nothing dies and nothing drops loot.
//! `Entity.spawnAtLocation` drops an item at one.
//! Outside a mod block-entity tick, `Entity.setRemoved` tells the world at the call:
//! the handle and selectors stop finding it, passengers get off and mods hear it leave.
//! Inside a transaction the handle stops finding it at the call, and the rest waits for the root
//! commit (`transactions/entities.rs`); an abort leaves it as it was.
//! Dropped-item removal inside that tick is instead staged with its final snapshot;
//! scoped reads hide it immediately, while the leave notification waits for publication. A mob's list entry is marked at once too; the other
//! kinds stay in their lists until the level's next sweep of departed
//! entities, a few times per tick, so a hook that removes one never shifts the
//! lists the game is walking. A vehicle with a chest spills it at that sweep, as vanilla's `remove` does.
use super::{Game, commands::Target};
use horizon_client::items::ItemStack;
use horizon_core::{geometry::Vec3, position::BlockPos};
use mod_api::{EntityRef, RemovalReason};

impl Game<'_> {
    /// Removes the entity with no death and no loot (`Entity.discard`): a
    /// chest boat or a minecart with a chest still drops what it holds, and
    /// a boat its lead, as vanilla's `remove` does. False for a player, a dragon part,
    /// or a stale handle.
    pub(super) fn entity_discard(&mut self, entity: EntityRef) -> bool {
        self.entity_remove(entity, RemovalReason::Discarded)
    }
    /// Removes the entity for `reason` (`Entity.remove`): `Discarded`, as
    /// `discard`, or `Killed`, which also runs a mob's effects that act on
    /// removal (Oozing's slimes, Weaving's cobwebs, Wind Charged's burst),
    /// still with no death and no loot. False for a player, a dragon part, or a stale
    /// handle.
    pub(super) fn entity_remove(&mut self, entity: EntityRef, reason: RemovalReason) -> bool {
        let killed = reason == RemovalReason::Killed;
        match self.native_entity_target(entity) {
            // A part leaves the level with its dragon, never alone.
            None | Some(Target::Player | Target::Guest(_) | Target::DragonPart(_)) => false,
            Some(Target::Item(id)) if self.block_entity_tick_key.is_some() => {
                self.stage_tick_item(id, None).unwrap_or(false)
            }
            Some(t) if self.journal_removal(entity, t, killed) => true,
            Some(Target::Mob(id)) => {
                self.remove_mob_now(id, killed);
                true
            }
            Some(t) => {
                self.remove_target_now(entity, t);
                true
            }
        }
    }
    /// `Entity.setRemoved` of `t`, which `entity` names and is no mob, player or dragon part.
    pub(in crate::game) fn remove_target_now(&mut self, entity: EntityRef, t: Target) {
        // Read before the entity counts as removed: its handle stops resolving.
        let vehicle_y = match t {
            Target::Boat(id) => self.boats.iter().find(|b| b.id == id).map(|b| b.position.y),
            Target::Cart(id) => self.carts.iter().find(|c| c.id == id).map(|c| c.position.y),
            _ => None,
        };
        let passengers: Vec<_> = self
            .entity_passengers(entity)
            .unwrap_or_default()
            .into_iter()
            .filter(|passenger| {
                vehicle_y.is_none()
                    || !matches!(self.native_entity_target(*passenger), Some(Target::Mob(_)))
            })
            .collect();
        // `Entity.setRemoved`: the reason first, so the hooks below see
        // it removed; then the passengers get off and the level is told.
        // Only players and mobs ride, so this kind has no vehicle to leave.
        self.persist.removals.push(t);
        self.retire_vehicle_registration(t);
        if vehicle_y.is_some() {
            self.persist.notified_vehicle_removals.insert(entity.id());
        }
        for passenger in passengers {
            self.entity_stop_riding(passenger);
        }
        if let Some(y) = vehicle_y {
            // Notify actual Mob edges here, including rejected objects.
            // A reentrant physical sweep must not notify them again.
            self.dismount_destroyed_vehicle_mobs(entity.id(), y);
            self.score_destroyed_vehicle_once(entity.id());
        }
        self.native_entity_left(entity.id());
    }
    /// Drops `stack` at the entity's feet, `y_offset` above
    /// (`Entity.spawnAtLocation`), or 0.15 out along a frame or painting's
    /// horizontal facing (`HangingEntity.spawnAtLocation`), moving as a new item does, with the
    /// default pickup delay: the item entity. None for an empty stack or an
    /// unknown item, a stale handle, a player in another level, or an item a
    /// mod kept out.
    pub(super) fn entity_spawn_at_location(
        &mut self,
        entity: EntityRef,
        stack: ItemStack,
        y_offset: f32,
    ) -> Option<EntityRef> {
        if self.entity_ids_exhausted() {
            return None;
        }
        if !self.stack_spawns(&stack) {
            return None;
        }
        let target = self.native_entity_target(entity)?;
        if let Target::Guest(conn) = target
            && self.command_guest(conn)?.dimension != self.dim.id()
        {
            return None;
        }
        let at = self.target_position(target)?;
        let [x, y, z] = match target {
            Target::Frame(id) => {
                let frame = self.item_frames.iter().find(|frame| frame.id == id)?;
                super::frames::hanging_drop_position(at, frame.step())
            }
            Target::Painting(id) => {
                let painting = self.paintings.iter().find(|painting| painting.id == id)?;
                super::frames::hanging_drop_position(at, painting.step())
            }
            _ => at,
        };
        // Found by its id: the hooks the spawn runs may add entities after it.
        let id = self.next_entity + 1;
        self.spawn_at_location(Vec3::new(x, y, z), f64::from(y_offset), stack);
        self.dropped
            .iter()
            .rev()
            .any(|d| d.id == id)
            .then(|| EntityRef::new(entity.epoch(), id))
    }
    /// A mob leaves the level: its effects act first when it was killed.
    fn remove_mob_now(&mut self, id: u32, killed: bool) {
        if !self.mobs.iter().any(|m| m.id == id && !m.removed) {
            return;
        }
        if killed {
            self.mob_removal_effects(id);
        }
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.removed = true;
        }
    }
    /// What a killed mob's effects do on removal (Oozing's slimes, Weaving's cobwebs, Wind
    /// Charged's burst).
    pub(in crate::game) fn mob_removal_effects(&mut self, id: u32) {
        let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
            return;
        };
        if !self.mobs[i].extra.effects.is_empty() {
            let m = &self.mobs[i];
            let names: Vec<&'static str> = m
                .extra
                .effects
                .iter()
                .map(|e| e.id.trim_start_matches("minecraft:"))
                .collect();
            let (bounds, mut random) = (m.bounds(), m.random.clone());
            self.effects_on_death(&names, bounds, false, Some(id), Some(&mut random));
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                m.random = random;
            }
        }
    }
    /// `AbstractMinecartContainer.remove` and `AbstractChestBoat.remove`: a
    /// chest or hopper minecart or a chest boat spills what it holds
    /// (`Containers.dropContents`) for every destroying reason.
    fn spill_vehicle(&mut self, t: Target) {
        let id = match t {
            Target::Cart(id) | Target::Boat(id) => id,
            _ => return,
        };
        // What an open menu edited goes back to the vehicle first.
        self.release_vehicle_container(id);
        let take = |at, items: &mut Vec<_>| {
            if items.iter().all(Option::is_none) {
                None
            } else {
                Some((at, std::mem::take(items)))
            }
        };
        let held = match t {
            Target::Cart(_) => self
                .carts
                .iter()
                .position(|c| c.id == id)
                .and_then(|i| self.carts.update(i, |c| take(c.position, &mut c.items))),
            _ => self
                .boats
                .iter()
                .position(|b| b.id == id)
                .and_then(|i| self.boats.update(i, |b| take(b.position, &mut b.items))),
        };
        let Some((at, items)) = held else {
            return;
        };
        let at = BlockPos {
            x: at.x.floor() as i32,
            y: at.y.floor() as i32,
            z: at.z.floor() as i32,
        };
        self.with_level(|_, level, _| level.drop_lost_contents(at, items));
        self.flush_spill();
    }
    /// Takes out the entities mods removed since the last sweep.
    pub(super) fn native_removals_flush(&mut self) {
        if self.persist.removals.is_empty() {
            return;
        }
        // Keep every deferred vehicle logically removed while callbacks in
        // this sweep can inspect the remaining physical owning slots.
        let pending = std::mem::take(&mut self.persist.removals);
        let mut already_marked = Vec::new();
        for &target in &pending {
            if matches!(target, Target::Boat(_) | Target::Cart(_))
                && !self.persist.destroying_vehicles.insert(target)
            {
                already_marked.push(target);
            }
        }
        for t in pending {
            match t {
                Target::Crystal(id) => self.crystals.retain(|c| c.id != id),
                Target::Dragon(id) => {
                    if self.dragon.as_ref().is_some_and(|d| d.id == id) {
                        self.discard_resident_dragon();
                    }
                }
                Target::Boat(id) | Target::Cart(id) => {
                    let marked = !already_marked.contains(&t);
                    self.spill_vehicle(t);
                    // Actual native removals already notified the edges. A
                    // raw deferred entry still needs its own guarded pass.
                    if marked && !self.persist.notified_vehicle_removals.contains(&id) {
                        self.persist.destroying_vehicles.remove(&t);
                    }
                    self.kill_vehicle_object(t);
                    self.score_destroyed_vehicle_once(id);
                    if marked {
                        self.persist.destroying_vehicles.remove(&t);
                    }
                    self.native_entity_left(id);
                }
                Target::Player | Target::Guest(_) | Target::Mob(_) => {}
                // The remaining kinds `/kill` already removes with no death.
                t => self.kill_target_in_level(t),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::api_values::{StackReadError, TryIntoApi, test_snapshot};
    #[test]
    #[ignore = "needs prepared resources"]
    fn destroying_a_cart_spills_all_slots_after_the_removal_flush() {
        use super::super::{
            minecarts::{CartKind, Minecart},
            native_entity_refs::tests::with_game,
        };
        use super::*;
        with_game(|game| {
            let mut cart = Minecart::new(600, CartKind::Chest, Vec3::new(1.0, 70.0, 2.0));
            cart.items[0] = Some(ItemStack::new("minecraft:stone", 64));
            cart.items[5] = Some(ItemStack::new("minecraft:stone", 19));
            cart.items[26] = Some(ItemStack::new("minecraft:sand", 7));
            game.carts.push(cart);
            let entity = game.entity(600).unwrap();
            assert!(game.entity_remove(entity, RemovalReason::Killed));
            assert!(game.dropped.is_empty());
            game.native_removals_flush();
            assert!(game.carts.is_empty());
            let mut totals = std::collections::BTreeMap::new();
            for item in game.dropped.iter() {
                *totals.entry(item.stack.item.to_string()).or_insert(0) += item.stack.count;
            }
            assert_eq!(
                totals,
                std::collections::BTreeMap::from([
                    ("minecraft:stone".to_owned(), 83),
                    ("minecraft:sand".to_owned(), 7),
                ])
            );
            let count = game.dropped.len();
            game.native_removals_flush();
            assert_eq!(game.dropped.len(), count);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn selectors_stop_matching_a_discarded_entity_before_the_sweep() {
        use super::super::{commands::OtherEntity, native_entity_refs::tests::with_game};
        use horizon_core::geometry::Vec3;

        with_game(|game| {
            game.others.push(OtherEntity {
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                item_transformation: None,
                interaction: None,
                id: 12,
                kind: "minecraft:marker".into(),
                position: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
            });
            game.ui.registries.permission = 4;
            let selector = "execute if entity @e[type=minecraft:marker]";
            assert!(game.run_command(selector));
            let entity = game.entity(12).unwrap();
            assert!(entity.discard(game));
            // No sweep has run, as in the hook that discarded it.
            assert_eq!(game.others.len(), 1);
            assert!(!game.run_command(selector));
            assert!(game.level_entity(12).is_none());
            game.native_removals_flush();
            assert!(game.others.is_empty());
        });
    }

    #[test]
    #[ignore = "needs prepared resources and an isolated process for native mods"]
    fn removal_dismounts_and_tells_mods_once_at_the_call() {
        use super::super::{boats::Boat, native_entity_refs::tests::with_game};
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        use mod_api::{EntityLeaveLevelEvent, Hooks, ServerHost};
        use std::sync::{Arc, Mutex};

        /// The entities heard leaving, and whether each read as gone by then.
        #[derive(Clone, Default)]
        struct Observer(Arc<Mutex<Vec<(u32, bool)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_leave_level(
                &self,
                host: &mut dyn ServerHost,
                event: &mut EntityLeaveLevelEvent<'_>,
            ) {
                let gone = host.entity(event.entity).is_none();
                self.0.lock().unwrap().push((event.entity, gone));
            }
        }
        let observer = Observer::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "removal-observer",
            Box::new(observer.clone()),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.native_joins_skip();
                let boat_id = game.take_entity_id();
                game.boats.push(Boat::new(
                    boat_id,
                    "oak".into(),
                    Vec3::new(0.0, 64.0, 0.0),
                    0.0,
                ));
                let mob_id = game.take_entity_id();
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                game.mobs
                    .push(Mob::new(mob_id, cow, Vec3::new(0.0, 64.0, 0.0), 0.0));
                game.native_joins_flush(false, 0);
                let boat = game.entity(boat_id).unwrap();
                let mob = game.entity(mob_id).unwrap();
                assert!(game.entity_start_riding(mob, boat, true));
                assert_eq!(game.entity_vehicle(mob), Some(boat));

                assert!(boat.discard(game));
                // Told, and off the vehicle, with the boat still in its list.
                assert!(game.boats.iter().any(|b| b.id == boat_id));
                assert_eq!(*observer.0.lock().unwrap(), vec![(boat_id, true)]);
                assert_eq!(game.entity_vehicle(mob), None);
                assert!(!game.mobs.iter().any(|m| m.extra.mount == Some(boat_id)));

                game.native_leaves_flush();
                assert!(!game.boats.iter().any(|b| b.id == boat_id));
                assert_eq!(*observer.0.lock().unwrap(), vec![(boat_id, true)]);
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn discard_other_clears_nbt_and_saved_entity() {
        use super::super::{commands::OtherEntity, native_entity_refs::tests::with_game};
        use horizon_client::{chunk::Chunk, save::SaveDir};
        use horizon_core::geometry::Vec3;
        use horizon_core::nbt::{Compound, Tag};
        use std::sync::Arc;

        with_game(|game| {
            let root = std::path::PathBuf::from(format!(
                "target/discard-save-test-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let shape = game.save.shape;
            game.save = Arc::new(SaveDir::new(root.clone(), shape));
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.others.push(OtherEntity {
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                item_transformation: None,
                interaction: None,
                id: 12,
                kind: "minecraft:text_display".into(),
                position: Vec3::new(1.0, 70.0, 1.0),
                yaw: 0.0,
                pitch: 0.0,
            });
            let mut tag = Compound::new();
            tag.insert("teleport_duration".into(), Tag::Int(4));
            game.persist.raw.insert(12, Tag::Compound(tag));
            game.save_entities(true).unwrap();
            assert_eq!(
                SaveDir::new(root.clone(), shape).load_entities(0, 0).len(),
                1
            );
            let entity = game.entity(12).unwrap();
            assert!(entity.discard(game));
            assert!(!entity.discard(game));
            game.native_removals_flush();
            assert!(!game.persist.raw.contains_key(&12));
            game.save_entities(true).unwrap();
            assert!(
                SaveDir::new(root.clone(), shape)
                    .load_entities(0, 0)
                    .is_empty()
            );
            std::fs::remove_dir_all(root).unwrap();
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn discard_mob_and_refuse_players_and_stale_handles() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::{mobs::Mob, motion::PlayerMotion};
        use horizon_core::geometry::Vec3;

        with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(12).unwrap();
            assert!(mob.discard(game));
            assert!(game.mobs[0].removed);
            assert!(!mob.discard(game));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            assert!(!player.discard(game));
            assert!(game.player.is_some());
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn spawn_at_location_drops_an_item_above_the_entity() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::{items::ItemStack, mobs::Mob};
        use horizon_core::geometry::Vec3;

        with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(12, kind, Vec3::new(3.5, 70.0, -2.5), 0.0));
            let mob = game.entity(12).unwrap();
            let item = mob
                .spawn_at_location(
                    game,
                    test_snapshot(&ItemStack::new("minecraft:carrot", 2)),
                    0.5,
                )
                .unwrap()
                .unwrap();
            let dropped = game.dropped.iter().find(|d| d.id == item.id()).unwrap();
            assert_eq!(&*dropped.stack.item, "minecraft:carrot");
            assert_eq!(dropped.stack.count, 2);
            assert_eq!(dropped.position, Vec3::new(3.5, 70.5, -2.5));
            assert_eq!(dropped.pickup_delay, 10);
            // Invalid engine values are refused before an API value can exist.
            let before = game.dropped.len();
            assert_eq!(
                ItemStack::new("minecraft:carrot", 0).try_into_api(),
                Err(StackReadError::Empty)
            );
            assert_eq!(
                ItemStack::new("minecraft:no_such_item", 1).try_into_api(),
                Err(StackReadError::UnknownItem)
            );
            assert!(mob.discard(game));
            assert_eq!(
                mob.spawn_at_location(
                    game,
                    test_snapshot(&ItemStack::new("minecraft:carrot", 1)),
                    0.0
                ),
                Ok(None)
            );
            assert_eq!(game.dropped.len(), before);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn spawn_at_location_offsets_hanging_entities_out_from_the_wall() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::entities::{ItemFrame, Painting};
        use horizon_client::items::ItemStack;
        use horizon_core::geometry::Vec3;

        with_game(|game| {
            for (id, glow, facing) in [(12, false, 5), (13, true, 4)] {
                game.item_frames.push(ItemFrame {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id,
                    pos: [0, 64, 0],
                    facing,
                    glow,
                    item: None,
                    rotation: 0,
                    drop_chance: 1.0,
                    fixed: false,
                    invisible: false,
                    check_interval: 0,
                    custom_name: None,
                    random: horizon_core::random::LegacyRandom::new(0),
                });
            }
            game.paintings.push(Painting {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 14,
                pos: [0, 64, 0],
                facing: 2,
                variant: "minecraft:kebab".into(),
                width: 1,
                height: 1,
                check_interval: 0,
            });
            // East frame, west glow frame, north painting. Expected origins
            // are the hanging center plus 0.15 facing out and the 0.5 Y offset.
            for (id, expected) in [
                (12, Vec3::new(0.18125, 65.0, 0.5)),
                (13, Vec3::new(0.81875, 65.0, 0.5)),
                (14, Vec3::new(0.5, 65.0, 0.81875)),
            ] {
                let entity = game.entity(id).unwrap();
                let item = entity
                    .spawn_at_location(
                        game,
                        test_snapshot(&ItemStack::new("minecraft:carrot", 2)),
                        0.5,
                    )
                    .unwrap()
                    .unwrap();
                let dropped = game.dropped.iter().find(|d| d.id == item.id()).unwrap();
                assert_eq!(dropped.position, expected);
                assert_eq!(dropped.stack.count, 2);
                assert_eq!(dropped.pickup_delay, 10);
            }
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn kill_command_forgets_other_entity_data() {
        use super::super::{commands::OtherEntity, native_entity_refs::tests::with_game};
        use horizon_core::geometry::Vec3;
        use horizon_core::nbt::{Compound, Tag};

        with_game(|game| {
            game.others.push(OtherEntity {
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                item_transformation: None,
                interaction: None,
                id: 12,
                kind: "minecraft:marker".into(),
                position: Vec3::ZERO,
                yaw: 0.0,
                pitch: 0.0,
            });
            game.persist.raw.insert(12, Tag::Compound(Compound::new()));
            game.ui.registries.permission = 4;
            assert!(game.run_command("kill @e[type=minecraft:marker]"));
            assert!(game.others.is_empty());
            assert!(!game.persist.raw.contains_key(&12));
        });
    }
}
