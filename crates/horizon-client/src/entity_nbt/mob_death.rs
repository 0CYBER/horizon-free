//! The existing living-health clocks plus Horizon's cancelled-die continuation.
use crate::{
    mobs::Mob,
    save::{number, put},
};
use horizon_core::nbt::Tag;

pub(super) fn write(mob: &Mob, tag: &mut Tag) {
    put(tag, "Health", Tag::Float(mob.health.max(0.0).to_bits()));
    // LivingEntity.addAdditionalSaveData: (short) casts, nothing clamped.
    put(tag, "HurtTime", Tag::Short(mob.hurt_time as i16));
    put(tag, "DeathTime", Tag::Short(mob.death_time as i16));
    if mob.death_canceled {
        put(tag, "horizon:death_canceled", Tag::Byte(1));
    } else if tag.get("horizon:death_canceled").is_some()
        && let Tag::Compound(map) = tag
    {
        map.remove(&"horizon:death_canceled".into());
    }
}

pub(super) fn read(mob: &mut Mob, tag: &Tag) {
    mob.death_time = i32::from(number(tag.get("DeathTime")).unwrap_or(0) as i16);
    mob.death_canceled = super::flag(tag, "horizon:death_canceled");
}
