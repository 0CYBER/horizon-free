//! The accepted DamageSource and weapon, retained only for listeners that need them.
use super::{Attack, HurtSource};
use crate::{items::ItemStack, mobs::Damage};

/// The source/weapon owner shared by mob and player death continuations.
#[derive(Clone, Debug, PartialEq)]
pub struct AcceptedHit {
    damage_type: String,
    causing: i32,
    direct: i32,
    position: Option<[f64; 3]>,
    pub weapon: Option<ItemStack>,
    /// A mob's committed source; synthetic replacements cannot inherit an old weapon.
    pub committed: Option<(Damage, i64)>,
}
impl AcceptedHit {
    pub fn capture(
        source: HurtSource<'_>,
        attack: &Attack,
        committed: Option<(Damage, i64)>,
    ) -> Self {
        Self {
            damage_type: source.damage_type.to_owned(),
            causing: source.causing,
            direct: source.direct,
            position: source.position,
            weapon: attack.weapon_of(source.direct).cloned(),
            committed,
        }
    }
    pub fn source(&self) -> HurtSource<'_> {
        HurtSource {
            damage_type: &self.damage_type,
            causing: self.causing,
            direct: self.direct,
            position: self.position,
        }
    }
    pub fn belongs_to(&self, damage: Option<(Damage, i64)>) -> bool {
        self.committed == damage
    }
}

impl super::DamageLevel<'_> {
    pub fn accepted_hit(&self, hurting: super::Hurting<'_>) -> Option<std::sync::Arc<AcceptedHit>> {
        self.hooks
            .accepted
            .and_then(|capture| capture(hurting, self.attack))
            .map(std::sync::Arc::new)
    }
}

impl super::PausedHurt {
    /// Borrow the hit's existing name and current factory data through an equipment pause.
    pub fn source<'a>(&'a self, attack: &Attack) -> HurtSource<'a> {
        attack.player_source(&self.source_name, self.causing)
    }
}
