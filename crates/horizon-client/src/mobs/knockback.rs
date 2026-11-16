use super::*;

impl Mob {
    /// Short `LivingEntity.knockback` entry, delegating to the full entry with
    /// `comes_from_effect = false`. Both entries are independent native internals points.
    /// API access refuses the `Mob` receiver and the unmirrored `DamageLevel`/`Damage`
    /// context; wasm refuses internals at load rather than silently skipping a listener.
    /// The living hook runs once, before resistance, motion changes and random draws.
    #[horizon_mod_api_macros::replaceable]
    pub fn knockback(
        &mut self,
        level: &mut DamageLevel<'_>,
        power: f64,
        xd: f64,
        zd: f64,
        source: &Damage,
        damage: f32,
    ) {
        level.through_short_knockback(|level| {
            self.knockback_full(level, power, xd, zd, source, damage, false);
        });
    }
    /// `getAttributeValue(KNOCKBACK_RESISTANCE)`: the mob's own, and what its gear adds.
    pub fn knockback_resistance(&self) -> f64 {
        self.gear_knockback_resistance
            .unwrap_or(self.attrs.knockback_resistance)
    }
    /// Full knockback entry, including whether this is an extra effect push.
    /// The short entry delegates here once; direct full calls bypass the short entry.
    /// A sulfur cube's effect push retains its special launch, before the living hook.
    /// API access has the same receiver/context refusal as the short entry.
    #[horizon_mod_api_macros::replaceable]
    pub fn knockback_full(
        &mut self,
        level: &mut DamageLevel<'_>,
        power: f64,
        xd: f64,
        zd: f64,
        source: &Damage,
        damage: f32,
        comes_from_effect: bool,
    ) {
        if comes_from_effect
            && self.species() == Species::SulfurCube
            && sulfur_cube::has_body(self)
            && source.attacker.is_some()
            && sulfur_cube::knockback(self, (xd, zd), source.from, damage, Some(power))
        {
            return;
        }
        if creaking::is_creaking(self) && !creaking::can_move(self) {
            return;
        }
        // The native hook runs first, on the raw strength, then the resistance scales what it left.
        let mut knock = crate::health::Knock {
            strength: power,
            x: xd,
            z: zd,
        };
        let pushed = crate::health::Hurting::Mob(self, source);
        if level.knockback_canceled(pushed, &mut knock) {
            return;
        }
        let crate::health::Knock {
            strength: power,
            x: mut xd,
            z: mut zd,
        } = knock;
        let power = power * (1.0 - self.knockback_resistance());
        if power <= 0.0 {
            return;
        }
        self.needs_sync = true;
        while xd * xd + zd * zd < f64::from(1.0e-5_f32) {
            xd = (self.random.next_double() - self.random.next_double()) * 0.01;
            zd = (self.random.next_double() - self.random.next_double()) * 0.01;
        }
        let len = (xd * xd + zd * zd).sqrt();
        let (kx, kz) = (xd / len * power, zd / len * power);
        self.velocity = Vec3::new(
            self.velocity.x / 2.0 - kx,
            if self.on_ground {
                (self.velocity.y / 2.0 + power).min(0.4)
            } else {
                self.velocity.y
            },
            self.velocity.z / 2.0 - kz,
        );
    }
}
