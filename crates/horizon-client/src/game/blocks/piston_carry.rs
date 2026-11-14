//! Pistons carry a mod block entity while its block moves (#3184), when the
//! block's part lets it move (`piston_carries_block_entity`): the game's side of the level's
//! `carry_block_entity` and `land_block_entity`, the carried block entity in
//! its moving piston's saved tag, and a client's copy. `BlockEntities` holds
//! what is carried (`carry`, `land`).
use super::{Game, GameLevel};
use horizon_client::{block_entities::Carried, save::ChunkExtras};
use horizon_core::{geometry::Direction, nbt::Tag, position::BlockPos};
use horizon_mod_runtime::attachments::{ATTACHMENTS_KEY, Attachments, attachment_compound};
use horizon_world::{level::MovingPiston, registry::BlockStateId};

/// The moving piston's saved key for the block entity it carries, with its
/// attachments. Horizon's own, written only while it carries one, so a
/// vanilla moving piston's tag is unchanged; vanilla reads only its own keys.
const CARRIED_KEY: &str = "horizon:carried_block_entity";

impl GameLevel<'_> {
    /// Whether the piston at `piston` may push `push` one step `direction` and
    /// destroy `destroy`: on an authoritative server, what `piston_pre`'s
    /// listeners leave of it.
    pub(super) fn piston_move_allowed(
        &self,
        piston: BlockPos,
        direction: Direction,
        extending: bool,
        push: &[BlockPos],
        destroy: &[BlockPos],
    ) -> bool {
        let mut canceled = self.world.carrier_fences.holds_move(push, destroy);
        if !canceled && self.native_authoritative {
            let step = direction.normal();
            self.native_piston_pre(mod_api::__private::PistonPreCall {
                dimension: self.dimension,
                piston,
                direction: step,
                extending,
                push,
                destroy,
                canceled: &mut canceled,
            });
        }
        !canceled
    }
    /// The piston at `piston` pushed `push` one step `direction` and destroyed
    /// `destroy`: on an authoritative server, `piston_post`'s listeners hear it.
    pub(super) fn piston_moved(
        &self,
        piston: BlockPos,
        direction: Direction,
        extending: bool,
        push: &[BlockPos],
        destroy: &[BlockPos],
    ) {
        if self.native_authoritative {
            let step = direction.normal();
            self.native_piston_post(self.dimension, piston, step, extending, push, destroy);
        }
    }
    /// `piston_carries_block_entity` for the block `state` at `pos`: what
    /// its part answers, or None.
    pub(super) fn piston_carries_block_entity(
        &self,
        pos: BlockPos,
        state: BlockStateId,
    ) -> Option<bool> {
        let mut movable = None;
        self.native_piston_carries_block_entity(self.dimension, pos, state, &mut movable);
        if movable == Some(true) {
            let at = (pos.x, pos.y, pos.z);
            if self.world.block_entities.is_taken(&at) {
                return Some(false);
            }
            // A live owned payload is required. Raw-only/lazy native stores cannot be
            // transferred by BlockEntities::carry and must remain refused, with their data intact.
            if (self.facts.get(state).block_entity
                || self.world.block_entities.stateful_entity(state.0))
                && self.world.block_entities.get(&at).is_none()
            {
                return Some(false);
            }
        }
        movable
    }
    /// Takes the block entity at `from` out of the world for the moving piston
    /// at `to`, with what removing it would take from its position (its custom
    /// name, its loaded tag) but none of its side effects.
    pub(super) fn carry_mod_block_entity(&mut self, from: BlockPos, to: BlockPos) {
        let (from, to) = ((from.x, from.y, from.z), (to.x, to.y, to.z));
        let partner = self
            .registry
            .state(self.world.block(from.0, from.1, from.2))
            .and_then(|state| super::storage::chest_connected_pos(state, [from.0, from.1, from.2]))
            .map(|p| (p[0], p[1], p[2]));
        let mut data = super::piston_carry_state::take_position_data(self.world, from);
        let cached = self.positions.hoppers.remove(&from).map(|h| h.cooldown);
        let is_hopper = horizon_client::block_entity_types::factory_block(
            &self
                .facts
                .get(self.world.block(from.0, from.1, from.2))
                .name,
        ) == "minecraft:hopper";
        let cooldown = if is_hopper {
            cached.or_else(|| {
                data.get(&"Source".into())
                    .and_then(|t| t.get("TransferCooldown"))
                    .and_then(|t| match t {
                        Tag::Int(n) => Some(*n),
                        _ => None,
                    })
            })
        } else {
            None
        };
        if let Some(n) = cooldown {
            data.insert("HopperCooldown".into(), Tag::Int(n));
        }
        self.world.block_entities.carry(from, to);
        if let Some(carried) = self.world.block_entities.carried_mut(&to) {
            carried.position_data = data;
        }
        self.lid_openers.remove(&from);
        self.world.chest_openers.remove(&from);
        self.world.shulkers.remove(&[from.0, from.1, from.2]);
        self.spill
            .piston_positions
            .push(super::piston_carry_state::PositionChange::Carry { from, to, partner });
    }
    /// Installs the block entity the moving piston at `pos` carried there; its
    /// custom name is the block's again, as for one loaded there, and on a
    /// server its synced attachments go whole to the players who see it.
    pub(super) fn land_carried_block_entity(&mut self, pos: BlockPos) -> bool {
        let key = (pos.x, pos.y, pos.z);
        let data = self
            .world
            .block_entities
            .carried(&key)
            .map(|c| c.position_data.clone())
            .unwrap_or_default();
        if !self.world.block_entities.land(key) {
            return false;
        }
        super::piston_carry_state::land_position_data(self.world, key, &data);
        let cooldown = match data.get(&"HopperCooldown".into()) {
            Some(Tag::Int(n)) => Some(*n),
            _ => None,
        };
        self.positions.hoppers.remove(&key);
        if let Some(n) = cooldown {
            self.positions
                .hoppers
                .insert(key, super::machines::HopperState::with_cooldown(n));
        }
        self.spill
            .piston_positions
            .push(super::piston_carry_state::PositionChange::Land { at: key });
        if self.native_authoritative && self.world.block_entities.attachments(key).is_some() {
            self.world.attachment_landings.insert(
                horizon_mod_runtime::attachments::Holder::BlockEntity([pos.x, pos.y, pos.z]),
            );
        }
        true
    }
}

