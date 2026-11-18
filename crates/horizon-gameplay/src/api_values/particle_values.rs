//! Particle keys cross the game boundary through the load-resolved provider table.
use super::{TryIntoApi, TryIntoEngine};
use horizon_client::particles::{ParticleOptions as EngineOptions, ParticleType};
use mod_api::{__private::ParticleKind, ParticleOptions as ApiOptions};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleValueError {
    UnknownProvider,
    NonFinite,
    InvalidStack,
}

fn finite(options: &ApiOptions) -> bool {
    options.color.into_iter().all(f32::is_finite)
        && options.to_color.into_iter().all(f32::is_finite)
        && options.scale.is_finite()
        && options
            .trail
            .as_ref()
            .is_none_or(|(at, _)| at.iter().all(|v| v.is_finite()))
        && options
            .geyser
            .is_none_or(|(_, impulse)| impulse.is_finite())
        && options.vibration.as_ref().is_none_or(|(source, at, _)| {
            at.iter().all(|v| v.is_finite())
                && match source {
                    mod_api::client::particles::PositionSource::Block(_) => true,
                    mod_api::client::particles::PositionSource::Entity { y_offset, .. } => {
                        y_offset.is_finite()
                    }
                    _ => false,
                }
        })
}
pub trait ParticleItems {
    fn particle_items(&self) -> Option<&horizon_items::items::Items>;
}
impl ParticleItems for () {
    fn particle_items(&self) -> Option<&horizon_items::items::Items> {
        None
    }
}
impl ParticleItems for horizon_items::items::Items {
    fn particle_items(&self) -> Option<&Self> {
        Some(self)
    }
}
impl<C: ParticleItems + ?Sized> TryIntoEngine<C> for ApiOptions {
    type Engine = EngineOptions;
    type Error = ParticleValueError;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error> {
        if !finite(self) {
            return Err(ParticleValueError::NonFinite);
        }
        let (ty, custom) = match mod_api::__private::particle_kind(self.ty)
            .ok_or(ParticleValueError::UnknownProvider)?
        {
            ParticleKind::Vanilla(index) => (
                *ParticleType::ALL
                    .get(index as usize)
                    .ok_or(ParticleValueError::UnknownProvider)?,
                None,
            ),
            ParticleKind::Custom => (
                ParticleType::Smoke,
                Some(std::borrow::Cow::Borrowed(
                    mod_api::__private::particle_name(self.ty)
                        .ok_or(ParticleValueError::UnknownProvider)?,
                )),
            ),
        };
        Ok(EngineOptions {
            polytone_seed: None,
            source: None,
            ty,
            mod_particle: custom,
            block: self.block.clone(),
            color: self.color,
            scale: self.scale,
            to_color: self.to_color,
            trail: self.trail,
            item: self.item.clone(),
            item_stack: self
                .item_stack
                .as_ref()
                .map(|stack| {
                    stack
                        .try_into_engine(
                            context
                                .particle_items()
                                .ok_or(ParticleValueError::InvalidStack)?,
                        )
                        .map_err(|_| ParticleValueError::InvalidStack)
                })
                .transpose()?,
            geyser: self.geyser,
            vibration: self.vibration.clone(),
            legacy_entity_effect: self.legacy_entity_effect,
            legacy_trail_lifetime: self.legacy_trail_lifetime,
        })
    }
}
impl TryIntoApi for EngineOptions {
    type Api = ApiOptions;
    type Error = ParticleValueError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        let key = match self.mod_particle.as_deref() {
            Some(name) => mod_api::__private::particle_named_key(name),
            None => mod_api::__private::particle_vanilla_key(self.ty as u32),
        }
        .ok_or(ParticleValueError::UnknownProvider)?;
        Ok(ApiOptions {
            ty: key,
            block: self.block.clone(),
            color: self.color,
            scale: self.scale,
            to_color: self.to_color,
            trail: self.trail,
            item: self.item.clone(),
            item_stack: self
                .item_stack
                .as_ref()
                .map(|stack| {
                    stack
                        .try_into_api()
                        .map_err(|_| ParticleValueError::InvalidStack)
                })
                .transpose()?,
            geyser: self.geyser,
            vibration: self.vibration.clone(),
            legacy_entity_effect: self.legacy_entity_effect,
            legacy_trail_lifetime: self.legacy_trail_lifetime,
        })
    }
}
impl TryIntoEngine<()> for mod_api::FireworkExplosion {
    type Engine = horizon_items::firework::FireworkExplosion;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, context: &()) -> Result<Self::Engine, Self::Error> {
        Ok(Self::Engine {
            shape: self.shape.try_into_engine(context)?,
            colors: self.colors.clone(),
            fade: self.fade_colors.clone(),
            trail: self.trail,
            twinkle: self.twinkle,
        })
    }
}
