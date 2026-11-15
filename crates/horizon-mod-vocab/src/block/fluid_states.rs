//! Read-only fluid-state definitions from the owning host's loaded registry.
use crate::{
    Backend, BackendRef, BlockStateId, FluidKey, FluidState, Host, StateConstraint, StateProperty,
};

/// A registered fluid's world-state definition, or an identity used only in tanks.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum FluidDefinition {
    /// A world fluid's default and legal serialized property values.
    World {
        /// Default world state.
        default: FluidState,
        /// Property names and legal values.
        properties: Vec<StateProperty>,
    },
    /// A registered fluid without world states.
    TankOnly,
}

/// What a registered fluid is, apart from any level or entity. Vanilla reads the entity
/// behaviours off `FluidTags.WATER` and `FluidTags.LAVA`, so `minecraft:flowing_water` answers as
/// `minecraft:water`. The empty fluid and a tank-only fluid emit no light, nothing can be done in
/// them, their fall modifier scales nothing and they drip nothing and never tick randomly.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct FluidTypeProperties {
    /// `FluidType.getDescriptionId`: `fluid_type.<mod>.<name>`, vanilla's by its block
    /// (`block.minecraft.water`, `block.minecraft.air` for the empty fluid).
    pub description_id: String,
    /// `FluidType.getLightLevel`: what the fluid's block emits, 0 through 15.
    pub light_level: u8,
    /// `FluidType.getFallDistanceModifier`: what an entity's fall distance is multiplied by
    /// each tick in the fluid: water's 0 resets it, lava's 0.5 halves it.
    pub fall_distance_modifier: f32,
    /// `FluidType.canPushEntity`: whether its current moves entities.
    pub can_push_entity: bool,
    /// `FluidType.canSwim`: whether a sprinting entity swims in it.
    pub can_swim: bool,
    /// `FluidType.canDrownIn`: whether an entity's air runs out with its eyes in it.
    pub can_drown: bool,
    /// `FluidType.canExtinguish`: whether it puts out a burning entity.
    pub can_extinguish: bool,
    /// `FluidType.canHydrate`: whether it keeps farmland wet.
    pub can_hydrate: bool,
    /// `FluidType.supportsBoating`: whether a boat floats on it.
    pub supports_boating: bool,
    /// `Fluid.getExplosionResistance`: how much of an explosion's power the fluid soaks up per
    /// block, 100 for water, lava and a mod's world fluid, 0 for the empty fluid and a
    /// tank-only fluid. A flowing form answers as its source.
    pub explosion_resistance: f32,
    /// `Fluid.isRandomlyTicking`: whether the fluid takes random ticks, which only lava does.
    pub randomly_ticks: bool,
    /// `Fluid.getDripParticle`: the particle id a block of the fluid drips beneath it
    /// (`minecraft:dripping_water`, `minecraft:dripping_lava`), none for any other fluid.
    pub drip_particle: Option<String>,
}

/// Why fluid-state constraints could not be validated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum FluidStateError {
    /// No such property, at this input index.
    UnknownProperty(u32),
    /// An illegal property value, at this input index.
    InvalidValue(u32),
    /// A repeated property, at this input index.
    Duplicate(u32),
    /// The host has no loaded world definition, or the fluid is tank-only.
    NoWorldState,
}
impl crate::NoBackend for FluidStateError {
    fn no_backend() -> Self {
        Self::NoWorldState
    }
}

/// Reaches fluid-state facts from any host.
pub trait HostFluidStates {
    /// Facts about registered world and tank-only fluids.
    fn fluid_states(&self) -> FluidStates<'_>;
}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// The fluid-state facts service, available in setup, reload and world callbacks after keys resolve.
// exhaustive: only the host constructs this service
pub struct FluidStates<'a> {
    backend: Option<&'a dyn Backend>,
}
impl<H: Host + ?Sized> HostFluidStates for H {
    fn fluid_states(&self) -> FluidStates<'_> {
        FluidStates {
            backend: self.backend().map(BackendRef::open),
        }
    }
}
impl FluidStates<'_> {
    /// The definition, or none without a resolved registry. Missing prepared world
    /// definitions fail resource loading; they are never reported as tank-only.
    pub fn definition(&self, fluid: FluidKey) -> Option<FluidDefinition> {
        self.backend?.fluid_state_definition(fluid)
    }
    /// The fluid's type properties, or none without a resolved registry or for a key it
    /// doesn't hold. Resolve a name with `Host::fluid_key`, which is none for an unknown one.
    pub fn type_properties(&self, fluid: FluidKey) -> Option<FluidTypeProperties> {
        self.backend?.fluid_type_properties(fluid)
    }
    /// `FluidState.createLegacyBlock`: the block state that holds `state` in the world, a water
    /// or lava block of the matching level, a mod's world fluid's block, or air for the empty
    /// fluid. None for a state no fluid can be in (a tank-only fluid, an amount above 8, a zero
    /// amount for anything but the empty fluid, a source that isn't full) and without a resolved
    /// registry.
    pub fn legacy_block(&self, state: FluidState) -> Option<BlockStateId> {
        self.backend?.fluid_legacy_block(state)
    }
    /// Validate partial constraints in input order, without matching a particular state.
    /// Tank-only or unavailable definitions return `NoWorldState`; use `definition`
    /// to distinguish them. Indices identify the first offending constraint.
    pub fn validate(
        &self,
        fluid: FluidKey,
        constraints: &[StateConstraint],
    ) -> Result<(), FluidStateError> {
        self.backend
            .ok_or(FluidStateError::NoWorldState)?
            .fluid_state_validate(fluid, constraints)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn carried_properties_and_unavailable_host() {
        struct Unavailable;
        impl Host for Unavailable {}
        crate::impl_host_areas!(Unavailable);
        let key = FluidKey(0);
        assert_eq!(Unavailable.fluid_states().definition(key), None);
        assert_eq!(Unavailable.fluid_states().type_properties(key), None);
        assert_eq!(
            Unavailable.fluid_states().legacy_block(FluidState {
                fluid: key,
                amount: 8,
                falling: false,
                source: true,
            }),
            None
        );
        assert_eq!(
            Unavailable.fluid_states().validate(key, &[]),
            Err(FluidStateError::NoWorldState)
        );
        let source = FluidState {
            fluid: key,
            amount: 8,
            falling: true,
            source: true,
        };
        let flowing = FluidState {
            source: false,
            falling: false,
            ..source
        };
        assert!(source.matches(&[StateConstraint::new("falling", "true")]));
        assert!(!source.matches(&[StateConstraint::new("level", "8")]));
        assert!(flowing.matches(&[
            StateConstraint::new("level", "8"),
            StateConstraint::new("falling", "false")
        ]));
        for value in ["08", "+8", "9", "0"] {
            assert!(!flowing.matches(&[StateConstraint::new("level", value)]));
        }
        assert!(!flowing.matches(&[StateConstraint::new("unknown", "true")]));
        let empty = FluidState {
            amount: 0,
            source: false,
            falling: false,
            ..source
        };
        assert!(empty.matches(&[]));
        assert!(!empty.matches(&[StateConstraint::new("falling", "false")]));
        assert!(
            !FluidState {
                amount: u8::MAX,
                ..flowing
            }
            .matches(&[StateConstraint::new("level", "8")])
        );
    }
}
