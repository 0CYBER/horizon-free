//! Movement distances, footsteps, swimming and flapping event dispatch.
use horizon_core::{geometry::Vec3, position::BlockPos};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementEmission {
    None,
    Sounds,
    Events,
    All,
}
impl MovementEmission {
    pub fn sounds(self) -> bool {
        matches!(self, Self::Sounds | Self::All)
    }
    pub fn events(self) -> bool {
        matches!(self, Self::Events | Self::All)
    }
    pub fn anything(self) -> bool {
        self != Self::None
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct MovementDistances {
    pub movement: f32,
    pub flight: f32,
    pub next_step: f32,
}
impl MovementDistances {
    /// Java narrows to int, adds with wrapping overflow, then widens to float.
    pub fn next_step(&self) -> f32 {
        (self.movement as i32).wrapping_add(1) as f32
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementEvent {
    Step,
    Swim,
    Flap,
}
pub trait MovementEmissionWorld {
    type Error;
    type State: Copy;
    fn distances(&self) -> MovementDistances;
    fn set_movement_distance(&mut self, value: f32);
    fn set_flight_distance(&mut self, value: f32);
    fn set_next_step(&mut self, value: f32);
    fn on_position(&mut self) -> Result<BlockPos, Self::Error>;
    fn block_state(&mut self, p: BlockPos) -> Result<Self::State, Self::Error>;
    fn is_air(&mut self, s: Self::State) -> Result<bool, Self::Error>;
    fn climbable_tag(&mut self, s: Self::State) -> Result<bool, Self::Error>;
    fn is_powder_snow(&mut self, s: Self::State) -> Result<bool, Self::Error>;
    fn on_ground(&mut self) -> Result<bool, Self::Error>;
    fn crouching(&mut self) -> Result<bool, Self::Error>;
    fn on_rails(&mut self) -> Result<bool, Self::Error>;
    fn swimming(&mut self) -> Result<bool, Self::Error>;
    fn in_water(&mut self) -> Result<bool, Self::Error>;
    fn walking_step_sound(&mut self, p: BlockPos, s: Self::State) -> Result<(), Self::Error>;
    /// STEP uses current entity position and a context including this state.
    fn step_event(&mut self, s: Self::State) -> Result<(), Self::Error>;
    fn next_step(&mut self) -> Result<f32, Self::Error>;
    fn water_swim_sound(&mut self) -> Result<(), Self::Error>;
    fn entity_event(&mut self, event: MovementEvent) -> Result<(), Self::Error>;
    fn flapping(&mut self) -> Result<bool, Self::Error>;
    fn on_flap(&mut self) -> Result<(), Self::Error>;
    fn movement_emission(&mut self) -> Result<MovementEmission, Self::Error>;
}
fn climbable<W: MovementEmissionWorld>(world: &mut W, state: W::State) -> Result<bool, W::Error> {
    Ok(world.climbable_tag(state)? || world.is_powder_snow(state)?)
}
pub fn block_movement_effects<W: MovementEmissionWorld>(
    world: &mut W,
    p: BlockPos,
    state: W::State,
    sounds: bool,
    events: bool,
    movement: Vec3,
) -> Result<bool, W::Error> {
    if world.is_air(state)? {
        return Ok(false);
    }
    let climbable = climbable(world, state)?;
    if (world.on_ground()?
        || climbable
        || world.crouching()? && movement.y == 0.0
        || world.on_rails()?)
        && !world.swimming()?
    {
        if sounds {
            world.walking_step_sound(p, state)?;
        }
        if events {
            world.step_event(state)?;
        }
        Ok(true)
    } else {
        Ok(false)
    }
}
pub fn process_flapping_movement<W: MovementEmissionWorld>(world: &mut W) -> Result<(), W::Error> {
    if world.flapping()? {
        world.on_flap()?;
        if world.movement_emission()?.events() {
            world.entity_event(MovementEvent::Flap)?;
        }
    }
    Ok(())
}
/// The caller applies Entity.move's authority, emission and passenger guards.
/// Provider callbacks can change distance fields; subsequent reads see changes.
pub fn apply_movement_emission<W: MovementEmissionWorld>(
    world: &mut W,
    emission: MovementEmission,
    movement: Vec3,
    legacy_position: BlockPos,
    legacy_state: W::State,
) -> Result<(), W::Error> {
    let full = (movement.length() * f64::from(0.6f32)) as f32;
    let horizontal =
        ((movement.x * movement.x + movement.z * movement.z).sqrt() * f64::from(0.6f32)) as f32;
    let p = world.on_position()?;
    let state = world.block_state(p)?;
    let climbable = climbable(world, state)?;
    world.set_movement_distance(
        world.distances().movement + if climbable { full } else { horizontal },
    );
    world.set_flight_distance(world.distances().flight + full);
    if world.distances().movement > world.distances().next_step && !world.is_air(state)? {
        let same = p == legacy_position;
        // The native legacy call uses `same` for events even for sound-only emission.
        let mut emitted = block_movement_effects(
            world,
            legacy_position,
            legacy_state,
            emission.sounds(),
            same,
            movement,
        )?;
        if !same {
            emitted |= block_movement_effects(world, p, state, false, emission.events(), movement)?;
        }
        if emitted {
            let next = world.next_step()?;
            world.set_next_step(next);
        } else if world.in_water()? {
            let next = world.next_step()?;
            world.set_next_step(next);
            if emission.sounds() {
                world.water_swim_sound()?;
            }
            if emission.events() {
                world.entity_event(MovementEvent::Swim)?;
            }
        }
    } else if world.is_air(state)? {
        process_flapping_movement(world)?;
    }
    Ok(())
}
