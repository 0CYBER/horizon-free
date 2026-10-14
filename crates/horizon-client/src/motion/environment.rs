//! Resident bindings for the existing native immersion, current and liquid-travel kernels.
use super::*;
use horizon_core::{geometry::Direction, random::RandomSource};
use horizon_world::{
    entity_fluid::{
        CurrentTarget, FluidInteractionWorld, FluidMembership, FluidSections, FluidTagId,
    },
    fluid::{FluidHeightWorld, FluidKind, FluidState},
    fluid_flow::{FluidFlowWorld, flow_at},
    living_fluid::{LivingFluidWorld, TravelFluidTag},
};
pub(super) const WATER: FluidTagId = FluidTagId(0);
pub(super) const LAVA: FluidTagId = FluidTagId(1);
impl MotionWorld<'_, '_, '_> {
    pub(super) fn fluid_at(&self, p: BlockPos) -> Result<FluidState> {
        Ok(FluidState::from_block(
            self.assets
                .registry
                .state(self.block(p))
                .ok_or("missing fluid state")?,
        )?)
    }
    pub(super) fn update_fluid_environment(&mut self) -> Result<()> {
        let mut fluids = std::mem::take(&mut self.player.fluids);
        let result = fluids
            .update(self, false, 4096)
            .map_err(|e| io::Error::other(e.to_string()));
        if let Err(e) = result {
            self.player.fluids = fluids;
            return Err(e.into());
        }
        self.player.in_water = fluids.is_in_fluid(self, WATER);
        self.player.in_lava = fluids.is_in_fluid(self, LAVA);
        self.player.eye_in_water = fluids.is_eye_in_fluid(self, WATER);
        if self.player.in_water {
            self.player.fall_distance = 0.0;
        }
        if !self.player.flying && !self.player.spectator {
            fluids.apply_current(self, WATER, 0.014)?;
            fluids.apply_current(
                self,
                LAVA,
                if self.scene.fast_lava {
                    0.007
                } else {
                    0.0023333333333333335
                },
            )?;
        }
        self.player.fluids = fluids;
        Ok(())
    }
    fn environment_hurt(&mut self, source: &str, damage: f32) -> Result<()> {
        if self.player.invulnerable || self.player.spectator {
            return Ok(());
        }
        let id = self
            .resources
            .health
            .damage
            .id(source)
            .ok_or("missing environment damage type")?;
        self.player
            .hurt(&self.resources.health, self.scene.health_rules, id, damage)?;
        Ok(())
    }
    pub(super) fn tick_environment_damage(&mut self) -> Result<()> {
        if self.player.in_water || self.player.spectator {
            self.player.fire_ticks = 0;
        }
        if self.player.invulnerable && self.player.fire_ticks > 1 {
            self.player.fire_ticks = 1;
        }
        if self.player.fire_ticks > 0 {
            if self.player.fire_ticks % 20 == 0 && !self.player.in_lava {
                self.environment_hurt("minecraft:on_fire", 1.0)?;
            }
            self.player.fire_ticks -= 1;
        }
        if self.player.eye_in_water
            && !self.player.status.water_breathing
            && !self.player.invulnerable
            && !self.player.spectator
        {
            let oxygen = self
                .player
                .attribute("minecraft:oxygen_bonus")?
                .max(f64::from(self.player.respiration));
            if oxygen <= 0.0 || self.scene.random.next_double() < 1.0 / (oxygen + 1.0) {
                self.player.air -= 1;
            }
            if self.player.air <= -20 {
                self.player.air = 0;
                self.environment_hurt("minecraft:drown", 2.0)?;
            }
        } else {
            self.player.air = (self.player.air + 4).min(300);
        }
        if self.player.in_lava {
            self.player.ignite_for_ticks(300)?;
            self.environment_hurt("minecraft:lava", 4.0)?;
            self.player.fall_distance *= 0.5;
        }
        Ok(())
    }
    pub(super) fn apply_fire_contact(&mut self) -> Result<()> {
        if self.player.spectator {
            self.player.fire_ticks = 0;
            return Ok(());
        }
        let damage = fire_contact(self.scene, self.assets.registry, self.player.bounds());
        if damage > 0.0 {
            self.player.ticks_frozen = 0;
            if !self.player.in_water {
                let multiplier = self.player.attribute("minecraft:burning_time")?;
                let ticks = ignite_contact(
                    self.player.fire_ticks,
                    true,
                    &mut self.scene.random,
                    multiplier,
                );
                self.player.set_remaining_fire_ticks(ticks);
            }
            self.environment_hurt("minecraft:in_fire", damage)?;
        } else if self.player.fire_ticks <= 0 {
            self.player.fire_ticks = -20;
        }
        Ok(())
    }
}
pub(crate) fn fire_contact(
    world: &World,
    registry: &horizon_world::registry::BlockRegistry,
    bounds: Aabb,
) -> f32 {
    let b = bounds.deflate(Vec3::new(1e-5, 1e-5, 1e-5));
    let mut damage = 0.0f32;
    for x in b.min_position().x.floor() as i32..=b.max_position().x.floor() as i32 {
        for y in b.min_position().y.floor() as i32..=b.max_position().y.floor() as i32 {
            for z in b.min_position().z.floor() as i32..=b.max_position().z.floor() as i32 {
                damage = damage.max(
                    match registry
                        .state(world.block(x, y, z))
                        .map(|s| s.name.as_str())
                    {
                        Some("minecraft:fire") => 1.0,
                        Some("minecraft:soul_fire") => 2.0,
                        _ => 0.0,
                    },
                );
            }
        }
    }
    damage
}
impl FluidMembership for MotionWorld<'_, '_, '_> {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        if tag == WATER {
            matches!(kind, FluidKind::Water | FluidKind::FlowingWater)
        } else {
            matches!(kind, FluidKind::Lava | FluidKind::FlowingLava)
        }
    }
}
impl CurrentTarget for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn is_player_class(&self) -> bool {
        true
    }
    fn velocity(&mut self) -> Result<Vec3> {
        Ok(self.player.velocity)
    }
    fn add_delta_movement(&mut self, v: Vec3) -> Result<()> {
        let old = self.player.velocity;
        self.player.velocity = Vec3::new(old.x + v.x, old.y + v.y, old.z + v.z);
        Ok(())
    }
}
impl FluidInteractionWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type Chunk = (i32, i32);
    fn interaction_box(&mut self) -> Result<Option<Aabb>> {
        Ok(Some(
            self.player.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5)),
        ))
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> Result<Option<Self::Chunk>> {
        Ok(self.scene.chunks.contains_key(&(x, z)).then_some((x, z)))
    }
    fn chunk_sections(&mut self, _: Self::Chunk) -> Result<FluidSections<'_>> {
        // Resident sections have no fluid-count cache; scan the small interaction box rather than skipping sections.
        static SECTIONS: [bool; crate::chunk::SECTIONS] = [true; crate::chunk::SECTIONS];
        Ok(FluidSections {
            minimum_section_y: crate::chunk::MIN_Y >> 4,
            has_fluid: &SECTIONS,
        })
    }
    fn bounding_box_min_y(&self) -> f64 {
        self.player.bounds().min_position().y
    }
    fn block_x(&self) -> i32 {
        self.player.position.x.floor() as i32
    }
    fn block_z(&self) -> i32 {
        self.player.position.z.floor() as i32
    }
    fn eye_y(&mut self) -> Result<f64> {
        Ok(self.player.position.y + f64::from(self.player.dimensions.eye_height))
    }
    fn fluid_state(&mut self, p: BlockPos) -> Result<FluidState> {
        self.fluid_at(p)
    }
    fn fluid_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32> {
        f.height_at(self, [p.x, p.y, p.z])
    }
    fn camera_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32> {
        f.camera_height_at(self, [p.x, p.y, p.z])
    }
    fn flow(&mut self, f: FluidState, p: BlockPos) -> Result<Vec3> {
        flow_at(self, f, p)
    }
}
impl FluidHeightWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn fluid(&mut self, p: [i32; 3]) -> Result<FluidState> {
        self.fluid_at(BlockPos::new(p[0], p[1], p[2]))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> Result<bool> {
        Ok(self
            .resources
            .facts
            .get(MotionWorld::block(self, BlockPos::new(p[0], p[1], p[2])))
            .sturdy_full
            & 1
            != 0)
    }
}
impl FluidFlowWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type Block = BlockStateId;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState> {
        self.fluid_at(p)
    }
    fn block(&mut self, p: BlockPos) -> Result<BlockStateId> {
        Ok(MotionWorld::block(self, p))
    }
    fn blocks_fluid_flow(&self, s: BlockStateId) -> bool {
        self.assets
            .blocks_fluid_flow
            .contains(&self.assets.registry.state(s).unwrap().name)
    }
    fn is_ice_class(&self, s: BlockStateId) -> bool {
        self.resources
            .facts
            .get(s)
            .classes
            .iter()
            .any(|c| c.as_ref() == "IceBlock")
    }
    fn is_face_sturdy(&mut self, s: BlockStateId, _: BlockPos, d: Direction) -> Result<bool> {
        Ok(self.resources.facts.get(s).sturdy_full & (1 << d as u8) != 0)
    }
}
impl LivingFluidWorld for MotionWorld<'_, '_, '_> {
    fn in_water(&mut self) -> Result<bool> {
        Ok(self.player.in_water)
    }
    fn sprinting(&mut self) -> Result<bool> {
        Ok(self.player.sprinting)
    }
    fn water_slow_down(&mut self) -> Result<f32> {
        Ok(0.8)
    }
    fn water_movement_efficiency(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:water_movement_efficiency")
    }
    fn dolphins_grace(&mut self) -> Result<bool> {
        Ok(self.player.status.dolphins_grace)
    }
    fn fluid_height(&mut self, tag: TravelFluidTag) -> Result<f64> {
        Ok(self.player.fluids.fluid_height(
            self,
            if tag == TravelFluidTag::Lava {
                LAVA
            } else {
                WATER
            },
        ))
    }
    fn fluid_jump_threshold(&mut self) -> Result<f64> {
        Ok(0.4)
    }
    fn is_free(&mut self, v: Vec3) -> Result<bool> {
        let b = self.player.bounds().move_by(v);
        Ok(self.no_player_collision(b)? && !self.has_liquid(b)?)
    }
    fn is_vehicle(&mut self) -> Result<bool> {
        Ok(false)
    }
    fn can_float_while_ridden(&mut self) -> Result<bool> {
        Ok(false)
    }
}
impl MotionWorld<'_, '_, '_> {
    fn has_liquid(&self, b: Aabb) -> Result<bool> {
        for x in b.min_position().x.floor() as i32..=b.max_position().x.floor() as i32 {
            for y in b.min_position().y.floor() as i32..=b.max_position().y.floor() as i32 {
                for z in b.min_position().z.floor() as i32..=b.max_position().z.floor() as i32 {
                    if self.fluid_at(BlockPos::new(x, y, z))?.kind() != FluidKind::Empty {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
}

pub(crate) fn ignite_contact(
    ticks: i32,
    player: bool,
    random: &mut impl RandomSource,
    multiplier: f64,
) -> i32 {
    let ticks = if ticks < 0 {
        ticks + 1
    } else if player {
        ticks + 1 + random.next_int_bound(2).unwrap_or(0)
    } else {
        ticks
    };
    if ticks >= 0 {
        ticks.max((160.0 * multiplier).ceil() as i32)
    } else {
        ticks
    }
}

#[cfg(test)]
mod fire_ignition_tests {
    use super::*;
    #[test]
    fn fire_hurts_before_twenty_tick_ignition_and_scales_duration() {
        let mut r = horizon_core::random::LegacyRandom::new(3);
        let mut ticks = -20;
        for _ in 0..19 {
            ticks = ignite_contact(ticks, true, &mut r, 0.4);
            assert!(ticks < 0);
        }
        assert_eq!(ignite_contact(ticks, true, &mut r, 0.4), 64);
    }
}
