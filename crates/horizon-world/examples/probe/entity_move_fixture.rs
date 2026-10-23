use crate::support::common::f64_bits;
use crate::support::shapes::bounds;
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape, resolve_shape_movement},
    position::BlockPos,
};
use horizon_world::{
    entity_move::*,
    movement_emission::MovementEmission,
    movement_restitution::{RestitutionInput, RestitutionWorld, restitute_movement},
    registry::BlockStateId,
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vector(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn vec(label: &str, v: Vec3) -> String {
    format!(
        "{label} {:016x} {:016x} {:016x}",
        f64_bits(v.x),
        f64_bits(v.y),
        f64_bits(v.z)
    )
}
fn pos(p: BlockPos) -> String {
    format!("{} {} {}", p.x, p.y, p.z)
}
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    position: Vec3,
    velocity: Vec3,
    stuck: Vec3,
    bounds: Aabb,
    flags: CollisionFlags,
    grounded: bool,
    removed: bool,
    fall: f64,
    history: MovementHistory,
    shapes: Vec<VoxelShape>,
}
impl World<'_> {
    fn flag(&self, n: &str) -> bool {
        self.f[n].as_bool().unwrap()
    }
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.f["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected move failure"))
        } else {
            Ok(())
        }
    }
}
impl EntityMoveWorld for World<'_> {
    type Error = io::Error;
    fn no_physics(&self) -> bool {
        self.flag("no_physics")
    }
    fn position(&self) -> Vec3 {
        self.position
    }
    fn set_position(&mut self, p: Vec3) -> io::Result<()> {
        self.query(vec("set-position", p))?;
        self.bounds = self.bounds.move_by(p.subtract(self.position));
        self.position = p;
        Ok(())
    }
    fn record_movement(&mut self, t: MoverType, v: Vec3) -> io::Result<()> {
        self.query(vec(&format!("record {}", t as u32), v))
    }
    fn collision_flags(&self) -> CollisionFlags {
        self.flags
    }
    fn set_collision_flags(&mut self, f: CollisionFlags) {
        self.flags = f;
    }
    fn limit_piston_movement(&mut self, v: Vec3) -> io::Result<Vec3> {
        self.query(vec("piston", v))?;
        Ok(vector(&self.f["piston_result"]))
    }
    fn stuck_multiplier(&self) -> Vec3 {
        self.stuck
    }
    fn set_stuck_multiplier(&mut self, v: Vec3) {
        self.stuck = v;
    }
    fn velocity(&mut self) -> io::Result<Vec3> {
        self.query("velocity".into())?;
        Ok(self.velocity)
    }
    fn set_velocity(&mut self, v: Vec3) -> io::Result<()> {
        self.query(vec("set-velocity", v))?;
        self.velocity = v;
        Ok(())
    }
    fn back_off_edge(&mut self, v: Vec3, t: MoverType) -> io::Result<Vec3> {
        self.query(vec(&format!("edge {}", t as u32), v))?;
        Ok(if self.flag("edge_override") {
            vector(&self.f["edge_result"])
        } else {
            v
        })
    }
    fn collide(&mut self, v: Vec3) -> io::Result<Vec3> {
        let b = self.bounds.expand_towards(v);
        self.query(bounds("entities", b))?;
        if v.length_squared() == 0.0 {
            return Ok(v);
        }
        self.query("border".into())?;
        self.query(bounds("border-test", b))?;
        self.query(bounds("blocks", b))?;
        Ok(resolve_shape_movement(
            v,
            self.bounds,
            &self.shapes.iter().collect::<Vec<_>>(),
        ))
    }
    fn fall_distance(&self) -> f64 {
        self.fall
    }
    fn fall_reset_clip(&mut self, from: Vec3, to: Vec3) -> io::Result<bool> {
        self.query(vec("clip-from", from))?;
        self.query(vec("clip-to", to))?;
        Ok(self.flag("clip_hit"))
    }
    fn reset_fall_distance(&mut self) -> io::Result<()> {
        self.query("reset-fall".into())?;
        self.fall = 0.0;
        Ok(())
    }
    fn movement_history(&mut self) -> &mut MovementHistory {
        &mut self.history
    }
    fn locally_authoritative(&mut self) -> io::Result<bool> {
        let client = self.client_side()?;
        self.query(
            if client {
                "local-client-authority"
            } else {
                "client-authority"
            }
            .into(),
        )?;
        Ok(if client {
            self.flag("local_client")
        } else {
            !self.flag("client_authority")
        })
    }
    fn set_on_ground_with_movement(
        &mut self,
        below: bool,
        horizontal: bool,
        v: Vec3,
    ) -> io::Result<()> {
        self.query(vec(&format!("ground-set {below} {horizontal}"), v))?;
        self.grounded = below;
        self.flags.horizontal = horizontal;
        Ok(())
    }
    fn minor_horizontal_collision(&mut self, v: Vec3) -> io::Result<bool> {
        self.query(vec("minor", v))?;
        Ok(self.flag("minor_result"))
    }
    fn legacy_on_position(&mut self) -> io::Result<BlockPos> {
        self.query("legacy".into())?;
        Ok(BlockPos::new(17, -64, -33))
    }
    fn block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        self.query(format!("block {}", pos(p)))?;
        Ok(BlockStateId(0))
    }
    fn on_ground(&mut self) -> io::Result<bool> {
        self.query("ground".into())?;
        Ok(self.grounded)
    }
    fn check_fall_damage(
        &mut self,
        dy: f64,
        ground: bool,
        _: BlockStateId,
        p: BlockPos,
    ) -> io::Result<()> {
        self.query(format!("fall {:016x} {ground} {}", f64_bits(dy), pos(p)))?;
        if self.flag("fall_mutates") {
            self.fall = 3.0;
            self.velocity = Vec3::new(17.0, 18.0, 19.0);
            self.flags.horizontal = self.flag("mutated_horizontal");
            self.flags.vertical = self.flag("mutated_vertical");
            self.flags.below = self.flag("mutated_below");
        }
        if self.flag("fall_removes") {
            self.removed = true;
        }
        Ok(())
    }
    fn removed(&mut self) -> io::Result<bool> {
        Ok(self.removed)
    }
    fn can_simulate(&mut self) -> io::Result<bool> {
        self.query("simulation".into())?;
        Ok(true)
    }
    fn restitute(
        &mut self,
        state: BlockStateId,
        x: bool,
        z: bool,
        resolved: Vec3,
    ) -> io::Result<()> {
        let input = RestitutionInput {
            block: state,
            collided_x: x,
            collided_z: z,
            vertical_collision: self.flags.vertical,
            collision_below: self.flags.below,
            living: false,
            resolved_movement: resolved,
            fall_distance: self.fall,
        };
        restitute_movement(&mut Bounce(self), input)
    }
    fn client_side(&mut self) -> io::Result<bool> {
        self.query("client".into())?;
        Ok(self.flag("client"))
    }
    fn movement_emission(&mut self) -> io::Result<MovementEmission> {
        self.query("emission".into())?;
        Ok(if self.flag("emits") {
            MovementEmission::All
        } else {
            MovementEmission::None
        })
    }
    fn passenger(&mut self) -> io::Result<bool> {
        self.query("passenger".into())?;
        Ok(true)
    }
    fn apply_movement_emission(
        &mut self,
        _: MovementEmission,
        _: Vec3,
        _: BlockPos,
        _: BlockStateId,
    ) -> io::Result<()> {
        panic!("passenger guard must skip emission in this fixture")
    }
    fn block_speed_factor(&mut self) -> io::Result<f32> {
        self.query("speed".into())?;
        Ok(single(&self.f["speed"]))
    }
}
struct Bounce<'a, 'b>(&'a mut World<'b>);
impl RestitutionWorld for Bounce<'_, '_> {
    type Error = io::Error;
    fn suppressing_bounce(&mut self) -> io::Result<bool> {
        self.0.query("suppress".into())?;
        Ok(false)
    }
    fn entity_bounciness(&mut self) -> io::Result<f64> {
        self.0.query("entity-bounce".into())?;
        Ok(0.0)
    }
    fn velocity(&mut self) -> io::Result<Vec3> {
        self.0.velocity()
    }
    fn effective_gravity(&mut self) -> io::Result<f64> {
        self.0.query("gravity".into())?;
        Ok(0.08)
    }
    fn block_suppresses_bounce(&mut self, _: BlockStateId) -> io::Result<bool> {
        self.0.query("block-suppresses".into())?;
        Ok(false)
    }
    fn block_bounce_restitution(&mut self, _: BlockStateId) -> io::Result<f32> {
        self.0.query("block-bounce".into())?;
        Ok(0.0)
    }
    fn air_drag(&mut self) -> io::Result<f32> {
        panic!("no positive restitution")
    }
    fn movement_support_position(&mut self) -> io::Result<BlockPos> {
        panic!("no bounce event")
    }
    fn bounce_on(&mut self, _: BlockStateId, _: BlockPos, _: f64) -> io::Result<()> {
        panic!("no bounce event")
    }
    fn bounce_event(&mut self) -> io::Result<()> {
        panic!("no bounce event")
    }
    fn mark_position_for_sync(&mut self) {
        panic!("no bounce event")
    }
    fn set_velocity(&mut self, v: Vec3) -> io::Result<()> {
        self.0.set_velocity(v)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let p = vector(&f["position"]);
        let flag = |n: &str| f[n].as_bool().unwrap();
        let shapes = f["shapes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                VoxelShape::from_box(Aabb::new(
                    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])),
                    Vec3::new(number(&v[3]), number(&v[4]), number(&v[5])),
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut history = MovementHistory::default();
        for i in 0..f["history"].as_u64().unwrap() {
            history.add(MovementRecord {
                from: Vec3::new(i as f64, 0.0, 0.0),
                to: Vec3::new((i + 1) as f64, 0.0, 0.0),
                original: Some(Vec3::new(1.0, 0.0, 0.0)),
            });
        }
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            position: p,
            velocity: vector(&f["velocity"]),
            stuck: vector(&f["stuck"]),
            bounds: Aabb::new(
                Vec3::new(p.x - 0.3, p.y, p.z - 0.3),
                Vec3::new(p.x + 0.3, p.y + 1.8, p.z + 0.3),
            ),
            flags: CollisionFlags {
                horizontal: flag("horizontal"),
                vertical: flag("vertical"),
                below: flag("below"),
                minor: flag("minor"),
            },
            grounded: flag("ground"),
            removed: flag("removed"),
            fall: number(&f["distance"]),
            history,
            shapes,
        };
        let mover = [
            MoverType::SelfMovement,
            MoverType::Player,
            MoverType::Piston,
            MoverType::ShulkerBox,
            MoverType::Shulker,
        ][f["mover"].as_u64().unwrap() as usize];
        let result = move_entity(&mut w, mover, vector(&f["movement"]));
        writeln!(w.out, "{}", if result.is_ok() { "ok" } else { "error" })?;
        writeln!(w.out, "{}", vec("position", w.position))?;
        writeln!(w.out, "{}", vec("velocity-final", w.velocity))?;
        writeln!(w.out, "{}", vec("stuck", w.stuck))?;
        writeln!(w.out, "{}", bounds("bounds", w.bounds))?;
        writeln!(
            w.out,
            "flags {} {} {} {} {} {} {:016x}",
            w.flags.horizontal,
            w.flags.vertical,
            w.flags.below,
            w.flags.minor,
            w.grounded,
            w.removed,
            f64_bits(w.fall)
        )?;
        writeln!(w.out, "history {}", w.history.records().len())?;
        for r in w.history.records() {
            writeln!(w.out, "{}", vec("from", r.from))?;
            writeln!(w.out, "{}", vec("to", r.to))?;
            writeln!(
                w.out,
                "{}",
                r.original
                    .map_or_else(|| "original none".to_string(), |v| vec("original", v))
            )?;
        }
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
