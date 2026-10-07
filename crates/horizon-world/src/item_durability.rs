//! Item-stack durability storage and mutation ordering. This is the durability
//! projection of a stack, not a replacement for its complete component map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurabilityStack {
    pub count: i32,
    pub air: bool,
    pub max_damage: Option<i32>,
    pub damage: Option<i32>,
    pub unbreakable: bool,
}

impl DurabilityStack {
    pub fn is_empty(&self) -> bool {
        self.air || self.count <= 0
    }
    pub fn count(&self) -> i32 {
        if self.is_empty() { 0 } else { self.count }
    }
    pub fn max_damage(&self) -> i32 {
        if self.is_empty() {
            0
        } else {
            self.max_damage.unwrap_or(0)
        }
    }
    pub fn damage(&self) -> i32 {
        let value = if self.is_empty() {
            0
        } else {
            self.damage.unwrap_or(0)
        };
        value.max(0).min(self.max_damage())
    }
    pub fn set_damage(&mut self, value: i32) {
        self.damage = Some(value.max(0).min(self.max_damage()));
    }
    pub fn is_damageable(&self) -> bool {
        !self.is_empty() && self.max_damage.is_some() && !self.unbreakable && self.damage.is_some()
    }
    pub fn is_damaged(&self) -> bool {
        self.is_damageable() && self.damage() > 0
    }
    pub fn is_broken(&self) -> bool {
        self.is_damageable() && self.damage() >= self.max_damage()
    }
    pub fn next_damage_will_break(&self) -> bool {
        self.is_damageable() && self.damage() >= self.max_damage().wrapping_sub(1)
    }
    pub fn grow(&mut self, amount: i32) {
        self.count = self.count().wrapping_add(amount);
    }
    pub fn shrink(&mut self, amount: i32) {
        self.grow(amount.wrapping_neg());
    }
}

/// Storage operations used by the native durability algorithm. Component-backed
/// implementations can preserve patch mutations and errors at each write.
pub trait DurabilityStorage: Clone {
    type Error;
    fn is_damageable(&self) -> bool;
    fn damage(&self) -> i32;
    fn max_damage(&self) -> i32;
    fn set_damage(&mut self, value: i32) -> Result<(), Self::Error>;
    fn is_broken(&self) -> bool;
    fn shrink(&mut self, amount: i32);
}
impl DurabilityStorage for DurabilityStack {
    type Error = std::convert::Infallible;
    fn is_damageable(&self) -> bool {
        self.is_damageable()
    }
    fn damage(&self) -> i32 {
        self.damage()
    }
    fn max_damage(&self) -> i32 {
        self.max_damage()
    }
    fn set_damage(&mut self, value: i32) -> Result<(), Self::Error> {
        self.set_damage(value);
        Ok(())
    }
    fn is_broken(&self) -> bool {
        self.is_broken()
    }
    fn shrink(&mut self, amount: i32) {
        self.shrink(amount);
    }
}
pub trait DurabilityWorld {
    type Error;
    type Stack: DurabilityStorage;
    fn storage_error(error: <Self::Stack as DurabilityStorage>::Error) -> Self::Error;
    fn infinite_materials(&mut self, stack: &mut Self::Stack) -> Result<bool, Self::Error>;
    /// Positive durability changes pass through the enchantment float pipeline,
    /// including when the stack has no enchantments.
    fn process_enchantments(
        &mut self,
        stack: &mut Self::Stack,
        amount: i32,
    ) -> Result<i32, Self::Error>;
    fn durability_changed(
        &mut self,
        stack: &mut Self::Stack,
        damage: i32,
    ) -> Result<(), Self::Error>;
    /// Called after shrinking the original stack, with the pre-shrink copy.
    fn broken(&mut self, stack: &mut Self::Stack, copy: Self::Stack) -> Result<(), Self::Error>;
}

pub fn unenchanted_change(amount: i32) -> i32 {
    (amount as f32) as i32
}

pub fn process_change<W: DurabilityWorld>(
    stack: &mut W::Stack,
    world: &mut W,
    amount: i32,
    server_player: bool,
) -> Result<i32, W::Error> {
    if !stack.is_damageable() || (server_player && world.infinite_materials(stack)?) {
        return Ok(0);
    }
    if amount > 0 {
        world.process_enchantments(stack, amount)
    } else {
        Ok(amount)
    }
}

pub fn apply_damage<W: DurabilityWorld>(
    stack: &mut W::Stack,
    world: &mut W,
    damage: i32,
    server_player: bool,
) -> Result<(), W::Error> {
    apply_damage_with_break_callback(stack, world, damage, server_player, true)
}

fn apply_damage_with_break_callback<W: DurabilityWorld>(
    stack: &mut W::Stack,
    world: &mut W,
    damage: i32,
    server_player: bool,
    notify_break: bool,
) -> Result<(), W::Error> {
    if server_player {
        world.durability_changed(stack, damage)?;
    }
    stack.set_damage(damage).map_err(W::storage_error)?;
    if stack.is_broken() {
        let copy = stack.clone();
        stack.shrink(1);
        if notify_break {
            world.broken(stack, copy)?;
        }
    }
    Ok(())
}

/// The server-level overload; callers supply the already selected player kind.
pub fn hurt_and_break<W: DurabilityWorld>(
    stack: &mut W::Stack,
    world: &mut W,
    amount: i32,
    server_player: bool,
) -> Result<(), W::Error> {
    let amount = process_change(stack, world, amount, server_player)?;
    if amount != 0 {
        let damage = stack.damage().wrapping_add(amount);
        apply_damage(stack, world, damage, server_player)?;
    }
    Ok(())
}

/// The body after the server-player/level checks of hurtWithoutBreaking.
pub fn hurt_without_breaking<W: DurabilityWorld>(
    stack: &mut W::Stack,
    world: &mut W,
    amount: i32,
) -> Result<(), W::Error> {
    let amount = process_change(stack, world, amount, true)?;
    if amount != 0 {
        let damage = stack
            .damage()
            .wrapping_add(amount)
            .min(stack.max_damage().wrapping_sub(1));
        apply_damage_with_break_callback(stack, world, damage, true, false)?;
    }
    Ok(())
}
