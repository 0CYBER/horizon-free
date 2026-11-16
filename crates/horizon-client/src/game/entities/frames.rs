//! Item frames and glow item frames: `ItemFrameItem` placement on any face,
//! `ItemFrame.survives`, `interact` (put an item in, then rotate it),
//! `hurtServer` (take the item out, then break the frame) and
//! `ItemFrameRenderer` (the frame model and the item at a 45 degree step).
use super::{Game, Hit, InteractTarget};
use horizon_assets::{
    bake::bake_model, item_model::OwnerState, model::CuboidModel, resolve::ModelLibrary,
};
use horizon_client::{entities::ItemFrame, items::ItemStack, mesher::Vertex};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
/// A baked frame model: (corners in block units, unit UVs, sprite) per quad.
pub(super) type FrameModel = Vec<([[f32; 3]; 4], [[f32; 2]; 4], String)>;

/// Item-frame variants prepared together for one resource generation.
pub(super) fn bake_frames(
    source: &horizon_assets::resource_source::ResourceSource,
) -> [FrameModel; 4] {
    [
        "item_frame",
        "glow_item_frame",
        "item_frame_map",
        "glow_item_frame_map",
    ]
    .map(|name| bake_frame(source, name))
}

/// Bake `block/<name>` (item_frame / glow_item_frame) from the resources.
pub(super) fn bake_frame(
    root: &horizon_assets::resource_source::ResourceSource,
    name: &str,
) -> FrameModel {
    let mut models = std::collections::BTreeMap::new();
    let mut pending = vec![format!("minecraft:block/{name}")];
    while let Some(model) = pending.pop() {
        let Some((ns, path)) = model.split_once(':') else {
            continue;
        };
        let Ok(text) = root.read_to_string(&format!("assets/{ns}/models/{path}.json")) else {
            continue;
        };
        let Ok(m) = CuboidModel::from_json(&text) else {
            continue;
        };
        if let Some(parent) = &m.parent {
            pending.push(parent.clone());
        }
        models.insert(model, m);
    }
    let Ok(library) = ModelLibrary::new(models) else {
        return Vec::new();
    };
    let Ok(resolved) = library.resolve(&format!("minecraft:block/{name}")) else {
        return Vec::new();
    };
    let Ok(quads) = bake_model(&resolved) else {
        return Vec::new();
    };
    let unit = if quads
        .iter()
        .flat_map(|q| q.positions)
        .any(|p| p.iter().any(|v| *v > 1.5))
    {
        16.0
    } else {
        1.0
    };
    quads
        .into_iter()
        .map(|q| {
            (
                q.positions.map(|p| p.map(|v| v / unit)),
                q.uv,
                q.material
                    .sprite
                    .trim_start_matches("minecraft:")
                    .to_owned(),
            )
        })
        .collect()
}
fn facing_of(face: [i32; 3]) -> u8 {
    match face {
        [0, -1, 0] => 0,
        [0, 1, 0] => 1,
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        _ => 5,
    }
}
fn sound(f: &ItemFrame, what: &str) -> String {
    let prefix = if f.glow {
        "glow_item_frame"
    } else {
        "item_frame"
    };
    format!("minecraft:entity.{prefix}.{what}")
}
fn center(f: &ItemFrame) -> [f64; 3] {
    let (lo, hi) = f.bounds();
    std::array::from_fn(|i| (lo[i] + hi[i]) / 2.0)
}
/// `HangingEntity.spawnAtLocation`: drops sit 0.15 out from the wall.
pub(super) fn hanging_drop_position(center: [f64; 3], step: [i32; 3]) -> [f64; 3] {
    [
        center[0] + f64::from(step[0]) * 0.15,
        center[1],
        center[2] + f64::from(step[2]) * 0.15,
    ]
}
/// A frame's item, waiting for its model: the frames' borrow of `self` ends
/// before `resolve_item` reads the pack.
struct FramedItem {
    stack: ItemStack,
    id: u32,
    matrix: glam::Mat4,
    light: u8,
    cull: bool,
    kind: &'static str,
    owner: OwnerState,
}
/// `ItemFrameRenderer.extractRenderState`'s `ItemOwner` (`updateForNonLiving`
/// with the frame entity): `ItemFrame.position` and
/// `ItemFrame.getVisualRotationYInDegrees`.
fn frame_owner(f: &ItemFrame) -> OwnerState {
    super::item_draw::owner_at(center(f), visual_rotation_y_degrees(f), f.kind())
}
/// `ItemFrame.getVisualRotationYInDegrees`: the frame's own yaw, as an item
/// model's owner reads it (a framed compass's needle turns with it).
fn visual_rotation_y_degrees(f: &ItemFrame) -> f32 {
    // `Direction.get2DDataValue` of a wall frame, and the axis direction's
    // step for the two flat ones.
    let s = f.step();
    let (two_d, correction) = match (s[0], s[1], s[2]) {
        (0, 0, 1) => (0, 0),
        (-1, 0, 0) => (1, 0),
        (0, 0, -1) => (2, 0),
        (1, 0, 0) => (3, 0),
        (0, y, _) => (-1, 90 * y),
        _ => (0, 0),
    };
    crate::camera::wrap_degrees(
        180.0 + (two_d * 90) as f32 + f32::from(f.rotation) * 45.0 + correction as f32,
    )
}
impl Game<'_> {
    /// `ItemFrame.survives` (not fixed): no collision, a solid block behind
    /// (or a diode for wall frames), no overlapping hanging entity.
    fn frame_survives(&self, f: &ItemFrame) -> bool {
        if f.fixed {
            return true;
        }
        let (lo, hi) = f.bounds();
        let bounds = Aabb::new(
            Vec3::new(lo[0], lo[1], lo[2]),
            Vec3::new(hi[0], hi[1], hi[2]),
        );
        let mut scene = horizon_client::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if !scene.unobstructed(bounds).unwrap_or(false) {
            return false;
        }
        let s = f.step();
        let behind = self.res.facts().get(self.world.block(
            f.pos[0] - s[0],
            f.pos[1] - s[1],
            f.pos[2] - s[2],
        ));
        let supported = behind.legacy_solid || (s[1] == 0 && behind.has_class("DiodeBlock"));
        // canCoexist(true): only hanging entities facing the same way block it.
        supported && super::paintings::frame_coexists(f, &self.item_frames, &self.paintings)
    }
    /// Asks the mods whether the acting guest may place a hanging entity
    /// of type `kind` at `pos` (`native_hanging_place`): true when one
    /// cancelled it. A game's own player is not asked.
    pub(super) fn hanging_place_cancelled(&mut self, kind: &str, pos: [i32; 3]) -> bool {
        let (Some(uuid), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) else {
            return false;
        };
        let mut cancel = false;
        self.native_hanging_place(uuid, &name, kind, pos, &mut cancel);
        cancel
    }
    /// Asks the mods whether a hanging entity of type `kind` at `pos` may
    /// break (`native_hanging_break`): true when one cancelled it. A break by
    /// a player is asked only for the acting guest, with them as the player;
    /// a break by anything else is asked with none.
    pub(super) fn hanging_break_cancelled(
        &mut self,
        kind: &str,
        pos: [i32; 3],
        by_player: bool,
    ) -> bool {
        let player = if by_player {
            let Some(uuid) = self.lan_acting_uuid else {
                return false;
            };
            Some(uuid)
        } else {
            None
        };
        let mut cancel = false;
        self.native_hanging_break(player, kind, pos, &mut cancel);
        cancel
    }
    /// `ItemFrameItem.useOn`.
    pub(super) fn place_item_frame(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        let glow = match &*stack.item {
            "minecraft:item_frame" => false,
            "minecraft:glow_item_frame" => true,
            _ => return false,
        };
        let pos: [i32; 3] = std::array::from_fn(|i| hit.pos[i] + hit.face[i]);
        // `ItemFrameItem.mayPlace`: `Level.isInsideBuildHeight(blockPos)`, so a frame is not
        // placed on the top face of the highest block or under the lowest one.
        if !(horizon_client::chunk::MIN_Y
            ..horizon_client::chunk::MIN_Y + horizon_client::chunk::HEIGHT)
            .contains(&pos[1])
        {
            // ItemFrameItem.mayPlace's FAIL.
            self.use_failed = true;
            return true;
        }
        let mut frame = ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: self.next_entity + 1,
            pos,
            facing: facing_of(hit.face),
            glow,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            // `applyComponentsFromItemStack` sets `CUSTOM_NAME`.
            custom_name: stack.data().and_then(|d| d.custom_name.clone()),
            random: horizon_core::random::EntityRandom::own(horizon_client::random_source::entity()),
        };
        // `EntityType.createDefaultStackConfig`: the item's `custom_data`, then its
        // `entity_data` merged over the frame (`TypedEntityData.loadInto`), before
        // `survives` runs on the configured frame.
        let kind = frame.kind();
        let own = horizon_client::entity_nbt::item_frame_to_nbt(&frame, [0; 4], None);
        let configured =
            horizon_client::block_entity_item::stack_configured_entity(stack, kind, &own);
        if let Some(mut loaded) =
            horizon_client::entity_nbt::item_frame_from_nbt(&configured, frame.id)
        {
            loaded.glow = glow;
            frame = loaded;
        }
        // HangingEntity.survives false: CONSUME, which ItemStack.useOn still counts as a use.
        if !self.frame_survives(&frame) {
            self.award_stat("used", &stack.item, 1);
            return true;
        }
        if self.hanging_place_cancelled(kind, pos) {
            return true;
        }
        self.next_entity += 1;
        self.audio
            .play(&sound(&frame, "place"), Some(center(&frame)), 1.0, 1.0);
        // HangingEntityItem.useOn: ENTITY_PLACE at the frame, by the player, before it is
        // added; ItemStack.useOn then counts the use.
        let ctx = self.player_context(None);
        self.post_game_event("minecraft:entity_place", center(&frame), ctx);
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        // The unmodelled rest of the loaded data (`data`, `Tags`...) stays with the frame.
        let mut raw = configured;
        if let horizon_core::nbt::Tag::Compound(map) = &mut raw {
            map.remove(&"UUID".into());
        }
        self.persist.raw.insert(frame.id, raw);
        self.item_frames.push(frame);
        if !self.creative {
            self.consume_held();
        }
        self.award_stat("used", &stack.item, 1);
        true
    }
    /// The frame under the crosshair within reach and its distance.
    pub(super) fn frame_hit(&self) -> Option<(usize, f64)> {
        let (eye, dir) = self.aim()?;
        let reach: f64 = self.entity_reach;
        let mut best: Option<(usize, f64)> = None;
        for (i, f) in self.item_frames.iter().enumerate() {
            let (lo, hi) = f.bounds();
            let (mut t0, mut t1) = (0.0f64, reach);
            let mut hit = true;
            for a in 0..3 {
                if dir[a].abs() < 1e-9 {
                    hit &= eye[a] >= lo[a] && eye[a] <= hi[a];
                    continue;
                }
                let (mut ta, mut tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            if hit && t0 <= t1 && best.is_none_or(|b| t0 < b.1) {
                best = Some((i, t0));
            }
        }
        best
    }
    pub(super) fn block_distance(&self) -> Option<f64> {
        if self.native_window() && self.targeting().hit_through_walls {
            return None;
        }
        self.pick_unfiltered().map(|h| {
            let eye = self.aim().map(|(eye, _)| eye).unwrap_or_default();
            (0..3)
                .map(|a| (h.point[a] - eye[a]).powi(2))
                .sum::<f64>()
                .sqrt()
        })
    }
    /// Nearer than the targeted block?
    pub(super) fn before_block(&self, distance: f64) -> bool {
        self.block_distance().is_none_or(|block| block > distance)
    }
    /// `ItemFrame.interact` for the frame under the crosshair, or the one a
    /// guest's interact packet named.
    pub(super) fn interact_frame(&mut self, target: Option<InteractTarget>) -> bool {
        let Some((i, d)) = (match target {
            Some(t) => Self::named_entity(&self.item_frames, t.id, |f| f.id).map(|i| (i, 0.0)),
            None => self.frame_hit(),
        }) else {
            return false;
        };
        // Only a client's own pick is stopped by a block: the range check passed.
        if (target.is_none() && !self.before_block(d)) || self.item_frames[i].fixed {
            return false;
        }
        let at = center(&self.item_frames[i]);
        let entity_id = self.item_frames[i].id;
        let action;
        if self.item_frames[i].item.is_none() {
            let Some(held) = self.held().cloned() else {
                return false;
            };
            // A map tracking more than 256 counted markers stays in the hand
            // (InteractionResult.FAIL): nothing else is tried, no swing (#11383).
            if let Some(id) = super::maps::map_id(&held)
                && self.map(id).is_some_and(|m| m.tracked_over_limit(256))
            {
                return true;
            }
            self.item_frames[i].set_item(Some(held));
            action = "put";
            self.audio
                .play(&sound(&self.item_frames[i], "add_item"), Some(at), 1.0, 1.0);
        } else {
            let s = sound(&self.item_frames[i], "rotate_item");
            self.audio.play(&s, Some(at), 1.0, 1.0);
            let f = &mut self.item_frames[i];
            f.rotation = (f.rotation + 1) % 8;
            action = "rotate";
        }
        let p = self.item_frames[i].pos;
        self.world.modified.insert((p[0] >> 4, p[2] >> 4));
        self.frame_output_changed(p);
        // ItemFrame.interact: BLOCK_CHANGE by the player after setItem or
        // setRotation, then the held item is consumed.
        let ctx = self.who_context(Some(horizon_client::mobs::Who::Player), None);
        self.post_game_event("minecraft:block_change", at, ctx);
        if action == "put" && !self.creative {
            self.consume_held();
        }
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            let frame = self.item_frames.iter().find(|f| f.id == entity_id);
            let item = frame
                .and_then(|f| f.item.as_ref())
                .map_or_else(String::new, |s| s.item.to_string());
            let rotation = frame.map_or(0, |f| f.rotation);
            self.native_player_item_frame_change(
                player,
                &name,
                entity_id,
                action,
                &item,
                rotation.rem_euclid(8) as u8,
            );
        }
        self.swing_arm();
        true
    }
    /// `ItemFrame.hurtServer` from the player: the item comes out first.
    pub(super) fn attack_frame(&mut self) -> bool {
        let Some((i, d)) = self.frame_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        // BlockAttachedEntity.skipAttackInteraction: `mayInteract` refuses the
        // hit before the frame is asked, fixed or not.
        let at = self.item_frames[i].pos;
        if !self.may_interact_at(BlockPos::new(at[0], at[1], at[2])) {
            return true;
        }
        if self.item_frames[i].fixed {
            // A fixed frame refuses the hit of a player who isn't in creative
            // (canHurtWhenFixed), so skipAttackInteraction is false and
            // Player.attack goes on to onAttack.
            if !self.creative {
                self.attack_ticks = 0;
            }
            return true;
        }
        // `isInvulnerableToBase`: a refused hit goes on to onAttack, as a fixed frame's does.
        if self.native_player_hit_invulnerable(self.item_frames[i].id, self.item_frames[i].kind()) {
            self.attack_ticks = 0;
            return true;
        }
        self.hit_frame(i, true, self.creative);
        true
    }
    /// `ItemFrame.hurtServer` from a projectile (`AbstractArrow.onHitEntity`,
    /// `Snowball`, `ThrownEgg`, `ThrownEnderpearl`, `SmallFireball`...): the
    /// damage source's entity is the projectile's owner. A fixed frame only
    /// gives way to a creative player's shot (`canHurtWhenFixed`), and then
    /// breaks whole (`BlockAttachedEntity.hurtServer`). `creative`: that player
    /// has infinite materials. False when the frame refused the hit.
    pub(super) fn projectile_hurts_frame(
        &mut self,
        id: u32,
        by_player: bool,
        creative: bool,
    ) -> bool {
        let Some(i) = self.item_frames.iter().position(|f| f.id == id) else {
            return false;
        };
        if self.item_frames[i].fixed {
            if !creative {
                return false;
            }
            self.item_frames[i].item = None;
        }
        self.hit_frame(i, by_player, creative);
        true
    }
    /// The non-fixed part of `ItemFrame.hurtServer`: a filled frame gives its
    /// item up (`shouldDamageDropItem`), an empty one breaks
    /// (`BlockAttachedEntity.hurtServer`). `by_player`: the damage source's
    /// entity is the acting player, `creative` when they have infinite
    /// materials (no drops).
    fn hit_frame(&mut self, mut i: usize, by_player: bool, creative: bool) {
        let with_frame = self.item_frames[i].item.is_none();
        let entity_id = self.item_frames[i].id;
        if with_frame {
            let (kind, pos) = (self.item_frames[i].kind(), self.item_frames[i].pos);
            if self.hanging_break_cancelled(kind, pos, by_player) {
                return;
            }
            // The hook had `&mut self`: find the frame again by id.
            let Some(now) = self.item_frames.iter().position(|f| f.id == entity_id) else {
                return;
            };
            i = now;
            if self.item_frames[i].item.is_some() {
                return;
            }
        }
        let old_item = self.item_frames[i]
            .item
            .as_ref()
            .map_or("", |s| &*s.item)
            .to_owned();
        let rotation = self.item_frames[i].rotation;
        let at = center(&self.item_frames[i]);
        let mut f = if with_frame {
            self.item_frames.remove(i)
        } else {
            self.item_frames[i].clone()
        };
        if !with_frame {
            self.item_frames[i].item = None;
        } else {
            // BlockAttachedEntity.kill precedes ItemFrame.dropItem's break sound.
            self.entity_killed_event(at, true);
            self.audio.play(&sound(&f, "break"), Some(at), 1.0, 1.0);
        }
        // ItemFrame.dropItem clears the item (updating comparators) before drops.
        if f.item.is_some() {
            self.frame_output_changed(f.pos);
        }
        self.drop_frame_items(&mut f, with_frame, by_player, creative);
        if !with_frame
            && let Some(now) = self.item_frames.iter().position(|g| g.id == entity_id)
        {
            self.item_frames[now].random = f.random.clone();
        }
        // BLOCK_CHANGE by the attacker after the drops: ItemFrame.hurtServer
        // for the item, ItemFrame.dropItem(level, causedBy) for the frame.
        let who = by_player.then_some(horizon_client::mobs::Who::Player);
        let ctx = self.who_context(who, None);
        self.post_game_event("minecraft:block_change", at, ctx);
        if !with_frame {
            // ItemFrame.hurtServer plays this after the event.
            self.audio
                .play(&sound(&f, "remove_item"), Some(at), 1.0, 1.0);
        }
        self.world.modified.insert((f.pos[0] >> 4, f.pos[2] >> 4));
        if by_player
            && let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone())
        {
            self.native_player_item_frame_change(
                player,
                &name,
                entity_id,
                if with_frame { "break" } else { "remove_item" },
                if with_frame { "" } else { &old_item },
                rotation.rem_euclid(8) as u8,
            );
        }
    }
    /// `ItemFrame.setItem(stack, true)`: keeps one item, plays the add sound
    /// for a non-empty stack and updates the comparators.
    pub(super) fn set_frame_item(&mut self, i: usize, stack: Option<ItemStack>) {
        let f = &mut self.item_frames[i];
        f.set_item(stack);
        let (pos, filled) = (f.pos, f.item.is_some());
        if filled {
            let (name, at) = (
                sound(&self.item_frames[i], "add_item"),
                center(&self.item_frames[i]),
            );
            self.audio.play(&name, Some(at), 1.0, 1.0);
        }
        self.frame_output_changed(pos);
    }
    /// `ItemFrame.setItem` / `setRotation`: `updateNeighbourForOutputSignal`, so
    /// the comparators reading the frame take its new signal.
    pub(in crate::game) fn frame_output_changed(&mut self, pos: [i32; 3]) {
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.update_neighbour_for_output_signal(level, ticks, at));
    }
    /// `ItemFrame.dropItem`: the frame (when breaking) and the held item.
    fn drop_frame_items(
        &mut self,
        f: &mut ItemFrame,
        with_frame: bool,
        by_player: bool,
        creative: bool,
    ) {
        // The framed map forgets the frame, except when the drops are off and
        // something caused the break (`removeFramedMap` runs only for a null
        // `causedBy` then).
        if self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::ENTITY_DROPS)
            || !by_player
        {
            self.remove_framed_map(f);
        }
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::ENTITY_DROPS)
            || (by_player && creative)
        {
            return;
        }
        let c = center(f);
        let at = hanging_drop_position(c, f.step());
        let mut drops = Vec::new();
        if with_frame {
            // `ItemFrame.getFrameItemStackWithData` (glow frames share it).
            drops.push(f.item_stack_with_data());
        }
        if let Some(item) = f.item.clone()
            && f.random.next_float() < f.drop_chance
        {
            drops.push(item);
        }
        if by_player && !drops.is_empty() {
            let tool = self.held().cloned();
            let player = Some(self.native_acting_id());
            self.native_use_drops(
                "",
                c,
                None,
                Some((f.id, f.kind())),
                player,
                tool.as_ref(),
                &mut drops,
            );
        }
        for stack in drops {
            self.spawn_item(stack, at, [0.0; 3], 10);
        }
    }
    /// `ServerExplosion.hurtEntities` for item frames: a frame within the blast
    /// is killed and dropped with its item (`BlockAttachedEntity.hurtServer`).
    /// A fixed frame is not hurt by an explosion, and `blast_spares_decorations`
    /// keeps every frame without `mobGriefing`.
    pub(super) fn explosion_breaks_frames(
        &mut self,
        blast: [f64; 3],
        diameter: f64,
        interaction: super::explosion::BlockInteraction,
    ) {
        if self.blast_spares_decorations(interaction) {
            return;
        }
        let mut i = 0;
        while i < self.item_frames.len() {
            let c = center(&self.item_frames[i]);
            let dist =
                ((c[0] - blast[0]).powi(2) + (c[1] - blast[1]).powi(2) + (c[2] - blast[2]).powi(2))
                    .sqrt()
                    / diameter;
            if dist > 1.0
                || self.item_frames[i].fixed
                || !self.combat.visits(self.item_frames[i].id)
                || !self.break_frame_by(i, self.explosion_kill_credit())
            {
                i += 1;
            }
        }
    }
    /// `BlockAttachedEntity.kill`, then `ItemFrame.dropItem(level, causedBy)`
    /// for the frame at `index`, the kill credited to `source`: the break
    /// sound, the frame and its item, then `BLOCK_CHANGE`. False when a mod
    /// kept it (`native_hanging_break`): it stays at `index`.
    pub(super) fn break_frame_by(
        &mut self,
        index: usize,
        source: Option<super::commands::Target>,
    ) -> bool {
        let frame_id = self.item_frames[index].id;
        let (kind, pos) = (self.item_frames[index].kind(), self.item_frames[index].pos);
        if self.hanging_break_cancelled(kind, pos, false) {
            return false;
        }
        // The hook had `&mut self`: find the frame again by id.
        let Some(index) = self.item_frames.iter().position(|g| g.id == frame_id) else {
            return false;
        };
        let mut f = self.item_frames.remove(index);
        self.entity_killed_event_by(center(&f), source);
        self.audio
            .play(&sound(&f, "break"), Some(center(&f)), 1.0, 1.0);
        self.drop_frame_items(&mut f, true, false, false);
        // ItemFrame.dropItem(level, causedBy): BLOCK_CHANGE after the drops.
        let ctx = self.who_context(None, None);
        self.post_game_event("minecraft:block_change", center(&f), ctx);
        self.world.modified.insert((f.pos[0] >> 4, f.pos[2] >> 4));
        if f.item.is_some() {
            self.frame_output_changed(f.pos);
        }
        true
    }
    /// BlockAttachedEntity.tick for frames.
    pub(super) fn tick_item_frames(&mut self) {
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut i = 0;
        while i < self.item_frames.len() {
            let at = center(&self.item_frames[i]);
            if !ticking(at[0].floor() as i32, at[2].floor() as i32) {
                i += 1;
                continue;
            }
            self.item_frames[i].check_interval += 1;
            if self.item_frames[i].check_interval >= 100 {
                self.item_frames[i].check_interval = 0;
                let mut f = self.item_frames[i].clone();
                if !self.frame_survives(&f) && !self.hanging_break_cancelled(f.kind(), f.pos, false)
                {
                    self.item_frames.remove(i);
                    // BlockAttachedEntity.tick discards without a death event.
                    self.audio
                        .play(&sound(&f, "break"), Some(center(&f)), 1.0, 1.0);
                    if f.item.is_some() {
                        self.frame_output_changed(f.pos);
                    }
                    self.drop_frame_items(&mut f, true, false, false);
                    // ItemFrame.dropItem(level, null): sound, drops, then event.
                    let ctx = self.who_context(None, None);
                    self.post_game_event("minecraft:block_change", center(&f), ctx);
                    continue;
                }
            }
            i += 1;
        }
    }
    /// ItemFrameRenderer: the frame model, then the item half size on its face.
    pub(super) fn frame_geometry(&mut self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        let sprites = &self.gui.atlas.sprites;
        // Each with its frame's type: a pack tells a glow frame and what it holds apart by its
        // `entityId` (`glow_item_frame` in entity.properties), the item drawn within its frame.
        let mut maps: Vec<(i32, glam::Mat4, u32, bool, &'static str)> = Vec::new();
        let mut items: Vec<FramedItem> = Vec::new();
        // ItemFrame.shouldRenderAtSqrDistance: 1024 blocks at a view scale
        // of 1.
        let range = 16.0 * 64.0 * self.view_scale();
        for f in &self.item_frames {
            // LevelExtractor: the camera entity is not drawn from its own eyes.
            if self.hides_as_camera(f.id) {
                continue;
            }
            let entity_start = out.len();
            self.mark_entity(entity_start, f.kind());
            let map = f.item.as_ref().and_then(super::maps::map_id);
            // EntityRenderer.shouldRender: its box (a block wide with a map),
            // measured from the box's centre.
            let (lo, hi) = f.bounds();
            let (mut lo, mut hi) = (glam::DVec3::from(lo), glam::DVec3::from(hi));
            if map.is_some() {
                let s = f.step();
                for i in 0..3 {
                    if s[i] == 0 {
                        let c = (lo[i] + hi[i]) / 2.0;
                        (lo[i], hi[i]) = (c - 0.5, c + 0.5);
                    }
                }
            }
            let Some(sight) = self.entity_sight(camera, (lo + hi) / 2.0, range, lo, hi) else {
                continue;
            };
            let culling = self.sighted_decision(sight, || self.entity_cull_decision(f, 1.0));
            if culling.hides_entity() {
                continue;
            }
            let entity_start = out.len();
            self.mark_motion(entity_start, Self::MOTION_OBJECT | u64::from(f.id));
            let s = f.step();
            // Pose: to the block centre, then X/Y rotation by facing.
            let (x_rot, y_rot) = if s[1] == 0 {
                let to_y = match f.facing {
                    3 => 0.0,
                    4 => 90.0,
                    2 => 180.0,
                    _ => 270.0,
                };
                (0.0f64, 180.0f64 - to_y)
            } else {
                (-90.0 * f64::from(s[1]), 180.0)
            };
            let pose = glam::DMat4::from_translation(
                glam::DVec3::new(
                    f64::from(f.pos[0]) + 0.5,
                    f64::from(f.pos[1]) + 0.5,
                    f64::from(f.pos[2]) + 0.5,
                ) - camera,
            ) * glam::DMat4::from_rotation_x(x_rot.to_radians())
                * glam::DMat4::from_rotation_y(y_rot.to_radians());
            let light = self.world.light(f.pos[0], f.pos[1], f.pos[2]);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            // ItemFrameRenderer.getBlockLightLevel: a glow frame's model is at least 5 bright.
            let frame_light = if f.glow {
                (light & 0xF0) | (light & 15).max(5)
            } else {
                light
            };
            let frame_packed =
                (u32::from(frame_light >> 4) * 16) | ((u32::from(frame_light & 15) * 16) << 8);
            let frame_start = out.len();
            let mut quad =
                |m: &glam::DMat4, corners: [[f64; 3]; 4], uv: [[f32; 2]; 4], light: u32| {
                    // The frame model is drawn with the entity shader, lit by its face normal.
                    let at = corners.map(|c| m.transform_point3(glam::DVec3::from_array(c)));
                    let n = (at[1] - at[0]).cross(at[2] - at[0]).normalize_or_zero();
                    let normal = [0, 1, 2].map(|k| (n[k].clamp(-1.0, 1.0) * 127.0) as i8);
                    for (p, uv) in at.into_iter().zip(uv) {
                        out.push(Vertex {
                            normal: [normal[0], normal[1], normal[2], -127],
                            pos: p.as_vec3().to_array(),
                            uv,
                            layer: 0,
                            color: 0xFFFF_FFFF,
                            light,

                            glint_uv: [0.0, 0.0],
                        });
                    }
                };
            let map_uv = |r: &horizon_client::hud::Rect, uv: [[f32; 2]; 4]| {
                uv.map(|[u, v]| [r.u0 + u * (r.u1 - r.u0), r.v0 + v * (r.v1 - r.v0)])
            };
            if !f.invisible {
                let model = match (f.glow, map.is_some()) {
                    (true, true) => &self.glow_map_frame_model,
                    (false, true) => &self.map_frame_model,
                    (true, false) => &self.glow_frame_model,
                    (false, false) => &self.frame_model,
                };
                for (corners, uv, sprite) in model {
                    let Some(r) = sprites.get(sprite) else {
                        continue;
                    };
                    quad(
                        &pose,
                        corners.map(|c| {
                            [
                                f64::from(c[0]) - 0.5,
                                f64::from(c[1]) - 0.5,
                                f64::from(c[2]) - 0.5,
                            ]
                        }),
                        map_uv(r, *uv),
                        frame_packed,
                    );
                }
            }
            // HDR output: a glow frame shines softly (entity.wgsl hdr_entity_mark); the item in it
            // is only lit, as vanilla draws it.
            if f.glow {
                for v in &mut out[frame_start..] {
                    v.layer |= u32::from(horizon_client::mesher::INCANDESCENT) << 16;
                }
            }
            self.finish_lit_entity(culling, f, out, entity_start);
            // The frame; what it shows is an item or a map, Horizon's to light: not the layer's.
            self.lit_entity(entity_start..out.len());
            if culling.rays_only() {
                continue;
            }
            let cull_contents = culling.checks_vertices();
            if let Some(stack) = &f.item {
                let mut canceled = false;
                self.native_render_item_in_frame(f.id, f.pos, stack, &mut canceled);
                if canceled {
                    continue;
                }
            }
            if let Some(id) = map {
                // ItemFrameRenderer: the map fills the face, in 90-degree steps.
                let depth = if f.invisible { 0.5 } else { 0.4375 };
                // A glow frame's map is 15728850 = 0xF000D2 (sky coordinate 240,
                // block coordinate 210), not full bright.
                let item_light = if f.glow { 240 | 210 << 8 } else { packed };
                let m = pose
                    * glam::DMat4::from_translation(glam::DVec3::new(0.0, 0.0, depth))
                    * glam::DMat4::from_rotation_z(
                        (f64::from(f.rotation % 4 * 2) * 45.0).to_radians(),
                    )
                    * glam::DMat4::from_rotation_z(std::f64::consts::PI)
                    * glam::DMat4::from_scale(glam::DVec3::splat(0.0078125))
                    * glam::DMat4::from_translation(glam::DVec3::new(-64.0, -64.0, -1.0));
                maps.push((id, m.as_mat4(), item_light, cull_contents, f.kind()));
                continue;
            }
            let Some(stack) = f.item.clone() else {
                continue;
            };
            // `GlowItemFrame` items render full bright (15728880).
            let item_light = if f.glow { 0xFF } else { light };
            let depth = if f.invisible { 0.5 } else { 0.4375 };
            let m = pose
                * glam::DMat4::from_translation(glam::DVec3::new(0.0, 0.0, depth))
                * glam::DMat4::from_rotation_z((f64::from(f.rotation) * 45.0).to_radians())
                * glam::DMat4::from_scale(glam::DVec3::splat(0.5));
            items.push(FramedItem {
                stack,
                id: f.id,
                matrix: m.as_mat4(),
                light: item_light,
                cull: cull_contents,
                kind: f.kind(),
                // `ItemFrameRenderer.extractRenderState` hands the frame entity
                // itself to the item model (`updateForNonLiving`), so a framed
                // compass's needle reads the frame's position and yaw.
                owner: frame_owner(f),
            });
        }
        // `extractRenderState`: FIXED, seeded by the frame entity.
        let skinning = self.skinning();
        let lights = horizon_client::block_entity_render::diffuse_lights(self.nether_lighting());
        for item in items {
            let entity_start = out.len();
            self.mark_entity(entity_start, item.kind);
            self.mark_motion(entity_start, Self::MOTION_FRAME_ITEM | u64::from(item.id));
            let state = self.resolve_item(
                &item.stack,
                horizon_assets::item_model::DisplayContext::Fixed,
                Some(&item.owner),
                item.id as i32,
            );
            let light = horizon_client::block_entity_render::vertex_light(item.light);
            // Posed on the GPU when every layer can be.
            let skinned = skinning
                .then(|| self.submit_item_skinned(&state, &[item.matrix], light, lights))
                .flatten();
            if skinned.is_none() {
                self.submit_item_world(&state, item.matrix, light, out);
            }
            if item.cull {
                self.cull_skinned(out, entity_start, skinned);
            }
        }
        // Maps: keyed by their map id, which several frames can show, so they stay unmarked.
        self.mark_motion(out.len(), 0);
        // Nearest first: past the atlas's map slots, the far ones go without.
        maps.sort_by(|a, b| {
            let d = |m: &glam::Mat4| {
                m.transform_point3(glam::Vec3::new(64.0, 64.0, 0.0))
                    .length_squared()
            };
            d(&a.1).total_cmp(&d(&b.1))
        });
        for (id, m, light, cull, kind) in maps {
            let entity_start = out.len();
            self.mark_entity(entity_start, kind);
            // Its outline first: a map out of sight takes no slot in the atlas, or with
            // more maps around than slots, the ones in view lost theirs to it.
            if cull {
                for c in [[0.0, 0.0], [128.0, 0.0], [128.0, 128.0], [0.0, 128.0]] {
                    out.push(Vertex {
                        pos: m
                            .transform_point3(glam::Vec3::new(c[0], c[1], 0.0))
                            .to_array(),
                        ..bytemuck::Zeroable::zeroed()
                    });
                }
                self.cull_entity_vertices(out, entity_start);
                if out.len() == entity_start {
                    continue;
                }
                out.truncate(entity_start);
            }
            self.map_vertices(id, true, m, light, out);
            if cull {
                self.cull_entity_vertices(out, entity_start);
            }
        }
    }
}

