//! Parrots on a player's shoulder: a tame parrot lands on its owner
//! (`LandOnOwnersShoulderGoal`), is kept as the player's `ShoulderEntityLeft`
//! or `ShoulderEntityRight`, and flies off again when the player is hurt,
//! falls, swims, flies or sleeps (`ServerPlayer.handleShoulderEntities`).
use super::Game;
use horizon_client::mobs::Species;
use horizon_world::nbt::Tag;

const KEYS: [&str; 2] = ["ShoulderEntityLeft", "ShoulderEntityRight"];

/// A shoulder slot holds a parrot when its compound is not empty.
fn occupied(extra: &Tag, key: &str) -> bool {
    matches!(extra.get(key), Some(Tag::Compound(c)) if !c.is_empty())
}

impl Game<'_> {
    /// `ServerPlayer.setEntityOnShoulder` for a parrot that touched its
    /// owner: stored in the first free slot, the parrot gone from the world.
    pub(super) fn parrot_to_shoulder(&mut self, mob: u32) {
        let Some(i) = self
            .mobs
            .iter()
            .position(|m| m.id == mob && m.alive() && !m.removed && m.species() == Species::Parrot)
        else {
            return;
        };
        let Some(p) = self.player.as_ref() else {
            return;
        };
        // The acting player must be the saved owner (`getOwner`).
        if !horizon_client::mobs::tame::owned_by(self.mobs[i].extra.owner, self.acting_uuid()) {
            return;
        }
        if self.riding.is_some()
            || !p.ground.on_ground
            || p.fluids.was_touching_water
            || p.in_powder_snow
            || p.flying
        {
            return;
        }
        let extra = self
            .level
            .player_extra
            .get_or_insert_with(|| Tag::Compound(Default::default()));
        let Some(key) = KEYS.into_iter().find(|k| !occupied(extra, k)) else {
            return;
        };
        let tag = self.mob_tree_nbt(i);
        if let Some(Tag::Compound(map)) = self.level.player_extra.as_mut() {
            map.insert(key.into(), tag);
        }
        let (uuid, now) = (self.acting_uuid(), self.world.game_time);
        self.creatures.shoulder_time.insert(uuid, now);
        self.mobs[i].removed = true;
    }

    /// `ServerPlayer.handleShoulderEntities`: the parrots leave the shoulder
    /// of a player who falls, swims, flies, sleeps, is hurt, dying, a
    /// spectator or spinning with a riptide.
    pub(super) fn handle_shoulder_entities(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let any = self
            .level
            .player_extra
            .as_ref()
            .is_some_and(|e| KEYS.iter().any(|k| occupied(e, k)));
        if any
            && (p.fall_distance > 0.5
                || p.fluids.was_touching_water
                || p.flying
                || p.spectator
                || p.in_powder_snow
                || p.auto_spin_attack_ticks > 0
                || p.health.hurt.hurt_time > 0
                || p.health.dead_or_dying()
                || self.sleeping.is_some())
        {
            self.remove_entities_on_shoulder();
        }
    }

    /// `ServerPlayer.removeEntitiesOnShoulder` (not within a second of the
    /// landing) and `respawnEntityOnShoulder`: the parrots come back at the
    /// player, still owned by them.
    fn remove_entities_on_shoulder(&mut self) {
        let (uuid, now) = (self.acting_uuid(), self.world.game_time);
        if self
            .creatures
            .shoulder_time
            .get(&uuid)
            .is_some_and(|t| t + 20 >= now)
        {
            return;
        }
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let Some(Tag::Compound(map)) = self.level.player_extra.as_mut() else {
            return;
        };
        let tags: Vec<Tag> = KEYS
            .iter()
            .filter_map(|k| map.remove(&(*k).into()))
            .collect();
        let pos = [p.x, p.y + f64::from(0.7_f32), p.z];
        for mut tag in tags {
            let Tag::Compound(c) = &mut tag else {
                continue;
            };
            if c.is_empty() {
                continue;
            }
            c.insert(
                "Pos".into(),
                Tag::List(pos.iter().map(|v| Tag::Double(v.to_bits())).collect()),
            );
            c.insert("Owner".into(), Tag::IntArray(uuid.to_vec()));
            self.load_mob(tag, None);
        }
    }
}
