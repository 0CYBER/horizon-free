//! Shared status-effect instances. Cloning retains identity; copy_instance
//! implements the native copy constructor, excluding hidden effects and blend.
use std::{
    cell::{Ref, RefCell, RefMut},
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Details {
    pub duration: i32,
    pub amplifier: i32,
    pub ambient: bool,
    pub visible: bool,
    pub show_icon: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct BlendState {
    pub factor: f32,
    pub previous: f32,
}
#[derive(Debug)]
pub struct EffectData<H> {
    pub effect: H,
    pub details: Details,
    pub hidden: Option<EffectInstance<H>>,
    pub blend: BlendState,
}
#[derive(Debug)]
pub struct EffectInstance<H>(Rc<RefCell<EffectData<H>>>);
impl<H> Clone for EffectInstance<H> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<H> EffectInstance<H> {
    pub fn new(effect: H, mut details: Details, hidden: Option<Self>) -> Self {
        details.amplifier = details.amplifier.clamp(0, 255);
        Self(Rc::new(RefCell::new(EffectData {
            effect,
            details,
            hidden,
            blend: BlendState::default(),
        })))
    }
    pub fn read(&self) -> Ref<'_, EffectData<H>> {
        self.0.borrow()
    }
    pub fn write(&self) -> RefMut<'_, EffectData<H>> {
        self.0.borrow_mut()
    }
    pub fn same_reference(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
    pub fn infinite(&self) -> bool {
        self.read().details.duration == -1
    }
    pub fn ends_within(&self, duration: i32) -> bool {
        !self.infinite() && self.read().details.duration <= duration
    }
    pub fn has_remaining(&self) -> bool {
        self.infinite() || self.read().details.duration > 0
    }
    fn shorter_than(&self, other: &Self) -> bool {
        !self.infinite()
            && (self.read().details.duration < other.read().details.duration || other.infinite())
    }
    pub fn map_duration<T>(&self, mapper: impl FnOnce(i32) -> Result<i32, T>) -> Result<i32, T> {
        let duration = self.read().details.duration;
        if duration == -1 || duration == 0 {
            Ok(duration)
        } else {
            mapper(duration)
        }
    }
    pub fn copy_blend_from(&self, other: &Self) {
        let blend = other.read().blend;
        self.write().blend = blend;
    }
}
impl<H: Clone> EffectInstance<H> {
    pub fn copy_instance(&self) -> Self {
        let data = self.read();
        Self(Rc::new(RefCell::new(EffectData {
            effect: data.effect.clone(),
            details: data.details,
            hidden: None,
            blend: BlendState::default(),
        })))
    }
    pub fn scaled(&self, factor: f32) -> Self {
        let result = self.copy_instance();
        let duration = result
            .map_duration::<std::convert::Infallible>(|duration| {
                let v = duration as f32 * factor;
                let floor = f64::from(v).floor() as i32;
                Ok(floor.max(1))
            })
            .unwrap();
        result.write().details.duration = duration;
        result
    }
    /// Updates details and the shared hidden chain. As in vanilla, differing
    /// effect holders do not prevent an update; diagnostic logging is external.
    pub fn update(&self, incoming: &Self) -> bool {
        let mut changed = false;
        if incoming.read().details.amplifier > self.read().details.amplifier {
            if incoming.shorter_than(self) {
                let old_hidden = self.read().hidden.clone();
                let hidden = self.copy_instance();
                hidden.write().hidden = old_hidden;
                self.write().hidden = Some(hidden);
            }
            let source = incoming.read().details;
            let mut data = self.write();
            data.details.amplifier = source.amplifier;
            data.details.duration = source.duration;
            changed = true;
        } else if self.shorter_than(incoming) {
            if incoming.read().details.amplifier == self.read().details.amplifier {
                let duration = incoming.read().details.duration;
                self.write().details.duration = duration;
                changed = true;
            } else {
                let hidden = self.read().hidden.clone();
                if let Some(hidden) = hidden {
                    hidden.update(incoming);
                } else {
                    self.write().hidden = Some(incoming.copy_instance());
                }
            }
        }
        let incoming = incoming.read().details;
        let mut data = self.write();
        if (!incoming.ambient && data.details.ambient) || changed {
            data.details.ambient = incoming.ambient;
            changed = true;
        }
        if incoming.visible != data.details.visible {
            data.details.visible = incoming.visible;
            changed = true;
        }
        if incoming.show_icon != data.details.show_icon {
            data.details.show_icon = incoming.show_icon;
            changed = true;
        }
        changed
    }
}
impl<H: PartialEq> PartialEq for EffectInstance<H> {
    fn eq(&self, other: &Self) -> bool {
        let a = self.read();
        let b = other.read();
        a.effect == b.effect && a.details == b.details
    }
}
impl<H: Eq> Eq for EffectInstance<H> {}

pub trait TickWorld<H> {
    type Error;
    type Level: Copy;
    fn actor_tick_count(&self) -> i32;
    fn should_apply(
        &mut self,
        effect: H,
        time: i32,
        amplifier: i32,
        instance: &EffectInstance<H>,
    ) -> Result<bool, Self::Error>;
    fn apply_tick(
        &mut self,
        effect: H,
        level: Self::Level,
        amplifier: i32,
        instance: &EffectInstance<H>,
    ) -> Result<bool, Self::Error>;
    fn downgraded(&mut self, instance: &EffectInstance<H>) -> Result<(), Self::Error>;
}
pub trait BlendWorld<H> {
    type Error;
    fn advance_ticks(
        &mut self,
        effect: H,
        instance: &EffectInstance<H>,
    ) -> Result<i32, Self::Error>;
    fn duration_ticks(
        &mut self,
        effect: H,
        blending_in: bool,
        instance: &EffectInstance<H>,
    ) -> Result<i32, Self::Error>;
}
fn float_clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v.is_nan() || hi.is_nan() {
        f32::NAN
    } else if v == 0.0 && hi == 0.0 {
        f32::from_bits(v.to_bits() | hi.to_bits())
    } else {
        v.min(hi)
    }
}
impl<H> EffectInstance<H> {
    fn tick_down(&self) {
        let hidden = self.read().hidden.clone();
        if let Some(hidden) = hidden {
            hidden.tick_down();
        }
        let mut data = self.write();
        if data.details.duration != -1 && data.details.duration != 0 {
            data.details.duration = data.details.duration.wrapping_sub(1);
        }
    }
    fn downgrade(&self) -> bool {
        if self.read().details.duration != 0 {
            return false;
        }
        let hidden = self.read().hidden.clone();
        if let Some(hidden) = hidden {
            let details = hidden.read().details;
            let next = hidden.read().hidden.clone();
            let mut data = self.write();
            data.details = details;
            data.hidden = next;
            true
        } else {
            false
        }
    }
    /// Entity.isRemoved is a final field-based query. Removal snaps the
    /// previous factor before interpolation, including for non-finite partials.
    pub fn blend_factor(&self, partial: f32, actor_removed: bool) -> f32 {
        let mut data = self.write();
        if actor_removed {
            data.blend.previous = data.blend.factor;
        }
        data.blend.previous + partial * (data.blend.factor - data.blend.previous)
    }
}
impl<H: Clone> EffectInstance<H> {
    pub fn tick_server<W: TickWorld<H>>(
        &self,
        world: &mut W,
        level: W::Level,
    ) -> Result<bool, W::Error> {
        if !self.has_remaining() {
            return Ok(false);
        }
        let time = if self.infinite() {
            world.actor_tick_count()
        } else {
            self.read().details.duration
        };
        let effect = self.read().effect.clone();
        let amplifier = self.read().details.amplifier;
        if world.should_apply(effect, time, amplifier, self)? {
            let effect = self.read().effect.clone();
            let amplifier = self.read().details.amplifier;
            if !world.apply_tick(effect, level, amplifier, self)? {
                return Ok(false);
            }
        }
        self.tick_down();
        if self.downgrade() {
            world.downgraded(self)?;
        }
        Ok(self.has_remaining())
    }
    fn blend_has_effect<W: BlendWorld<H>>(&self, world: &mut W) -> Result<bool, W::Error> {
        let effect = self.read().effect.clone();
        let advance = world.advance_ticks(effect, self)?;
        Ok(!self.ends_within(advance))
    }
    pub fn skip_blending<W: BlendWorld<H>>(&self, world: &mut W) -> Result<(), W::Error> {
        let target = if self.blend_has_effect(world)? {
            1.0
        } else {
            0.0
        };
        self.write().blend = BlendState {
            factor: target,
            previous: target,
        };
        Ok(())
    }
    pub fn tick_client<W: BlendWorld<H>>(&self, world: &mut W) -> Result<(), W::Error> {
        if self.has_remaining() {
            self.tick_down();
            self.downgrade();
        }
        let previous = self.read().blend.factor;
        self.write().blend.previous = previous;
        let has_effect = self.blend_has_effect(world)?;
        let target = if has_effect { 1.0 } else { 0.0 };
        if self.read().blend.factor == target {
            return Ok(());
        }
        let effect = self.read().effect.clone();
        let ticks = world.duration_ticks(effect, has_effect, self)?;
        if ticks == 0 {
            self.write().blend.factor = target;
        } else {
            let step = 1.0 / ticks as f32;
            let mut data = self.write();
            data.blend.factor += float_clamp(target - data.blend.factor, -step, step);
        }
        Ok(())
    }
}
