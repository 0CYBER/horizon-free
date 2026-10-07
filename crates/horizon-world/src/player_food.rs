//! Player-specific hunger guards and virtual/direct state access distinctions.
use crate::food::FoodData;
pub trait PlayerFoodWorld {
    type Error;
    /// Reads the abilities field directly, not the virtual getAbilities method.
    fn invulnerable_direct(&self) -> bool;
    fn level_is_client(&mut self) -> Result<bool, Self::Error>;
    fn add_exhaustion_direct(&mut self, value: f32) -> Result<(), Self::Error>;
    fn needs_food_direct(&mut self) -> Result<bool, Self::Error>;
    fn enough_food_virtual(&mut self) -> Result<bool, Self::Error>;
    fn may_fly_virtual(&mut self) -> Result<bool, Self::Error>;
}
pub fn cause_exhaustion<W: PlayerFoodWorld>(world: &mut W, value: f32) -> Result<(), W::Error> {
    if !world.invulnerable_direct() && !world.level_is_client()? {
        world.add_exhaustion_direct(value)?;
    }
    Ok(())
}
pub fn can_eat<W: PlayerFoodWorld>(world: &mut W, always_edible: bool) -> Result<bool, W::Error> {
    Ok(world.invulnerable_direct() || always_edible || world.needs_food_direct()?)
}
pub fn enough_food_for_manoeuvres<W: PlayerFoodWorld>(world: &mut W) -> Result<bool, W::Error> {
    Ok(world.enough_food_virtual()? || world.may_fly_virtual()?)
}
/// Binding for ordinary stored player state. Specialized entity implementations
/// can supply their own virtual food/ability lookups through PlayerFoodWorld.
pub struct StoredPlayerFood<'a> {
    pub food: &'a mut FoodData,
    pub invulnerable: bool,
    pub client_side: bool,
    pub may_fly: bool,
}
impl PlayerFoodWorld for StoredPlayerFood<'_> {
    type Error = std::convert::Infallible;
    fn invulnerable_direct(&self) -> bool {
        self.invulnerable
    }
    fn level_is_client(&mut self) -> Result<bool, Self::Error> {
        Ok(self.client_side)
    }
    fn add_exhaustion_direct(&mut self, value: f32) -> Result<(), Self::Error> {
        self.food.add_exhaustion(value);
        Ok(())
    }
    fn needs_food_direct(&mut self) -> Result<bool, Self::Error> {
        Ok(self.food.needs_food())
    }
    fn enough_food_virtual(&mut self) -> Result<bool, Self::Error> {
        Ok(self.food.has_enough_food())
    }
    fn may_fly_virtual(&mut self) -> Result<bool, Self::Error> {
        Ok(self.may_fly)
    }
}
