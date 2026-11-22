//! An entity's saved data, tags and name for native server mods, through an
//! `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): what
//! `/data` and `/tag` read and change, its custom name, its type's category
//! and whether it is in its level. They return `None` (or `false`) for a
//! stale handle or on a client, which the server sends no saved data or tags;
//! only the name reads there.
use super::{Game, commands::Target};
use horizon_client::{entity_nbt::flag, scoreboard::json_to_tag, text::Text};
use horizon_core::nbt::{Compound, Tag};
use mod_api::{EntityRef, MobCategory};

/// The most tags an entity holds (`Entity.addTag`).
const MAX_TAGS: usize = 1024;

impl Game<'_> {
    /// The entity's data as `/data get entity` shows it
    /// (`Entity.saveWithoutId`): its compound without `id`. An entity with no
    /// UUID yet gets the one it will be saved with. API 386.
    pub(super) fn entity_nbt(&mut self, entity: EntityRef) -> Option<Compound> {
        if self.remote.is_some() {
            return None;
        }
        let t = self.native_entity_target(entity)?;
        match self.command_target_nbt(t)? {
            Tag::Compound(mut c) => {
                c.remove(&"id".into());
                Some(c)
            }
            _ => None,
        }
    }
    /// Merges `nbt` into the entity's data and loads the entity from it, as
    /// `/data merge entity` does (`EntityDataAccessor.setData`): true when it
    /// changed. False for a player (vanilla refuses them too), a kind Horizon
    /// can't reload yet (only mobs, dropped items, armor stands and item
    /// frames), data nested too deep, or data that changes nothing. API
    /// 386.
    pub(super) fn entity_merge_nbt(&mut self, entity: EntityRef, nbt: Compound) -> bool {
        if self.remote.is_some() {
            return false;
        }
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        let Some(old) = self.entity_nbt(entity).map(Tag::Compound) else {
            return false;
        };
        let nbt = Tag::Compound(nbt);
        if horizon_client::nbt_path::too_deep(&nbt, 0) {
            return false;
        }
        let merged = horizon_client::data_cmd::merged(&old, &nbt);
        merged != old && self.apply_entity_nbt(t, merged).is_ok()
    }
    /// Keeps the entity out of the level's saves (`saved` false) or puts it
    /// back in (true, what every entity starts as): the autosave and the save
    /// of its chunk's unload write the level without it, so it is gone at the
    /// next start. For an entity a mod owns and removes itself (a display that
    /// follows another entity, a floating number): call it right after
    /// `summon`, so a crash or an unload before the mod's own cleanup leaves
    /// nothing behind. Whatever rides it is saved under it, so goes unsaved
    /// with it; an entity that rides another is saved with its vehicle, so
    /// keep the vehicle out. False for a player, a stale handle and on a
    /// client.
    pub(super) fn entity_set_saved(&mut self, entity: EntityRef, saved: bool) -> bool {
        if !self.is_logical_server() {
            return false;
        }
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        if matches!(t, Target::Player | Target::Guest(_)) {
            return false;
        }
        let Some(id) = super::native_entities::target_id(t) else {
            return false;
        };
        if saved {
            self.persist.unsaved.remove(&id);
        } else {
            // The save finds the entity by the UUID it writes: fix one.
            self.uuid_for(id);
            self.persist.unsaved.insert(id);
        }
        true
    }
    /// The entity's tags (`Entity.getTags`), as `/tag <entity> list` shows
    /// them. API 386.
    pub(super) fn entity_tags(&self, entity: EntityRef) -> Option<Vec<String>> {
        if self.remote.is_some() {
            return None;
        }
        Some(self.tags_of(self.native_entity_target(entity)?))
    }
    /// Adds tag `tag` (`Entity.addTag`): false when it has it already, holds
    /// 1024, its kind can't store tags yet, or the handle is stale. API 386.
    pub(super) fn entity_add_tag(&mut self, entity: EntityRef, tag: &str) -> bool {
        if self.remote.is_some() {
            return false;
        }
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        let mut tags = self.tags_of(t);
        if tags.len() >= MAX_TAGS || tags.iter().any(|x| x == tag) {
            return false;
        }
        tags.push(tag.to_owned());
        // False, not a silent no-op, for a kind with no saved compound to
        // write tags to (a natural leash knot).
        self.set_entity_tags(t, &tags)
    }
    /// Removes tag `tag` (`Entity.removeTag`): false when it didn't have it.
    /// API 386.
    pub(super) fn entity_remove_tag(&mut self, entity: EntityRef, tag: &str) -> bool {
        if self.remote.is_some() {
            return false;
        }
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        let mut tags = self.tags_of(t);
        let before = tags.len();
        tags.retain(|x| x != tag);
        tags.len() != before && self.set_entity_tags(t, &tags)
    }
    /// The entity's name as plain text (`Entity.getName`): its custom name,
    /// a player's name, else its type's name. On a client (API
    /// @API:client_entity_refs@), from the custom name the server synced
    /// (`Entity.DATA_CUSTOM_NAME`) and a player's profile name. API
    /// 386.
    pub(super) fn entity_name(&self, entity: EntityRef) -> Option<String> {
        let t = self.native_entity_read_target(entity)?;
        if let Some(r) = &self.remote {
            let id = i32::try_from(entity.id()).ok()?;
            if let Some((_, Some(name))) = r.level_player(id) {
                return Some(name.to_owned());
            }
            if let Target::Stand(id) = t
                && let Some(name) = self
                    .armor_stands
                    .iter()
                    .find(|a| a.id == id)
                    .and_then(|a| a.custom_name.as_ref())
            {
                return Some(name.plain());
            }
        }
        Some(self.target_name(t).plain())
    }
    /// Names the entity (`Entity.setCustomName`), or takes its name away
    /// with `None`: its name, its saved `CustomName` and, for a mob or an
    /// armor stand, what the players tracking it are sent. False for a
    /// player, a kind whose name Horizon keeps nowhere yet (projectiles, the
    /// dragon and crystals, primed TNT, falling blocks, experience orbs,
    /// ominous item spawners), a stale handle or on a client.
    pub(super) fn entity_set_custom_name(&mut self, entity: EntityRef, name: Option<Text>) -> bool {
        let Some(id) = self.name_holder(entity) else {
            return false;
        };
        let saved = name.as_ref().map(|n| json_to_tag(&n.component()));
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.custom_name = name.as_ref().map(Text::plain);
            m.custom_name_text = name;
        } else if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id) {
            a.custom_name = name;
        } else if let Some(f) = self.item_frames.iter_mut().find(|f| f.id == id) {
            f.custom_name = saved.clone();
        }
        self.put_saved(id, "CustomName", saved);
        true
    }
    /// Whether the entity's name shows over it at any distance
    /// (`Entity.isCustomNameVisible`); false for the kinds that keep no name.
    pub(super) fn entity_custom_name_visible(&self, entity: EntityRef) -> Option<bool> {
        let t = self.native_entity_target(entity)?;
        let Some(id) = named_id(t) else {
            return Some(false);
        };
        Some(match t {
            Target::Mob(_) => self.mobs.iter().any(|m| m.id == id && m.name_visible),
            Target::Stand(_) => self
                .armor_stands
                .iter()
                .any(|a| a.id == id && a.name_visible),
            _ => self
                .persist_raw(id)
                .is_some_and(|r| flag(r, "CustomNameVisible")),
        })
    }
    /// Shows the entity's name at any distance, or only when looked at
    /// (`Entity.setCustomNameVisible`): saved, and sent to the players
    /// tracking a mob or an armor stand. False where `set_custom_name` is.
    pub(super) fn entity_set_custom_name_visible(
        &mut self,
        entity: EntityRef,
        visible: bool,
    ) -> bool {
        let Some(id) = self.name_holder(entity) else {
            return false;
        };
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.name_visible = visible;
        } else if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id) {
            a.name_visible = visible;
        }
        // `Entity.saveWithoutId` writes the flag only when set.
        let flag = visible.then_some(Tag::Byte(1));
        self.put_saved(id, "CustomNameVisible", flag);
        true
    }
    /// The category of the entity's type (`EntityType.getCategory`): a mob's
    /// from its type, `Monster` for the dragon, `Misc` for every other kind.
    pub(super) fn entity_classification(&self, entity: EntityRef) -> Option<MobCategory> {
        Some(match self.native_entity_target(entity)? {
            Target::Mob(id) => match self.mobs.iter().find(|m| m.id == id)?.category() {
                "monster" => MobCategory::Monster,
                "creature" => MobCategory::Creature,
                "ambient" => MobCategory::Ambient,
                "axolotls" => MobCategory::Axolotls,
                "underground_water_creature" => MobCategory::UndergroundWaterCreature,
                "water_creature" => MobCategory::WaterCreature,
                "water_ambient" => MobCategory::WaterAmbient,
                _ => MobCategory::Misc,
            },
            Target::Dragon(_) => MobCategory::Monster,
            _ => MobCategory::Misc,
        })
    }
    /// Whether the entity is in its level: from when `entity_join_level` let
    /// it in (false while that hook runs for it and before) until it
    /// leaves or a mod removes it. Players, and the kinds that hook doesn't
    /// tell (displays, markers, interactions, evoker fangs, area effect
    /// clouds, ominous item spawners), are in it while the handle names them.
    /// False for a stale handle or on a client.
    pub(super) fn entity_added_to_level(&self, entity: EntityRef) -> bool {
        match self.native_entity_target(entity) {
            None => false,
            Some(
                Target::Player
                | Target::Guest(_)
                | Target::Other(_)
                | Target::Fangs(_)
                | Target::Cloud(_)
                | Target::Spawner(_),
            ) => true,
            Some(_) => self
                .persist
                .live
                .binary_search_by_key(&entity.id(), |l| l.0)
                .is_ok(),
        }
    }
    /// The id of the entity a custom-name write changes, or None for a kind
    /// that keeps no name, a stale handle or on a client.
    fn name_holder(&self, entity: EntityRef) -> Option<u32> {
        if self.remote.is_some() {
            return None;
        }
        named_id(self.native_entity_target(entity)?)
    }
    /// Sets (or with `None` removes) `key` in entity `id`'s saved compound.
    fn put_saved(&mut self, id: u32, key: &str, value: Option<Tag>) {
        match value {
            Some(v) => {
                let raw = (self.persist.raw)
                    .entry(id)
                    .or_insert_with(|| Tag::Compound(Compound::new()));
                if let Tag::Compound(c) = raw {
                    c.insert(key.into(), v);
                }
            }
            None => {
                if let Some(Tag::Compound(c)) = self.persist.raw.get_mut(&id) {
                    c.remove(&key.into());
                }
            }
        }
    }
}

