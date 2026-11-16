//! Engine particle provider options, independent of API registry keys.
use super::ParticleType;
use mod_api::client::particles::PositionSource;
/// `ParticleOptions` with the parameters of the option classes used here.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods use the constructors
pub struct ParticleOptions {
    pub ty: ParticleType,
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
    /// Unresolved stack for server particle delivery.
    pub item_stack: Option<horizon_items::items::ItemStack>,
    /// `GeyserParticleOptions.waterBlocks` and `GeyserBaseParticleOptions`'
    /// `waterBlocks` and `burstImpulseBase` (0 where the type has none).
    pub geyser: Option<(i32, f32)>,
    /// `VibrationParticleOption`: the listener's position source, the resolved
    /// point the signal flies to and its arrival time in ticks. The source is
    /// sent to clients; the resolved point is used for local rendering
    /// (`ParticleEngine::vibration` is the same particle).
    pub vibration: Option<(PositionSource, [f64; 3], i32)>,
    /// A native mod's own particle type, by its namespaced name (`mymod:spark`):
    /// `SimpleParticleType` plus the provider `particles/<name>.json` names.
    /// API requests borrow the immutable load table; native file input can own its name.
    pub mod_particle: Option<std::borrow::Cow<'static, str>>,
    /// Pre-1.20.5 entity-effect providers use spawn velocity as RGB; true is ambient.
    pub legacy_entity_effect: Option<bool>,
    /// A pre-1.21.4 server's trail, which had no duration: TrailParticle.Provider chose
    /// `nextInt(40) + 10` after constructing the particle. Set by the client for an older
    /// server only, never read from the wire.
    pub legacy_trail_lifetime: bool,
    pub source: Option<std::sync::Arc<str>>,
    pub polytone_seed: Option<std::sync::Arc<super::polytone::Seed>>,
}
impl ParticleOptions {
    pub fn simple(ty: ParticleType) -> Self {
        Self {
            ty,
            block: None,
            color: [1.0; 4],
            scale: 1.0,
            to_color: [1.0, 0.0, 0.0],
            trail: None,
            item: None,
            item_stack: None,
            geyser: None,
            vibration: None,
            mod_particle: None,
            legacy_entity_effect: None,
            legacy_trail_lifetime: false,
            polytone_seed: None,
            source: None,
        }
    }
    /// A native mod's `SimpleParticleType`: the sprite set of
    /// `assets/<namespace>/particles/<path>.json`, drawn as a plain billboard.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            mod_particle: Some(std::borrow::Cow::Owned(name.into())),
            ..Self::simple(ParticleType::Smoke)
        }
    }
    /// `GeyserParticleOptions(type, waterBlocks)`.
    pub fn geyser(ty: ParticleType, water_blocks: i32) -> Self {
        Self {
            geyser: Some((water_blocks, 0.0)),
            ..Self::simple(ty)
        }
    }
    /// `GeyserBaseParticleOptions(type, waterBlocks, burstImpulseBase)`.
    pub fn geyser_base(ty: ParticleType, water_blocks: i32, burst_impulse_base: f32) -> Self {
        Self {
            geyser: Some((water_blocks, burst_impulse_base)),
            ..Self::simple(ty)
        }
    }
    /// `VibrationParticleOption(destination, arrivalInTicks)`: the listener's
    /// position source and the resolved point this engine flies the signal to.
    pub fn vibration(source: PositionSource, dest: [f64; 3], ticks: i32) -> Self {
        Self {
            vibration: Some((source, dest, ticks)),
            ..Self::simple(ParticleType::Vibration)
        }
    }
    /// `ItemParticleOption`: the stack's layer particle sprites (`BreakingItemParticle`).
    pub fn item(layers: Vec<String>) -> Self {
        Self {
            item: Some(layers),
            ..Self::simple(ParticleType::Item)
        }
    }
    /// `DustColorTransitionOptions`.
    pub fn dust_transition(from: [f32; 3], to: [f32; 3], scale: f32) -> Self {
        Self {
            color: [from[0], from[1], from[2], 1.0],
            to_color: to,
            scale: scale.clamp(0.01, 4.0),
            ..Self::simple(ParticleType::DustColorTransition)
        }
    }
    /// `DustColorTransitionOptions.SCULK_TO_REDSTONE`.
    pub fn sculk_to_redstone() -> Self {
        let rgb = |c: u32| {
            [
                ((c >> 16) & 255) as f32 / 255.0,
                ((c >> 8) & 255) as f32 / 255.0,
                (c & 255) as f32 / 255.0,
            ]
        };
        Self::dust_transition(rgb(3_790_560), rgb(16_711_680), 1.0)
    }
    /// `SculkChargeParticleOptions(roll)`.
    pub fn sculk_charge(roll: f32) -> Self {
        Self {
            scale: roll,
            ..Self::simple(ParticleType::SculkCharge)
        }
    }
    /// `ShriekParticleOption(delay)`.
    pub fn shriek(delay: i32) -> Self {
        Self {
            scale: delay as f32,
            ..Self::simple(ParticleType::Shriek)
        }
    }
    /// `TrailParticleOption(target, color, duration)`.
    pub fn trail(target: [f64; 3], rgb: u32, duration: i32) -> Self {
        Self {
            color: [
                ((rgb >> 16) & 255) as f32 / 255.0,
                ((rgb >> 8) & 255) as f32 / 255.0,
                (rgb & 255) as f32 / 255.0,
                1.0,
            ],
            trail: Some((target, duration)),
            ..Self::simple(ParticleType::Trail)
        }
    }
    pub fn block(ty: ParticleType, sprite: String, tint: u32) -> Self {
        Self {
            block: Some((sprite, tint)),
            ..Self::simple(ty)
        }
    }
    /// `DustParticleOptions(color, scale)`.
    pub fn dust(rgb: [f32; 3], scale: f32) -> Self {
        Self {
            color: [rgb[0], rgb[1], rgb[2], 1.0],
            scale: scale.clamp(0.01, 4.0),
            ..Self::simple(ParticleType::Dust)
        }
    }
    pub fn colored(ty: ParticleType, rgba: [f32; 4]) -> Self {
        Self {
            color: rgba,
            ..Self::simple(ty)
        }
    }
}
