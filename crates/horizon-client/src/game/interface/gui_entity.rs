//! A living entity drawn in a mod's screen (`DrawContext::entity`): vanilla's
//! `InventoryScreen.renderEntityInInventoryFollowsMouse` and
//! `renderEntityInInventory`. The entity is drawn by the same route the world
//! draws it with (a mod's renderer for its type, else its model, then the
//! mods' layers), then seen as the inventory's player is (`draw_player_preview`):
//! the box's centre, `size` pixels a block, turned, depth-sorted, back faces
//! culled where the model's render type culls them (as in the world) and lit by
//! `Lighting.ENTITY_IN_UI`.
//!
//! Nothing here runs unless a mod calls `DrawContext::entity`. The buffers and
//! the entities made for `GuiEntity::Kind` are kept per thread and reused, so a
//! screen drawing the same entity every frame makes it once and allocates
//! nothing once the buffers have grown.
use super::{Game, commands::Target};
use glam::{DVec3, Mat4, Vec3};
use horizon_client::{hud::HudBatch, mesher::Vertex, mobs::Mob};
use horizon_core::{
    geometry::Vec3 as Position,
    nbt::{Compound, JavaString, Tag},
};
use mod_api::{GuiEntity, GuiEntityError, GuiEntityLook, GuiEntityView};
use std::{cell::RefCell, sync::Arc};

/// How many entities made for `GuiEntity::Kind` are kept: a mob page shows a
/// few at once, and a learner's four models.
const KEPT: usize = 8;

/// An entity made for the screen from a type and its data.
struct Made {
    kind: String,
    data: Compound,
    mob: Mob,
}

/// A face of the entity as seen: its depth, corners, UVs, ABGR colour and its
/// place in the model's order.
type SeenQuad = (f32, [[f32; 3]; 4], [[f32; 2]; 4], u32, usize);

#[derive(Default)]
struct Scratch {
    made: Vec<Made>,
    verts: Vec<Vertex>,
    quads: Vec<SeenQuad>,
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::default();
}

/// Full bright, as `renderEntityInInventory` draws (`LightTexture.FULL_BRIGHT`).
const FULL_BRIGHT: u32 = 240 | 240 << 8;

/// The turn `look` gives an entity standing in the box centred at `centre`:
/// its body yaw, head yaw and head pitch (degrees, as the mob keeps them), and
/// the pitch the whole entity is tilted by (`cameraOrientation`).
fn turn(look: GuiEntityLook, centre: [f32; 2]) -> (f32, f32, f32, f32) {
    match look {
        GuiEntityLook::At([x, y]) => {
            let x_angle = ((centre[0] - x) / 40.0).atan();
            let y_angle = ((centre[1] - y) / 40.0).atan();
            let body = 180.0 + x_angle * 20.0;
            (body, body + x_angle * 20.0, -y_angle * 20.0, y_angle * 20.0)
        }
        GuiEntityLook::Turned { yaw, pitch } => (180.0 + yaw, 180.0 + yaw, pitch, 0.0),
        // A look a later API adds is drawn facing the viewer until the game knows it.
        _ => (180.0, 180.0, 0.0, 0.0),
    }
}

/// `mob` posed still for the screen with the turn `turn` gives it.
fn pose(mob: &mut Mob, (body, head, pitch, _): (f32, f32, f32, f32)) {
    mob.previous = mob.position;
    mob.body_yaw = body;
    mob.previous_body_yaw = body;
    mob.head_yaw = head;
    mob.previous_head_yaw = head;
    mob.head_pitch = pitch;
    mob.previous_head_pitch = pitch;
    // `extractEntityInInventory` divides the size by the entity's scale.
    mob.extra.model_scale = 0.0;
}

