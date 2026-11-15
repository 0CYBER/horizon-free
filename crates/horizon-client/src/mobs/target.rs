//! Target writes and the species-specific side effects of `setTarget`.
use super::{
    Mob, MobEvent, Species, Who, enderman,
    finalize::{self, Attr, Op},
    fox,
};
use horizon_core::random::RandomSource;

impl Mob {
    /// `Mob.setTarget` and the fox, enderman and zombified piglin overrides.
    /// The caller validates the target before invoking this setter.
    pub fn set_target(&mut self, target: Option<Who>) {
        match self.species() {
            Species::Fox if target.is_none() => fox::set_flag(self, fox::DEFENDING, false),
            Species::Enderman => {
                if self.extra.flag != target.is_some()
                    && self.extra.flag2
                    && let Some(at) = enderman::stare_sound(self)
                {
                    self.outbox
                        .push(MobEvent::LocalSound("entity.enderman.stare", at, 2.5, 1.0));
                }
                self.extra.flag = target.is_some();
                self.extra.counter = if target.is_some() { self.age as i32 } else { 0 };
                if target.is_none() {
                    self.extra.flag2 = false;
                    finalize::remove_modifier(self, Attr::MovementSpeed, "minecraft:attacking");
                } else if !finalize::has_modifier(self, Attr::MovementSpeed, "minecraft:attacking")
                    && !finalize::has_transient_modifier(
                        self,
                        Attr::MovementSpeed,
                        "minecraft:attacking",
                    )
                {
                    finalize::add_transient_modifier(
                        self,
                        Attr::MovementSpeed,
                        "minecraft:attacking",
                        f64::from(0.15f32),
                        Op::AddValue,
                    );
                }
            }
            Species::ZombifiedPiglin if self.ai.target.is_none() && target.is_some() => {
                self.extra.counter = self.random.next_int_bound(21).unwrap_or(0);
                self.extra.counter2 = 80 + self.random.next_int_bound(41).unwrap_or(0);
            }
            _ => {}
        }
        self.ai.target = target;
    }
}
