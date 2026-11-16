use super::*;
use mod_api::KnockbackEntry;

impl Game<'_> {
    /// `LivingEntity.knockback` on the player: pushed `power` away from the
    /// direction (`xd`, `zd`), less its knockback resistance, and up only from
    /// the ground.
    /// The native knockback hook runs before resistance is applied, with
    /// `source`, the hit's. This short entry enters `knock_back_player_full` once.
    /// Both entries are native internals points. API access refuses `HurtSource` and
    /// `Attack`, which have no point mirrors; wasm refuses internals at load.
    pub(in crate::game) fn knock_back_player(
        &mut self,
        power: f64,
        xd: f64,
        zd: f64,
        source: HurtSource<'_>,
        attack: &Attack,
        _damage: f32,
    ) {
        self.knock_back_player_entry(power, xd, zd, source, attack, KnockbackEntry::Short);
    }

    /// Full living knockback entry. Effect pushes enter here directly.
    /// Damage and the effect flag are carried even though vanilla players do not use them.
    pub(in crate::game) fn knock_back_player_full(
        &mut self,
        power: f64,
        xd: f64,
        zd: f64,
        source: HurtSource<'_>,
        attack: &Attack,
        _damage: f32,
        _comes_from_effect: bool,
    ) {
        self.knock_back_player_entry(power, xd, zd, source, attack, KnockbackEntry::Full);
    }

    /// The player's push, entered through `entry`: what `living_knockback` tells its listener.
    fn knock_back_player_entry(
        &mut self,
        power: f64,
        xd: f64,
        zd: f64,
        source: HurtSource<'_>,
        attack: &Attack,
        entry: KnockbackEntry,
    ) {
        // No attacker named: the mob the player was last hurt by.
        let credited;
        let attack = if attack.attacker == 0
            && let Some((Who::Mob(id), _)) = self.player_hurt_by()
        {
            credited = self.attack_of(id as i32);
            &credited
        } else {
            attack
        };
        let legacy = self.legacy_rules();
        if let Some(p) = self.player.as_mut() {
            let level = crate::game::entity::damage_level_of!(self, attack);
            let mut push = |level: &mut horizon_client::health::DamageLevel<'_>| {
                apply_player_knockback(p, level, power, (xd, zd), source, legacy);
            };
            match entry {
                KnockbackEntry::Short => level.through_short_knockback(push),
                KnockbackEntry::Full => push(level),
            }
        }
    }
}

/// Apply the full entry's physical push, including 1.8.9's
/// `EntityLivingBase.knockBack` under `legacy` rules.
pub(super) fn apply_player_knockback(
    p: &mut horizon_client::motion::PlayerMotion,
    level: &mut horizon_client::health::DamageLevel<'_>,
    power: f64,
    (xd, zd): (f64, f64),
    source: HurtSource<'_>,
    legacy: bool,
) {
    let resistance = p.attribute("minecraft:knockback_resistance").unwrap_or(0.0);
    // The native hook runs first, on the raw strength (`LivingKnockBackEvent`), then the
    // resistance scales what it left.
    let mut knock = Knock {
        strength: power,
        x: xd,
        z: zd,
    };
    if level.knockback_canceled(Hurting::Player(p.entity_id, source), &mut knock) {
        return;
    }
    let Knock {
        strength: power,
        x: mut xd,
        z: mut zd,
    } = knock;
    if legacy && horizon_client::legacy_combat::resists(resistance, level.random().next_double()) {
        return;
    }
    let power = if legacy {
        power
    } else {
        power * (1.0 - resistance)
    };
    if power <= 0.0 {
        return;
    }
    while xd * xd + zd * zd < f64::from(1.0e-5_f32) {
        let random = level.random();
        xd = (random.next_double() - random.next_double()) * 0.01;
        zd = (random.next_double() - random.next_double()) * 0.01;
    }
    if legacy {
        let v = p.velocity;
        let [x, y, z] = horizon_client::legacy_combat::knockback([v.x, v.y, v.z], power, (xd, zd));
        p.velocity = Vec3::new(x, y, z);
        return;
    }
    let l = (xd * xd + zd * zd).sqrt();
    let v = p.velocity;
    p.velocity = Vec3::new(
        v.x / 2.0 - xd / l * power,
        if p.ground.on_ground {
            (v.y / 2.0 + power).min(0.4)
        } else {
            v.y
        },
        v.z / 2.0 - zd / l * power,
    );
}
