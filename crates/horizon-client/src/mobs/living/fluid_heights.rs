//! A mob's fluid heights and the fluid at its eyes (`Entity.updateFluidHeightAndDoFluidPushing`,
//! `updateFluidOnEyes`), as its tick reads them over the scene.
use super::super::MobWorld;
use super::Fluids;
use crate::blocks::Fluid;
use horizon_core::geometry::{Aabb, Vec3};

impl MobWorld<'_> {
    /// FluidState.getHeight: a full block under the same fluid, else amount / 9.
    pub fn fluid_height(&self, x: i32, y: i32, z: i32) -> (Fluid, f64) {
        let f = self.facts.get(self.world.block(x, y, z));
        if f.fluid == Fluid::None {
            return (Fluid::None, 0.0);
        }
        let above = self.facts.get(self.world.block(x, y + 1, z));
        if above.fluid == f.fluid {
            return (f.fluid, 1.0);
        }
        (f.fluid, f64::from(f.fluid_level) / 9.0)
    }
    /// Entity.updateFluidHeightAndDoFluidPushing heights over the deflated box,
    /// measured from the box's own bottom.
    pub fn fluids(&self, bounds: Aabb, eye_y: f64) -> Fluids {
        self.fluids_from(bounds, bounds.min_position().y, eye_y)
    }
    /// [`Self::fluids`] for a fluid interaction box another than the entity's
    /// (a boat passenger's): `EntityFluidInteraction.update` finds the fluids
    /// in `bounds` deflated by 0.001 and measures each depth from `bottom`, the
    /// entity's own box bottom (`getBoundingBox().minY`).
    pub fn fluids_from(&self, bounds: Aabb, bottom: f64, eye_y: f64) -> Fluids {
        let b = bounds.deflate(Vec3::new(0.001, 0.001, 0.001));
        let (lo, hi) = (b.min_position(), b.max_position());
        let mut out = Fluids::default();
        // The eye column below, in the box but for a box thinner than the
        // deflation.
        let (ex, ez) = (
            ((lo.x + hi.x) / 2.0).floor() as i32,
            ((lo.z + hi.z) / 2.0).floor() as i32,
        );
        // Most boxes are in sections without a fluid: no cell to read.
        let cells = (
            [
                (lo.x.floor() as i32).min(ex),
                lo.y.floor() as i32,
                (lo.z.floor() as i32).min(ez),
            ],
            [
                (hi.x.ceil() as i32 - 1).max(ex),
                hi.y.ceil() as i32 - 1,
                (hi.z.ceil() as i32 - 1).max(ez),
            ],
        );
        if !self
            .world
            .may_hold(cells.0, cells.1, |s| self.facts.get(s).fluid != Fluid::None)
        {
            return out;
        }
        for x in lo.x.floor() as i32..=(hi.x.ceil() as i32 - 1) {
            for y in lo.y.floor() as i32..=(hi.y.ceil() as i32 - 1) {
                for z in lo.z.floor() as i32..=(hi.z.ceil() as i32 - 1) {
                    let (fluid, h) = self.fluid_height(x, y, z);
                    if fluid == Fluid::None {
                        continue;
                    }
                    let top = f64::from(y) + h;
                    if top >= lo.y {
                        let depth = top - bottom;
                        match fluid {
                            Fluid::Water => {
                                out.water = out.water.max(depth);
                                out.in_water = true;
                            }
                            Fluid::Lava => {
                                out.lava = out.lava.max(depth);
                                out.in_lava = true;
                            }
                            // A mod's fluid is in neither tag: no entity swims in it.
                            Fluid::None | Fluid::Modded => {}
                        }
                    }
                }
            }
        }
        // EntityFluidInteraction.update: the eyes are in a fluid block of
        // the box in the entity's column, between its bottom and its height
        // for the camera, for water and for lava.
        for y in lo.y.floor() as i32..=(hi.y.ceil() as i32 - 1) {
            let (fluid, h) = self.fluid_height(ex, y, ez);
            // `updateFluidOnEyes` tracks every fluid tag: water and lava.
            let eyes = match fluid {
                Fluid::Water => &mut out.eye_in_water,
                Fluid::Lava => &mut out.eye_in_lava,
                Fluid::None | Fluid::Modded => continue,
            };
            if f64::from(y) + h < lo.y || eye_y < f64::from(y) {
                continue;
            }
            let state = self.world.block(ex, y, ez);
            let source = self
                .registry
                .state(state)
                .and_then(|s| s.properties.get("level"))
                .is_none_or(|l| l == "0");
            let above = self.facts.get(self.world.block(ex, y + 1, ez));
            let camera = if source && above.sturdy_full & 1 != 0 {
                1.0
            } else {
                h
            };
            if eye_y <= f64::from(y) + camera {
                *eyes = true;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::Mob;
    use horizon_core::geometry::{Aabb, Vec3};
    use std::sync::Arc;

    /// Lava sources fill x 0..=4 and water sources x 5..=9, z 0..=4, y 0..=2: each surface is
    /// at 2 + 8/9. A zombie (1.95 tall, 0.6 wide) standing at y 1 is that surface less 1 deep,
    /// measured from its own bottom, not the box deflated by 0.001: in lava, in water, in both
    /// at once where the two meet (a tie), and in neither out at x 12.5. A boat passenger's
    /// raised box still measures from the passenger's bottom. Its eyes (y + 1.74) are in lava
    /// at y 1, and in air at y 2.5 while its feet stay in.
    #[test]
    #[ignore = "daily"]
    fn a_mob_measures_fluids_from_its_bottom_and_tracks_lava_at_its_eyes() {
        let build = |world: &mut crate::world::World,
                     registry: &horizon_world::registry::BlockRegistry| {
            world
                .chunks
                .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
            let lava = registry.default_state("minecraft:lava").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            for x in 0..=9 {
                for z in 0..=4 {
                    for y in 0..=2 {
                        world
                            .set(x, y, z, if x <= 4 { lava } else { water })
                            .unwrap();
                    }
                }
            }
        };
        let mut seen = None;
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = Arc::new(types.types["minecraft:zombie"].clone());
            let mob = |x: f64, y: f64| Mob::new(1, kind.clone(), Vec3::new(x, y, 2.5), 0.0);
            let at = |x: f64, y: f64| {
                let m = mob(x, y);
                w.fluids(m.bounds(), y + m.eye_height())
            };
            // A passenger box raised to a boat top at 2.5, as `fluid_interaction` builds it.
            let passenger = mob(7.5, 1.0);
            let b = passenger.bounds();
            let raised = Aabb::new(
                Vec3::new(b.min_position().x, 2.5 - 0.001, b.min_position().z),
                b.max_position(),
            );
            let boat = w.fluids_from(raised, b.min_position().y, 1.0 + passenger.eye_height());
            seen = Some([
                at(2.5, 1.0),
                at(7.5, 1.0),
                at(5.0, 1.0),
                at(12.5, 1.0),
                at(2.5, 2.5),
                boat,
            ]);
        });
        if !ran {
            eprintln!("Prepared mob data unavailable; the mob fluid checks did not run");
            return;
        }
        let [lava, water, tie, neither, shallow, boat] = seen.expect("the world ran");
        let depth = 2.0 + 8.0 / 9.0 - 1.0;
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(lava.in_lava && !lava.in_water && near(lava.lava, depth) && lava.water == 0.0);
        assert!(lava.eye_in_lava && !lava.eye_in_water);
        assert!(water.in_water && !water.in_lava && near(water.water, depth) && water.lava == 0.0);
        assert!(tie.in_water && tie.in_lava && near(tie.water, depth) && near(tie.lava, depth));
        assert!(!neither.in_water && !neither.in_lava);
        assert_eq!((neither.water, neither.lava), (0.0, 0.0));
        assert!(shallow.in_lava && !shallow.eye_in_lava);
        assert!(near(shallow.lava, 2.0 + 8.0 / 9.0 - 2.5));
        assert!(boat.in_water && near(boat.water, depth));
    }
}