#[cfg(test)]
mod source_tests {
    #[test]
    fn frame_model_reads_resolved_resources_without_files() {
        use horizon_assets::resource_source::{Resource, ResourceSource};
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            ("assets/minecraft/models/block/item_frame.json".into(), Resource::bytes(br##"{"textures":{"x":"example:block/frame"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#x"}}}]}"##.to_vec())),
        ]));
        let model = super::bake_frame(&source, "item_frame");
        assert_eq!(model.len(), 1);
        assert_eq!(model[0].2, "example:block/frame");
    }
}

#[cfg(test)]
mod map_limit_tests {
    use super::super::maps::{Decoration, Key, MapData};
    use super::*;

    fn frame(id: u32) -> ItemFrame {
        ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            pos: [0, 64, id as i32],
            facing: 3,
            glow: false,
            item: None,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            custom_name: None,
            random: horizon_core::random::EntityRandom::own(
                horizon_core::random::LegacyRandom::new(0),
            ),
        }
    }
    /// A map whose `count` frame markers all count toward the limit.
    fn map_tracking(count: i32) -> MapData {
        let mut map = MapData::fresh(0.0, 0.0, 0, "minecraft:overworld");
        map.decorations = (0..count)
            .map(|i| Decoration {
                sprite: "map/decorations/frame",
                x: 0,
                y: 0,
                rot: 0,
                name: None,
                key: Key::Frame(1000 + i),
            })
            .collect();
        map
    }
    fn filled_map(id: i32) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:filled_map", 1);
        stack.data_mut().map_id = Some(id);
        stack
    }
    fn at(id: u32) -> Option<InteractTarget> {
        Some(InteractTarget {
            id: id as i32,
            location: [0.0; 3],
        })
    }

    #[test]
    #[ignore = "daily"]
    fn a_map_tracking_more_than_256_markers_stays_in_the_hand() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.item_frames.extend([frame(1), frame(2)]);
            game.maps.data.insert(7, map_tracking(257));
            game.maps.data.insert(8, map_tracking(256));

            // ItemFrame.interact: FAIL, the frame stays empty and the map is kept.
            *game.held_mut() = Some(filled_map(7));
            assert!(
                game.interact_frame(at(1)),
                "the refusal ends the interaction"
            );
            assert!(game.item_frames[0].item.is_none());
            assert_eq!(game.held().and_then(|s| s.data()?.map_id), Some(7));

            // At the limit (not over it) the map goes in.
            *game.held_mut() = Some(filled_map(8));
            assert!(game.interact_frame(at(2)));
            let framed = game.item_frames[1].item.as_ref();
            assert_eq!(framed.and_then(|s| s.data()?.map_id), Some(8));
        });
    }
}

