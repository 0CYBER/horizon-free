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
