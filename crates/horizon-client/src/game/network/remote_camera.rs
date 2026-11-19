//! A server's spectator camera (`ClientPacketListener.handleSetCamera`,
//! `Minecraft.cameraEntity`): `/spectate` or a spectator's click (sent as
//! `MultiPlayerGameMode.spectate`) attaches the view to another entity. The
//! view follows that entity while the player stands still and sends no moves
//! (`LocalPlayer.isControlledCamera`); the server keeps its player on the
//! camera and lets go when the player sneaks.
#[cfg(test)]
#[path = "tests/remote_camera_tests.rs"]
mod camera_tests;
#[cfg(test)]
#[path = "tests/remote_camera_display_tests.rs"]
mod display_camera_tests;

use super::Game;
use horizon_client::{
    camera::{CameraEntity, LivingCamera},
    entities::{ArmorStand, ItemFrame},
    mobs::Mob,
};

/// `Minecraft.setCameraEntity` from a ClientboundSetCameraPacket: the player's
/// own id puts the view back on the player; an id the level does not hold
/// changes nothing (`ClientboundSetCameraPacket.getEntity` is null).
pub(super) fn camera_after_packet(
    current: Option<i32>,
    player_id: i32,
    camera_id: i32,
    in_level: bool,
) -> Option<i32> {
    if camera_id == player_id {
        None
    } else if in_level {
        Some(camera_id)
    } else {
        current
    }
}

/// `Camera.alignWithEntity`'s view of a mob or another player: its
/// interpolated position, `getViewYRot`/`getViewXRot` and eye height, with
/// its scale and camera distance for a detached view. A living entity's
/// `getViewYRot` is its head's yaw (`yHeadRot`), not its body's `yRot`.
pub(super) fn mob_camera_entity(m: &Mob) -> CameraEntity {
    let attribute =
        |name: &str, default: f64| m.kind.attributes.get(name).copied().unwrap_or(default);
    CameraEntity {
        position: [m.position.x, m.position.y, m.position.z],
        previous: [m.previous.x, m.previous.y, m.previous.z],
        x_rot: m.head_pitch,
        y_rot: m.head_yaw,
        x_rot_o: m.previous_head_pitch,
        y_rot_o: m.previous_head_yaw,
        eye_height: m.eye_height() as f32,
        living: Some(LivingCamera {
            scale: attribute("minecraft:scale", 1.0) as f32,
            camera_distance: attribute("minecraft:camera_distance", 4.0),
            sleeping: None,
            dead: None,
        }),
        mount: None,
    }
}

/// `Camera.alignWithEntity`'s view of an armor stand, the usual camera of a
/// server's cutscene or menu scene. `ArmorStand.setYHeadRot` keeps its head
/// on `yRot`, so its view turns with its yaw; its eyes are its dimensions'
/// (`getDimensionsMarker`): none for a marker, `BABY_DIMENSIONS`' 0.9875 for
/// a small stand, the type's 1.7775 otherwise.
pub(super) fn stand_camera_entity(a: &ArmorStand) -> CameraEntity {
    let eye_height = if a.marker {
        0.0
    } else {
        a.probe_dimensions().2 as f32
    };
    CameraEntity {
        position: [a.position.x, a.position.y, a.position.z],
        previous: [a.previous.x, a.previous.y, a.previous.z],
        x_rot: a.pitch,
        y_rot: a.yaw,
        x_rot_o: a.pitch,
        y_rot_o: a.previous_yaw,
        eye_height,
        living: Some(LivingCamera {
            scale: 1.0,
            camera_distance: 4.0,
            sleeping: None,
            dead: None,
        }),
        mount: None,
    }
}