impl Game<'_> {
    /// `PistonMovingBlockEntity.saveAdditional` for a chunk save, with the
    /// block entity the piston carries under [`CARRIED_KEY`].
    pub(super) fn saved_moving_piston(&self, pos: BlockPos, piston: &MovingPiston) -> Tag {
        let mut tag = self.moving_piston_to_nbt(pos, piston);
        if let Some(Carried {
            entity,
            attachments,
            position_data,
        }) = self.world.block_entities.carried(&(pos.x, pos.y, pos.z))
            && let Tag::Compound(map) = &mut tag
        {
            let block = &self.res.facts().get(piston.moved_state).name;
            let mut carried = horizon_client::save::block_entity_encode::encode(
                (pos.x, pos.y, pos.z),
                entity,
                block,
            );
            if let Tag::Compound(fields) = &mut carried {
                fields.insert(
                    "horizon:position_data".into(),
                    Tag::Compound(position_data.clone()),
                );
                match attachments.as_ref().map(Attachments::to_nbt) {
                    Some(data) if !data.is_empty() => {
                        fields.insert(ATTACHMENTS_KEY.into(), Tag::Compound(data));
                    }
                    _ => {
                        fields.remove(&ATTACHMENTS_KEY.into());
                    }
                }
            }
            map.insert(CARRIED_KEY.into(), carried);
        }
        tag
    }
    /// `PistonMovingBlockEntity.loadAdditional`: the moving piston at `pos`
    /// as a new ticker, holding again the block entity its saved tag in
    /// `saved` carries. One that can't be read is logged and left out: the
    /// block lands without it.
    pub(super) fn restore_moving_piston(
        &mut self,
        pos: BlockPos,
        piston: MovingPiston,
        saved: Option<&ChunkExtras>,
    ) {
        self.block_ticks.load_moving_piston(pos, piston);
        let Some(tag) = saved
            .into_iter()
            .flat_map(|s| &s.block_entities)
            .find(|tag| super::persist::block_entity_pos(tag) == [pos.x, pos.y, pos.z])
            .and_then(|tag| tag.get(CARRIED_KEY))
        else {
            return;
        };
        let block = &self.res.facts().get(piston.moved_state).name;
        match horizon_client::save::decode_block_entity_for_block(tag, block) {
            Some((_, entity)) => {
                let attachments = match attachment_compound(tag.get(ATTACHMENTS_KEY)) {
                    Ok(Some(data)) => Some(Attachments::from_nbt(data.clone())),
                    Ok(None) => None,
                    Err(_) => {
                        log::warn!(
                            "Horizon: unreadable attachments on the block entity in the moving piston at {} {} {}; it lands without them",
                            pos.x,
                            pos.y,
                            pos.z
                        );
                        None
                    }
                };
                self.world.block_entities.hold_carried(
                    (pos.x, pos.y, pos.z),
                    Carried {
                        entity,
                        position_data: match tag.get("horizon:position_data") {
                            Some(Tag::Compound(data)) => data.clone(),
                            _ => Default::default(),
                        },
                        attachments,
                    },
                );
            }
            None => log::warn!(
                "Horizon: unreadable block entity in the moving piston at {} {} {}; it lands without it",
                pos.x,
                pos.y,
                pos.z
            ),
        }
    }
    /// A client's moving piston at `at` once the server's block took its
    /// place (`LevelChunk.setBlockState`), with the block entity its copy
    /// carried: the server sends its own (`BlockEntityData`).
    pub(super) fn forget_moving_piston(&mut self, at: BlockPos) {
        self.block_ticks.moving_pistons.retain(|(p, _)| *p != at);
        self.world
            .block_entities
            .discard_carried(&(at.x, at.y, at.z));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::{chunk::Chunk, containers::BlockEntity, items::ItemStack};
    use std::sync::Arc;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_save_mid_move_keeps_the_carried_block_entity_and_a_bad_key_is_left_out() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let moving = registry.default_state("minecraft:moving_piston").unwrap();
            let (from, to) = ((1, 64, 1), (2, 64, 1));
            let at = BlockPos::new(to.0, to.1, to.2);
            let mut chunk = Chunk::empty(0, 0);
            chunk.set(to.0, to.1, to.2, moving.0);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let Tag::Compound(saved_attachments) = Tag::compound([(
                "carry_test",
                Tag::compound([("value", Tag::compound([("value", Tag::Int(7))]))]),
            )]) else {
                unreachable!()
            };
            game.world.block_entities.insert(
                from,
                BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:diamond", 3))]),
            );
            game.world
                .block_entities
                .restore_attachments(from, Attachments::from_nbt(saved_attachments.clone()));
            assert!(game.world.block_entities.carry(from, to));
            let piston = MovingPiston {
                moved_state: chest,
                direction: Direction::East,
                extending: true,
                source: false,
                progress: 0.5,
                progress_o: 0.5,
                last_ticked: 0,
                death_ticks: 0,
                id: 0,
            };

            // Saved with one key more than vanilla's tag, and only while it carries.
            let saved = game.saved_moving_piston(at, &piston);
            let vanilla = game.moving_piston_to_nbt(at, &piston);
            let Tag::Compound(mut without) = saved.clone() else {
                unreachable!()
            };
            assert!(without.remove(&CARRIED_KEY.into()).is_some());
            assert_eq!(Tag::Compound(without), vanilla);

            // Unloaded, then loaded again with its chunk.
            game.world.block_entities.unload(|p| *p == to);
            let mut extras = ChunkExtras::default();
            extras.block_entities.push(saved);
            game.restore_moving_piston(at, piston, Some(&extras));
            assert!(game.world.block_entities.land(to));
            let diamonds = match game.world.block_entities.get(&to) {
                Some(BlockEntity::Chest(items)) => items[0].as_ref().map(|s| s.count),
                _ => None,
            };
            assert_eq!(diamonds, Some(3));
            assert_eq!(
                game.world
                    .block_entities
                    .attachments(to)
                    .map(Attachments::to_nbt),
                Some(saved_attachments)
            );
            assert_eq!(
                game.saved_moving_piston(at, &piston),
                vanilla,
                "nothing carried, nothing added"
            );

            // Attachments that can't be read: the block entity lands without them.
            assert!(game.world.block_entities.carry(to, to));
            let mut saved = game.saved_moving_piston(at, &piston);
            let Some(Tag::Compound(carried)) = saved.get_mut(CARRIED_KEY) else {
                panic!("the carried block entity is saved")
            };
            carried.insert(ATTACHMENTS_KEY.into(), Tag::Int(1));
            let mut extras = ChunkExtras::default();
            extras.block_entities.push(saved);
            game.world.block_entities.unload(|p| *p == to);
            game.restore_moving_piston(at, piston, Some(&extras));
            assert!(game.world.block_entities.land(to));
            assert!(matches!(
                game.world.block_entities.get(&to),
                Some(BlockEntity::Chest(items)) if items[0].as_ref().is_some_and(|s| s.count == 3)
            ));
            assert!(game.world.block_entities.attachments(to).is_none());

            // A key that can't be read: the piston loads, its block lands bare.
            let Tag::Compound(mut bad) = vanilla.clone() else {
                unreachable!()
            };
            bad.insert(CARRIED_KEY.into(), Tag::Int(3));
            let mut extras = ChunkExtras::default();
            extras.block_entities.push(Tag::Compound(bad));
            game.world.block_entities.remove(&to);
            game.restore_moving_piston(at, piston, Some(&extras));
            assert!(game.world.block_entities.carried(&to).is_none());
            assert!(
                game.block_ticks
                    .moving_pistons
                    .iter()
                    .any(|(p, _)| *p == at)
            );
        });
    }
}
