//! Pandas on the game side: the food they find on the ground and pick up
//! (`Mob.aiStep`, `Panda.pickUpItem`), the bamboo a player gives them
//! (`Panda.mobInteract`) and the start a sneeze gives the grown pandas about run on the
//! game's `PandaLevel` (`horizon_gameplay::pandas`, implemented below: the food tag is
//! the private `Game::panda_eats_from_ground`, which a mod can listen to); the crumbs and the sneeze a window
//! draws stay here.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mobs::{Species, panda},
    particles::{ParticleOptions, ParticleType},
};
use horizon_gameplay::panda_level::PandaLevel;

impl Game<'_> {
    /// `Panda.canPickUpAndEat`'s tag.
    fn panda_eats_from_ground(&self, item: &str) -> bool {
        self.res
            .items()
            .item_tags
            .get("minecraft:panda_eats_from_ground")
            .is_some_and(|tag| tag.contains(item))
    }
    /// A panda takes what lies at its feet when its mouth is free, and
    /// looks for food within 6 and 8 blocks for `PandaSitGoal`.
    pub(super) fn tick_pandas(&mut self) {
        horizon_gameplay::pandas::tick_pandas(self);
    }
    /// `Panda.mobInteract` for a click with `item`; whether the click is taken.
    pub(super) fn interact_panda(&mut self, index: usize, item: &str) -> bool {
        horizon_gameplay::pandas::interact_panda(self, index, item)
    }
    /// `Panda.afterSneeze`: the grown pandas within 10 blocks of a cub jump.
    pub(super) fn panda_sneezed(&mut self, id: u32) {
        horizon_gameplay::pandas::panda_sneezed(self, id);
    }
    /// What a window draws of its pandas: the crumbs of a bite
    /// (`addEatingParticles`) and the sneeze's particle.
    pub(super) fn panda_effects(&mut self) {
        let mut bites = Vec::new();
        let mut sneezes = Vec::new();
        for m in self
            .mobs
            .iter_mut()
            .filter(|m| m.species() == Species::Panda)
        {
            let (crumbs, sneeze) = panda::effects(m);
            if let (Some(crumbs), Some(item)) = (
                crumbs,
                horizon_client::mobs::finalize::item(m, horizon_client::mobs::finalize::MAINHAND),
            ) {
                bites.push((item, crumbs));
            }
            sneezes.extend(sneeze);
        }
        for (item, crumbs) in bites {
            let options = self.item_particle(&ItemStack::new(&item, 1));
            for (at, velocity) in crumbs {
                self.particles.add(&options, at, velocity);
            }
        }
        let options = ParticleOptions::simple(ParticleType::Sneeze);
        for (at, velocity) in sneezes {
            self.particles.add(&options, at, velocity);
        }
    }
}

impl PandaLevel for Game<'_> {
    fn panda_eats_from_ground(&self, item: &str) -> bool {
        Game::panda_eats_from_ground(self, item)
    }
    fn is_creative(&self) -> bool {
        self.creative
    }
    fn panda_love_decision(&mut self, index: usize) -> Option<(usize, bool)> {
        self.animal_love_decision(index, true)
    }
    fn panda_in_love(&mut self, index: usize) {
        let panda = &self.mobs[index];
        let id = panda.id;
        let kind = super::native_hooks::mob_kind(panda);
        self.love_cause.insert(id, self.lan_acting);
        self.native_entity_enter_love_mode(id, &kind, self.lan_acting_uuid);
        // Animal.setInLove: entity event 18, the love hearts.
        if self.feeding_animal_index(index, id).is_some() {
            self.mob_event_particles(id, 18);
        }
    }
    fn consume_held(&mut self) {
        Game::consume_held(self);
    }
    fn after_mobs_lifted(&mut self) {
        #[cfg(test)]
        super::entity::taken_pass_tests::after_take(self);
    }
}