impl Game<'_> {
    /// `DrawContext::entity` on a screen's `batch`.
    pub(in crate::game) fn native_draw_entity(
        &self,
        batch: &mut HudBatch,
        entity: &GuiEntity,
        view: GuiEntityView,
    ) -> Result<(), GuiEntityError> {
        // A client's call: a logical server (the integrated server thread too) has no screen.
        if self.is_logical_server() {
            return Err(GuiEntityError::Unavailable);
        }
        if !view.is_drawable() {
            return Err(GuiEntityError::BadView);
        }
        let mut scratch = SCRATCH.with_borrow_mut(std::mem::take);
        let result = self.draw_entity_with(batch, entity, view, &mut scratch);
        SCRATCH.with_borrow_mut(|kept| *kept = scratch);
        result
    }

    fn draw_entity_with(
        &self,
        batch: &mut HudBatch,
        entity: &GuiEntity,
        view: GuiEntityView,
        scratch: &mut Scratch,
    ) -> Result<(), GuiEntityError> {
        let rect = view.rect;
        let centre = [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0];
        // Finite view values can still overflow once combined: refused before any vertex.
        if !centre.iter().all(|v| v.is_finite()) {
            return Err(GuiEntityError::BadView);
        }
        let turned = turn(view.look, centre);
        let mut existing;
        let mob: &mut Mob = match entity {
            GuiEntity::Kind { kind, data } => {
                let at = self.made_entity(&mut scratch.made, kind, data)?;
                &mut scratch.made[at].mob
            }
            GuiEntity::Existing(handle) => {
                existing = self.existing_gui_entity(*handle)?;
                &mut existing
            }
            // An entity a later API adds is one this game can't name.
            _ => return Err(GuiEntityError::UnknownType),
        };
        pose(mob, turned);
        scratch.verts.clear();
        self.gui_entity_geometry(mob, &mut scratch.verts);
        let height = mob.height();
        // PictureInPictureRenderer: the box centre, (size, size, -size); then
        // the translation and Rz(pi) * Rx(the look's pitch).
        let gui = Mat4::from_translation(Vec3::new(centre[0], centre[1], 0.0))
            * Mat4::from_scale(Vec3::new(view.size, view.size, -view.size))
            * Mat4::from_translation(Vec3::new(0.0, height / 2.0 + view.offset_y, 0.0))
            * Mat4::from_rotation_z(std::f32::consts::PI)
            * Mat4::from_rotation_x(turned.3.to_radians());
        if !gui.is_finite() {
            return Err(GuiEntityError::BadView);
        }
        seen_quads(&scratch.verts, gui, &mut scratch.quads);
        let placed = |q: &SeenQuad| q.0.is_finite() && q.1.iter().flatten().all(|v| v.is_finite());
        if !scratch.quads.iter().all(placed) {
            return Err(GuiEntityError::BadView);
        }
        let start = batch.vertices.len();
        for &(_, p, uv, color, _) in &scratch.quads {
            batch.quad_3d(p, uv, color);
        }
        let end = batch.vertices.len();
        batch.depth_glint_since(start, end, rect);
        Ok(())
    }

    /// The index in `made` of the entity of `kind` loaded from `data`, made
    /// now when it isn't kept (or its type was replaced by a resource reload).
    fn made_entity(
        &self,
        made: &mut Vec<Made>,
        kind: &str,
        data: &Compound,
    ) -> Result<usize, GuiEntityError> {
        let types = self.res.mob_types();
        let ty = types.get(kind).ok_or(GuiEntityError::UnknownType)?;
        let same = |m: &Made| m.kind == kind && m.data == *data && Arc::ptr_eq(&m.mob.kind, ty);
        if let Some(at) = made.iter().position(same) {
            return Ok(at);
        }
        let mob = self.new_gui_entity(kind, data)?;
        if made.len() == KEPT {
            made.remove(0);
        }
        made.push(Made {
            kind: kind.into(),
            data: data.clone(),
            mob,
        });
        Ok(made.len() - 1)
    }

    /// `entity` prepared for drawing, as `DrawContext::entity` prepares it (a screen scene's
    /// entities too): a new entity of a type made for drawing only, or a copy of the
    /// client's entity as it is this frame. Refused by the same named causes.
    pub(in crate::game) fn prepare_gui_entity(
        &self,
        entity: &GuiEntity,
    ) -> Result<Mob, GuiEntityError> {
        match entity {
            GuiEntity::Kind { kind, data } => self.new_gui_entity(kind, data),
            GuiEntity::Existing(handle) => self.existing_gui_entity(*handle),
            // An entity a later API adds is one this game can't name.
            _ => Err(GuiEntityError::UnknownType),
        }
    }

    /// One copy of the client's entity `handle`, as vanilla's `createRenderState` makes one a
    /// frame.
    fn existing_gui_entity(&self, handle: mod_api::EntityRef) -> Result<Mob, GuiEntityError> {
        let found = match self.native_entity_read_target(handle) {
            Some(Target::Mob(id)) => self.mobs.iter().find(|m| m.id == id && !m.removed),
            Some(Target::Player) => self.player_model.as_ref(),
            _ => None,
        };
        Ok(found.ok_or(GuiEntityError::NoEntity)?.clone())
    }

    /// A new entity of living type `kind` loaded from `data`, made for drawing only.
    fn new_gui_entity(&self, kind: &str, data: &Compound) -> Result<Mob, GuiEntityError> {
        let types = self.res.mob_types();
        let ty = types.get(kind).ok_or(GuiEntityError::UnknownType)?;
        Ok(if data.is_empty() {
            // Built for drawing only, as a client's copy of a server's mob: no mod part's
            // `register_goals` or behaviour registration runs for a screen's entity.
            Mob::new_remote(u32::MAX, ty.clone(), Position::ZERO, 0.0)
        } else {
            let named = JavaString::from(kind);
            if data
                .get(&JavaString::from("id"))
                .is_some_and(|id| !matches!(id, Tag::String(s) if *s == named))
            {
                return Err(GuiEntityError::BadData);
            }
            let mut tag = data.clone();
            tag.insert("id".into(), Tag::String(named));
            tag.remove(&JavaString::from("Pos"));
            horizon_client::entity_nbt::mob_from_nbt_remote(
                &Tag::Compound(tag),
                types,
                u32::MAX,
                Some(self.pack.assets.registry),
            )
            .filter(|m| Arc::ptr_eq(&m.kind, ty))
            .ok_or(GuiEntityError::BadData)?
        })
    }

    /// `mob`'s geometry around its feet, as the world draws it: a mod's
    /// renderer for its type in place of its model, else the model; then the
    /// mods' layers. Full bright, seen from its own position.
    pub(in crate::game) fn gui_entity_geometry(&self, mob: &Mob, out: &mut Vec<Vertex>) {
        let p = mob.position;
        self.gui_entity_geometry_at(mob, 1.0, DVec3::new(p.x, p.y, p.z), out);
    }

    /// `gui_entity_geometry` at `partial` into the tick, around `camera`: what a screen
    /// scene's entities submit, in the scene's own coordinates.
    pub(in crate::game) fn gui_entity_geometry_at(
        &self,
        mob: &Mob,
        partial: f32,
        camera: DVec3,
        out: &mut Vec<Vertex>,
    ) {
        let added = self.mod_layers(mob);
        let mut parts = None;
        // A mod's renderer for the type draws in place of the model only: the mods' layers
        // still follow, as in the world.
        let modded = self.mod_living_renderer_geometry(mob, partial, camera, FULL_BRIGHT, out);
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        // A screen's entity: custom entity model animations see is_in_gui = 1.
        let worn = self.mob_armor_layers(mob);
        horizon_client::mobs::cem_anim::in_gui(|| {
            if modded {
                return;
            }
            horizon_client::mobs::render::render_with_hidden(
                mob,
                partial,
                camera,
                &lookup,
                FULL_BRIGHT,
                self.res.mob_data(),
                &worn,
                &[],
                Default::default(),
                added.is_some().then_some(&mut parts),
                out,
            );
        });
        self.mod_layer_geometry(mob, None, parts, added, partial, camera, FULL_BRIGHT, out);
    }
}

