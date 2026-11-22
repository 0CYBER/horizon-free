//! Shears and entity parts (NeoForge's `IShearable`, arismoko/horizon#3351): a player's shears and a dispenser's
//! ask the mob's entity parts first (`EntityPart::is_shearable`), and a part that says yes
//! shears it (`EntityPart::on_sheared`) in place of the species' own shearing.
use super::api_values::{TryIntoApi, TryIntoEngine};
use super::{Game, Hand};
use horizon_client::{
    items::ItemStack,
    mobs::{Species, Who},
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use mod_api::{
    PlayerRef,
    content::crash::{ModCall, enter_native_mod},
};

impl Game<'_> {
    /// `IShearable.isShearable` as live mob `index`'s entity parts answer it: the answering part's
    /// position in dispatch order and its answer, the first part to answer winning; none when no
    /// part answers. A type with no part costs one table lookup. Server-side.
    pub(super) fn part_shearable(
        &self,
        index: usize,
        player: Option<PlayerRef>,
        shears: &ItemStack,
    ) -> Option<(usize, bool)> {
        if !mod_api::__private::has_entity_parts() || !self.mobs[index].alive() {
            return None;
        }
        let parts = mod_api::__private::entity_parts();
        let list = parts.entity_parts(&self.mobs[index].kind.id);
        if list.is_empty() {
            return None;
        }
        let entity = self.entity(self.mobs[index].id)?;
        let shears = shears.try_into_api().ok()?;
        list.iter().enumerate().find_map(|(at, attached)| {
            let _guard = enter_native_mod(ModCall {
                id: attached.mod_id(),
                hook: "is_shearable",
                part: Some(attached.type_name()),
            });
            let answer = attached
                .part()
                .is_shearable(self, entity, player, &shears)?;
            Some((at, answer))
        })
    }

    /// `IShearable.onSheared` on mob `index`'s entity part `part` (as [`Self::part_shearable`]
    /// named it), then `spawnShearedDrop` for each drop it returns: one block above where the mob
    /// stands afterwards, nudged by the mob's own random as `Sheep.shear` nudges its wool.
    pub(super) fn part_shear(
        &mut self,
        index: usize,
        part: usize,
        player: Option<PlayerRef>,
        shears: &ItemStack,
    ) {
        let (id, at) = (self.mobs[index].id, self.mobs[index].position);
        let parts = mod_api::__private::entity_parts();
        let (Some(attached), Some(entity), Ok(shears)) = (
            parts.entity_parts(&self.mobs[index].kind.id).get(part),
            self.entity(id),
            shears.try_into_api(),
        ) else {
            return;
        };
        let drops = {
            let _guard = enter_native_mod(ModCall {
                id: attached.mod_id(),
                hook: "on_sheared",
                part: Some(attached.type_name()),
            });
            attached.part().on_sheared(self, entity, player, &shears)
        };
        // The part may have moved or removed the mob: its drops fall where it is now.
        let index = self.mobs.iter().position(|m| m.id == id && !m.removed);
        let at = index.map_or(at, |i| self.mobs[i].position);
        for drop in drops {
            let Ok(stack) = drop.try_into_engine(self.items()) else {
                continue;
            };
            let Some((id, _)) = self.spawn_at_location(at, 1.0, stack) else {
                continue;
            };
            let Some(i) = index else { continue };
            let r = &mut self.mobs[i].random;
            let push = [
                f64::from((r.next_float() - r.next_float()) * 0.1),
                f64::from(r.next_float() * 0.05),
                f64::from((r.next_float() - r.next_float()) * 0.1),
            ];
            if let Some(item) = self.dropped_item_mut(id) {
                let v = item.velocity;
                item.velocity = Vec3::new(v.x + push[0], v.y + push[1], v.z + push[2]);
            }
        }
    }

    /// Shears in the acting player's hand on mob `index` (`ShearsItem.interactLivingEntity`):
    /// the entity parts' answer, else the species' own (`Sheep`, `MushroomCow`, `SnowGolem`,
    /// `Bogged`), and a shearable mob fires `player_shear_entity`. `Some(consumed)` ends the
    /// interaction: a part's refusal (false), the hook's cancel or a part's shearing (true);
    /// none leaves the shearing to the species' own interaction.
    pub(super) fn player_shear(&mut self, index_slot: &mut usize) -> Option<bool> {
        let mut index = *index_slot;
        let shears = self.held().cloned()?;
        let player = self.native_acting_player();
        let part = self.part_shearable(index, player, &shears);
        if part.is_some_and(|(_, shearable)| !shearable) {
            return Some(false);
        }
        let mob = &self.mobs[index];
        let (id, at) = (mob.id, mob.position);
        let shearable = part.is_some()
            || match mob.species() {
                Species::Sheep => !mob.extra.sheared && !mob.is_baby(),
                Species::Mooshroom => !mob.is_baby(),
                Species::SnowGolem | Species::Bogged => mob.alive() && !mob.extra.sheared,
                _ => false,
            };
        if shearable
            && let (Some(uuid), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone())
        {
            let mut cancel = false;
            let off_hand = self.hand == Hand::Off;
            self.native_player_shear_entity(uuid, &name, id as i32, off_hand, &mut cancel);
            if cancel {
                return Some(true);
            }
            // The hook had `&mut self`: find the mob again by id; a mob a mod
            // removed, or shears it swapped, ends the interaction.
            let Some(now) = self.mobs.iter().position(|m| m.id == id && !m.removed) else {
                return Some(true);
            };
            index = now;
            *index_slot = now;
            let same = self
                .held()
                .is_some_and(|h| h.item == shears.item && h.count == shears.count);
            if !same {
                return Some(true);
            }
        }
        let (part, _) = part?;
        self.part_shear(index, part, player, &shears);
        let ctx = self.who_context(Some(Who::Player), None);
        self.post_game_event("minecraft:shear", [at.x, at.y, at.z], ctx);
        self.damage_held_tool();
        self.swing_arm();
        Some(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::mobs::Mob;

    /// A cow that shears into one leather.
    struct LeatherCow;
    #[mod_api::horizon_part]
    impl mod_api::EntityPart for LeatherCow {
        fn is_shearable(
            &self,
            _: &dyn mod_api::Host,
            _: mod_api::EntityRef,
            player: Option<PlayerRef>,
            shears: &mod_api::ItemStack,
        ) -> Option<bool> {
            assert!(player.is_none(), "a dispenser has no player");
            assert_eq!(shears.count(), 1);
            Some(true)
        }
        fn on_sheared(
            &self,
            _: &mut dyn mod_api::ServerHost,
            _: mod_api::EntityRef,
            _: Option<PlayerRef>,
            _: &mod_api::ItemStack,
        ) -> Vec<mod_api::ItemStack> {
            let leather = mod_api::__private::item_key("minecraft:leather").unwrap();
            vec![mod_api::ItemStack::new(leather, 1).unwrap()]
        }
    }
    /// A sheep that refuses the shears.
    struct WoollySheep;
    #[mod_api::horizon_part]
    impl mod_api::EntityPart for WoollySheep {
        fn is_shearable(
            &self,
            _: &dyn mod_api::Host,
            _: mod_api::EntityRef,
            _: Option<PlayerRef>,
            _: &mod_api::ItemStack,
        ) -> Option<bool> {
            Some(false)
        }
    }
    struct ShearMod;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ShearMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.entity_part("minecraft:cow", LeatherCow);
            r.entity_part("minecraft:sheep", WoollySheep);
        }
    }

    /// A dispenser's shears ask the mob's entity parts: a cow whose part says yes drops what
    /// the part returns, and a woolly sheep whose part says no keeps its wool.
    #[test]
    #[ignore = "daily"]
    fn entity_parts_decide_and_perform_a_dispensers_shearing() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        if run_fresh_cases(
            "game::part_shearing::tests::entity_parts_decide_and_perform_a_dispensers_shearing",
            "HORIZON_PART_SHEARING_CHILD",
            &["valid"],
        ) {
            return;
        }
        let entries = vec![mod_api::ModEntry::new("shearing", Box::new(ShearMod))];
        mod_platform::__private::install(entries.into());
        crate::game::api_values::install_parts(&[mod_api::ModEntry::new(
            "shearing",
            Box::new(ShearMod),
        )])
        .unwrap();
        with_game(|game| {
            game.resolve_parts().unwrap();
            // The mobs below take ids 1 and 2 by hand: reserve them in the game's allocator, or the
            // first drop is given id 1 too.
            game.next_entity = game.next_entity.max(2);
            let mut add = |id: u32, kind: &str, x: i32| {
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let at = Vec3::new(f64::from(x) + 0.5, 64.0, 0.5);
                game.mobs.push(Mob::new(id, kind, at, 0.0));
            };
            add(1, "minecraft:cow", 0);
            add(2, "minecraft:sheep", 4);

            assert!(game.shear_entity((0, 64, 0)).unwrap(), "the cow is sheared");
            let leather: Vec<_> = game
                .dropped
                .iter()
                .filter(|d| &*d.stack.item == "minecraft:leather")
                .map(|d| d.stack.count)
                .collect();
            assert_eq!(leather, [1]);

            assert!(!game.shear_entity((4, 64, 0)).unwrap(), "the sheep refuses");
            assert!(!game.mobs.iter().any(|m| m.id == 2 && m.extra.sheared));
            assert!(!game.dropped.iter().any(|d| d.stack.item.ends_with("_wool")));
        });
    }
}
