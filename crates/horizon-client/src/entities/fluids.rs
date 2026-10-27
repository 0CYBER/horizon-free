//! Dropped-item immersion and currents through the shared vanilla fluid helpers.
use super::ItemEntity;
use crate::{
    motion::{FluidResources, FluidTags, LAVA, WATER},
    world::World,
};
use horizon_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
};
use horizon_world::{
    entity_fluid::{
        CurrentTarget, EntityFluidInteraction, FluidInteractionWorld, FluidMembership,
        FluidSections, FluidTagId,
    },
    fluid::{FluidHeightWorld, FluidKind, FluidState},
    fluid_flow::{FluidFlowWorld, flow_at},
    registry::BlockStateId,
};
type Error = std::convert::Infallible;
struct ItemFluidWorld<'a> {
    item: &'a mut ItemEntity,
    world: &'a World,
    fluids: &'a FluidResources,
}
impl ItemEntity {
    pub(super) fn update_fluids(&mut self, world: &World, fluids: &FluidResources) -> (f64, f64) {
        let mut interaction = EntityFluidInteraction::new([WATER, LAVA]);
        let mut scene = ItemFluidWorld {
            item: self,
            world,
            fluids,
        };
        if interaction.update(&mut scene, false, 4096).is_err() {
            return (0.0, 0.0);
        }
        let water_height = interaction.fluid_height(&scene, WATER);
        let lava_height = interaction.fluid_height(&scene, LAVA);
        let _ = interaction.apply_current(&mut scene, WATER, 0.014);
        let _ = interaction.apply_current(
            &mut scene,
            LAVA,
            if world.fast_lava {
                0.007
            } else {
                0.0023333333333333335
            },
        );
        (water_height, lava_height)
    }
}
impl FluidMembership for ItemFluidWorld<'_> {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        FluidTags.fluid_in_tag(kind, tag)
    }
}
impl CurrentTarget for ItemFluidWorld<'_> {
    type Error = Error;
    fn is_player_class(&self) -> bool {
        false
    }
    fn velocity(&mut self) -> Result<Vec3, Error> {
        Ok(self.item.velocity)
    }
    fn add_delta_movement(&mut self, delta: Vec3) -> Result<(), Error> {
        let v = self.item.velocity;
        let sum = Vec3::new(v.x + delta.x, v.y + delta.y, v.z + delta.z);
        if delta.is_finite() && sum.is_finite() {
            self.item.velocity = sum;
        }
        Ok(())
    }
}
impl FluidInteractionWorld for ItemFluidWorld<'_> {
    type Error = Error;
    type Chunk = ();
    fn interaction_box(&mut self) -> Result<Option<Aabb>, Error> {
        Ok(Some(
            self.item.bounds().deflate(Vec3::new(0.001, 0.001, 0.001)),
        ))
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> Result<Option<()>, Error> {
        Ok(self.world.chunks.contains_key(&(x, z)).then_some(()))
    }
    fn chunk_sections(&mut self, _: ()) -> Result<FluidSections<'_>, Error> {
        static SECTIONS: [bool; crate::chunk::SECTIONS] = [true; crate::chunk::SECTIONS];
        Ok(FluidSections {
            minimum_section_y: crate::chunk::MIN_Y >> 4,
            has_fluid: &SECTIONS,
        })
    }
    fn bounding_box_min_y(&self) -> f64 {
        self.item.position.y
    }
    fn block_x(&self) -> i32 {
        self.item.position.x.floor() as i32
    }
    fn eye_y(&mut self) -> Result<f64, Error> {
        Ok(self.item.position.y + f64::from(0.2125f32))
    }
    fn block_z(&self) -> i32 {
        self.item.position.z.floor() as i32
    }
    fn fluid_state(&mut self, p: BlockPos) -> Result<FluidState, Error> {
        Ok(self.fluids.fluid(self.world.block(p.x, p.y, p.z)))
    }
    fn fluid_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32, Error> {
        f.height_at(self, [p.x, p.y, p.z])
    }
    fn camera_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32, Error> {
        f.camera_height_at(self, [p.x, p.y, p.z])
    }
    fn flow(&mut self, f: FluidState, p: BlockPos) -> Result<Vec3, Error> {
        flow_at(self, f, p)
    }
}
impl FluidHeightWorld for ItemFluidWorld<'_> {
    type Error = Error;
    fn fluid(&mut self, p: [i32; 3]) -> Result<FluidState, Error> {
        Ok(self.fluids.fluid(self.world.block(p[0], p[1], p[2])))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> Result<bool, Error> {
        let state = self.world.block(p[0], p[1], p[2]);
        Ok(self.fluids.sturdy(state, Direction::Down))
    }
}
impl FluidFlowWorld for ItemFluidWorld<'_> {
    type Error = Error;
    type Block = BlockStateId;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState, Error> {
        Ok(self.fluids.fluid(self.world.block(p.x, p.y, p.z)))
    }
    fn block(&mut self, p: BlockPos) -> Result<BlockStateId, Error> {
        Ok(self.world.block(p.x, p.y, p.z))
    }
    fn blocks_fluid_flow(&self, s: BlockStateId) -> bool {
        self.fluids.blocks_flow(s)
    }
    fn is_ice_class(&self, s: BlockStateId) -> bool {
        self.fluids.ice(s)
    }
    fn is_face_sturdy(
        &mut self,
        s: BlockStateId,
        _: BlockPos,
        d: Direction,
    ) -> Result<bool, Error> {
        Ok(self.fluids.sturdy(s, d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn items_follow_flowing_water_but_still_water_has_no_horizontal_push() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene(&assets, &render, 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let mut state = assets.registry.state(water).unwrap().clone();
        state.properties.insert("level".into(), "1".into());
        let flowing = assets.registry.resolve(&state).unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
                world.set(x, 4, z, water).unwrap();
            }
        }
        let at = Vec3::new(20.5, 4.0, 20.5);
        let item = ItemEntity {
            id: 1,
            health: 5,
            fire_ticks: 0,
            stack: crate::items::ItemStack {
                item: "minecraft:sugar_cane".into(),
                count: 1,
                damage: 0,
                extra: None,
            },
            thrower: None,
            target: None,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            on_ground: false,
            age: 0,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
        };
        let mut still = item.clone();
        assert!(still.tick(&world, &facts, &collision, false, &fluids));
        assert_eq!(still.position.x, at.x);
        assert_eq!(still.velocity.x, 0.0);
        world.set(21, 4, 20, flowing).unwrap();
        let mut carried = item;
        assert!(carried.tick(&world, &facts, &collision, false, &fluids));
        // First push is damped by buoyancy before motion, second follows drag.
        assert!((carried.position.x - at.x - 0.014 * 0.99).abs() < 1e-12);
        assert!((carried.velocity.x - (0.014 * 0.99 * 0.98 + 0.014)).abs() < 1e-12);
        for _ in 0..20 {
            carried.tick(&world, &facts, &collision, false, &fluids);
        }
        assert!(carried.position.x > at.x + 0.5);
    }
}
