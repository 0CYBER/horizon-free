//! Player absorption uses virtual synchronized-data access, unlike base living
//! absorption field storage. Health keeps its direct synchronized-data access.
use crate::{
    attribute_health::HealthAttributes,
    attributes::entity::{EntityAttributes, LookupError},
    compression::invalid,
    living_health::{self, HealthWorld},
    synced_floats::{DataError, Observer, SyncedFloats},
};
use std::{fmt, io};
pub trait PlayerHealthWorld: HealthWorld {
    fn entity_data(&mut self) -> Result<Self::Data, Self::Error>;
    fn read_absorption(&mut self, data: Self::Data) -> Result<f32, Self::Error>;
    fn write_absorption(&mut self, data: Self::Data, value: f32) -> Result<(), Self::Error>;
}
pub fn absorption<W: PlayerHealthWorld>(world: &mut W) -> Result<f32, W::Error> {
    let data = world.entity_data()?;
    world.read_absorption(data)
}
pub fn internal_set_absorption<W: PlayerHealthWorld>(
    world: &mut W,
    value: f32,
) -> Result<(), W::Error> {
    let data = world.entity_data()?;
    world.write_absorption(data, value)
}
#[derive(Clone, Copy, Debug)]
pub struct PlayerFloatIds {
    pub health: u8,
    pub absorption: u8,
}
#[derive(Debug)]
pub enum PlayerHealthError<E, A = LookupError> {
    Attribute(A),
    Data(DataError<E>),
}
impl<E: fmt::Display, A: fmt::Debug> fmt::Display for PlayerHealthError<E, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attribute(e) => write!(f, "player health attribute lookup: {e:?}"),
            Self::Data(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static, A: fmt::Debug> std::error::Error for PlayerHealthError<E, A> {}
/// Ordinary registered-player state, borrowing its actual float table. IDs come
/// from the pinned entity-data schema; construction does not invent spawn state.
pub struct PlayerAttributeHealth<'a, O, A: HealthAttributes = EntityAttributes> {
    attributes: &'a mut A,
    data: &'a mut SyncedFloats,
    ids: PlayerFloatIds,
    max_health: A::Key,
    max_absorption: A::Key,
    dead: bool,
    observer: &'a mut O,
}
impl<'a, O: Observer, A: HealthAttributes> PlayerAttributeHealth<'a, O, A> {
    pub fn new(
        attributes: &'a mut A,
        data: &'a mut SyncedFloats,
        ids: PlayerFloatIds,
        dead: bool,
        observer: &'a mut O,
    ) -> io::Result<Self> {
        if ids.health == ids.absorption
            || data.get(ids.health).is_err()
            || data.get(ids.absorption).is_err()
        {
            return Err(invalid("missing or aliased player float fields"));
        }
        let max_health = attributes
            .health_attribute_key("minecraft:max_health")
            .ok_or_else(|| invalid("missing MAX_HEALTH definition"))?;
        let max_absorption = attributes
            .health_attribute_key("minecraft:max_absorption")
            .ok_or_else(|| invalid("missing MAX_ABSORPTION definition"))?;
        Ok(Self {
            attributes,
            data,
            ids,
            max_health,
            max_absorption,
            dead,
            observer,
        })
    }
    pub fn heal(&mut self, amount: f32) -> Result<(), PlayerHealthError<O::Error, A::Error>> {
        living_health::heal(self, amount)
    }
    pub fn set_health(&mut self, value: f32) -> Result<(), PlayerHealthError<O::Error, A::Error>> {
        living_health::set_health(self, value)
    }
    pub fn set_absorption(
        &mut self,
        value: f32,
    ) -> Result<(), PlayerHealthError<O::Error, A::Error>> {
        living_health::set_absorption(self, value)
    }
    pub fn absorption(&mut self) -> Result<f32, PlayerHealthError<O::Error, A::Error>> {
        absorption(self)
    }
    pub fn is_hurt(&mut self) -> Result<bool, PlayerHealthError<O::Error, A::Error>> {
        living_health::is_hurt(self)
    }
    pub fn is_dead_or_dying(&mut self) -> Result<bool, PlayerHealthError<O::Error, A::Error>> {
        living_health::is_dead_or_dying(self)
    }
}
impl<O: Observer, A: HealthAttributes> HealthWorld for PlayerAttributeHealth<'_, O, A> {
    type Error = PlayerHealthError<O::Error, A::Error>;
    type Data = ();
    fn health(&mut self) -> Result<f32, Self::Error> {
        self.data
            .get(self.ids.health)
            .map_err(|id| PlayerHealthError::Data(DataError::UnknownId(id)))
    }
    fn health_data(&self) {}
    fn max_health_attribute(&mut self) -> Result<f64, Self::Error> {
        self.attributes
            .health_attribute_value(self.max_health.clone())
            .map_err(PlayerHealthError::Attribute)
    }
    fn max_absorption_attribute(&mut self) -> Result<f64, Self::Error> {
        self.attributes
            .health_attribute_value(self.max_absorption.clone())
            .map_err(PlayerHealthError::Attribute)
    }
    fn store_health(&mut self, _: (), value: f32) -> Result<(), Self::Error> {
        self.data
            .set(self.ids.health, value, false, self.observer)
            .map_err(PlayerHealthError::Data)
    }
    fn store_absorption(&mut self, value: f32) -> Result<(), Self::Error> {
        internal_set_absorption(self, value)
    }
    fn dead_flag(&self) -> bool {
        self.dead
    }
    fn dispatch_set_health(&mut self, value: f32) -> Result<(), Self::Error> {
        living_health::set_health(self, value)
    }
}
impl<O: Observer, A: HealthAttributes> PlayerHealthWorld for PlayerAttributeHealth<'_, O, A> {
    fn entity_data(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn read_absorption(&mut self, _: ()) -> Result<f32, Self::Error> {
        self.data
            .get(self.ids.absorption)
            .map_err(|id| PlayerHealthError::Data(DataError::UnknownId(id)))
    }
    fn write_absorption(&mut self, _: (), value: f32) -> Result<(), Self::Error> {
        self.data
            .set(self.ids.absorption, value, false, self.observer)
            .map_err(PlayerHealthError::Data)
    }
}
