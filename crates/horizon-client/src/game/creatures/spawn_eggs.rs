//! Spawn eggs: a baby from an egg used on its own kind, an egg used on a block or a fluid, and the
//! egg's entity data on the mob it spawned.
use super::*;

#[cfg(test)]
#[path = "spawn_egg_mob_tests.rs"]
mod spawn_egg_mob_tests;

impl Game<'_> {
    /// `Mob.checkAndHandleImportantInteractions` with a spawn egg of mob
    /// `index`'s own kind (`SpawnEggItem.spawnOffspringFromSpawnEgg`): a baby
    /// at the mob's feet, with the variant and colour `getBreedOffspring`
    /// gives it (the parent's, but a mooshroom's colour rolled 1 in 1024), and
    /// one egg used. False (PASS) when the mob has no baby form.
    pub(super) fn spawn_egg_offspring(&mut self, index: usize) -> bool {
        let Some(stack) = self.held().cloned() else {
            return false;
        };
        let parent = &self.mobs[index];
        if !parent.alive()
            || !horizon_client::mobs::finalize::spawn_egg_is_for(&stack.item, &parent.kind.id)
            || !horizon_client::mobs::species::can_be_a_baby(parent.species())
            // WanderingTrader.getBreedOffspring returns null: no offspring, the
            // click carries on to trading.
            || parent.species() == Species::WanderingTrader
        {
            return false;
        }
        let (kind, at) = (
            parent.kind.id.clone(),
            [parent.position.x, parent.position.y, parent.position.z],
        );
        let (species, variant, color) =
            (parent.species(), parent.extra.variant, parent.extra.color);
        // `MushroomCow.getBreedOffspring(level, this)`: the egg's mate is the
        // parent itself, so the colour always rolls `nextInt(1024) == 0` to
        // flip, and every other roll spends the coin flip that can only pick
        // that same colour again. Both come from the parent's own random.
        let variant = if species == Species::Mooshroom {
            let random = &mut self.mobs[index].random;
            if random.next_int_bound(1024).unwrap_or(1) == 0 {
                if variant == 1 { 0 } else { 1 }
            } else {
                let _ = random.next_boolean();
                variant
            }
        } else {
            variant
        };
        let target = self.mob_entity(&self.mobs[index]);
        // SpawnEggItem.spawnOffspringFromSpawnEgg: `type.create(level, SPAWN_ITEM_USE)`
        // creates the baby without finalizeSpawn, and `snapTo(pos, 0, 0)` faces
        // it south without a level-random draw.
        let Some(id) =
            self.spawn_mob_internal(&kind, at, false, false, false, false, SpawnYaw::Fixed(0.0))
        else {
            return false;
        };
        if let Some(child) = self.mobs.iter_mut().find(|m| m.id == id) {
            horizon_client::mobs::finalize::spawn_egg_offspring(child, species, variant, color);
        }
        // SpawnEggItem.spawnOffspringFromSpawnEgg: applyComponentsFromItemStack
        // (the egg's name, custom data and variants), no entity data.
        if let Some(child) = self.mobs.iter_mut().find(|m| m.id == id) {
            horizon_client::entity_nbt::apply_item_components(child, &stack);
        }
        self.apply_spawn_egg_entity_data(id, &stack, false);
        if species == Species::Piglin {
            self.native_mobs_joined(self.mobs.len() - 1, false);
        }
        // `Mob.checkAndHandleImportantInteractions` then calls
        // `onOffspringSpawnedFromEgg`: `Fox`'s makes the baby trust the player
        // who used the egg (a fox egg used on a block trusts nobody).
        if species == Species::Fox {
            let spawner = self.acting_player_uuid();
            if let Some(child) = self.mobs.iter_mut().find(|m| m.id == id) {
                horizon_client::mobs::fox::add_trusted_entity(child, spawner);
            }
        }
        self.trigger_item_entity("minecraft:player_interacted_with_entity", &stack, &target);
        self.consume_held();
        // `Mob.interact`: the egg's `SUCCESS_SERVER` consumes the click, so the
        // interaction posts `ENTITY_INTERACT` with the player, at the mob.
        let ctx = self.who_context(Some(Who::Player), None);
        self.post_game_event("minecraft:entity_interact", at, ctx);
        true
    }
    /// `SpawnEggItem.use`: the SOURCE_ONLY ray must hit a liquid block.
    pub(in crate::game) fn spawn_egg_fluid_hit(&self) -> Option<super::super::Hit> {
        let hit = self.bucket_hit()?;
        matches!(
            &*self.block_name(hit.pos),
            "minecraft:water" | "minecraft:lava"
        )
        .then_some(hit)
    }
    /// Shared peaceful-mode eligibility for spawn eggs.
    pub(in crate::game) fn spawn_egg_allowed(&self, item: &str) -> bool {
        let kind = item.trim_end_matches("_spawn_egg");
        self.ui.difficulty != 0
            || self
                .res
                .mob_types()
                .get(kind)
                .is_none_or(|t| horizon_client::mobs::spawn_rules::allowed_in_peaceful(t.species))
    }
    /// `SpawnEggItem.useOn` for a block, or `use` for a fluid source.
    pub(super) fn use_spawn_egg(&mut self, stack: &ItemStack) -> Result<bool> {
        let fluid = matches!(self.guest_use, Some(super::super::GuestUse::Item(_)));
        let Some(hit) = (if fluid {
            self.spawn_egg_fluid_hit()
        } else {
            self.pick()
        }) else {
            return Ok(false);
        };
        let kind = stack.item.trim_end_matches("_spawn_egg").to_owned();
        // ItemStack.useOn in adventure mode: an egg used on a block without a
        // matching `can_place_on` passes to `use`, whose fluid ray finds none.
        if !fluid && !self.may_use_on(hit.pos) {
            return Ok(false);
        }
        if !self.spawn_egg_allowed(&stack.item) {
            self.use_failed = true;
            return Ok(true);
        }
        if fluid
            && !self.may_build()
            && !self.can_place_on_in_adventure([
                hit.pos[0] - hit.face[0],
                hit.pos[1] - hit.face[1],
                hit.pos[2] - hit.face[2],
            ])
        {
            self.use_failed = true;
            return Ok(true);
        }
        // SpawnEggItem.useOn on a spawner sets its mob (and an op's egg data).
        let (spawner_id, data) = horizon_client::block_entity_item::spawn_egg_on_spawner(
            stack,
            self.acting_player_is_op(),
        );
        if let Some(done) = self.use_spawn_egg_on_spawner(hit.pos, &spawner_id, data.as_ref()) {
            if done {
                self.swing_arm();
                self.award_stat("used", &stack.item, 1);
                self.consume_held();
            }
            return Ok(true);
        }
        // useOn: a clicked block without a collision shape (vines, grass)
        // takes the mob itself, any other the one on the clicked face.
        let p = if fluid || self.collision_empty(hit.pos) {
            hit.pos
        } else {
            [
                hit.pos[0] + hit.face[0],
                hit.pos[1] + hit.face[1],
                hit.pos[2] + hit.face[2],
            ]
        };
        let at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]),
            f64::from(p[2]) + 0.5,
        ];
        // SpawnEggItem.spawnMob: `EntityType.spawn`.
        let Some(id) = self.spawn_mob_spawned(&kind, at, SpawnYaw::Created) else {
            return Ok(false);
        };
        // SpawnEggItem.spawnMob: `EntityType.create` finalizes the mob, then
        // `PostSpawnProcessor` puts the egg's components on it.
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            horizon_client::entity_nbt::apply_item_components(m, stack);
        }
        self.apply_spawn_egg_entity_data(id, stack, true);
        self.post_block_game_event("minecraft:entity_place", p, Some(Who::Player), None);
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        if !self.creative
            && let Some(s) = self.held_mut().as_mut()
        {
            s.count -= 1;
            if s.count == 0 {
                *self.held_mut() = None;
            }
        }
        Ok(true)
    }
    /// `MobBucketItem.spawn`: the bucket's components, then
    /// `EntityType.createDefaultStackConfig`.
    pub(in crate::game) fn apply_bucket_stack_config(&mut self, id: u32, stack: &ItemStack) {
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            horizon_client::entity_nbt::apply_item_components(m, stack);
        }
        self.apply_spawn_egg_entity_data(id, stack, true);
    }
    /// `EntityType.createDefaultStackConfig` on the mob an egg or a bucket
    /// spawned: the item's custom name (the whole text, style included) and
    /// custom data (`Entity.applyImplicitComponent`, saved as `CustomName` and
    /// `data`), then, with `entity_data`, `EntityType.updateCustomEntityTag`:
    /// its `minecraft:entity_data`, once the guards let it through, merged
    /// over the mob's own compound as `TypedEntityData.loadInto` does and
    /// loaded back into the same mob (`/data merge entity`'s loader). An egg's
    /// baby (`spawnOffspringFromSpawnEgg`) takes the components only.
    pub(in crate::game) fn apply_spawn_egg_entity_data(
        &mut self,
        id: u32,
        stack: &ItemStack,
        entity_data: bool,
    ) {
        use horizon_world::nbt::Tag;
        let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
            return;
        };
        let kind = self.mobs[i].kind.id.clone();
        let op = self.acting_player_is_op();
        let name = stack.data().and_then(|d| d.custom_name.clone());
        let custom = stack
            .data()
            .and_then(|d| d.other("minecraft:custom_data"))
            .cloned();
        let data = horizon_client::block_entity_item::spawn_egg_entity_data(stack, &kind, op)
            .filter(|_| entity_data);
        if name.is_none() && custom.is_none() && data.is_none() {
            return;
        }
        let mut own = self.mob_tree_nbt(i);
        if let Tag::Compound(map) = &mut own {
            if let Some(name) = name {
                map.insert("CustomName".into(), name);
            }
            if let Some(custom) = custom {
                map.insert("data".into(), custom);
            }
        }
        let tag = match data {
            Some(data) => horizon_client::data_cmd::merged(&own, data),
            None => own,
        };
        // Nothing to report: the mob's own compound always loads back.
        let _ = self.apply_entity_nbt(super::super::commands::Target::Mob(id), tag);
    }
}
