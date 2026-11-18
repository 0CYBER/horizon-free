//! The game side of piglins (`PiglinAi`): the picking up, admiring and bartering run
//! on the game's `PiglinLevel` (`horizon_gameplay::piglins`); what stays here needs the
//! player, the LAN guests, the scoreboard or the mods' hooks (anger, safe armour).
use super::Game;
use horizon_client::{
    items::{ItemStack, Items},
    mobs::{
        Mob, Species,
        piglin::{BARTERING_ITEM, is_idle},
    },
};
use horizon_core::geometry::Vec3;
use horizon_gameplay::piglin_level::PiglinLevel;

impl Game<'_> {
    /// `ZombifiedPiglin.alertOthers`: keep the source's player binding and
    /// refuse recipients that already have a target or are allied to it.
    pub(in crate::game) fn piglin_alert_target(
        &mut self,
        alert: horizon_client::mobs::undead::PiglinAlert,
    ) {
        use horizon_client::mobs::Who;
        let horizon_client::mobs::undead::PiglinAlert {
            mob,
            source,
            target,
        } = alert;
        let player = self
            .lan
            .as_ref()
            .and_then(|host| host.mob_players.get(&source).copied())
            .or(self.lan_acting);
        let target_team = match target {
            Who::Mob(id) => self.mob_team(id),
            Who::Player => match player {
                Some(conn) => self
                    .command_guest(conn)
                    .and_then(|guest| self.score.board.players_team(&guest.name)),
                None => self.player_team(),
            },
        };
        if target == Who::Mob(mob)
            || horizon_client::scoreboard::teams_allied(self.mob_team(mob), target_team)
            || self
                .mobs
                .iter()
                .find(|other| other.id == mob)
                .is_none_or(|other| other.ai.target.is_some())
        {
            return;
        }
        self.set_event_ai_target(mob, Some(target), player);
    }
    fn item_in_tag(&self, tag: &str, item: &str) -> bool {
        horizon_gameplay::piglins::item_in_tag(self, tag, item)
    }
    pub(in crate::game) fn piglin_loved(&self, item: &str) -> bool {
        horizon_gameplay::piglins::piglin_loved(self, item)
    }
    /// `PiglinAi.isBarterCurrency`: vanilla's answer (`BARTERING_ITEM`), then the
    /// parts of the stack's item (`ItemPart::is_piglin_currency`). An item with no
    /// part for it costs one table lookup.
    pub(in crate::game) fn piglin_currency(&self, stack: &ItemStack) -> bool {
        let mut currency = &*stack.item == BARTERING_ITEM;
        if mod_api::__private::item_listens(mod_api::HookId::IsPiglinCurrency, &stack.item) {
            self.native_is_piglin_currency(stack, &mut currency);
        }
        currency
    }
    /// `ItemStack.has(DataComponents.BLOCKS_ATTACKS)` for an item id.
    fn item_blocks_attacks(&self, item: &str) -> bool {
        horizon_gameplay::piglins::item_blocks_attacks(self, item)
    }
    /// `PiglinAi.isLovedItem` or a preferred weapon (`Piglin.canReplaceCurrentItem`).
    fn piglin_wanted(&self, m: &Mob, item: &str) -> bool {
        horizon_gameplay::piglins::piglin_wanted(self, m, item)
    }
    /// The `SLOT_NAMES` index `getEquipmentSlotForItem` picks: the
    /// `equippable` slot (armour, a shield's off hand), else the main hand.
    fn piglin_slot(&self, item: &str) -> usize {
        horizon_gameplay::piglins::piglin_slot(self, item)
    }
    /// `Piglin.canReplaceCurrentItem`: an empty slot takes anything, a
    /// `PREVENT_ARMOR_CHANGE` piece stays, a wanted item replaces an unwanted
    /// one and never the other way round; when both or neither are wanted,
    /// `Mob.canReplaceCurrentItem` compares armour or weapons.
    fn piglin_can_replace(&self, m: &Mob, stack: &ItemStack, slot: usize) -> bool {
        horizon_gameplay::piglins::piglin_can_replace(self, m, stack, slot)
    }
    /// `SimpleContainer.canAddItem` on the piglin's eight slots.
    fn piglin_can_add(&self, m: &Mob, stack: &ItemStack) -> bool {
        horizon_gameplay::piglins::piglin_can_add(self, m, stack)
    }
    fn piglin_max_stack(&self, item: &str) -> u32 {
        horizon_gameplay::piglins::piglin_max_stack(self, item)
    }
    /// `PiglinAi.wantsToPickup`: babies ignore their own item tag, but can
    /// pick up currency and other loved items just like adults.
    fn piglin_wants(&self, m: &Mob, stack: &ItemStack) -> bool {
        horizon_gameplay::piglins::piglin_wants(self, m, stack)
    }
    /// `Mob.equipItemIfPossible` for a piglin: put it in its slot when that
    /// slot takes it (an armour piece that cannot replace what is worn goes
    /// to an empty main hand), dropping what it replaces by drop chance.
    fn piglin_equip(&mut self, index: usize, stack: &ItemStack) -> bool {
        horizon_gameplay::piglins::piglin_equip(self, index, stack)
    }
    /// `PiglinAi.putInInventory`: what does not fit is thrown at a random
    /// spot nearby.
    fn piglin_stash(&mut self, index: usize, stack: ItemStack) {
        horizon_gameplay::piglins::piglin_stash(self, index, stack);
    }
    /// `PiglinAi.stopHoldingOffHandItem` for something that is not
    /// bartering currency: wear it if it can, else keep it in the inventory.
    fn piglin_keep(&mut self, index: usize, stack: ItemStack) {
        horizon_gameplay::piglins::piglin_keep(self, index, stack);
    }
    /// `BehaviorUtils.throwItem`: toss a stack from a piglin toward `target_pos`.
    fn piglin_throw(&mut self, index: usize, target_pos: [f64; 3], stack: ItemStack) {
        horizon_gameplay::piglins::piglin_throw(self, index, target_pos, stack);
    }
    /// `PiglinAi.isWearingSafeArmor` for the acting player: a stack in an armour
    /// slot (feet, legs, chest, head, `EquipmentSlotGroup.ARMOR`'s order) in
    /// `#minecraft:piglin_safe_armor`, once the parts of its item answered
    /// (`ItemPart::makes_piglins_neutral`). An item with no part for it costs one
    /// table lookup.
    pub(super) fn wearing_gold(&self) -> bool {
        let tag = self
            .res
            .items()
            .item_tags
            .get("minecraft:piglin_safe_armor");
        self.armor.iter().rev().flatten().any(|s| {
            let mut neutral = tag.is_some_and(|t| t.contains(&*s.item));
            if mod_api::__private::item_listens(mod_api::HookId::MakesPiglinsNeutral, &s.item)
                && let Some(epoch) = self.native_lifecycle.epoch()
                && let Some(p) = self.player.as_ref()
            {
                let wearer =
                    mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, p.entity_id as u32));
                self.native_makes_piglins_neutral(s, wearer, &mut neutral);
            }
            neutral
        })
    }
    /// Piglins notice wanted items on the ground within 9 blocks (`PiglinAi.wantsToPickup`:
    /// gold and other loved items, nuggets, food, gear), walk to them and
    /// take them (`GoToWantedItem`, `Piglin.pickUpItem`).
    pub(super) fn piglin_pickups(&mut self) {
        horizon_gameplay::piglins::piglin_pickups(self);
    }
    /// `StartAdmiringItemIfSeen` and the admire activity's stop behaviours, from the
    /// distance squared to `NEAREST_VISIBLE_WANTED_ITEM` and whether that item is loved.
    fn admire_seen_item(&mut self, index: usize, nearest: Option<f64>, loved: bool) {
        horizon_gameplay::piglins::admire_seen_item(self, index, nearest, loved);
    }
    /// `PiglinAi.holdInOffhand` and `admireGoldItem`: the piglin holds `stack`, the one item it
    /// took (`split(1)` from the ground, `consumeAndReturn(1)` from a hand), with its components.
    fn start_admiring(&mut self, index: usize, stack: ItemStack) {
        horizon_gameplay::piglins::start_admiring(self, index, stack);
    }
    /// `PiglinAi.mobInteract`: hand a piglin the held `stack` (`canAdmire`: only
    /// piglin currency is accepted from the hand; other loved items are passed over).
    pub(super) fn give_piglin(&mut self, index: usize, stack: &ItemStack) -> bool {
        horizon_gameplay::piglins::give_piglin(self, index, stack)
    }
    /// `PiglinAi.stopHoldingOffHandItem`, when the admiring ran out or a hit
    /// stopped it (`piglin::stop_admiring`, which bars the barter). An adult
    /// barters currency after admiring it, loses it to a hit, and equips or keeps
    /// anything else. Babies equip the admired item, or hold it in the main hand
    /// after stashing or throwing the old item. An empty off hand (a second stop
    /// of the same hit) does nothing.
    pub(super) fn finish_admiring(&mut self, mob: u32) {
        horizon_gameplay::piglins::finish_admiring(self, mob);
    }
    /// `PiglinAi.throwItems`: barters and a baby's unwanted main-hand item
    /// use the same player-directed throw.
    fn piglin_throw_items(&mut self, i: usize, items: Vec<ItemStack>) {
        horizon_gameplay::piglins::piglin_throw_items(self, i, items);
    }
    /// `PiglinAi.setAngerTargetToNearestTargetablePlayerIfFound`: the nearest
    /// visible targetable player when UNIVERSAL_ANGER is on.
    fn nearest_targetable_player_for_piglin(&self, m: &Mob) -> Option<Option<[i32; 4]>> {
        let range = m.attrs.follow_range;
        let mut best: Option<(Option<[i32; 4]>, f64)> = None;
        if let Some(p) = self.player.as_ref()
            && !self.creative
            && !self.spectator()
            && !p.health.dead_or_dying()
        {
            let eye = [p.position.x, p.position.y + 1.62, p.position.z];
            let d2 = m.distance_sqr(p.position);
            if d2 <= range * range && self.can_see(m.center(), eye) {
                let uuid = self
                    .lan_acting_uuid
                    .map(horizon_client::lan::playerdata::uuid_ints)
                    .or(self.level.player_uuid);
                best = Some((uuid, d2));
            }
        }
        if let Some(h) = self.lan.as_ref() {
            for g in &h.hub.guests {
                if h.hub.here(g) && g.loaded && !g.dead && !g.spectator() && g.game_mode != 1 {
                    let eye = [g.position[0], g.position[1] + 1.62, g.position[2]];
                    let pos = Vec3::new(g.position[0], g.position[1], g.position[2]);
                    let d2 = m.distance_sqr(pos);
                    if d2 <= range * range
                        && self.can_see(m.center(), eye)
                        && best.as_ref().is_none_or(|(_, bd)| d2 < *bd)
                    {
                        let uuid = horizon_client::lan::playerdata::uuid_ints(g.uuid);
                        best = Some((Some(uuid), d2));
                    }
                }
            }
        }
        best.map(|(uuid, _)| uuid)
    }
    /// `PiglinAi.angerNearbyPiglins`.
    pub(super) fn anger_piglins(&mut self, only_if_seen: bool) {
        // Fake players never become a mob's target.
        if self.lan_acting_fake {
            return;
        }
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if self.creative || self.spectator() {
            return;
        }
        let at = p.position;
        let eye = [at.x, at.y + 1.62, at.z];
        let player_uuid = self
            .lan_acting_uuid
            .map(horizon_client::lan::playerdata::uuid_ints)
            .or(self.level.player_uuid);
        let universal = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::UNIVERSAL_ANGER);
        for i in 0..self.mobs.len() {
            // A hook below may have shortened the list.
            if i >= self.mobs.len() {
                break;
            }
            let m = &self.mobs[i];
            if m.species() != Species::Piglin
                || m.is_baby()
                || !m.alive()
                || !is_idle(m)
                || (m.position.x - at.x).abs() > 16.0
                || (m.position.y - at.y).abs() > 16.0
                || (m.position.z - at.z).abs() > 16.0
            {
                continue;
            }
            if only_if_seen && !self.can_see(m.center(), eye) {
                continue;
            }
            let target_uuid = if universal {
                self.nearest_targetable_player_for_piglin(m)
                    .unwrap_or(player_uuid)
            } else {
                player_uuid
            };
            let id = self.mobs[i].id;
            let change = if mod_api::__private::listened(mod_api::HookId::LivingChangeTarget) {
                let original = self
                    .lan_acting_entity
                    .or_else(|| self.player_identity().map(|(_, id)| id))
                    .and_then(|id| self.entity(id))
                    .and_then(|entity| self.living(entity));
                let Some(original) = original else { continue };
                let Some(epoch) = self.native_lifecycle.epoch() else {
                    continue;
                };
                let mob = mod_api::MobRef::new(mod_api::EntityRef::new(epoch, id));
                self.native_mob_target_event(mob, Some(original), true)
                    .and_then(|target| self.native_mob_bind_target(id, target))
                    .map_or(
                        horizon_client::mobs::TargetChange::Keep,
                        horizon_client::mobs::TargetChange::Set,
                    )
            } else {
                horizon_client::mobs::TargetChange::Set(Some(horizon_client::mobs::Who::Player))
            };
            // The hook had `&mut self`: find the piglin again by id.
            let Some(i) = self.mobs.iter().position(|m| m.id == id && !m.removed) else {
                continue;
            };
            let m = &mut self.mobs[i];
            let previous = m.ai.target;
            horizon_client::mobs::piglin::set_guarded_anger_target(m, target_uuid);
            m.ai.target = match change {
                horizon_client::mobs::TargetChange::Keep => previous,
                horizon_client::mobs::TargetChange::Set(target) => target,
            };
        }
    }
    /// A clear line of sight between two points (blocks with collision stop it).
    fn can_see(&self, from: [f64; 3], to: [f64; 3]) -> bool {
        let steps = 32;
        (1..steps).all(|i| {
            let f = f64::from(i) / f64::from(steps);
            let p = [
                from[0] + (to[0] - from[0]) * f,
                from[1] + (to[1] - from[1]) * f,
                from[2] + (to[2] - from[2]) * f,
            ];
            !self
                .res
                .facts()
                .get(self.world.block(
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ))
                .full_collision
        })
    }
    /// Whether breaking or opening this block angers piglins.
    pub(super) fn guarded_by_piglins(&self, name: &str) -> bool {
        self.res
            .items()
            .block_tags
            .get("minecraft:guarded_by_piglins")
            .is_some_and(|t| t.contains(name))
    }
}