#[cfg(test)]
mod game_event_tests {
    use super::super::game_tests::tests::server_game;
    use super::*;
    use horizon_client::mobs::Who;

    fn frame(id: u32, pos: [i32; 3], item: Option<ItemStack>) -> ItemFrame {
        ItemFrame {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            pos,
            facing: 3,
            glow: false,
            item,
            rotation: 0,
            drop_chance: 1.0,
            fixed: false,
            invisible: false,
            check_interval: 0,
            custom_name: None,
            random: horizon_core::random::EntityRandom::own(
                horizon_core::random::LegacyRandom::new(0),
            ),
        }
    }
    fn at(id: u32) -> Option<InteractTarget> {
        Some(InteractTarget {
            id: id as i32,
            location: [0.0; 3],
        })
    }
    /// The events posted since [`reset`].
    fn posted() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
        super::super::sculk::TEST_POSTED_EVENTS.with(|e| e.borrow().clone().unwrap_or_default())
    }
    fn reset() {
        super::super::sculk::TEST_POSTED_EVENTS.with(|e| *e.borrow_mut() = Some(Vec::new()));
    }

    /// `ItemFrame.interact`: putting an item in and turning it each post
    /// `GameEvent.BLOCK_CHANGE` from the player, at the frame's position.
    #[test]
    #[ignore = "daily"]
    fn putting_and_rotating_a_frame_item_posts_block_change() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.item_frames.clear();
        game.item_frames.extend(vec![frame(1, [0, 64, 1], None)]);
        let center = [0.5, 64.5, 1.03125];
        *game.held_mut() = Some(ItemStack::new("minecraft:stick", 3));

        let expected = [("minecraft:block_change", center, Some(Who::Player))];
        reset();
        assert!(game.interact_frame(at(1)));
        assert_eq!(posted(), expected);
        assert_eq!(game.held().map(|s| s.count), Some(2), "one goes in");
        assert_eq!(game.item_frames[0].item.as_ref().map(|s| s.count), Some(1));

        reset();
        assert!(game.interact_frame(at(1)));
        assert_eq!(posted(), expected);
        assert_eq!(game.item_frames[0].rotation, 1);
    }

    /// `ItemFrame.hurtServer`: the item coming out posts `BLOCK_CHANGE` from
    /// the player hitting the frame.
    #[test]
    #[ignore = "daily"]
    fn taking_a_frame_item_out_posts_block_change_from_the_player() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        game.item_frames.clear();
        game.item_frames.extend(vec![frame(
            1,
            [0, 64, 1],
            Some(ItemStack::new("minecraft:stick", 1)),
        )]);
        let center = [0.5, 64.5, 1.03125];
        // A player looking straight at the frame's centre from two blocks away
        // (eye height 1.62 above the feet).
        let feet = Vec3::new(0.5, 64.5 - 1.62, -1.0);
        game.player =
            Some(horizon_client::motion::PlayerMotion::new(game.motion, feet).expect("a player"));
        game.yaw = 0.0;
        game.pitch = 0.0;

        let expected = [("minecraft:block_change", center, Some(Who::Player))];
        reset();
        assert!(game.attack_frame(), "the frame is in reach");
        assert!(game.item_frames[0].item.is_none());
        assert_eq!(posted(), expected);

        // The empty frame itself then breaks (`BlockAttachedEntity.hurtServer`
        // then `ItemFrame.dropItem`), with the same event.
        reset();
        assert!(game.attack_frame());
        assert!(game.item_frames.is_empty());
        assert_eq!(
            posted(),
            [
                ("minecraft:entity_die", center, Some(Who::Player)),
                ("minecraft:block_change", center, Some(Who::Player))
            ]
        );
    }

    /// `ItemFrame.dropItem` from `BlockAttachedEntity.tick`, when a frame
    /// loses the block it hangs on: the event has no source.
    #[test]
    #[ignore = "daily"]
    fn a_frame_that_falls_posts_block_change_without_source() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let mut f = frame(1, [0, 64, 1], None);
        // BlockAttachedEntity.tick checks every 100 ticks.
        f.check_interval = 99;
        game.item_frames.clear();
        game.item_frames.extend(vec![f]);
        let center = [0.5, 64.5, 1.03125];

        reset();
        game.tick_item_frames();
        assert!(game.item_frames.is_empty(), "nothing holds the frame");
        assert_eq!(posted(), [("minecraft:block_change", center, None)]);
    }
}

#[cfg(test)]
#[path = "tests/lodestone_compass_tests.rs"]
mod lodestone_compass_tests;
