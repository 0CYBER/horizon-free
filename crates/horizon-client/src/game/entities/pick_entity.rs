//! Pick Block on an entity (`Minecraft.pickBlockOrEntity`'s entity case and
//! `ServerGamePacketListenerImpl.handlePickItemFromEntity`): the client aims,
//! the server answers with `Entity.getPickResult`.
use super::Game;
use horizon_client::{items::ItemStack, save::uuid_text};
use mod_api::{BlockPos, CrosshairTarget, EntityRef};

#[derive(Clone, Copy, Debug, PartialEq)]
struct CrosshairEntityHit {
    id: i32,
    distance: f64,
    pickable: bool,
    suggest_uuid: bool,
}

fn nearest_entity(
    hits: [Option<CrosshairEntityHit>; 4],
    block_distance: Option<f64>,
) -> Option<CrosshairEntityHit> {
    hits.into_iter().flatten().fold(None, |best, hit| {
        if block_distance.is_none_or(|distance| hit.distance < distance)
            && best.is_none_or(|best| hit.distance < best.distance)
        {
            Some(hit)
        } else {
            best
        }
    })
}

impl Game<'_> {
    fn crosshair_entity_hit(&self) -> Option<CrosshairEntityHit> {
        if let Some(entity) = self.targeting().entity {
            let (_, distance) = self.targeting_entity_hit(entity)?;
            return Some(CrosshairEntityHit {
                id: entity.id() as i32,
                distance,
                pickable: !matches!(
                    self.native_client_entity_target(entity)?,
                    crate::game::commands::Target::Projectile(_)
                        | crate::game::commands::Target::DragonPart(_)
                        | crate::game::commands::Target::Other(_)
                ),
                suggest_uuid: self.level_dragon_part(entity.id()).is_none(),
            });
        }
        let hit = |id: u32, distance, pickable| CrosshairEntityHit {
            id: id as i32,
            distance,
            pickable,
            suggest_uuid: true,
        };
        let hits = [
            self.remote_object_hit(true).map(|object| {
                hit(
                    object.id as u32,
                    object.distance,
                    !self.interactions.contains_key(&object.id),
                )
            }),
            self.mob_hit()
                .map(|(index, distance, _)| hit(self.mobs[index].id, distance, true)),
            self.redirectable_hit().map(|(index, distance)| {
                hit(self.creatures.projectiles[index].id, distance, false)
            }),
            self.dragon_hit().map(|(id, distance)| CrosshairEntityHit {
                id: id as i32,
                distance,
                pickable: false,
                suggest_uuid: false,
            }),
        ];
        if hits.iter().all(Option::is_none) {
            return None;
        }
        nearest_entity(hits, self.block_distance())
    }

    /// The ordinary block-or-entity pick (Minecraft.hitResult), using the
    /// game's targeting ray, reach and occlusion. None without a client world
    /// or on a miss. Entity hits carry this client level's epoch. Unlike
    /// view_hit, this never traces distant terrain.
    pub(super) fn crosshair_target(&self) -> Option<CrosshairTarget> {
        use crate::game::api_values::TryIntoApi;
        if !self.native_window() || self.title.is_some() {
            return None;
        }
        self.player.as_ref()?;
        if let Some(entity) = self.targeting().entity {
            let (point, _) = self.targeting_entity_hit(entity)?;
            return Some(CrosshairTarget::Entity { entity, point });
        }
        if let Some(hit) = self.crosshair_entity_hit() {
            let (eye, direction) = self.aim()?;
            let point = std::array::from_fn(|axis| eye[axis] + direction[axis] * hit.distance);
            return Some(CrosshairTarget::Entity {
                entity: EntityRef::new(self.native_lifecycle.epoch()?, hit.id as u32),
                point,
            });
        }
        let hit = self.pick()?;
        Some(CrosshairTarget::Block {
            pos: BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            state: self
                .world
                .block(hit.pos[0], hit.pos[1], hit.pos[2])
                .try_into_api()
                .ok()?,
            hit: hit.native(),
        })
    }

    /// The entity the crosshair hits before any block, as `Entity.getId`; an
    /// entity with nothing to pick (a projectile, a player, the dragon) is
    /// `Some(None)`. `None` when the view hits a block or nothing.
    pub(super) fn pick_entity_hit_at(&self) -> Option<(f64, Option<i32>)> {
        self.crosshair_entity_hit()
            .map(|hit| (hit.distance, hit.pickable.then_some(hit.id)))
    }
    pub(super) fn pick_entity_hit(&self) -> Option<Option<i32>> {
        self.pick_entity_hit_at().map(|hit| hit.1)
    }

    /// `ClientSuggestionProvider.getSelectedEntities`: the UUID
    /// (`Entity.getStringUUID`) of the entity the crosshair hits before any
    /// block (`Minecraft.hitResult` an `EntityHitResult`).
    pub(super) fn crosshair_entity_uuid(&self) -> Option<String> {
        self.crosshair_entity_hit()
            .filter(|hit| hit.suggest_uuid)
            .and_then(|hit| self.persist.uuids.get(&(hit.id as u32)).copied())
            .map(uuid_text)
    }

    /// `Entity.getPickResult` of the entity `id`: a mob's spawn egg, the item
    /// of a boat, minecart, armor stand, cushion, painting, item frame (the
    /// framed item when it holds one) or end crystal.
    pub(super) fn entity_pick_result(&self, id: i32) -> Option<ItemStack> {
        let id = u32::try_from(id).ok()?;
        let item = if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            // Mob.getPickResult: SpawnEggItem.byId.
            let egg = format!("{}_spawn_egg", m.kind.id);
            return self.items().get(&egg).map(|_| ItemStack::new(&egg, 1));
        } else if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            b.entity_type()
        } else if let Some(c) = self.carts.iter().find(|c| c.id == id) {
            c.kind.item().to_owned()
        } else if self.armor_stands.iter().any(|a| a.id == id) {
            "minecraft:armor_stand".to_owned()
        } else if let Some(c) = self.cushions.iter().find(|c| c.id == id) {
            c.item()
        } else if let Some(f) = self.item_frames.iter().find(|f| f.id == id) {
            // ItemFrame.getPickResult: what it frames, else the frame itself.
            return Some(
                f.item
                    .clone()
                    .unwrap_or_else(|| ItemStack::new(f.kind(), 1)),
            );
        } else if self.paintings.iter().any(|p| p.id == id) {
            "minecraft:painting".to_owned()
        } else if self.crystals.iter().any(|c| c.id == id) {
            "minecraft:end_crystal".to_owned()
        } else {
            return None;
        };
        Some(ItemStack::new(&item, 1))
    }

    /// `Player.isWithinEntityInteractionRange(entity, 3.0)`: the entity's box
    /// within the interaction range plus 3 of the eyes.
    pub(super) fn within_entity_range(&self, id: i32) -> bool {
        let Some(eye) = self.eye_position() else {
            return false;
        };
        let mob = u32::try_from(id)
            .ok()
            .and_then(|id| self.mobs.iter().find(|m| m.id == id))
            .map(|m| m.bounds());
        let Some(b) = self
            .object_target(id)
            .map(|(lo, hi, _)| (lo, hi))
            .or_else(|| {
                mob.or_else(|| self.tracked_special_box(id)).map(|b| {
                    let (lo, hi) = (b.min_position(), b.max_position());
                    ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z])
                })
            })
        else {
            return false;
        };
        let d2: f64 = (0..3)
            .map(|a| (eye[a] - eye[a].clamp(b.0[a], b.1[a])).powi(2))
            .sum();
        let range = self.entity_reach + 3.0;
        d2 < range * range
    }
}

