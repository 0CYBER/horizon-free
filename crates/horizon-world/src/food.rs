//! Hunger, saturation, exhaustion and server-side natural regeneration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Peaceful,
    Easy,
    Normal,
    Hard,
}
#[derive(Clone, Copy, Debug)]
pub struct FoodData {
    pub food: i32,
    pub saturation: f32,
    pub exhaustion: f32,
    pub timer: i32,
}
impl Default for FoodData {
    fn default() -> Self {
        Self {
            food: 20,
            saturation: 5.0,
            exhaustion: 0.0,
            timer: 0,
        }
    }
}
fn min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() | b.to_bits())
    } else {
        a.min(b)
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() & b.to_bits())
    } else {
        a.max(b)
    }
}
fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo { lo } else { min(v, hi) }
}
impl FoodData {
    pub fn eat(&mut self, nutrition: i32, saturation_modifier: f32) {
        self.eat_properties(nutrition, nutrition as f32 * saturation_modifier * 2.0);
    }
    pub fn eat_properties(&mut self, nutrition: i32, saturation: f32) {
        self.food = nutrition.wrapping_add(self.food).clamp(0, 20);
        self.saturation = clamp(saturation + self.saturation, 0.0, self.food as f32);
    }
    pub fn add_exhaustion(&mut self, value: f32) {
        self.exhaustion = min(self.exhaustion + value, 40.0);
    }
    pub fn has_enough_food(&self) -> bool {
        self.food as f32 > 6.0
    }
    pub fn needs_food(&self) -> bool {
        self.food < 20
    }
    pub fn tick<W: FoodWorld>(&mut self, world: &mut W) -> Result<(), W::Error> {
        let level = world.level(self)?;
        let difficulty = world.difficulty(level, self)?;
        if self.exhaustion > 4.0 {
            self.exhaustion -= 4.0;
            if self.saturation > 0.0 {
                self.saturation = max(self.saturation - 1.0, 0.0);
            } else if difficulty != Difficulty::Peaceful {
                self.food = self.food.wrapping_sub(1).max(0);
            }
        }
        let regenerate = world.natural_regeneration(level, self)?;
        if regenerate && self.saturation > 0.0 && world.is_hurt(self)? && self.food >= 20 {
            self.timer = self.timer.wrapping_add(1);
            if self.timer >= 10 {
                let amount = min(self.saturation, 6.0);
                world.heal(self, amount / 6.0)?;
                self.add_exhaustion(amount);
                self.timer = 0;
            }
        } else if regenerate && self.food >= 18 && world.is_hurt(self)? {
            self.timer = self.timer.wrapping_add(1);
            if self.timer >= 80 {
                world.heal(self, 1.0)?;
                self.add_exhaustion(6.0);
                self.timer = 0;
            }
        } else if self.food <= 0 {
            self.timer = self.timer.wrapping_add(1);
            if self.timer >= 80 {
                if world.health(self)? > 10.0
                    || difficulty == Difficulty::Hard
                    || (world.health(self)? > 1.0 && difficulty == Difficulty::Normal)
                {
                    world.hurt_starvation(level, self, 1.0)?;
                }
                self.timer = 0;
            }
        } else {
            self.timer = 0;
        }
        Ok(())
    }
}
/// World hooks preserve short-circuit query order. Callbacks can mutate food
/// state; errors retain earlier writes and effects. The level is captured once.
pub trait FoodWorld {
    type Error;
    type Level: Copy;
    fn level(&mut self, food: &mut FoodData) -> Result<Self::Level, Self::Error>;
    fn difficulty(
        &mut self,
        level: Self::Level,
        food: &mut FoodData,
    ) -> Result<Difficulty, Self::Error>;
    fn natural_regeneration(
        &mut self,
        level: Self::Level,
        food: &mut FoodData,
    ) -> Result<bool, Self::Error>;
    fn is_hurt(&mut self, food: &mut FoodData) -> Result<bool, Self::Error>;
    fn heal(&mut self, food: &mut FoodData, amount: f32) -> Result<(), Self::Error>;
    fn health(&mut self, food: &mut FoodData) -> Result<f32, Self::Error>;
    /// Resolve the player's starvation damage source and dispatch hurtServer on
    /// the captured level; the native boolean hurt result is ignored.
    fn hurt_starvation(
        &mut self,
        level: Self::Level,
        food: &mut FoodData,
        amount: f32,
    ) -> Result<(), Self::Error>;
}
