//! `explosion_detonate`: the blocks and entities a server explosion is about to
//! reach (NeoForge's `ExplosionEvent.Detonate`), and the list of them the
//! listeners leave behind.
use super::Game;
use crate::game::entity::EntityHandle;
use mod_api::EntityRef;
use std::collections::HashSet;

impl Game<'_> {
    /// `ServerExplosion.explode` posting `onExplosionDetonate` before any block or
    /// entity is hit. The entities are `level.getEntities(source, box)`, the box
    /// the centre ± (2 × radius + 1) floored, without the exploding source; every
    /// kind is in it: the entity-list walk, then the item frames, paintings,
    /// cushions and end crystals the walk misses. The list they leave in
    /// `combat.blast_hit_list` is what the blast's walks then use: what a
    /// listener leaves in it is eligible for the normal hurt/push checks.
    /// Order controls only the ordinary-storage walk, not the separate crystal,
    /// dragon, player and decoration passes. Without a listener nothing is listed, and the walks
    /// enumerate the storages as before.
    pub(super) fn explosion_detonate(
        &mut self,
        center: [f64; 3],
        radius: f32,
        blocks: &mut Vec<[i32; 3]>,
    ) {
        // `ServerExplosion.hurtEntities` fires it only for a radius of at least 0.00001.
        if radius < 1.0e-5 || !mod_api::__private::listened(mod_api::HookId::ExplosionDetonate) {
            return;
        }
        let reach = f64::from(radius) * 2.0 + 1.0;
        let min = center.map(|c| (c - reach).floor());
        let max = center.map(|c| (c + reach).floor());
        let source = self
            .combat
            .explosion_direct
            .or(self.combat.explosion_source);
        let mut entities = self
            .level(self.dim.id())
            .and_then(|level| self.level_entities_in(level, min, max))
            .unwrap_or_default();
        entities.retain(|entity| Some(entity.id()) != source);
        self.append_blast_specials(&mut entities, source, min, max);
        let dimension = self.dim.id();
        self.native_explosion_detonate(dimension, center, blocks, &mut entities);
        // The walks follow the list: one a listener added is walked as any
        // other, so a handle that names nothing real is refused here, and a
        // repeated id is taken once (`getEntities` lists an entity once).
        entities.retain(|entity| self.native_entity_target(*entity).is_some());
        let mut seen = HashSet::new();
        entities.retain(|entity| seen.insert(entity.id()));
        self.combat.blast_hit_ids = seen;
        self.combat.blast_hit_list = Some(entities.iter().map(|entity| entity.id()).collect());
    }

    /// The four entity kinds the entity-list walk misses: item frames,
    /// paintings, cushions and end crystals. Appended after it, in their
    /// storage order, those whose box meets the blast's box
    /// (`Level.getEntities`), the exploding source excluded with the rest.
    fn append_blast_specials(
        &self,
        entities: &mut Vec<EntityRef>,
        source: Option<u32>,
        min: [f64; 3],
        max: [f64; 3],
    ) {
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return;
        };
        let in_box = |lo: [f64; 3], hi: [f64; 3]| (0..3).all(|k| lo[k] < max[k] && hi[k] > min[k]);
        let listed = |id: u32, lo: [f64; 3], hi: [f64; 3], entities: &mut Vec<EntityRef>| {
            if Some(id) != source && in_box(lo, hi) {
                entities.push(EntityRef::new(epoch, id));
            }
        };
        for frame in self.item_frames.iter() {
            let (lo, hi) = frame.bounds();
            listed(frame.id, lo, hi, entities);
        }
        for painting in self.paintings.iter() {
            let (lo, hi) = painting.bounds();
            listed(painting.id, lo, hi, entities);
        }
        for cushion in self.cushions.iter() {
            let (lo, hi) = cushion.bounds();
            listed(cushion.id, lo, hi, entities);
        }
        for crystal in self.crystals.iter() {
            let bounds = crystal.bounds();
            let lo = [
                bounds.min(horizon_core::geometry::Axis::X),
                bounds.min(horizon_core::geometry::Axis::Y),
                bounds.min(horizon_core::geometry::Axis::Z),
            ];
            let hi = [
                bounds.max(horizon_core::geometry::Axis::X),
                bounds.max(horizon_core::geometry::Axis::Y),
                bounds.max(horizon_core::geometry::Axis::Z),
            ];
            listed(crystal.id, lo, hi, entities);
        }
    }

    /// The handle of the storaged entity `id` names, for the listed walk.
    /// The kinds outside the storages (frames, paintings, cushions, crystals,
    /// the dragon and its parts, hooks, players) answer None: their own
    /// passes and the player walk read the list for them.
    pub(in crate::game) fn blast_handle(&self, id: u32) -> Option<EntityHandle> {
        if let Some(i) = self.mobs.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Mob(i));
        }
        if let Some(i) = self.dropped.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Item(i));
        }
        if let Some(i) = self.boats.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Boat(i));
        }
        if let Some(i) = self.carts.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Cart(i));
        }
        if let Some(i) = self.armor_stands.iter().position(|m| m.id == id) {
            return Some(EntityHandle::ArmorStand(i));
        }
        if let Some(i) = self.tnts.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Tnt(i));
        }
        if let Some(i) = self.falling.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Falling(i));
        }
        if let Some(i) = self.orbs.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Orb(i));
        }
        if let Some(i) = self.creatures.projectiles.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Projectile(i));
        }
        if let Some(i) = self.rockets.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Rocket(i));
        }
        if let Some(i) = self.tridents.iter().position(|m| m.id == id) {
            return Some(EntityHandle::Trident(i));
        }
        if let Some(i) = self
            .resident_entities
            .general
            .rows
            .iter()
            .position(|m| m.id == id)
        {
            return Some(EntityHandle::General(i));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::mobs::Mob;
    use horizon_core::{geometry::Vec3, random::RandomSource};
    use mod_api::{EntityRef, ExplosionDetonateEvent, Hooks, ServerHost};
    use std::sync::{Arc, Mutex};

    struct Noop;
    #[mod_api::__subscriptions]
    impl Hooks for Noop {
        fn explosion_detonate(&self, _: &mut dyn ServerHost, _: &mut ExplosionDetonateEvent<'_>) {}
    }

    const SPARED: u32 = 901;
    const FAR: u32 = 905;

    struct Probe(Arc<Mutex<Vec<u32>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn explosion_detonate(
            &self,
            _: &mut dyn ServerHost,
            event: &mut ExplosionDetonateEvent<'_>,
        ) {
            *self.0.lock().unwrap() = event.entities.iter().map(|e| e.id()).collect();
            event.entities.retain(|e| e.id() != SPARED);
        }
    }

    fn zombie(game: &mut crate::game::Game<'_>, id: u32, x: f64) {
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs
            .push(Mob::new(id, zombie, Vec3::new(x, 1.0, 0.0), 0.0));
    }

    fn explode(game: &mut crate::game::Game<'_>) {
        game.explode(
            None,
            [0.0, 1.0, 0.0],
            3.0,
            super::super::explosion::ExplosionSource::default(),
        )
        .unwrap();
    }

    fn health_of(game: &crate::game::Game<'_>, id: u32) -> f32 {
        game.mobs.iter().find(|m| m.id == id).unwrap().health
    }

    #[test]
    #[ignore = "daily"]
    fn an_entity_taken_out_is_neither_hurt_nor_pushed() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "detonate",
            Box::new(Probe(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                zombie(game, 900, 2.0);
                zombie(game, SPARED, -2.0);
                explode(game);
                let (hit, spared) = (
                    game.mobs.iter().find(|m| m.id == 900).unwrap(),
                    game.mobs.iter().find(|m| m.id == SPARED).unwrap(),
                );
                assert!(hit.health < 20.0);
                assert_ne!(hit.velocity, Vec3::ZERO);
                assert_eq!(spared.health, 20.0);
                assert_eq!(spared.velocity, Vec3::ZERO);
            });
        });
        let heard = heard.lock().unwrap();
        assert!(heard.contains(&900) && heard.contains(&SPARED), "{heard:?}");
    }

    /// `getEntities(source, box)` leaves the blast's exploding source out:
    /// it is not in the list and its own blast does not hurt it.
    #[test]
    #[ignore = "daily"]
    fn the_exploding_source_is_not_in_the_list_nor_hurt() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "detonate",
            Box::new(Probe(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                zombie(game, 900, 2.0);
                zombie(game, 910, -2.0);
                game.combat.explosion_direct = Some(900);
                explode(game);
                let source = game.mobs.iter().find(|m| m.id == 900).unwrap();
                assert_eq!(source.health, 20.0);
                assert_eq!(source.velocity, Vec3::ZERO);
            });
        });
        let heard = heard.lock().unwrap();
        assert!(!heard.contains(&900) && heard.contains(&910), "{heard:?}");
    }

    /// The list is the walk: an entity a listener adds is hurt as any other,
    /// and a handle that names nothing real is refused, with no panic.
    #[test]
    #[ignore = "daily"]
    fn an_added_entity_is_hurt_and_an_empty_handle_is_refused() {
        struct Add;
        #[mod_api::__subscriptions]
        impl Hooks for Add {
            fn explosion_detonate(
                &self,
                _: &mut dyn ServerHost,
                event: &mut ExplosionDetonateEvent<'_>,
            ) {
                let Some(first) = event.entities.first() else {
                    return;
                };
                let epoch = first.epoch();
                // Rebuild the list, adding an eligible live entity as well as
                // the out-of-range and nonexistent entries below.
                event.entities.clear();
                event.entities.push(EntityRef::new(epoch, 900));
                event.entities.push(EntityRef::new(epoch, FAR));
                event.entities.push(EntityRef::new(epoch, 424242));
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("add", Box::new(Add))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                zombie(game, 900, 2.0);
                zombie(game, FAR, 40.0);
                explode(game);
                assert_eq!(health_of(game, FAR), 20.0);
                assert_eq!(
                    game.mobs.iter().find(|mob| mob.id == FAR).unwrap().velocity,
                    Vec3::ZERO
                );
                assert!(health_of(game, 900) < 20.0);
            });
        });
    }

    /// The list's order is the visit order: the reversed list reverses the
    /// order the mobs take their hurt in (`living_damage_post`'s).
    #[test]
    #[ignore = "daily"]
    fn the_list_order_is_the_walk_order() {
        struct Reverse(Arc<Mutex<Vec<u32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Reverse {
            fn explosion_detonate(
                &self,
                _: &mut dyn ServerHost,
                event: &mut ExplosionDetonateEvent<'_>,
            ) {
                event.entities.reverse();
            }
            fn living_damage_post(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::LivingDamagePostEvent<'_>,
            ) {
                if let Ok(id) = u32::try_from(event.entity) {
                    self.0.lock().unwrap().push(id);
                }
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "reverse",
            Box::new(Reverse(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                zombie(game, 900, 2.0);
                zombie(game, 901, 1.0);
                zombie(game, 902, -1.0);
                explode(game);
            });
        });
        let heard = heard.lock().unwrap();
        let at = |id| {
            heard
                .iter()
                .position(|&h| h == id)
                .unwrap_or_else(|| panic!("{heard:?}"))
        };
        assert!(
            at(900) > at(901) && at(901) > at(902),
            "reversed, not storage order: {heard:?}"
        );
    }

    /// A position a listener takes out keeps its block and gets no fire,
    /// while the blast still reaches the entities.
    #[test]
    #[ignore = "daily"]
    fn a_position_taken_out_keeps_its_block() {
        struct SpareBlocks;
        #[mod_api::__subscriptions]
        impl Hooks for SpareBlocks {
            fn explosion_detonate(
                &self,
                _: &mut dyn ServerHost,
                event: &mut ExplosionDetonateEvent<'_>,
            ) {
                event.blocks.retain(|pos| *pos != [0, 0, 0]);
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "spare_blocks",
            Box::new(SpareBlocks),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                zombie(game, 900, 2.0);
                let ground = game.world.block(0, 0, 0);
                assert!(!game.res.facts().is_air(ground), "the ground is solid");
                explode(game);
                assert_eq!(game.world.block(0, 0, 0), ground, "the block is kept");
                assert!(health_of(game, 900) < 20.0, "the entities are still walked");
            });
        });
    }

    /// A blast set off inside another's window keeps its own: the nested
    /// blast hurts what the outer one spared, and the outer's list is back
    /// afterwards for its own walk.
    #[test]
    #[ignore = "daily"]
    fn a_nested_blast_keeps_its_own_spared() {
        struct FirstOnly(Arc<Mutex<usize>>);
        #[mod_api::__subscriptions]
        impl Hooks for FirstOnly {
            fn explosion_detonate(
                &self,
                _: &mut dyn ServerHost,
                event: &mut ExplosionDetonateEvent<'_>,
            ) {
                let mut seen = self.0.lock().unwrap();
                *seen += 1;
                if *seen == 1 {
                    // The outer blast spares the zombie.
                    event.entities.retain(|e| e.id() != SPARED);
                }
            }
        }
        let heard = Arc::new(Mutex::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "first_only",
            Box::new(FirstOnly(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                zombie(game, SPARED, -2.0);
                // The outer blast lists its entities and spares the zombie.
                let mut blocks = Vec::new();
                game.explosion_detonate([0.0, 1.0, 0.0], 3.0, &mut blocks);
                assert!(
                    game.combat.blast_hit_list.is_some(),
                    "the outer list is set"
                );
                let before = health_of(game, SPARED);
                // A blast set off inside the outer's window, as a listener's
                // `level_explode` is: it keeps its own list, and does not
                // inherit the outer's spared.
                game.server_explode_from(
                    [0.0, 1.0, 0.0],
                    3.0,
                    super::super::explosion::BlockInteraction::Keep,
                    false,
                    super::super::explosion::ExplosionSource::default(),
                )
                .unwrap();
                assert!(
                    health_of(game, SPARED) < before,
                    "the nested blast does not inherit the outer's spared"
                );
                // The outer's list is back: its walk still spares the zombie.
                assert!(
                    game.combat.blast_hit_list.is_some() && !game.combat.visits(SPARED),
                    "the outer's list is restored"
                );
            });
        });
        assert_eq!(*heard.lock().unwrap(), 2, "outer and nested");
    }

    /// With no listener the blast is the blast that was: the same blocks
    /// broken and the same hurt as with a listener that edits nothing, so
    /// the hook costs the random sequence nothing.
    #[test]
    #[ignore = "daily"]
    fn with_no_listener_the_blast_is_unchanged() {
        fn blast(mods: mod_api::Mods) -> (Vec<([i32; 3], u32)>, f32, i32) {
            let mut blocks = Vec::new();
            let mut health = 20.0;
            let mut random = 0;
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    zombie(game, 900, 2.0);
                    let mut boat = crate::game::boats::Boat::new(
                        930,
                        "oak".into(),
                        Vec3::new(-2.0, 1.0, 0.0),
                        0.0,
                    )
                    .with_chest();
                    boat.items[0] =
                        Some(horizon_client::items::ItemStack::new("minecraft:stick", 3));
                    game.boats.push(boat);
                    game.world.random.set_seed(7);
                    explode(game);
                    for y in 0..4 {
                        for z in -4..=4 {
                            for x in -4..=4 {
                                let at = [x, y, z];
                                blocks.push((at, game.world.block(at[0], at[1], at[2]).0));
                            }
                        }
                    }
                    health = health_of(game, 900);
                    random = game.world.random.next_int();
                });
            });
            (blocks, health, random)
        }
        let plain = blast(Vec::new().into());
        let noop: mod_api::Mods = vec![mod_api::ModEntry::new("noop", Box::new(Noop))].into();
        let hooked = blast(noop);
        assert_eq!(plain.0, hooked.0, "the same blocks broken and burnt");
        assert_eq!(plain.1, hooked.1, "the same hurt");
        assert_eq!(plain.2, hooked.2, "the same subsequent level-random draw");
    }
    #[test]
    #[ignore = "daily"]
    fn destroying_two_vehicles_keeps_the_next_stable_identity_with_or_without_a_listener() {
        for listened in [false, true] {
            let mods: mod_api::Mods = if listened {
                vec![mod_api::ModEntry::new("noop", Box::new(Noop))].into()
            } else {
                Vec::new().into()
            };
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    for (id, x) in [(920, -1.0), (921, 1.0)] {
                        game.boats.push(crate::game::boats::Boat::new(
                            id,
                            "oak".into(),
                            Vec3::new(x, 1.0, 0.0),
                            0.0,
                        ));
                        game.carts.push(crate::game::minecarts::Minecart::new(
                            id + 10,
                            crate::game::minecarts::CartKind::Rideable,
                            Vec3::new(x, 1.0, 2.0),
                        ));
                    }
                    explode(game);
                    assert!(game.boats.iter().all(|boat| ![920, 921].contains(&boat.id)));
                    assert!(game.carts.iter().all(|cart| ![930, 931].contains(&cart.id)));
                    assert_eq!(game.combat.blast_hit_list, None);
                })
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn special_query_excludes_a_box_that_only_touches_the_boundary() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("noop", Box::new(Noop))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                for (id, x) in [(940, 8.0), (941, 7.99)] {
                    game.crystals.push(crate::game::dragon::Crystal {
                        attachment_lifetime: Default::default(),
                        join: Default::default(),
                        id,
                        position: Vec3::new(x, 1.0, 0.0),
                        show_bottom: false,
                        time: 0,
                        beam: None,
                        invulnerable: false,
                        fall: (false, 0.0),
                    });
                }
                game.explosion_detonate([0.0, 1.0, 0.0], 3.0, &mut Vec::new());
                let listed = game.combat.blast_hit_list.as_ref().unwrap();
                assert!(!listed.contains(&940));
                assert!(listed.contains(&941));
            })
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removing_the_next_target_from_a_callback_still_visits_later_originals() {
        struct RemoveNext(Arc<Mutex<Vec<u32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for RemoveNext {
            fn explosion_knockback(
                &self,
                host: &mut dyn ServerHost,
                event: &mut mod_api::ExplosionKnockbackEvent<'_>,
            ) {
                if let Some(id) = event.entity {
                    self.0.lock().unwrap().push(id);
                    if id == 970 {
                        assert!(host.entity(971).unwrap().discard(host));
                    }
                }
            }
        }
        for listened in [false, true] {
            let heard = Arc::new(Mutex::new(Vec::new()));
            let mut mods = vec![mod_api::ModEntry::new(
                "remove",
                Box::new(RemoveNext(heard.clone())),
            )];
            if listened {
                mods.push(mod_api::ModEntry::new("noop", Box::new(Noop)));
            }
            mod_platform::testing::with_mods(mods.into(), || {
                with_game(|game| {
                    for (id, x) in [(970, -1.0), (971, 0.0), (972, 1.0)] {
                        game.boats.push(crate::game::boats::Boat::new(
                            id,
                            "oak".into(),
                            Vec3::new(x, 1.0, 0.0),
                            0.0,
                        ));
                    }
                    explode(game);
                    assert!(
                        game.boats
                            .iter()
                            .all(|boat| ![970, 971, 972].contains(&boat.id))
                    );
                })
            });
            assert_eq!(
                *heard.lock().unwrap(),
                [970, 972],
                "subscription: {listened}"
            );
        }
    }

    fn specials(game: &mut crate::game::Game<'_>) {
        use horizon_client::entities::{ArmorStand, Cushion, ItemFrame, Painting};
        use horizon_core::random::LegacyRandom;
        game.armor_stands
            .push(ArmorStand::new(950, Vec3::new(2.0, 1.0, 0.0), 0.0));
        game.item_frames.push(ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 951,
            pos: [2, 1, 0],
            facing: 3,
            glow: false,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            custom_name: None,
            random: LegacyRandom::new(1),
        });
        game.paintings.push(Painting {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 952,
            pos: [2, 1, 0],
            facing: 0,
            variant: "minecraft:kebab".into(),
            width: 1,
            height: 1,
            check_interval: 0,
        });
        game.cushions.push(Cushion {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 953,
            position: Vec3::new(2.0, 1.0, 0.0),
            pos: [2, 1, 0],
            yaw: 0.0,
            color: 0,
            check_interval: 0,
            custom_name: None,
        });
        game.crystals.push(crate::game::dragon::Crystal {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 954,
            position: Vec3::new(2.0, 1.0, 0.0),
            show_bottom: false,
            time: 0,
            beam: None,
            invulnerable: false,
            fall: (false, 0.0),
        });
        zombie(game, 955, -2.0);
        game.boats.push(crate::game::boats::Boat::new(
            960,
            "oak".into(),
            Vec3::new(1.0, 1.0, 0.0),
            0.0,
        ));
    }

    #[test]
    #[ignore = "daily"]
    fn empty_and_mixed_kind_lists_control_direct_membership_with_ordinary_order_only() {
        struct Filter(bool, Arc<Mutex<Vec<u32>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Filter {
            fn explosion_detonate(
                &self,
                _: &mut dyn ServerHost,
                event: &mut ExplosionDetonateEvent<'_>,
            ) {
                let ids = event.entities.iter().map(|e| e.id()).collect::<Vec<_>>();
                for id in [950, 951, 952, 953, 954, 955, 960] {
                    assert!(ids.contains(&id), "{ids:?}");
                }
                event.blocks.clear();
                event
                    .entities
                    .retain(|e| !self.0 && [951, 960, 955].contains(&e.id()));
                event
                    .entities
                    .sort_by_key(|e| [951, 960, 955].iter().position(|id| *id == e.id()));
            }
            fn explosion_knockback(
                &self,
                _: &mut dyn ServerHost,
                event: &mut mod_api::ExplosionKnockbackEvent<'_>,
            ) {
                if let Some(id) = event.entity {
                    self.1.lock().unwrap().push(id);
                }
            }
        }
        for empty in [true, false] {
            let heard = Arc::new(Mutex::new(Vec::new()));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "filter",
                Box::new(Filter(empty, heard.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    specials(game);
                    explode(game);
                    assert!(game.armor_stands.iter().any(|entity| entity.id == 950));
                    assert!(game.paintings.iter().any(|entity| entity.id == 952));
                    assert!(game.cushions.iter().any(|entity| entity.id == 953));
                    assert!(game.crystals.iter().any(|entity| entity.id == 954));
                    assert_eq!(
                        game.item_frames.iter().any(|entity| entity.id == 951),
                        empty
                    );
                    assert_eq!(game.boats.iter().any(|entity| entity.id == 960), empty);
                    if empty {
                        assert_eq!(health_of(game, 955), 20.0);
                    } else {
                        assert!(health_of(game, 955) < 20.0);
                    }
                })
            });
            assert_eq!(
                *heard.lock().unwrap(),
                if empty { vec![] } else { vec![960, 955] }
            );
        }
    }
}
