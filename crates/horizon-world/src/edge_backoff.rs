//! Player crouching edge backoff and its lazy collision-free probes.
use horizon_core::geometry::{Aabb, Vec3};

pub trait EdgeBackoffWorld {
    type Error;
    fn max_up_step(&mut self) -> Result<f32, Self::Error>;
    fn staying_on_ground_surface(&mut self) -> Result<bool, Self::Error>;
    fn on_ground(&mut self) -> Result<bool, Self::Error>;
    fn no_collision(&mut self, bounds: Aabb) -> Result<bool, Self::Error>;
}
pub struct EdgeBackoffInput {
    pub movement: Vec3,
    pub bounds: Aabb,
    pub flying: bool,
    /// True only for native SELF and PLAYER mover types.
    pub self_or_player: bool,
    pub fall_distance: f64,
    pub query_budget: usize,
}
#[derive(Debug)]
pub enum EdgeBackoffError<E> {
    World(E),
    QueryBudget,
}
impl<E: std::fmt::Display> std::fmt::Display for EdgeBackoffError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::QueryBudget => f.write_str("edge backoff collision query budget exhausted"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for EdgeBackoffError<E> {}
fn signum(v: f64) -> f64 {
    if v == 0.0 || v.is_nan() {
        v
    } else {
        v.signum()
    }
}
fn decrease(v: f64, step: f64) -> f64 {
    if v.abs() <= 0.05 { 0.0 } else { v - step }
}
/// The budget bounds pathological/nonconverging motion. Exhaustion is an error,
/// never a successful clamped displacement; earlier provider effects remain.
pub fn back_off_from_edge<W: EdgeBackoffWorld>(
    world: &mut W,
    input: EdgeBackoffInput,
) -> Result<Vec3, EdgeBackoffError<W::Error>> {
    let step = f64::from(world.max_up_step().map_err(EdgeBackoffError::World)?);
    if input.flying
        || input.movement.y > 0.0
        || !input.self_or_player
        || !world
            .staying_on_ground_surface()
            .map_err(EdgeBackoffError::World)?
    {
        return Ok(input.movement);
    }
    let grounded = world.on_ground().map_err(EdgeBackoffError::World)?;
    let mut queries = 0usize;
    let mut can_fall =
        |dx: f64, dz: f64, amount: f64| -> Result<bool, EdgeBackoffError<W::Error>> {
            if queries == input.query_budget {
                return Err(EdgeBackoffError::QueryBudget);
            }
            queries += 1;
            let min = input.bounds.min_position();
            let max = input.bounds.max_position();
            let bounds = Aabb::new(
                Vec3::new(
                    min.x + 1.0e-7 + dx,
                    min.y - amount - 1.0e-7,
                    min.z + 1.0e-7 + dz,
                ),
                Vec3::new(max.x - 1.0e-7 + dx, min.y, max.z - 1.0e-7 + dz),
            );
            world.no_collision(bounds).map_err(EdgeBackoffError::World)
        };
    let should_back_off = grounded
        || (input.fall_distance < step && !can_fall(0.0, 0.0, step - input.fall_distance)?);
    if !should_back_off {
        return Ok(input.movement);
    }
    let mut x = input.movement.x;
    let mut z = input.movement.z;
    let dx = signum(x) * 0.05;
    let dz = signum(z) * 0.05;
    while x != 0.0 && can_fall(x, 0.0, step)? {
        if x.abs() <= 0.05 {
            x = 0.0;
            break;
        }
        x -= dx;
    }
    while z != 0.0 && can_fall(0.0, z, step)? {
        if z.abs() <= 0.05 {
            z = 0.0;
            break;
        }
        z -= dz;
    }
    while x != 0.0 && z != 0.0 && can_fall(x, z, step)? {
        x = decrease(x, dx);
        z = decrease(z, dz);
    }
    Ok(Vec3::new(x, input.movement.y, z))
}