#[cfg(test)]
mod tests {
    use super::{CrosshairEntityHit, nearest_entity};

    #[test]
    fn nearest_crosshair_entity_preserves_occlusion_ties_and_pick_metadata() {
        let object = CrosshairEntityHit {
            id: 17,
            distance: 2.0,
            pickable: true,
            suggest_uuid: true,
        };
        let projectile = CrosshairEntityHit {
            id: 23,
            distance: 1.0,
            pickable: false,
            suggest_uuid: true,
        };
        let dragon = CrosshairEntityHit {
            id: 29,
            distance: 0.5,
            pickable: false,
            suggest_uuid: false,
        };
        assert_eq!(nearest_entity([None; 4], None), None);
        assert_eq!(
            nearest_entity([Some(object), None, None, None], None),
            Some(object)
        );
        for block_distance in [1.0, 2.0] {
            assert_eq!(
                nearest_entity([Some(object), None, None, None], Some(block_distance)),
                None
            );
        }
        assert_eq!(
            nearest_entity([Some(object), None, Some(projectile), None], Some(3.0)),
            Some(projectile)
        );
        assert_eq!(
            nearest_entity([Some(object), None, Some(projectile), Some(dragon)], None),
            Some(dragon)
        );
        assert_eq!(
            nearest_entity(
                [
                    Some(object),
                    Some(CrosshairEntityHit {
                        distance: 2.0,
                        ..projectile
                    }),
                    None,
                    None
                ],
                None
            ),
            Some(object)
        );
    }
}
