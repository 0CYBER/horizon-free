//! Consequences of LivingEntity attribute changes, preserving virtual call order.
use crate::living_health::{self, HealthWorld};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdatedAttribute {
    MaxHealth,
    MaxAbsorption,
    Scale,
    WaypointRange,
    Other,
}
pub trait UpdateWorld<H>: HealthWorld {
    type Level: Copy;
    type Manager: Copy;
    type Attributes: Copy;
    fn attribute_kind(&self, attribute: &H) -> UpdatedAttribute;
    fn absorption(&mut self) -> Result<f32, Self::Error>;
    fn refresh_dimensions(&mut self) -> Result<(), Self::Error>;
    fn level(&mut self) -> Result<Self::Level, Self::Error>;
    fn is_server_level(&self, level: Self::Level) -> bool;
    fn waypoint_manager(&mut self, level: Self::Level) -> Result<Self::Manager, Self::Error>;
    /// Direct attributes field, read after obtaining the waypoint manager.
    fn attributes_field(&self) -> Self::Attributes;
    fn attribute_value_in(
        &mut self,
        attributes: Self::Attributes,
        attribute: H,
    ) -> Result<f64, Self::Error>;
    fn track_waypoint(&mut self, manager: Self::Manager, track: bool) -> Result<(), Self::Error>;
}
pub fn updated<H, W: UpdateWorld<H>>(world: &mut W, attribute: H) -> Result<(), W::Error> {
    match world.attribute_kind(&attribute) {
        UpdatedAttribute::MaxHealth => {
            let maximum = world.max_health_attribute()? as f32;
            if world.health()? > maximum {
                world.dispatch_set_health(maximum)?;
            }
        }
        UpdatedAttribute::MaxAbsorption => {
            let maximum = world.max_absorption_attribute()? as f32;
            if world.absorption()? > maximum {
                living_health::set_absorption(world, maximum)?;
            }
        }
        UpdatedAttribute::Scale => world.refresh_dimensions()?,
        UpdatedAttribute::WaypointRange => {
            let level = world.level()?;
            if world.is_server_level(level) {
                let manager = world.waypoint_manager(level)?;
                let attributes = world.attributes_field();
                let range = world.attribute_value_in(attributes, attribute)?;
                world.track_waypoint(manager, range > 0.0)?;
            }
        }
        UpdatedAttribute::Other => {}
    }
    Ok(())
}
