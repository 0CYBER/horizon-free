//! `RespawnAnchorBlock` in the game: glowstone charges it (`useItemOn`), a
//! right click on a charged anchor sets the respawn point in the Nether and
//! blows it up anywhere else (`useWithoutItem`), and respawning goes to it
//! (`ServerPlayer.findRespawnAndUseSpawnBlock`, `PlayerList.respawn`),
//! using a charge.
use super::{Dim, Game};
use horizon_client::first_person::Hand;
use horizon_client::{
    Result,
    blocks::Fluid,
    collisions::{SceneCollisions, no_collision_empty},
    items::ItemStack,
    mobs::Who,
    motion::PlayerMotion,
    text::Text,
};
use horizon_core::{
    geometry::{Aabb, Axis, Vec3, VoxelShape},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::{collision_context::CollisionContext, registry::BlockStateId};

/// `ServerPlayer.RespawnConfig`: a respawn point other than the world spawn
/// that the game resolves through its block (a respawn anchor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RespawnConfig {
    pub dim: Dim,
    pub pos: [i32; 3],
    pub yaw: f32,
    pub forced: bool,
}

/// `RespawnAnchorBlock.RESPAWN_HORIZONTAL_OFFSETS`, then the same a block
/// below, a block above, and straight above.
const HORIZONTAL: [[i32; 3]; 8] = [
    [0, 0, -1],
    [-1, 0, 0],
    [0, 0, 1],
    [1, 0, 0],
    [-1, 0, -1],
    [1, 0, -1],
    [-1, 0, 1],
    [1, 0, 1],
];
fn respawn_offsets() -> impl Iterator<Item = [i32; 3]> {
    HORIZONTAL
        .into_iter()
        .chain(HORIZONTAL.into_iter().map(|[x, y, z]| [x, y - 1, z]))
        .chain(HORIZONTAL.into_iter().map(|[x, y, z]| [x, y + 1, z]))
        .chain(std::iter::once([0, 1, 0]))
}

/// What a right click with an item does to an anchor before `useWithoutItem`.
pub(super) enum AnchorUse {
    /// Glowstone charged it.
    Charged,
    /// `PASS`: the off hand's glowstone charges it instead.
    Pass,
    /// `TRY_WITH_EMPTY_HAND`, or not an anchor.
    Other,
}