/// `Camera.alignWithEntity`'s view of a boat, which is not living (no
/// `getScale`, no camera distance): its interpolated position and `Entity`'s
/// view rotation, with `EntityType.BOAT`'s `eyeHeight(0.5625F)`.
pub(super) fn boat_camera_entity(b: &super::boats::Boat) -> CameraEntity {
    CameraEntity {
        position: [b.position.x, b.position.y, b.position.z],
        previous: [b.previous.x, b.previous.y, b.previous.z],
        x_rot: b.pitch,
        y_rot: b.yaw,
        x_rot_o: b.pitch,
        y_rot_o: b.previous_yaw,
        eye_height: 0.5625_f32,
        living: None,
        mount: None,
    }
}
/// `Camera.alignWithEntity`'s view of an item frame: a hanging entity sits at
/// the centre of its box, the block's centre pulled 0.46875 out of the wall
/// (`HangingEntity.recalculateBoundingBox` of `ItemFrame.createBoundingBox`),
/// does not move, and has `EntityType.ITEM_FRAME`'s zero eye height.
/// `ItemFrame.setDirection` turns it: `get2DDataValue() * 90` in yaw (south 0,
/// west 1, north 2, east 3), or `-90 * getAxisDirection().getStep()` in pitch
/// on the floor and ceiling.
pub(super) fn frame_camera_entity(f: &ItemFrame) -> CameraEntity {
    let step = f.step().map(f64::from);
    let at: [f64; 3] = std::array::from_fn(|i| f64::from(f.pos[i]) + 0.5 - step[i] * 0.46875);
    let (y_rot, x_rot) = match f.facing {
        0 => (0.0, 90.0),
        1 => (0.0, -90.0),
        2 => (180.0, 0.0),
        4 => (90.0, 0.0),
        5 => (270.0, 0.0),
        _ => (0.0, 0.0),
    };
    CameraEntity {
        position: at,
        previous: at,
        x_rot,
        y_rot,
        x_rot_o: x_rot,
        y_rot_o: y_rot,
        eye_height: 0.0,
        living: None,
        mount: None,
    }
}
/// `Camera.alignWithEntity`'s view of primed TNT: its interpolated position
/// and `Entity`'s view rotation (priming never changes it), with
/// `EntityType.TNT`'s `eyeHeight(0.15F)`.
pub(super) fn tnt_camera_entity(t: &super::tnt::PrimedTnt) -> CameraEntity {
    CameraEntity {
        position: [t.position.x, t.position.y, t.position.z],
        previous: [t.previous.x, t.previous.y, t.previous.z],
        x_rot: t.rotation[1],
        y_rot: t.rotation[0],
        x_rot_o: t.rotation[1],
        y_rot_o: t.rotation[0],
        eye_height: 0.15,
        living: None,
        mount: None,
    }
}

/// The entity lists a server's camera can name (`Level.getEntity`): the kinds
/// this client can follow.
pub(super) struct CameraEntities<'a> {
    pub mobs: &'a [Mob],
    pub stands: &'a [ArmorStand],
    pub boats: &'a [super::boats::Boat],
    pub frames: &'a [ItemFrame],
    pub tnts: &'a [super::tnt::PrimedTnt],
    /// Text, block and item displays (`Display`), by entity id.
    pub displays: &'a std::collections::HashMap<i32, super::text_displays::RemoteTextDisplay>,
}

impl CameraEntities<'_> {
    /// `ClientboundSetCameraPacket.getEntity`: the level holds the id.
    pub(super) fn holds(&self, id: u32) -> bool {
        self.mobs.iter().any(|m| m.id == id)
            || self.stands.iter().any(|a| a.id == id)
            || self.boats.iter().any(|b| b.id == id)
            || self.frames.iter().any(|f| f.id == id)
            || self.tnts.iter().any(|t| t.id == id)
            || self.displays.contains_key(&(id as i32))
    }
    /// `Camera.alignWithEntity`: the view out of the entity, `None` when the
    /// level does not hold it.
    pub(super) fn view(&self, id: u32) -> Option<CameraEntity> {
        if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            return Some(mob_camera_entity(m));
        }
        if let Some(a) = self.stands.iter().find(|a| a.id == id) {
            return Some(stand_camera_entity(a));
        }
        if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            return Some(boat_camera_entity(b));
        }
        if let Some(f) = self.frames.iter().find(|f| f.id == id) {
            return Some(frame_camera_entity(f));
        }
        if let Some(t) = self.tnts.iter().find(|t| t.id == id) {
            return Some(tnt_camera_entity(t));
        }
        self.displays
            .get(&(id as i32))
            .map(super::text_displays::RemoteTextDisplay::camera_view)
    }
}

/// `GameRenderer.checkEntityPostEffect`: the chain a spectated creeper, spider or
/// enderman puts on the screen, or one a mod registered for its own entity type
/// (`Hooks::register_entity_spectator_shaders`).
pub(super) fn entity_post_effect(kind: &str) -> Option<&'static str> {
    horizon_client_render::spectator_shaders::chain(kind)
}

