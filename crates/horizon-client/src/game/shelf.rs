//! The displayed items of `ShelfBlockEntity`.
use super::Game;
use horizon_assets::item_model::DisplayContext;
use horizon_client::{block_entity_render::vertex_light, containers::BlockEntity, mesher::Vertex};
use horizon_core::position::BlockPos;

impl Game<'_> {
    /// `ShelfRenderer`: three items face out, aligned to the bottom or centre.
    pub(super) fn shelf_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        for (key, entity) in &self.world.block_entities {
            let BlockEntity::Shelf(shelf) = entity else {
                continue;
            };
            let at = glam::DVec3::new(f64::from(key.0), f64::from(key.1), f64::from(key.2));
            if at.distance_squared(camera) >= 64.0 * 64.0 {
                continue;
            }
            let block = self.world.block(key.0, key.1, key.2);
            let facing = self
                .pack
                .assets
                .registry
                .state(block)
                .and_then(|s| s.properties.get("facing"))
                .map(String::as_str)
                .unwrap_or("north");
            let yaw: f32 = match facing {
                "south" => 0.0,
                "west" => -90.0,
                "north" => -180.0,
                "east" => -270.0,
                _ => 180.0,
            };
            let packed = BlockPos::new(key.0, key.1, key.2).pack() as u64;
            let seed = (packed ^ (packed >> 32)) as i32;
            let light = vertex_light(self.world.light(key.0, key.1, key.2));
            for (slot, stack) in shelf.items.iter().enumerate() {
                let Some(stack) = stack else { continue };
                let item = self.resolve_item(
                    stack,
                    DisplayContext::OnShelf,
                    None,
                    seed.wrapping_add(slot as i32),
                );
                if item.is_empty() {
                    continue;
                }
                let (lo, hi) = item.bounding_box();
                let offset_y = -lo.y
                    - if shelf.align_to_bottom {
                        0.0
                    } else {
                        (hi.y - lo.y) / 2.0
                    };
                let pose =
                    glam::Mat4::from_translation((at - camera).as_vec3() + glam::Vec3::splat(0.5))
                        * glam::Mat4::from_rotation_y(yaw.to_radians())
                        * glam::Mat4::from_translation(glam::Vec3::new(
                            (slot as f32 - 1.0) * 0.3125,
                            if shelf.align_to_bottom { -0.25 } else { 0.0 },
                            -0.25,
                        ))
                        * glam::Mat4::from_scale(glam::Vec3::splat(0.25))
                        * glam::Mat4::from_translation(glam::Vec3::new(0.0, offset_y, 0.0));
                self.submit_item_world(&item, pose, light, out);
            }
        }
    }
}