/// `verts` seen through `gui`, into `quads` far to near: the faces that look
/// away left out where the world's entity shader drops them
/// (`mesher::culls_back`), each lit by `Lighting.ENTITY_IN_UI`
/// (`entity_in_ui_shade`) from the side facing out, or the seen side of a
/// two-sided quad.
fn seen_quads(verts: &[Vertex], gui: Mat4, quads: &mut Vec<SeenQuad>) {
    quads.clear();
    for (order, q) in verts.as_chunks::<4>().0.iter().enumerate() {
        let p: [Vec3; 4] =
            std::array::from_fn(|i| gui.transform_point3(Vec3::from_array(q[i].pos)));
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        // The winding gives inward normals: faces with n.z >= 0 point away.
        let back = n.z >= 0.0;
        if back && horizon_client::mesher::culls_back(q[0].normal) {
            continue;
        }
        // The outward normal; `entity.wgsl` lights the back of a two-sided
        // quad (normal w -127 or -128) by the negated normal, its visible side.
        let outward = -n.normalize_or_zero();
        let two_sided = matches!(q[0].normal[3], -128 | -127);
        let shade = super::perspective::entity_in_ui_shade(if back && two_sided {
            -outward
        } else {
            outward
        });
        let c = q[0].color;
        let ch = |s: u32| ((((c >> s) & 255) as f32 * shade) as u32) << s;
        let color = (c & 0xFF00_0000) | ch(0) | ch(8) | ch(16);
        let depth = p.iter().map(|v| v.z).sum::<f32>() / 4.0;
        // Kept in their order (the HUD pipelines cull neither winding), so
        // the quad splits along the same diagonal from either side.
        quads.push((
            depth,
            p.map(|v| v.to_array()),
            std::array::from_fn(|i| q[i].uv),
            color,
            order,
        ));
    }
    // Translucent faces over what is behind them; faces at one depth (a layer
    // over its model) keep the model's order. Unstable on (depth, order), so no
    // buffer is allocated and the order is still total.
    quads.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.4.cmp(&b.4)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_turns_the_entity_as_follows_mouse_does() {
        // The cursor on the centre: facing the viewer, untilted.
        assert_eq!(
            turn(GuiEntityLook::At([50.0, 60.0]), [50.0, 60.0]),
            (180.0, 180.0, 0.0, 0.0)
        );
        // Left of the centre: atan(40 / 40) = pi/4, body 20 of it, head 40.
        let (body, head, pitch, tilt) = turn(GuiEntityLook::At([10.0, 60.0]), [50.0, 60.0]);
        let quarter = std::f32::consts::FRAC_PI_4;
        assert_eq!(
            (body, head),
            (180.0 + quarter * 20.0, 180.0 + quarter * 40.0)
        );
        assert_eq!((pitch, tilt), (0.0, 0.0));
        // Above: the head pitches up (negative) and the whole entity tilts.
        let (_, _, pitch, tilt) = turn(GuiEntityLook::At([50.0, 20.0]), [50.0, 60.0]);
        assert_eq!((pitch, tilt), (-quarter * 20.0, quarter * 20.0));
        let fixed = turn(
            GuiEntityLook::Turned {
                yaw: 30.0,
                pitch: 10.0,
            },
            [0.0; 2],
        );
        assert_eq!(fixed, (210.0, 210.0, 10.0, 0.0));
    }

    /// A quad of four vertices at depth `z` with `normal`, towards the viewer
    /// or (reversed) away from it, its UVs telling the corners apart.
    fn face(z: f32, normal: [i8; 4], toward: bool) -> [Vertex; 4] {
        let v = |pos: [f32; 3], uv: [f32; 2]| Vertex {
            pos,
            uv,
            layer: 0,
            color: 0xFFFF_FFFF,
            light: 0,
            normal,
            glint_uv: [0.0; 2],
        };
        let mut face = [
            v([0.0, 0.0, z], [0.0, 0.0]),
            v([0.0, 1.0, z], [0.0, 1.0]),
            v([1.0, 1.0, z], [1.0, 1.0]),
            v([1.0, 0.0, z], [1.0, 0.0]),
        ];
        if !toward {
            face.reverse();
        }
        face
    }

    #[test]
    fn faces_looking_away_are_culled_and_the_rest_sorted_far_to_near() {
        let cull = horizon_client::mesher::CULL_BACK;
        let mut verts = Vec::new();
        verts.extend(face(2.0, cull, true));
        verts.extend(face(1.0, cull, true));
        verts.extend(face(0.0, cull, false));
        let mut quads = Vec::new();
        seen_quads(&verts, Mat4::IDENTITY, &mut quads);
        let depths: Vec<f32> = quads.iter().map(|q| q.0).collect();
        assert_eq!(depths.len(), 2, "{depths:?}");
        assert!(depths[0] < depths[1]);
    }

    /// `NoCull` model parts (a mob's legs and clothes drawn as single planes)
    /// draw from behind as the world draws them, and layers at the depth of
    /// their model keep the model's order.
    #[test]
    fn double_sided_faces_are_kept_and_equal_depths_keep_the_model_order() {
        let two_sided = [0, 0, 127, -127];
        // A double-sided face seen from behind, nearest of all.
        let mut verts = Vec::from(face(3.0, two_sided, false));
        // Layers over two parts at depths 1 and 2, in the model's order (the
        // alpha byte): enough faces that a sort without a tie-break reorders them.
        for order in 0..64u32 {
            let mut quad = face(1.0 + (order % 2) as f32, two_sided, true);
            for v in &mut quad {
                v.color = order << 24 | 0x00FF_FFFF;
            }
            verts.extend(quad);
        }
        let mut quads = Vec::new();
        seen_quads(&verts, Mat4::IDENTITY, &mut quads);
        assert_eq!(quads.len(), 65, "the face seen from behind is kept");
        assert_eq!(quads.last().unwrap().0, 3.0);
        let order = |depth: f32| -> Vec<u32> {
            quads.iter().filter(|q| q.0 == depth).map(|q| q.3 >> 24).collect()
        };
        assert_eq!(order(1.0), (0..64u32).step_by(2).collect::<Vec<_>>());
        assert_eq!(order(2.0), (1..64u32).step_by(2).collect::<Vec<_>>());
    }

    /// `entity.wgsl` lights the back of a two-sided quad as its visible side:
    /// a face turned straight away is as bright as one turned straight at the
    /// viewer. A one-sided shaded quad (w 127) keeps its own normal's light.
    #[test]
    fn the_back_of_a_two_sided_face_is_lit_as_its_visible_side() {
        let shade_of = |normal: [i8; 4], toward: bool| {
            let mut quads = Vec::new();
            seen_quads(&face(0.0, normal, toward), Mat4::IDENTITY, &mut quads);
            quads[0].3 & 0xFF
        };
        let front = shade_of([0, 0, 127, -127], true);
        for w in [-127, -128] {
            assert_eq!(shade_of([0, 0, -127, w], false), front, "w {w}");
        }
        assert!(shade_of([0, 0, -127, 127], false) < front);
    }

    #[test]
    fn views_that_cant_be_drawn_are_refused() {
        let look = GuiEntityLook::At([0.0; 2]);
        assert!(GuiEntityView::new([0.0, 0.0, 50.0, 70.0], 30.0, 0.0625, look).is_drawable());
        assert!(!GuiEntityView::new([0.0, 0.0, 0.0, 70.0], 30.0, 0.0, look).is_drawable());
        assert!(!GuiEntityView::new([0.0, 0.0, 50.0, 70.0], 0.0, 0.0, look).is_drawable());
        assert!(!GuiEntityView::new([0.0, 0.0, 50.0, 70.0], 30.0, f32::NAN, look).is_drawable());
        let nan = GuiEntityLook::Turned {
            yaw: f32::INFINITY,
            pitch: 0.0,
        };
        assert!(!GuiEntityView::new([0.0, 0.0, 50.0, 70.0], 30.0, 0.0, nan).is_drawable());
    }

    #[test]
    #[ignore = "daily"]
    fn a_screen_draws_a_mob_by_type_once_and_refuses_what_it_cant_draw() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let look = GuiEntityLook::At([50.0, 40.0]);
            let view = GuiEntityView::new([26.0, 8.0, 75.0, 78.0], 30.0, 0.0625, look);
            let mut batch = HudBatch::scratch([320, 240]);
            let zombie = GuiEntity::kind("minecraft:zombie");
            assert_eq!(game.native_draw_entity(&mut batch, &zombie, view), Ok(()));
            let once = batch.vertices.len();
            assert!(once > 0, "the zombie's model is drawn");
            let mut data = Compound::new();
            data.insert("IsBaby".into(), Tag::Byte(1));
            let baby = GuiEntity::with_data("minecraft:zombie", data.clone());
            assert_eq!(game.native_draw_entity(&mut batch, &baby, view), Ok(()));
            assert!(batch.vertices.len() > once);
            // Drawn again every frame, each is made once and kept.
            for _ in 0..3 {
                game.native_draw_entity(&mut batch, &zombie, view).unwrap();
                game.native_draw_entity(&mut batch, &baby, view).unwrap();
            }
            assert_eq!(SCRATCH.with_borrow(|s| s.made.len()), 2);
            let spin = GuiEntityLook::Turned {
                yaw: 90.0,
                pitch: 0.0,
            };
            let turned = GuiEntityView::new(view.rect, 30.0, 0.0, spin);
            assert_eq!(game.native_draw_entity(&mut batch, &zombie, turned), Ok(()));
            let before = batch.vertices.len();
            let refused = |entity: &GuiEntity, view| {
                let mut batch = HudBatch::scratch([320, 240]);
                let answer = game.native_draw_entity(&mut batch, entity, view);
                assert!(batch.vertices.is_empty(), "{answer:?} drew nothing");
                answer
            };
            for kind in ["minecraft:no_such_mob", "minecraft:arrow"] {
                let answer = refused(&GuiEntity::kind(kind), view);
                assert_eq!(answer, Err(GuiEntityError::UnknownType), "{kind}");
            }
            data.insert("id".into(), Tag::String("minecraft:skeleton".into()));
            let other = GuiEntity::with_data("minecraft:zombie", data);
            assert_eq!(refused(&other, view), Err(GuiEntityError::BadData));
            let stale = mod_api::EntityRef::new(mod_api::WorldEpoch::new(999), 5);
            assert_eq!(
                refused(&GuiEntity::existing(stale), view),
                Err(GuiEntityError::NoEntity)
            );
            let empty = GuiEntityView::new([26.0, 8.0, 26.0, 78.0], 30.0, 0.0, look);
            assert_eq!(refused(&zombie, empty), Err(GuiEntityError::BadView));
            // Finite values whose sums or products overflow are refused too, drawing nothing.
            let huge = [
                GuiEntityView::new([f32::MAX / 2.0, 0.0, f32::MAX, 100.0], 30.0, 0.0, look),
                GuiEntityView::new([0.0, f32::MAX / 2.0, 100.0, f32::MAX], 30.0, 0.0, look),
                GuiEntityView::new([0.0, 0.0, 100.0, 100.0], f32::MAX, 0.0, look),
                GuiEntityView::new([0.0, 0.0, 100.0, 100.0], 30.0, f32::MAX, look),
            ];
            for view in huge {
                assert!(view.is_drawable(), "{:?}", view.rect);
                assert_eq!(
                    refused(&zombie, view),
                    Err(GuiEntityError::BadView),
                    "{:?}",
                    view.rect
                );
            }
            assert_eq!(batch.vertices.len(), before);
        });
    }
}