impl Game<'_> {
    /// `ClientPacketListener.handleSetCamera`.
    pub(in crate::game) fn remote_set_camera(&mut self, camera_id: i32) {
        let id = camera_id as u32;
        let in_level = self.camera_entities().holds(id);
        let player = self
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == camera_id);
        if let Some(r) = self.remote.as_mut() {
            r.camera = camera_after_packet(r.camera, r.entity_id, camera_id, in_level);
        }
        // Minecraft.setCameraEntity, for an entity the level holds.
        if in_level || player {
            self.check_entity_post_effect(true);
        }
    }
    /// `GameRenderer.checkEntityPostEffect` for the camera entity, or for none
    /// (`first_person` false: the third-person view clears it).
    pub(super) fn check_entity_post_effect(&mut self, first_person: bool) {
        self.spectated_post_effect = self
            .camera_mob()
            .filter(|_| first_person)
            .and_then(|m| entity_post_effect(&m.kind.id));
    }
    /// The entity a server attached the view to, when it is not the player.
    pub(in crate::game) fn remote_camera_id(&self) -> Option<u32> {
        self.remote.as_ref()?.camera.map(|id| id as u32)
    }
    /// `LocalPlayer.isControlledCamera`: the view is the player's own.
    pub(super) fn controls_camera(&self) -> bool {
        self.remote_camera_id().is_none()
    }
    /// The mob the view follows, while the level still holds it.
    pub(super) fn camera_mob(&self) -> Option<&Mob> {
        let id = self.remote_camera_id()?;
        self.mobs.iter().find(|m| m.id == id)
    }
    /// The view of the entity a server attached the camera to, while the
    /// level still holds it.
    pub(super) fn spectated_camera_snapshot(&self) -> Option<CameraEntity> {
        let id = self.remote_camera_id()?;
        self.camera_entities().view(id)
    }
    /// The level's entities a server camera may name.
    fn camera_entities(&self) -> CameraEntities<'_> {
        CameraEntities {
            mobs: &self.mobs,
            stands: &self.armor_stands,
            boats: &self.boats,
            frames: &self.item_frames,
            tnts: &self.tnts,
            displays: &self.text_displays,
        }
    }
    /// The entity under a spectator's crosshair (`Minecraft.hitResult` as an
    /// `EntityHitResult`): the nearest mob or object before any block.
    pub(super) fn spectator_click_target(&self) -> Option<i32> {
        let mob = self.mob_hit().map(|(i, d, _)| (self.mobs[i].id as i32, d));
        let object = self.remote_object_hit(false).map(|h| (h.id, h.distance));
        [mob, object]
            .into_iter()
            .flatten()
            .filter(|(_, d)| self.before_block(*d))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }
    /// `LevelRenderer`: the camera entity is not drawn from its own eyes.
    pub(super) fn hides_as_camera(&self, id: u32) -> bool {
        self.camera_type.is_first_person() && self.remote_camera_id() == Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::{camera_after_packet, entity_post_effect, stand_camera_entity};
    use horizon_client::entities::ArmorStand;
    use horizon_core::geometry::Vec3;

    /// A server's camera on an armor stand looks out of the stand's eyes
    /// (EntityType.ARMOR_STAND `eyeHeight(1.7775F)`, `BABY_DIMENSIONS`
    /// 0.9875F for a small one, nothing for a marker), along its yaw.
    #[test]
    fn an_armor_stand_holds_the_camera_at_its_eyes() {
        let mut stand = ArmorStand::new(7, Vec3::new(10.5, 64.0, -3.5), 90.0);
        stand.yaw = 135.0;
        stand.pitch = -20.0;
        let view = stand_camera_entity(&stand);
        assert_eq!(view.position, [10.5, 64.0, -3.5]);
        assert_eq!(view.eye_height, 1.7775f32);
        assert_eq!((view.y_rot_o, view.y_rot), (90.0, 135.0));
        assert_eq!(view.x_rot, -20.0);
        stand.small = true;
        assert_eq!(stand_camera_entity(&stand).eye_height, 0.9875f32);
        stand.marker = true;
        assert_eq!(stand_camera_entity(&stand).eye_height, 0.0);
    }

    /// Only a creeper, spider or enderman has a spectator chain.
    #[test]
    fn spectated_entities_pick_their_chain() {
        assert_eq!(
            entity_post_effect("minecraft:creeper"),
            Some("minecraft:creeper")
        );
        assert_eq!(
            entity_post_effect("minecraft:spider"),
            Some("minecraft:spider")
        );
        assert_eq!(
            entity_post_effect("minecraft:enderman"),
            Some("minecraft:invert")
        );
        assert_eq!(
            entity_post_effect("minecraft:cave_spider"),
            Some("minecraft:spider")
        );
        assert_eq!(entity_post_effect("minecraft:cow"), None);
    }

    /// The camera goes to a known entity, back to the player on its own id,
    /// and stays where it is for an entity the level does not hold.
    #[test]
    fn set_camera_follows_vanilla() {
        assert_eq!(camera_after_packet(None, 5, 9, true), Some(9));
        assert_eq!(camera_after_packet(Some(9), 5, 5, false), None);
        assert_eq!(camera_after_packet(Some(9), 5, 12, false), Some(9));
        assert_eq!(camera_after_packet(None, 5, 12, false), None);
    }
}
