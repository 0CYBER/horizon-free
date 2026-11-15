//! Owned particle requests, with provider identities resolved once when content loads.
#[allow(unused_imports)]
use crate::*;

/// The listener of a vibration signal (`VibrationParticleOption`), independent
/// of its resolved rendering destination and of the network protocol.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PositionSource {
    /// A block listener, addressed by its block coordinates (not its centre).
    Block([i32; 3]),
    /// An entity listener, addressed by its network id and an offset above its feet.
    Entity {
        /// The entity's network id, preserved as the protocol's signed value.
        id: i32,
        /// The vertical offset from the entity's feet, in blocks.
        y_offset: f32,
    },
}

/// `ParticleOptions` with the parameters of the option classes used here.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods use the constructors
pub struct ParticleOptions {
    /// Provider identity issued by `Registrar::particle_key`, never an engine enum.
    pub ty: ParticleKey,
    /// `BlockParticleOption`: the block's particle sprite and terrain tint.
    pub block: Option<(String, u32)>,
    /// RGB(A) for dust, spells, entity effects and tinted leaves.
    pub color: [f32; 4],
    /// `DustParticleOptions.scale`, `PowerParticleOption.power`, `SpellParticleOption.power`,
    /// `SculkChargeParticleOptions.roll`, `ShriekParticleOption.delay`.
    pub scale: f32,
    /// `DustColorTransitionOptions.toColor` (`color` is the from colour).
    pub to_color: [f32; 3],
    /// `TrailParticleOption`: the point it flies to and how many ticks it takes.
    pub trail: Option<([f64; 3], i32)>,
    /// `ItemParticleOption`: the particle sprite of each layer of the item's
    /// model as resolved for the ground.
    pub item: Option<Vec<String>>,
    /// Stack sent by `ServerLevel.sendParticles`; clients resolve its ground model at seed 0.
    /// Invalid, unknown or oversized stacks cause the send to return false.
    pub item_stack: Option<crate::ItemStack>,
    /// `GeyserParticleOptions.waterBlocks` and `GeyserBaseParticleOptions`'
    /// `waterBlocks` and `burstImpulseBase` (0 where the type has none).
    pub geyser: Option<(i32, f32)>,
    /// `VibrationParticleOption`: the listener's position source, the resolved
    /// point the signal flies to and its arrival time in ticks. The source is
    /// sent to clients; the resolved point is used for local rendering
    /// (`ParticleEngine::vibration` is the same particle).
    pub vibration: Option<(PositionSource, [f64; 3], i32)>,
    /// Older entity-effect providers use velocity as RGB; true is ambient.
    pub legacy_entity_effect: Option<bool>,
    /// Older trails choose their lifetime in the provider rather than on the wire.
    pub legacy_trail_lifetime: bool,
}
impl ParticleOptions {
    /// Construct an option for a retained vanilla or custom sprite-provider key.
    pub fn simple(ty: ParticleKey) -> Self {
        Self {
            ty,
            legacy_entity_effect: None,
            legacy_trail_lifetime: false,
            block: None,
            color: [1.0; 4],
            scale: 1.0,
            to_color: [1.0, 0.0, 0.0],
            trail: None,
            item: None,
            item_stack: None,
            geyser: None,
            vibration: None,
        }
    }
    /// `GeyserParticleOptions(type, waterBlocks)`.
    pub fn geyser(ty: ParticleKey, water_blocks: i32) -> Self {
        Self {
            geyser: Some((water_blocks, 0.0)),
            ..Self::simple(ty)
        }
    }
    /// `GeyserBaseParticleOptions(type, waterBlocks, burstImpulseBase)`.
    pub fn geyser_base(ty: ParticleKey, water_blocks: i32, burst_impulse_base: f32) -> Self {
        Self {
            geyser: Some((water_blocks, burst_impulse_base)),
            ..Self::simple(ty)
        }
    }
    /// `VibrationParticleOption(destination, arrivalInTicks)`: the listener's
    /// position source and the resolved point this engine flies the signal to.
    pub fn vibration(ty: ParticleKey, source: PositionSource, dest: [f64; 3], ticks: i32) -> Self {
        Self {
            vibration: Some((source, dest, ticks)),
            ..Self::simple(ty)
        }
    }
    /// `ItemParticleOption`: the stack's layer particle sprites (`BreakingItemParticle`).
    pub fn item(ty: ParticleKey, layers: Vec<String>) -> Self {
        Self {
            item: Some(layers),
            ..Self::simple(ty)
        }
    }
    /// `DustColorTransitionOptions`.
    pub fn dust_transition(ty: ParticleKey, from: [f32; 3], to: [f32; 3], scale: f32) -> Self {
        Self {
            color: [from[0], from[1], from[2], 1.0],
            to_color: to,
            scale: scale.clamp(0.01, 4.0),
            ..Self::simple(ty)
        }
    }
    /// `DustColorTransitionOptions.SCULK_TO_REDSTONE`.
    pub fn sculk_to_redstone(ty: ParticleKey) -> Self {
        let rgb = |c: u32| {
            [
                ((c >> 16) & 255) as f32 / 255.0,
                ((c >> 8) & 255) as f32 / 255.0,
                (c & 255) as f32 / 255.0,
            ]
        };
        Self::dust_transition(ty, rgb(3_790_560), rgb(16_711_680), 1.0)
    }
    /// `SculkChargeParticleOptions(roll)`.
    pub fn sculk_charge(ty: ParticleKey, roll: f32) -> Self {
        Self {
            scale: roll,
            ..Self::simple(ty)
        }
    }
    /// `ShriekParticleOption(delay)`.
    pub fn shriek(ty: ParticleKey, delay: i32) -> Self {
        Self {
            scale: delay as f32,
            ..Self::simple(ty)
        }
    }
    /// `TrailParticleOption(target, color, duration)`.
    pub fn trail(ty: ParticleKey, target: [f64; 3], rgb: u32, duration: i32) -> Self {
        Self {
            color: [
                ((rgb >> 16) & 255) as f32 / 255.0,
                ((rgb >> 8) & 255) as f32 / 255.0,
                (rgb & 255) as f32 / 255.0,
                1.0,
            ],
            trail: Some((target, duration)),
            ..Self::simple(ty)
        }
    }
    pub fn block(ty: ParticleKey, sprite: String, tint: u32) -> Self {
        Self {
            block: Some((sprite, tint)),
            ..Self::simple(ty)
        }
    }
    /// `DustParticleOptions(color, scale)`.
    pub fn dust(ty: ParticleKey, rgb: [f32; 3], scale: f32) -> Self {
        Self {
            color: [rgb[0], rgb[1], rgb[2], 1.0],
            scale: scale.clamp(0.01, 4.0),
            ..Self::simple(ty)
        }
    }
    pub fn colored(ty: ParticleKey, rgba: [f32; 4]) -> Self {
        Self {
            color: rgba,
            ..Self::simple(ty)
        }
    }
}

