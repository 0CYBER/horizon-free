//! Bind health calculations to registered entity attributes and mutable data.
use crate::{
    attributes::entity::{AttributeId, EntityAttributes, LookupError},
    living_health::{self, HealthWorld, SyncedFloat},
};
use std::fmt;

#[derive(Debug)]
pub enum HealthError<E> {
    Attribute(LookupError),
    Observer(E),
}
impl<E: fmt::Display> fmt::Display for HealthError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attribute(e) => write!(f, "health attribute lookup: {e:?}"),
            Self::Observer(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for HealthError<E> {}
pub trait HealthObserver {
    type Error;
    fn updated(&mut self, health: &mut SyncedFloat) -> Result<(), Self::Error>;
}
/// A borrow of living state, without inventing spawn or save initialization.
/// Absorption here uses base LivingEntity field storage. Player-specific
/// synchronized absorption requires a different storage binding.
pub struct AttributeHealth<'a, O> {
    attributes: &'a mut EntityAttributes,
    health: &'a mut SyncedFloat,
    absorption: &'a mut f32,
    dead: bool,
    observer: &'a mut O,
    max_health: AttributeId,
    max_absorption: AttributeId,
}
impl<'a, O: HealthObserver> AttributeHealth<'a, O> {
    pub fn new(
        attributes: &'a mut EntityAttributes,
        health: &'a mut SyncedFloat,
        absorption: &'a mut f32,
        dead: bool,
        observer: &'a mut O,
    ) -> Option<Self> {
        let max_health = attributes.catalog().id("minecraft:max_health")?;
        let max_absorption = attributes.catalog().id("minecraft:max_absorption")?;
        Some(Self {
            attributes,
            health,
            absorption,
            dead,
            observer,
            max_health,
            max_absorption,
        })
    }
    pub fn heal(&mut self, amount: f32) -> Result<(), HealthError<O::Error>> {
        living_health::heal(self, amount)
    }
    pub fn set_health(&mut self, value: f32) -> Result<(), HealthError<O::Error>> {
        living_health::set_health(self, value)
    }
    pub fn set_absorption(&mut self, value: f32) -> Result<(), HealthError<O::Error>> {
        living_health::set_absorption(self, value)
    }
    pub fn is_hurt(&mut self) -> Result<bool, HealthError<O::Error>> {
        living_health::is_hurt(self)
    }
    pub fn is_dead_or_dying(&mut self) -> Result<bool, HealthError<O::Error>> {
        living_health::is_dead_or_dying(self)
    }
}
impl<O: HealthObserver> HealthWorld for AttributeHealth<'_, O> {
    type Error = HealthError<O::Error>;
    type Data = ();
    fn health(&mut self) -> Result<f32, Self::Error> {
        Ok(self.health.value())
    }
    fn health_data(&self) {}
    fn max_health_attribute(&mut self) -> Result<f64, Self::Error> {
        self.attributes
            .value(self.max_health)
            .map_err(HealthError::Attribute)
    }
    fn max_absorption_attribute(&mut self) -> Result<f64, Self::Error> {
        self.attributes
            .value(self.max_absorption)
            .map_err(HealthError::Attribute)
    }
    fn store_health(&mut self, _: (), value: f32) -> Result<(), Self::Error> {
        self.health
            .set(value, false, |health| self.observer.updated(health))
            .map_err(HealthError::Observer)
    }
    fn store_absorption(&mut self, value: f32) -> Result<(), Self::Error> {
        *self.absorption = value;
        Ok(())
    }
    fn dead_flag(&self) -> bool {
        self.dead
    }
    fn dispatch_set_health(&mut self, value: f32) -> Result<(), Self::Error> {
        living_health::set_health(self, value)
    }
}
