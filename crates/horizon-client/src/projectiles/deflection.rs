//! What a deflection does to a projectile besides turning it
//! (`Projectile.onDeflection` and the `AbstractHurtingProjectile` that
//! overrides it): the acceleration power a breeze's deflection halves.
use super::{Projectile, ProjectileKind};

impl Projectile {
    /// `AbstractHurtingProjectile`: no gravity, and its own direction times
    /// `accelerationPower` added to its motion every tick. A thrown item, an
    /// arrow and a shulker bullet are none of them.
    pub fn is_hurting(&self) -> bool {
        matches!(
            self.kind,
            ProjectileKind::SmallFireball
                | ProjectileKind::LargeFireball { .. }
                | ProjectileKind::DragonFireball
                | ProjectileKind::WitherSkull { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        )
    }

    /// `AbstractHurtingProjectile.accelerationPower`, which a server sends to
    /// its clients with every velocity update; only a hurting projectile has
    /// one to send.
    pub fn acceleration_power(&self) -> Option<f64> {
        self.is_hurting().then_some(self.acceleration)
    }

    /// `AbstractHurtingProjectile.onDeflection`: a blow (`byAttack`) puts the
    /// acceleration power back to 0.1, any other deflection halves it. Only a
    /// hurting projectile has one (`Projectile.onDeflection` is empty).
    pub fn on_deflection(&mut self, by_attack: bool) {
        if !self.is_hurting() {
            return;
        }
        if by_attack {
            self.acceleration = 0.1;
        } else {
            self.acceleration *= 0.5;
        }
    }
}
