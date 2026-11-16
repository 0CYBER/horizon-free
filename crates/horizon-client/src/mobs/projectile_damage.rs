use super::{Aim, Damage, DamageLevel, Mob};

impl Mob {
    /// `damage` of a blow dealt by a projectile (`motion`: its horizontal movement) whose owner
    /// is `aim`.
    pub fn damage_projectile(
        &mut self,
        level: &mut DamageLevel<'_>,
        amount: f32,
        source: Damage,
        aim: Option<Aim>,
        motion: [f64; 2],
    ) -> bool {
        self.projectile_motion = Some(motion);
        let hurt = self.damage_aimed(level, amount, source, aim);
        self.projectile_motion = None;
        hurt
    }
}