impl Game<'_> {
    fn is_anchor(&self, state: BlockStateId) -> bool {
        &*self.res.facts.get(state).name == "minecraft:respawn_anchor"
    }
    fn anchor_charges(&self, state: BlockStateId) -> i32 {
        self.res
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("charges"))
            .and_then(|c| c.parse().ok())
            .unwrap_or(0)
    }
    fn anchor_with_charges(&self, state: BlockStateId, charges: i32) -> Option<BlockStateId> {
        let mut s = self.res.assets.registry.state(state)?.clone();
        s.properties.insert("charges".into(), charges.to_string());
        self.res.assets.registry.resolve(&s).ok()
    }
    /// `RespawnAnchorBlock.canSetSpawn`: the `respawn_anchor_works` attribute
    /// (only the Nether's dimension type sets it).
    pub(super) fn anchor_works(&self) -> bool {
        self.dim == Dim::Nether
    }
    /// `RespawnAnchorBlock.charge`: one more charge, the block change and the sound.
    fn charge_anchor(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        who: Option<Who>,
    ) -> Result<()> {
        let Some(next) = self.anchor_with_charges(state, self.anchor_charges(state) + 1) else {
            return Ok(());
        };
        self.level_set(pos, next)?;
        self.post_block_game_event("minecraft:block_change", pos, who, Some(next));
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        self.audio
            .play("minecraft:block.respawn_anchor.charge", Some(at), 1.0, 1.0);
        Ok(())
    }
    /// `RespawnAnchorBlock.useItemOn` with `held` in the hand that acts.
    pub(super) fn use_respawn_anchor(
        &mut self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> Result<AnchorUse> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if !self.is_anchor(state) {
            return Ok(AnchorUse::Other);
        }
        let fuel = |s: Option<&ItemStack>| s.is_some_and(|s| &*s.item == "minecraft:glowstone");
        let chargeable = self.anchor_charges(state) < 4;
        if fuel(held) && chargeable {
            let used = held.cloned();
            self.charge_anchor(pos, state, Some(Who::Player))?;
            // itemStack.consume(1, player); the criterion sees the stack as used.
            self.consume_held();
            if let Some(stack) = used {
                self.trigger_block("minecraft:item_used_on_block", pos, &stack);
            }
            return Ok(AnchorUse::Charged);
        }
        if self.hand == Hand::Main && fuel(self.offhand.as_ref()) && chargeable {
            return Ok(AnchorUse::Pass);
        }
        Ok(AnchorUse::Other)
    }
    /// `RespawnAnchorBlock.useWithoutItem` on a charged anchor: outside the
    /// Nether it explodes; inside, it becomes the respawn point.
    pub(super) fn respawn_anchor_used(&mut self, pos: [i32; 3]) -> Result<()> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if !self.anchor_works() {
            return self.explode_anchor(pos, state);
        }
        let config = RespawnConfig {
            dim: self.dim,
            pos,
            yaw: 0.0,
            forced: false,
        };
        // RespawnConfig.isSamePosition: dimension and block.
        let same = self
            .respawn_config
            .is_some_and(|c| c.dim == config.dim && c.pos == config.pos);
        if !same {
            // setRespawnPosition(config, true): the message, the point; the
            // anchor replaces a bed.
            self.system_message(Text::translate("block.minecraft.set_spawn", &[]));
            self.respawn_config = Some(config);
            if let Some([x, y, z]) = self.level.world_spawn {
                self.spawn = Some(Vec3::new(
                    f64::from(x) + 0.5,
                    f64::from(y),
                    f64::from(z) + 0.5,
                ));
            }
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ];
            self.audio.play(
                "minecraft:block.respawn_anchor.set_spawn",
                Some(at),
                1.0,
                1.0,
            );
        }
        Ok(())
    }
    /// `RespawnAnchorBlock.isWaterThatWouldFlow`.
    fn water_would_flow(&self, p: [i32; 3]) -> bool {
        let f = self.res.facts.get(self.world.block(p[0], p[1], p[2]));
        if f.fluid != Fluid::Water {
            return false;
        }
        if f.fluid_level == 8 && !f.fluid_falling {
            return true;
        }
        if f.fluid_level < 2 {
            return false;
        }
        self.res
            .facts
            .get(self.world.block(p[0], p[1] - 1, p[2]))
            .fluid
            != Fluid::Water
    }
    /// `RespawnAnchorBlock.explode`: the anchor goes, then a power 5 fiery
    /// explosion (`badRespawnPointExplosion`, `ExplosionInteraction.BLOCK`)
    /// whose centre resists like water when water would flow into it.
    fn explode_anchor(&mut self, pos: [i32; 3], _state: BlockStateId) -> Result<()> {
        self.level_set(pos, self.world.air)?;
        let horizontal = [[0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]]
            .into_iter()
            .any(|d| self.water_would_flow([pos[0] + d[0], pos[1], pos[2] + d[2]]));
        let above = self
            .res
            .facts
            .get(self.world.block(pos[0], pos[1] + 1, pos[2]))
            .fluid
            == Fluid::Water;
        let in_water = horizontal || above;
        let center = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        self.block_explosion(center, in_water.then_some(pos))
    }
    /// `BedRule.destroyOnUse` of the dimension's `bed_rule` and
    /// `straw_bed_rule`: true in the Nether and the End.
    pub(super) fn beds_destroy_on_use(&self) -> bool {
        self.dim != Dim::Overworld
    }
    /// `AbstractBedBlock.useWithoutItem` where the bed rule destroys on use
    /// (the bed's head at `head`): `BedBlock.destroyOnUse` removes both
    /// halves and explodes; `StrawBedBlock` just falls apart.
    pub(super) fn destroy_bed_on_use(&mut self, head: [i32; 3]) -> Result<()> {
        let state = self.world.block(head[0], head[1], head[2]);
        let name = self.res.facts.get(state).name.clone();
        let center = [
            f64::from(head[0]) + 0.5,
            f64::from(head[1]) + 0.5,
            f64::from(head[2]) + 0.5,
        ];
        if &*name == "minecraft:straw_bed" {
            // StrawBedBlock.destroyBed.
            self.audio.play(
                "minecraft:block.straw_bed.break_leave",
                Some(center),
                1.0,
                1.0,
            );
            return self.level_set(head, self.world.air);
        }
        // removeBlock(head), then the foot behind it if it is still this bed.
        let facing = self
            .res
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("facing").cloned())
            .unwrap_or_default();
        let back = match facing.as_str() {
            "north" => [0, 0, 1],
            "south" => [0, 0, -1],
            "west" => [1, 0, 0],
            _ => [-1, 0, 0],
        };
        self.level_set(head, self.world.air)?;
        let foot = [head[0] + back[0], head[1], head[2] + back[2]];
        if self
            .res
            .facts
            .get(self.world.block(foot[0], foot[1], foot[2]))
            .name
            == name
        {
            self.level_set(foot, self.world.air)?;
        }
        self.block_explosion(center, None)
    }
    /// `Level.explode(null, badRespawnPointExplosion(center), calculator,
    /// center, 5, true, ExplosionInteraction.BLOCK)` (beds and anchors):
    /// `water_at` is the block that resists like water.
    pub(super) fn block_explosion(
        &mut self,
        center: [f64; 3],
        water_at: Option<[i32; 3]>,
    ) -> Result<()> {
        let interaction = if self.ui.rules.bool("block_explosion_drop_decay") {
            super::explosion::BlockInteraction::DestroyWithDecay
        } else {
            super::explosion::BlockInteraction::Destroy
        };
        self.server_explode_as(
            center,
            5.0,
            interaction,
            true,
            "minecraft:bad_respawn_point",
            water_at.map(|p| (p, 100.0)),
        )
    }

    // ---- respawning -------------------------------------------------------

    /// `DismountHelper.nonClimbableShape`: the collision shape, empty for
    /// climbable blocks and open trapdoors.
    fn non_climbable_shape(&self, p: [i32; 3]) -> VoxelShape {
        let state = self.world.block(p[0], p[1], p[2]);
        let name = &self.res.facts.get(state).name;
        let open_trapdoor = name.ends_with("_trapdoor")
            && self
                .res
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("open"))
                .is_some_and(|v| v == "true");
        if self.block_in_tag("minecraft:climbable", name) || open_trapdoor {
            return VoxelShape::empty();
        }
        let binding = self.collision;
        let mut scene = SceneCollisions {
            world: &self.world,
            binding,
            subject: None,
        };
        binding
            .collision_shape_with_context(
                &mut scene,
                state,
                BlockPos::new(p[0], p[1], p[2]),
                &CollisionContext::empty(false),
            )
            .unwrap_or_else(|_| VoxelShape::empty())
    }
    fn block_in_tag(&self, tag: &str, block: &str) -> bool {
        self.res
            .items
            .block_tags
            .get(tag)
            .is_some_and(|s| s.contains(block))
    }
    /// `EntityType.isBlockDangerous` for the player.
    fn dangerous_for_player(&self, p: [i32; 3]) -> bool {
        let state = self.world.block(p[0], p[1], p[2]);
        let name = &*self.res.facts.get(state).name;
        let lit_campfire = (name == "minecraft:campfire" || name == "minecraft:soul_campfire")
            && self
                .res
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("lit"))
                .is_some_and(|v| v == "true");
        // NodeEvaluator.isBurningBlock, then the hurting plants and snow.
        self.block_in_tag("minecraft:fire", name)
            || matches!(
                name,
                "minecraft:lava"
                    | "minecraft:magma_block"
                    | "minecraft:lava_cauldron"
                    | "minecraft:wither_rose"
                    | "minecraft:sweet_berry_bush"
                    | "minecraft:cactus"
                    | "minecraft:powder_snow"
            )
            || lit_campfire
    }
    /// `DismountHelper.findSafeDismountLocation` for a standing player.
    fn safe_dismount_location(&self, p: [i32; 3], check_dangerous: bool) -> Option<Vec3> {
        if check_dangerous && self.dangerous_for_player(p) {
            return None;
        }
        let shape = self.non_climbable_shape(p);
        let below = self.non_climbable_shape([p[0], p[1] - 1, p[2]]);
        let floor = horizon_core::geometry::block_floor_height(&shape, || &below);
        // DismountHelper.isBlockFloorValid.
        if floor.is_infinite() || floor >= 1.0 {
            return None;
        }
        if check_dangerous && floor <= 0.0 && self.dangerous_for_player([p[0], p[1] - 1, p[2]]) {
            return None;
        }
        let position = Vec3::new(
            f64::from(p[0]) + 0.5,
            f64::from(p[1]) + floor,
            f64::from(p[2]) + 0.5,
        );
        // EntityDimensions.makeBoundingBox for 0.6 by 1.8 (the half width is a float).
        let half = f64::from(0.6f32 / 2.0);
        let bounds = Aabb::new(
            Vec3::new(position.x - half, position.y, position.z - half),
            Vec3::new(
                position.x + half,
                position.y + f64::from(1.8f32),
                position.z + half,
            ),
        );
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if !no_collision_empty(&mut scene, bounds) {
            return None;
        }
        let name = |q: [i32; 3]| {
            self.res
                .facts
                .get(self.world.block(q[0], q[1], q[2]))
                .name
                .clone()
        };
        if self.block_in_tag("minecraft:invalid_spawn_inside", &name(p))
            || self.block_in_tag(
                "minecraft:invalid_spawn_inside",
                &name([p[0], p[1] + 1, p[2]]),
            )
        {
            return None;
        }
        // WorldBorder.isWithinBounds(box).
        let (lo, hi) = (bounds.min(Axis::X), bounds.max(Axis::X) - 1.0e-5);
        let (lz, hz) = (bounds.min(Axis::Z), bounds.max(Axis::Z) - 1.0e-5);
        (self.border.contains(lo, lz) && self.border.contains(hi, hz)).then_some(position)
    }
    /// `RespawnAnchorBlock.findStandUpPosition`: a safe spot around the
    /// anchor, else any spot.
    pub(super) fn anchor_stand_up(&self, pos: [i32; 3]) -> Option<Vec3> {
        for check in [true, false] {
            for d in respawn_offsets() {
                let p = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                if let Some(at) = self.safe_dismount_location(p, check) {
                    return Some(at);
                }
            }
        }
        None
    }
    /// `ServerPlayer.findRespawnAndUseSpawnBlock` for an anchor config in the
    /// loaded dimension: the position and yaw, using a charge.
    pub(super) fn find_respawn_and_use(
        &mut self,
        c: RespawnConfig,
        consume: bool,
    ) -> Result<Option<(Vec3, f32)>> {
        let state = self.world.block(c.pos[0], c.pos[1], c.pos[2]);
        if !(self.is_anchor(state)
            && (c.forced || self.anchor_charges(state) > 0)
            && self.anchor_works())
        {
            return Ok(None);
        }
        let stand = self.anchor_stand_up(c.pos);
        if !c.forced
            && consume
            && stand.is_some()
            && let Some(next) = self.anchor_with_charges(state, self.anchor_charges(state) - 1)
        {
            self.level_set(c.pos, next)?;
        }
        Ok(stand.map(|p| (p, look_at_yaw(p, c.pos))))
    }
    /// The death screen's respawn with an anchor as the respawn point: travel
    /// to its dimension and wait for its chunks (`travelling`), then resolve
    /// it in [`Game::place_respawned`].
    pub(super) fn begin_anchor_respawn(&mut self) -> Result<bool> {
        let Some(config) = self.respawn_config else {
            return Ok(false);
        };
        if self.player.is_none() {
            return Ok(false);
        }
        if self.dim != config.dim {
            self.end_portal_travel = false;
            self.enter_dimension(config.dim)?;
        } else {
            let player = self.player.take().ok_or("player")?;
            self.travelling = Some((player, Vec3::ZERO));
        }
        let target = Vec3::new(
            f64::from(config.pos[0]) + 0.5,
            f64::from(config.pos[1]),
            f64::from(config.pos[2]) + 0.5,
        );
        if let Some((_, t)) = self.travelling.as_mut() {
            *t = target;
        }
        self.end_travel = false;
        self.respawn_travel = Some(config);
        Ok(true)
    }
    /// `PlayerList.respawn` at an anchor once its dimension is loaded around
    /// it: a fresh player at the stand-up position facing the anchor and the
    /// deplete sound, or, with no anchor left, the world spawn and
    /// `block.minecraft.spawn.not_valid`.
    pub(super) fn place_respawned(&mut self, config: RespawnConfig) -> Result<()> {
        match self.find_respawn_and_use(config, true)? {
            Some((position, yaw)) => {
                let mut player = PlayerMotion::new(self.motion, position)?;
                player.yaw = yaw;
                player.head_yaw = yaw;
                self.yaw = yaw;
                self.pitch = 0.0;
                player.pitch = 0.0;
                self.eye = 1.62;
                self.previous_eye = 1.62;
                self.clock.reset();
                self.previous_position = position;
                self.player = Some(player);
                self.sway = Default::default();
                self.apply_equipment();
                // ClientboundSoundPacket at the block's corner, seeded from the level random.
                let state = self
                    .world
                    .block(config.pos[0], config.pos[1], config.pos[2]);
                if self.is_anchor(state) {
                    let _seed = self.world.random.next_long();
                    let at = [
                        f64::from(config.pos[0]),
                        f64::from(config.pos[1]),
                        f64::from(config.pos[2]),
                    ];
                    self.audio
                        .play("minecraft:block.respawn_anchor.deplete", Some(at), 1.0, 1.0);
                }
                Ok(())
            }
            None => {
                // TeleportTransition.missingRespawnBlock: the world spawn, and
                // the point is forgotten.
                self.respawn_config = None;
                self.system_message(Text::translate("block.minecraft.spawn.not_valid", &[]));
                let dead = PlayerMotion::new(self.motion, Vec3::ZERO)?;
                if self.dim != Dim::Overworld {
                    // A dead stand-in travels; the arrival makes a fresh player.
                    let mut dead = dead;
                    dead.set_health(0.0, false)?;
                    self.player = Some(dead);
                    self.end_portal_travel = true;
                    let result = self.enter_dimension(Dim::Overworld);
                    self.end_portal_travel = false;
                    self.end_travel = true;
                    return result;
                }
                self.player = None;
                self.spawn_player()
            }
        }
    }
}

/// `RespawnPosAngle.calculateLookAtYaw`: the yaw from the position to the
/// bottom centre of the block.
fn look_at_yaw(position: Vec3, block: [i32; 3]) -> f32 {
    let d = Vec3::new(
        f64::from(block[0]) + 0.5 - position.x,
        f64::from(block[1]) - position.y,
        f64::from(block[2]) + 0.5 - position.z,
    )
    .normalize();
    // Mth.atan2(z, x) * 180.0F / (float) Math.PI - 90.0: the compiled code
    // multiplies by the folded float constant widened to double.
    let degrees = horizon_core::geometry::trig::atan2(d.z, d.x)
        * f64::from(horizon_core::geometry::trig::RAD_TO_DEG)
        - 90.0;
    wrap_degrees(degrees) as f32
}
/// `Mth.wrapDegrees(double)`.
fn wrap_degrees(degrees: f64) -> f64 {
    let mut d = degrees % 360.0;
    if d >= 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}