impl PiglinLevel for Game<'_> {
    fn res_items(&self) -> &Items {
        self.res.items()
    }
    fn world_items(&self) -> &Items {
        self.items()
    }
    fn item_in_tag(&self, tag: &str, item: &str) -> bool {
        Game::item_in_tag(self, tag, item)
    }
    fn piglin_loved(&self, item: &str) -> bool {
        Game::piglin_loved(self, item)
    }
    fn piglin_currency(&self, stack: &ItemStack) -> bool {
        Game::piglin_currency(self, stack)
    }
    fn item_blocks_attacks(&self, item: &str) -> bool {
        Game::item_blocks_attacks(self, item)
    }
    fn piglin_wanted(&self, m: &Mob, item: &str) -> bool {
        Game::piglin_wanted(self, m, item)
    }
    fn piglin_slot(&self, item: &str) -> usize {
        Game::piglin_slot(self, item)
    }
    fn piglin_can_replace(&self, m: &Mob, stack: &ItemStack, slot: usize) -> bool {
        Game::piglin_can_replace(self, m, stack, slot)
    }
    fn piglin_can_add(&self, m: &Mob, stack: &ItemStack) -> bool {
        Game::piglin_can_add(self, m, stack)
    }
    fn piglin_max_stack(&self, item: &str) -> u32 {
        Game::piglin_max_stack(self, item)
    }
    fn piglin_wants(&self, m: &Mob, stack: &ItemStack) -> bool {
        Game::piglin_wants(self, m, stack)
    }
    fn piglin_equip(&mut self, index: usize, stack: &ItemStack) -> bool {
        Game::piglin_equip(self, index, stack)
    }
    fn piglin_stash(&mut self, index: usize, stack: ItemStack) {
        Game::piglin_stash(self, index, stack);
    }
    fn piglin_keep(&mut self, index: usize, stack: ItemStack) {
        Game::piglin_keep(self, index, stack);
    }
    fn piglin_throw(&mut self, index: usize, target_pos: [f64; 3], stack: ItemStack) {
        Game::piglin_throw(self, index, target_pos, stack);
    }
    fn admire_seen_item(&mut self, index: usize, nearest: Option<f64>, loved: bool) {
        Game::admire_seen_item(self, index, nearest, loved);
    }
    fn start_admiring(&mut self, index: usize, stack: ItemStack) {
        Game::start_admiring(self, index, stack);
    }
    fn piglin_throw_items(&mut self, i: usize, items: Vec<ItemStack>) {
        Game::piglin_throw_items(self, i, items);
    }
    fn piglin_throw_target(&self, at: Vec3) -> Option<[f64; 3]> {
        let host = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !self.spectator())
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        let guests = self.lan.iter().flat_map(|h| {
            h.hub
                .guests
                .iter()
                .filter(|g| h.hub.here(g) && g.loaded && !g.dead && !g.spectator())
                .map(|g| g.position)
        });
        let d = |p: &[f64; 3]| at.distance_squared(Vec3::new(p[0], p[1], p[2]));
        host.into_iter()
            .chain(guests)
            .filter(|p| d(p) < 256.0)
            .min_by(|a, b| d(a).total_cmp(&d(b)))
            .map(|p| [p[0], p[1] + 1.0, p[2]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `PiglinAi.throwItemsTowardPos` swings the off hand once when it has
    /// something to throw, so barters leave the admiring hand (#12512).
    #[test]
    #[ignore = "daily"]
    fn a_throw_swings_the_off_hand_once() {
        let mut resources = None;
        let mut game = crate::game::game_tests::tests::server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        assert!(game.run_command("summon piglin 8.5 80 8.5 {NoAI:1b,PersistenceRequired:1b}"));
        let i = game
            .mobs
            .iter()
            .position(|m| m.species() == Species::Piglin)
            .expect("the summoned piglin");
        // An empty list has nothing to throw and leaves the body alone.
        game.piglin_throw_items(i, Vec::new());
        assert!(!game.mobs[i].ai.swinging);
        game.piglin_throw_items(i, vec![ItemStack::new("minecraft:gold_ingot", 1)]);
        let m = &game.mobs[i];
        assert!(m.ai.swinging, "the throw starts a swing");
        // The off hand's arm: the right one only when the piglin is left-handed.
        assert_eq!(m.extra.swing_right_arm, Some(m.main_arm_left()));
        assert_eq!(m.swing_duration(), 6, "SwingAnimation.DEFAULT's ticks");
    }
}
