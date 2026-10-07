//! Living health/absorption limits and healing/query dispatch order.
pub trait HealthWorld {
    type Error;
    type Data: Copy;
    fn health(&mut self) -> Result<f32, Self::Error>;
    /// Captures the current synchronized-data object before attribute lookup.
    fn health_data(&self) -> Self::Data;
    fn max_health_attribute(&mut self) -> Result<f64, Self::Error>;
    fn max_absorption_attribute(&mut self) -> Result<f64, Self::Error>;
    fn store_health(&mut self, data: Self::Data, value: f32) -> Result<(), Self::Error>;
    fn store_absorption(&mut self, value: f32) -> Result<(), Self::Error>;
    fn dead_flag(&self) -> bool;
    fn dispatch_set_health(&mut self, value: f32) -> Result<(), Self::Error>;
}
fn clamp(value: f32, max: f32) -> f32 {
    if value < 0.0 {
        0.0
    } else if value.is_nan() || max.is_nan() {
        f32::NAN
    } else if value == 0.0 && max == 0.0 {
        f32::from_bits(value.to_bits() | max.to_bits())
    } else {
        value.min(max)
    }
}
pub fn set_health<W: HealthWorld>(world: &mut W, value: f32) -> Result<(), W::Error> {
    let data = world.health_data();
    let maximum = world.max_health_attribute()? as f32;
    world.store_health(data, clamp(value, maximum))
}
pub fn set_absorption<W: HealthWorld>(world: &mut W, value: f32) -> Result<(), W::Error> {
    let maximum = world.max_absorption_attribute()? as f32;
    world.store_absorption(clamp(value, maximum))
}
pub fn heal<W: HealthWorld>(world: &mut W, amount: f32) -> Result<(), W::Error> {
    let health = world.health()?;
    if health > 0.0 {
        world.dispatch_set_health(health + amount)?;
    }
    Ok(())
}
pub fn is_hurt<W: HealthWorld>(world: &mut W) -> Result<bool, W::Error> {
    Ok(world.health()? > 0.0 && world.health()? < world.max_health_attribute()? as f32)
}
pub fn is_dead_or_dying<W: HealthWorld>(world: &mut W) -> Result<bool, W::Error> {
    Ok(world.health()? <= 0.0 || world.dead_flag())
}
/// One synchronized Float data item, using Java boxed-float equality. The native
/// callback observes the new value before dirty flags are set. Exceptions leave
/// that value changed without newly marking it dirty.
#[derive(Clone, Copy, Debug)]
pub struct SyncedFloat {
    initial: f32,
    value: f32,
    item_dirty: bool,
    dirty: bool,
}
fn equal(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}
impl SyncedFloat {
    pub fn new(value: f32) -> Self {
        Self {
            initial: value,
            value,
            item_dirty: false,
            dirty: false,
        }
    }
    pub fn value(&self) -> f32 {
        self.value
    }
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    pub fn item_dirty(&self) -> bool {
        self.item_dirty
    }
    pub fn is_default(&self) -> bool {
        equal(self.initial, self.value)
    }
    pub fn set<E>(
        &mut self,
        value: f32,
        force: bool,
        updated: impl FnOnce(&mut Self) -> Result<(), E>,
    ) -> Result<(), E> {
        if force || !equal(value, self.value) {
            self.value = value;
            updated(self)?;
            self.item_dirty = true;
            self.dirty = true;
        }
        Ok(())
    }
    /// Extracts this single item's update. Multi-item synchronized storage and
    /// network serialization are separate concerns.
    pub fn take_dirty(&mut self) -> Option<f32> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        if self.item_dirty {
            self.item_dirty = false;
            Some(self.value)
        } else {
            None
        }
    }
}