/// The id of an entity whose custom name the game keeps and saves
/// (`target_name` reads it): the struct's own for mobs, armor stands and
/// item frames, the saved compound's for the rest.
fn named_id(t: Target) -> Option<u32> {
    match t {
        Target::Mob(id)
        | Target::Stand(id)
        | Target::Frame(id)
        | Target::Item(id)
        | Target::Painting(id)
        | Target::Boat(id)
        | Target::Cart(id)
        | Target::Other(id)
        | Target::Fangs(id)
        | Target::Cloud(id) => Some(id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::{entities::ArmorStand, mobs::Mob, text::Text};
    use horizon_core::geometry::Vec3;
    use horizon_core::nbt::Tag;
    use mod_api::{EntityRef, MobCategory};

    fn push_mob(game: &mut super::Game<'_>, id: u32, kind: &str) -> EntityRef {
        let kind = game.res.mob_types().get(kind).unwrap().clone();
        game.mobs
            .push(Mob::new(id, kind, Vec3::new(0.5, 70.0, 0.5), 0.0));
        game.entity(id).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn custom_name_and_visibility_are_saved_and_load_again() {
        with_game(|game| {
            let mob = push_mob(game, 12, "minecraft:zombie");
            assert_eq!(mob.custom_name_visible(game), Some(false));
            let name = Text::colored("Grumbles", 0xFF5555);
            assert!(
                mob.set_custom_name(game, Some(name.try_into_api().unwrap()))
                    .unwrap()
            );
            assert!(mob.set_custom_name_visible(game, true));
            assert_eq!(mob.name(game).as_deref(), Some("Grumbles"));
            assert_eq!(mob.custom_name_visible(game), Some(true));
            let saved = game.entity_nbt(mob).unwrap();
            assert_eq!(saved.get(&"CustomNameVisible".into()), Some(&Tag::Byte(1)));

            // Taken away: neither the name nor the flag is saved.
            assert!(mob.set_custom_name(game, None).unwrap());
            assert!(mob.set_custom_name_visible(game, false));
            assert_ne!(mob.name(game).as_deref(), Some("Grumbles"));
            assert_eq!(mob.custom_name_visible(game), Some(false));
            let cleared = game.entity_nbt(mob).unwrap();
            assert!(!cleared.contains_key(&"CustomName".into()));
            assert!(!cleared.contains_key(&"CustomNameVisible".into()));

            // The saved data loads back with the styled name and the flag.
            let t = game.native_entity_target(mob).unwrap();
            game.apply_entity_nbt(t, Tag::Compound(saved)).unwrap();
            let m = game.mobs.iter().find(|m| m.id == 12).unwrap();
            assert_eq!(m.custom_name_text.as_ref(), Some(&name));
            assert_eq!(mob.name(game).as_deref(), Some("Grumbles"));
            assert_eq!(mob.custom_name_visible(game), Some(true));

            // An armor stand keeps its name in its own field.
            game.armor_stands.push(ArmorStand::new(13, Vec3::ZERO, 0.0));
            let stand = game.entity(13).unwrap();
            assert!(
                stand
                    .set_custom_name(game, Some(mod_api::Text::literal("Sign")))
                    .unwrap()
            );
            assert_eq!(stand.name(game).as_deref(), Some("Sign"));
            let saved = game.entity_nbt(stand).unwrap();
            let loaded = saved
                .get(&"CustomName".into())
                .map(horizon_client::entity_nbt::name_text);
            assert_eq!(loaded, Some(Text::literal("Sign")));
            assert!(stand.set_custom_name(game, None).unwrap());
            let saved = game.entity_nbt(stand).unwrap();
            assert!(!saved.contains_key(&"CustomName".into()));

            // The dragon keeps no name: refused, and a stale handle too.
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 14;
            game.dragon.replace(dragon);
            let dragon = game.entity(14).unwrap();
            assert!(
                !dragon
                    .set_custom_name(game, Some(mod_api::Text::literal("Puff")))
                    .unwrap()
            );
            assert!(!dragon.set_custom_name_visible(game, true));
            assert!(mob.discard(game));
            assert!(
                !mob.set_custom_name(game, Some(mod_api::Text::literal("Gone")))
                    .unwrap()
            );
            assert_eq!(mob.custom_name_visible(game), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn classification_is_the_types_category() {
        with_game(|game| {
            for (id, kind, category) in [
                (20, "minecraft:zombie", MobCategory::Monster),
                (21, "minecraft:cow", MobCategory::Creature),
                (22, "minecraft:bat", MobCategory::Ambient),
                (23, "minecraft:axolotl", MobCategory::Axolotls),
                (
                    24,
                    "minecraft:glow_squid",
                    MobCategory::UndergroundWaterCreature,
                ),
                (25, "minecraft:dolphin", MobCategory::WaterCreature),
                (26, "minecraft:cod", MobCategory::WaterAmbient),
            ] {
                let mob = push_mob(game, id, kind);
                assert_eq!(mob.classification(game), Some(category), "{kind}");
            }
            game.armor_stands.push(ArmorStand::new(27, Vec3::ZERO, 0.0));
            let stand = game.entity(27).unwrap();
            assert_eq!(stand.classification(game), Some(MobCategory::Misc));
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 28;
            game.dragon.replace(dragon);
            let dragon = game.entity(28).unwrap();
            assert_eq!(dragon.classification(game), Some(MobCategory::Monster));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_entity_is_in_its_level_from_its_join_until_its_removal() {
        use mod_api::{EntityJoinLevelEvent, Flow, Hooks, Host, ServerHost};
        use std::sync::{Arc, Mutex};

        /// Whether the joining entity was in its level while the hook ran.
        struct Observer(Arc<Mutex<Vec<bool>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_join_level(
                &self,
                host: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                let host: &dyn Host = host;
                let entity = host.entity(event.entity).unwrap();
                self.0.lock().unwrap().push(entity.added_to_level(host));
                Flow::Continue
            }
        }
        let during = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "level-membership-observer",
            Box::new(Observer(during.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.native_joins_skip();
                game.next_entity += 1;
                let id = game.next_entity;
                let cow = push_mob(game, id, "minecraft:cow");
                // In the lists, but not yet let in.
                assert!(!cow.added_to_level(game));
                game.native_joins_flush(false, 0);
                assert_eq!(*during.lock().unwrap(), [false]);
                assert!(cow.added_to_level(game));
                assert!(cow.discard(game));
                assert!(!cow.added_to_level(game));
            });
        });
    }
}
