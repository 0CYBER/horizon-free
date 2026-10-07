//! Bind health calculations to registered entity attributes and mutable data.
use crate::{
    attribute_map::ValueKind,
    attributes::{
        entity::{AttributeId, EntityAttributes, LookupError},
        live::{self, Key, LiveAttributes},
    },
    living_health::{self, HealthWorld, SyncedFloat},
};
use std::fmt;

#[derive(Debug)]
pub enum HealthError<E, A = LookupError> {
    Attribute(A),
    Observer(E),
}
impl<E: fmt::Display, A: fmt::Debug> fmt::Display for HealthError<E, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attribute(e) => write!(f, "health attribute lookup: {e:?}"),
            Self::Observer(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static, A: fmt::Debug> std::error::Error for HealthError<E, A> {}
/// Computed registered limits for a borrowed base LivingEntity health binding.
/// The borrow keeps the attribute map fixed. Virtual dispatch that can replace
/// the selected map belongs in a full HealthWorld implementation.
pub trait HealthAttributes {
    type Key: Clone;
    type Error;
    fn health_attribute_key(&self, name: &str) -> Option<Self::Key>;
    fn health_attribute_value(&mut self, key: Self::Key) -> Result<f64, Self::Error>;
}
impl HealthAttributes for EntityAttributes {
    type Key = AttributeId;
    type Error = LookupError;
    fn health_attribute_key(&self, name: &str) -> Option<AttributeId> {
        self.catalog().id(name)
    }
    fn health_attribute_value(&mut self, key: AttributeId) -> Result<f64, LookupError> {
        self.value(key)
    }
}
impl<G: FnMut() -> i32> HealthAttributes for LiveAttributes<G> {
    type Key = Key;
    type Error = live::Error;
    fn health_attribute_key(&self, name: &str) -> Option<Key> {
        self.key(name)
    }
    fn health_attribute_value(&mut self, key: Key) -> Result<f64, live::Error> {
        self.value(Some(key), ValueKind::Computed)
    }
}
pub trait HealthObserver {
    type Error;
    fn updated(&mut self, health: &mut SyncedFloat) -> Result<(), Self::Error>;
}
/// A borrow of living state, without inventing spawn or save initialization.
/// Absorption here uses base LivingEntity field storage. Player-specific
/// synchronized absorption requires a different storage binding.
pub struct AttributeHealth<'a, O, A: HealthAttributes = EntityAttributes> {
    attributes: &'a mut A,
    health: &'a mut SyncedFloat,
    absorption: &'a mut f32,
    dead: bool,
    observer: &'a mut O,
    max_health: A::Key,
    max_absorption: A::Key,
}
impl<'a, O: HealthObserver, A: HealthAttributes> AttributeHealth<'a, O, A> {
    pub fn new(
        attributes: &'a mut A,
        health: &'a mut SyncedFloat,
        absorption: &'a mut f32,
        dead: bool,
        observer: &'a mut O,
    ) -> Option<Self> {
        let max_health = attributes.health_attribute_key("minecraft:max_health")?;
        let max_absorption = attributes.health_attribute_key("minecraft:max_absorption")?;
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
    pub fn heal(&mut self, amount: f32) -> Result<(), HealthError<O::Error, A::Error>> {
        living_health::heal(self, amount)
    }
    pub fn set_health(&mut self, value: f32) -> Result<(), HealthError<O::Error, A::Error>> {
        living_health::set_health(self, value)
    }
    pub fn set_absorption(&mut self, value: f32) -> Result<(), HealthError<O::Error, A::Error>> {
        living_health::set_absorption(self, value)
    }
    pub fn is_hurt(&mut self) -> Result<bool, HealthError<O::Error, A::Error>> {
        living_health::is_hurt(self)
    }
    pub fn is_dead_or_dying(&mut self) -> Result<bool, HealthError<O::Error, A::Error>> {
        living_health::is_dead_or_dying(self)
    }
}
impl<O: HealthObserver, A: HealthAttributes> HealthWorld for AttributeHealth<'_, O, A> {
    type Error = HealthError<O::Error, A::Error>;
    type Data = ();
    fn health(&mut self) -> Result<f32, Self::Error> {
        Ok(self.health.value())
    }
    fn health_data(&self) {}
    fn max_health_attribute(&mut self) -> Result<f64, Self::Error> {
        self.attributes
            .health_attribute_value(self.max_health.clone())
            .map_err(HealthError::Attribute)
    }
    fn max_absorption_attribute(&mut self) -> Result<f64, Self::Error> {
        self.attributes
            .health_attribute_value(self.max_absorption.clone())
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