/// One explosion of a [`FireworksRequest`] (a firework star's `FireworkExplosion`): the
/// pattern its sparks fly out in and how they look.
#[derive(horizon_mod_api_macros::Wire, Clone, Debug, Default, PartialEq)]
// exhaustive: mods build it with a literal, as a firework star's components spell it
pub struct FireworkExplosion {
    /// The pattern the sparks fly out in.
    pub shape: FireworkShape,
    /// The sparks' colours as `0xRRGGBB`, one picked at random for each spark; none draws
    /// black sparks, as vanilla's (`DyeColor.BLACK.getFireworkColor`).
    pub colors: Vec<u32>,
    /// The colours the sparks fade to as `0xRRGGBB`, one picked at random for each spark;
    /// none keeps their colour.
    pub fade_colors: Vec<u32>,
    /// Each spark leaves a trail of smaller sparks (`hasTrail`).
    pub trail: bool,
    /// The sparks flicker, and a crackle follows the burst (`hasTwinkle`).
    pub twinkle: bool,
}

/// A firework's explosions drawn in this client's world, for
/// [`crate::ClientHost::create_fireworks`] (`ClientLevel.createFireworks`): the picture and
/// the sound of a rocket bursting, with no rocket.
#[derive(horizon_mod_api_macros::Wire, Clone, Debug, Default, PartialEq)]
// exhaustive: mods build it with a literal
pub struct FireworksRequest {
    /// Where it bursts, in blocks.
    pub at: [f64; 3],
    /// The rocket's velocity in blocks per tick, which the sparks carry.
    pub velocity: [f64; 3],
    /// The explosions, one every other tick in this order; none puffs a little smoke, as a
    /// rocket without stars does. At most 256, as vanilla's fireworks component.
    pub explosions: Vec<FireworkExplosion>,
    /// The blast, and the crackle of a twinkling explosion, play; false bursts silently, as a
    /// silent rocket does (`!FireworkRocketEntity.isSilent()`).
    pub audible: bool,
}