#[cfg(test)]
mod nested_spawn_tests {
    #[test]
    #[ignore = "daily"]
    fn lifted_mob_callback_keeps_nested_spawns_until_the_next_pass() {
        crate::game::entity::taken_pass_tests::in_both_dimensions(|game, balance| {
            let kind = game.res.mob_types().get("minecraft:panda").unwrap().clone();
            game.mobs.push(horizon_client::mobs::Mob::new(
                99,
                kind,
                horizon_core::geometry::Vec3::new(0.5, 80.0, 0.5),
                0.0,
            ));
            crate::game::entity::taken_pass_tests::assert_mob_spawn_survives(
                game,
                balance,
                |game| {
                    game.panda_sneezed(99);
                },
            );
        });
    }
}

#[cfg(test)]
mod pickup_tests {
    use super::*;
    use crate::game::game_tests::tests::server_game;
    use horizon_client::advancements as adv;
    use horizon_client::motion::PlayerMotion;
    use horizon_core::geometry::Vec3;
    use horizon_world::lazy_json::LazyJson;

    const ADV: &str = "horizon:test/panda_pickup";
    const TRIGGER: &str = "minecraft:thrown_item_picked_up_by_entity";
    const THROWER: [i32; 4] = [413, 0, 0, 1];

    /// An advancement on the pickup trigger, as a data pack would add one: no
    /// vanilla advancement tests it, so the tree needs one of its own.
    fn install_pickup_advancement(game: &mut Game<'_>) {
        let mut tree = std::sync::Arc::new(adv::Tree::default());
        let one = std::sync::Arc::get_mut(&mut tree).unwrap();
        one.list.push(adv::Advancement {
            id: ADV.into(),
            parent: None,
            display: None,
            criteria: vec![adv::Criterion {
                name: "pickup".into(),
                trigger: TRIGGER.into(),
                conditions: LazyJson::new(&serde_json::json!({})),
            }],
            requirements: vec![vec!["pickup".into()]],
            rewards: Default::default(),
        });
        one.by_id.insert(ADV.into(), 0);
        one.by_trigger.insert(TRIGGER.into(), vec![(0, 0)]);
        game.adv.tree = tree;
    }

    /// `Panda.pickUpItem` calls `onItemPickup`, which fires
    /// `CriteriaTriggers.THROWN_ITEM_PICKED_UP_BY_ENTITY` for whoever threw the
    /// bamboo, as the fox's pickup does (`LivingEntity.onItemPickup`).
    #[test]
    #[ignore = "daily"]
    fn a_panda_that_takes_a_thrown_item_credits_the_thrower() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.options.dedicated = None;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 4.5)).unwrap());
        game.level.player_uuid = Some(THROWER);
        game.level.health = 20.0;
        game.creative = false;
        // A world of its own under the temporary folder, never the cwd.
        game.world_root =
            std::env::temp_dir().join(format!("horizon-panda-pickup-world-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&game.world_root);
        game.build_registries();
        game.set_allow_commands(true);
        install_pickup_advancement(&mut game);
        // `DistanceManager` keeps a chunk entity-ticking only while its whole
        // 5x5 neighbourhood is resident.
        for cx in -2..=2 {
            for cz in -2..=2 {
                game.world.chunks.insert(
                    (cx, cz),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(cx, cz)),
                );
            }
        }
        for command in [
            "forceload add 0 0",
            "tick freeze",
            "fill 0 99 0 15 99 15 stone",
            "summon minecraft:panda 4.5 100 6.5 {UUID:[I;9,9,9,9],PersistenceRequired:1b,CanPickUpLoot:1b}",
            "summon minecraft:item 4.5 100 6.5 {Item:{id:\"minecraft:bamboo\",count:1},Thrower:[I;413,0,0,1],PickupDelay:0s}",
        ] {
            assert!(game.run_command(command), "{command}");
        }
        let id = game.mob_by_uuid([9, 9, 9, 9]).unwrap();
        let holding = |g: &Game<'_>| {
            g.mobs
                .iter()
                .any(|m| m.id == id && m.extra.main_hand_id().is_some())
        };
        for _ in 0..20 {
            if holding(&game) {
                break;
            }
            assert!(game.run_command("tick step 1"));
            game.tick_server(false).unwrap();
        }
        assert!(holding(&game), "the panda took the bamboo");
        assert!(
            game.adv.progress.has(ADV, "pickup"),
            "the thrower's advancement: {:?}",
            game.adv.progress.criteria
        );
        let _ = std::fs::remove_dir_all(&game.world_root);
    }
}
