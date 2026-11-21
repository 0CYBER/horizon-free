use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A mob or server player's death has finished, after the loot, the
    /// experience and the death message (`ServerLivingEntityEvents.AFTER_DEATH`).
    pub(in crate::game) fn native_living_death_post(
        &mut self,
        entity: u32,
        damage_type: &str,
        attacker: u32,
    ) {
        self.fire_living_death_post(entity, damage_type, attacker);
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::native_entity_refs::tests::with_game;
    use super::*;
    use horizon_client::mobs::{DamageKind, Mob};
    use horizon_core::geometry::Vec3;
    use mod_api::{
        Flow, Hooks, LivingDeathEvent, LivingDeathPostEvent, LivingDropsEvent,
        LivingExperienceDropEvent, MobKilledEvent, ModEntry, ServerHost,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    #[derive(Default)]
    struct Post {
        entity: Option<u32>,
        damage_type: Option<String>,
        attacker: Option<u32>,
        drops_source: Option<String>,
        drops_before: bool,
        experience_before: bool,
    }

    struct Observer {
        heard: Arc<Mutex<Vec<&'static str>>>,
        cancel: Arc<AtomicBool>,
        drops: Arc<AtomicUsize>,
        experience: Arc<AtomicBool>,
        post: Arc<Mutex<Post>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn mob_killed(&self, _: &mut dyn ServerHost, _: &mut MobKilledEvent) {
            self.heard.lock().unwrap().push("killed");
        }
        fn living_death(&self, _: &mut dyn ServerHost, _: &mut LivingDeathEvent<'_>) -> Flow {
            self.heard.lock().unwrap().push("death");
            if self.cancel.load(Ordering::SeqCst) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn living_drops(&self, _: &mut dyn ServerHost, event: &mut LivingDropsEvent<'_>) -> Flow {
            self.heard.lock().unwrap().push("loot");
            self.drops.store(event.drops.len(), Ordering::SeqCst);
            self.post.lock().unwrap().drops_source = Some(event.source.damage_type.to_owned());
            Flow::Continue
        }
        fn living_experience_drop(
            &self,
            _: &mut dyn ServerHost,
            _: &mut LivingExperienceDropEvent,
        ) -> Flow {
            self.heard.lock().unwrap().push("xp");
            self.experience.store(true, Ordering::SeqCst);
            Flow::Continue
        }
        fn living_death_post(&self, _: &mut dyn ServerHost, event: &mut LivingDeathPostEvent<'_>) {
            self.heard.lock().unwrap().push("post");
            let mut post = self.post.lock().unwrap();
            post.entity = Some(event.entity);
            post.damage_type = Some(event.damage_type.to_owned());
            post.attacker = Some(event.attacker);
            post.drops_before = self.drops.load(Ordering::SeqCst) > 0;
            post.experience_before = self.experience.load(Ordering::SeqCst);
        }
    }

    /// A dead sheep: its table always drops at least one mutton, so the
    /// ordering against the drop list does not wait for a lucky roll.
    fn sheep(game: &mut super::Game<'_>, damage: Damage) -> Mob {
        game.next_entity += 1;
        let kind = game.res.mob_types().get("minecraft:sheep").unwrap().clone();
        let mut mob = Mob::new(game.next_entity, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        mob.health = 0.0;
        mob.last_damage = Some((damage, 0));
        game.mobs.push(mob.clone());
        mob
    }

    /// One notice per death, at the end: after `living_death` (unless it
    /// cancelled), the drop list and the experience, and before the tick is
    /// done. A player's blow and fall damage carry their damage types; the
    /// hook names the same last hurt `living_death` heard. A cancelled death
    /// is not told. The kill-blow hook heard before, the notice comes after.
    #[test]
    #[ignore = "daily"]
    fn living_death_post_fires_once_after_each_finished_death() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let cancel = Arc::new(AtomicBool::new(false));
        let drops = Arc::new(AtomicUsize::new(0));
        let experience = Arc::new(AtomicBool::new(false));
        let post = Arc::new(Mutex::new(Post::default()));
        let mods = vec![ModEntry::new(
            "deaths",
            Box::new(Observer {
                heard: heard.clone(),
                cancel: cancel.clone(),
                drops: drops.clone(),
                experience: experience.clone(),
                post: post.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();

                let player_blow = Damage {
                    kind: DamageKind::PlayerAttack,
                    attacker: Some(Who::Player),
                    from: Some([0.0; 3]),
                    attacker_species: None,
                };
                let mut mob = sheep(game, player_blow);
                // The last hurt by a player, so the death rolls experience.
                mob.ai.last_hurt_by_player = 100;
                game.mobs.last_mut().unwrap().ai.last_hurt_by_player = 100;
                // The blow that killed it, at the attack: mob_killed's moment.
                game.native_kill_event(game.mobs.len() - 1);
                game.mob_died(&mob);
                assert_eq!(
                    *heard.lock().unwrap(),
                    ["killed", "death", "loot", "xp", "post"]
                );
                {
                    let post = post.lock().unwrap();
                    assert_eq!(post.entity, Some(mob.id));
                    assert_eq!(post.damage_type.as_deref(), Some("minecraft:player_attack"));
                    assert_eq!(post.attacker, Some(0), "no player entity is credited");
                    assert!(post.drops_before, "the drop list was built before it");
                    assert!(post.experience_before, "the experience rolled before it");
                }
                assert_eq!(
                    game.dropped.len(),
                    drops.load(Ordering::SeqCst),
                    "every item drop it heard of is in the world"
                );
                assert!(!game.dropped.is_empty());

                heard.lock().unwrap().clear();
                *post.lock().unwrap() = Post::default();
                let fall = Damage {
                    kind: DamageKind::Fall,
                    attacker: None,
                    from: None,
                    attacker_species: None,
                };
                let mob = sheep(game, fall);
                game.mob_died(&mob);
                assert_eq!(*heard.lock().unwrap(), ["death", "loot", "post"]);
                assert_eq!(
                    post.lock().unwrap().damage_type.as_deref(),
                    Some("minecraft:fall")
                );

                heard.lock().unwrap().clear();
                *post.lock().unwrap() = Post::default();
                let mut mob = sheep(
                    game,
                    Damage {
                        kind: DamageKind::Generic,
                        attacker: None,
                        from: None,
                        attacker_species: None,
                    },
                );
                mob.health = 20.0;
                game.mobs.last_mut().unwrap().health = 20.0;
                // `/kill` replaces the earlier damage source with generic_kill.
                game.kill_target(crate::game::commands::Target::Mob(mob.id));
                assert_eq!(*heard.lock().unwrap(), ["death", "loot", "post"]);
                assert_eq!(
                    post.lock().unwrap().damage_type.as_deref(),
                    Some("minecraft:generic_kill")
                );

                // A death `living_death` kept: no drop, no notice.
                heard.lock().unwrap().clear();
                cancel.store(true, Ordering::SeqCst);
                let mob = sheep(
                    game,
                    Damage {
                        kind: DamageKind::PlayerAttack,
                        attacker: Some(Who::Player),
                        from: Some([0.0; 3]),
                        attacker_species: None,
                    },
                );
                game.mob_died(&mob);
                assert_eq!(*heard.lock().unwrap(), ["death"]);
                assert_eq!(
                    game.mobs.last().unwrap().health,
                    0.0,
                    "cancellation alone does not heal the body"
                );
            });
        });
    }

    /// A server player's death is told at `ServerPlayer.die`'s tail too: after
    /// its death message, the inventory drops and the experience. A death with
    /// no recorded hit still has a source, so its drop list is heard too.
    #[test]
    #[ignore = "daily"]
    fn living_death_post_hears_a_player_death_after_the_message_and_loot() {
        use crate::game::native_player_death::tests::add_guest;
        let heard = Arc::new(Mutex::new(Vec::new()));
        let drops = Arc::new(AtomicUsize::new(0));
        let experience = Arc::new(AtomicBool::new(false));
        let post = Arc::new(Mutex::new(Post::default()));
        let mods = vec![ModEntry::new(
            "deaths",
            Box::new(Observer {
                heard: heard.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                drops: drops.clone(),
                experience: experience.clone(),
                post: post.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();
                add_guest(game);
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.health = 0.0;
                    guest.inventory[0] =
                        Some(horizon_client::items::ItemStack::new("minecraft:stone", 1));
                });
                let id = game.lan.as_ref().unwrap().hub.guests[0].entity_id as u32;

                game.guest_die(0, 1);

                assert_eq!(
                    *heard.lock().unwrap(),
                    ["death", "loot", "xp", "post"],
                    "the inventory drop, the message and the experience ran before it"
                );
                let post = post.lock().unwrap();
                assert_eq!(post.entity, Some(id));
                assert_eq!(post.damage_type.as_deref(), Some("minecraft:generic"));
                assert_eq!(post.attacker, Some(0), "no attacker was recorded");
                assert!(post.experience_before, "the experience ran before it");
                assert!(
                    game.dropped
                        .iter()
                        .any(|d| &*d.stack.item == "minecraft:stone"),
                    "the inventory was dropped before it"
                );
            });
        });
    }

    /// `/kill` on a player records no combat hit, yet `dropAllDeathLoot` still
    /// gets its generic_kill source: with mods loaded the drop list is heard
    /// with that source, the one `living_death_post` names too.
    #[test]
    #[ignore = "daily"]
    fn living_drops_hears_a_killed_players_drops_with_generic_kill() {
        use crate::game::native_player_death::tests::add_guest;
        let heard = Arc::new(Mutex::new(Vec::new()));
        let post = Arc::new(Mutex::new(Post::default()));
        let mods = vec![ModEntry::new(
            "deaths",
            Box::new(Observer {
                heard: heard.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                drops: Arc::new(AtomicUsize::new(0)),
                experience: Arc::new(AtomicBool::new(false)),
                post: post.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.load_server_loot();
                add_guest(game);
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.loaded = true;
                    guest.load_timeout = 0;
                    guest.inventory[0] =
                        Some(horizon_client::items::ItemStack::new("minecraft:stone", 1));
                });

                game.kill_target(crate::game::commands::Target::Guest(1));
                game.guest_die(0, 1);

                assert_eq!(*heard.lock().unwrap(), ["death", "loot", "xp", "post"]);
                let post = post.lock().unwrap();
                assert_eq!(
                    post.drops_source.as_deref(),
                    Some("minecraft:generic_kill"),
                    "the drop list hears /kill's source"
                );
                assert_eq!(post.damage_type.as_deref(), Some("minecraft:generic_kill"));
            });
        });
    }
}
