//! Save visibility while authoritative rows are lent out.
use super::*;

impl Game<'_> {
    /// Spawn the entities of chunks that finished loading.
    pub(in crate::game) fn spawn_loaded_entities(&mut self) {
        let _save_lease = horizon_client::join_seq::visibility::PassLease::enter();
        let loaded = std::mem::take(&mut self.world.loaded_entities);
        for (key, tags) in loaded {
            let count = tags.len();
            for pending in tags {
                let (tag, reason) = pending.into_parts();
                self.spawn_loaded_entity_reason(key, tag, None, reason);
            }
            self.native_entities_load(self.dim.id(), [key.0, key.1], count);
        }
    }

    /// Install one fixture record or explicitly owned live transfer through the same
    /// metadata and join boundary. The transfer token belongs only to this call.
    #[cfg(test)]
    pub(in crate::game) fn spawn_loaded_entity(
        &mut self,
        key: ChunkKey,
        tag: Tag,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
    ) {
        self.spawn_loaded_entity_reason(
            key,
            tag,
            transferred,
            horizon_client::world::EntityLoadReason::Ordinary,
        );
    }

    pub(in crate::game) fn spawn_loaded_entity_reason(
        &mut self,
        key: ChunkKey,
        mut tag: Tag,
        transferred: Option<crate::game::entity_initialization::TransferredEntityState>,
        reason: horizon_client::world::EntityLoadReason,
    ) {
        let player_uuid = self.level.player_uuid;
        // A world older builds named by the offline profile names the player that
        // way in its entities too: migrate them as they load, so the next save of
        // the chunk keeps the account's UUID (`playerdata::migrate_owner`).
        if let Some((old, new)) = self.owner_migration {
            entity_nbt::migrate_player_uuid(
                &mut tag,
                horizon_client::lan::playerdata::uuid_ints(old),
                horizon_client::lan::playerdata::uuid_ints(new),
            );
        }
        // PersistentEntitySectionManager.addEntity's `loaded`.
        let from_disk = !take_fresh(&mut tag);
        // An End gateway's traveller never left the level: nothing joins.
        let moved = take_moved(&mut tag);
        // Whatever was added since the last tick joins before this one.
        self.native_joins_flush(false, 0);
        let kind = entity_nbt::kind(&tag).unwrap_or_default();
        let is_mob = self.res.mob_types().contains_key(kind.as_str());
        if transferred.is_none()
            && from_disk
            && is_mob
            && entity_nbt::position(&tag).is_none()
            && !self.world.chunks.contains_key(&(0, 0))
        {
            // The decoder defaults Pos to zero, but that destination has
            // not been read. Keep the source record rather than admitting
            // an entity no resident chunk can save (or overwriting origin).
            // Admission is irreversible: a duplicate is rejected now,
            // not retried after its current owner has left the level.
            let duplicate = entity_nbt::uuid(&tag).is_some_and(|uuid| {
                self.restored_vehicle(uuid).is_some() || self.target_with_uuid(uuid).is_some()
            });
            if !duplicate {
                self.persist.unknown.entry(key).or_default().push(tag);
            }
            return;
        }
        if !super::super::general::quarantined_kind(&kind) {
            drop_unleashable_leash(&mut tag, &kind, is_mob);
        }
        let uuid = entity_nbt::uuid(&tag);
        // This shortcut deduplicates disk copies, never explicit runtime ownership.
        if transferred.is_none() && uuid.is_some_and(|u| self.restored_vehicle(u).is_some()) {
            return;
        }
        if (matches!(
            kind.as_str(),
            "minecraft:experience_orb"
                | "minecraft:arrow"
                | "minecraft:spectral_arrow"
                | "minecraft:llama_spit"
                | "minecraft:shulker_bullet"
                | "minecraft:experience_bottle"
                | "minecraft:fireball"
                | "minecraft:small_fireball"
                | "minecraft:dragon_fireball"
                | "minecraft:wind_charge"
                | "minecraft:breeze_wind_charge"
                | "minecraft:wither_skull"
                | "minecraft:snowball"
                | "minecraft:egg"
                | "minecraft:ender_pearl"
                | "minecraft:splash_potion"
                | "minecraft:lingering_potion"
                | "minecraft:falling_block"
                | "minecraft:firework_rocket"
                | "minecraft:trident"
                | "minecraft:end_crystal"
                | "minecraft:ender_dragon"
        ) || horizon_client::projectiles::thrown_type(&kind).is_some())
            && let Some(at) = entity_nbt::position(&tag)
        {
            let key = anon_key(&kind, at);
            let was = self.persist.anon.insert(key.clone(), (at, tag.clone()));
            self.journal_anon((key, was));
        }
        // Kinds outside `entity_lists!` first, then the listed kinds
        // (`Entity::load_nbt`), then whatever else is a compound.
        let transferring = transferred.is_some();
        let unloaded = if kind == "minecraft:tnt"
            && !is_mob
            && reason == horizon_client::world::EntityLoadReason::TeleportedTnt
        {
            self.load_tnt_with_portal(tag, uuid, transferred, true);
            Ok(())
        } else if let Some(state) = transferred {
            self.load_listed_transferred(&kind, uuid, player_uuid, tag, state)
        } else {
            self.load_unlisted(key, &kind, uuid, tag)
                .or_else(|tag| self.load_listed(&kind, uuid, player_uuid, tag))
        };
        if let Err(tag) = unloaded {
            if transferring {
                log::warn!("refusing unsupported transferred entity kind {kind}");
                return;
            }
            if !kind.is_empty() && entity_nbt::position(&tag).is_some() {
                // A record no listed family claimed: the missing-mod gate
                // decides whether the unknown-kind path may keep it.
                let at = entity_nbt::position(&tag).unwrap();
                if self.general_quarantine_admits(&tag, &kind, at) {
                    self.load_other(tag, &kind);
                }
            } else {
                self.persist.unknown.entry(key).or_default().push(tag);
            }
        }
        // PersistentEntitySectionManager.addEntity: every kind that
        // just joined tells the mods; a moved one does not join.
        if moved {
            self.native_joins_skip();
        } else {
            self.native_joins_flush(from_disk, 0);
        }
    }
}

impl EntityMemory {
    /// Puts back the raw record `kept` holds, or takes out the one a spawn added where none was:
    /// the abort of a transaction whose spawn wrote over it (`journal_anon`).
    pub(in crate::game) fn put_back_anon(
        &mut self,
        (key, was): ((String, [u64; 3]), Option<(Vec3, Tag)>),
    ) {
        match was {
            Some(record) => self.anon.insert(key, record),
            None => self.anon.remove(&key),
        };
    }
}
