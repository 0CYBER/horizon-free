//! Prints Horizon's mob attributes and dimensions in the format of
//! `VanillaMobAttributesOracle` (tools/test_mob_attributes_parity.py).
//!
//! usage: mob_attributes_fixture RESOURCES OUTPUT
use horizon_client::mobs::{Mob, MobTypes};
use horizon_core::geometry::Vec3;
use std::{fmt::Write as _, fs, path::Path, sync::Arc};

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: mob_attributes_fixture RESOURCES OUTPUT")?,
    );
    let types = MobTypes::load(root)?;
    let mut ids: Vec<&String> = types.types.keys().collect();
    ids.sort();
    let mut out = String::new();
    for id in ids {
        let kind = Arc::new(types.types[id].clone());
        let mut mob = Mob::new(1, kind.clone(), Vec3::new(0.0, 64.0, 0.0), 0.0);
        let d = mob.dims();
        let _ = write!(out, "{id}\tdim {:?} {:?} {:?}", d.0, d.1, d.2);
        let _ = write!(out, "\tscale {:?}", 1.0);
        let a = &mob.attrs;
        let _ = write!(
            out,
            "\tattrs max_health={:?} movement_speed={:?} attack_damage={:?} follow_range={:?}",
            f64::from(a.max_health),
            a.speed,
            f64::from(a.attack_damage),
            a.follow_range
        );
        let _ = write!(
            out,
            " flying_speed={:?} attack_knockback={:?} step_height={:?} armor={:?} armor_toughness={:?} knockback_resistance={:?} jump_strength={:?} gravity={:?} safe_fall_distance={:?} fall_damage_multiplier={:?} water_movement_efficiency={:?}",
            kind.flying_speed,
            kind.attack_knockback,
            kind.step_height,
            f64::from(kind.armor),
            f64::from(kind.armor_toughness),
            kind.knockback_resistance,
            kind.jump_strength,
            kind.gravity,
            kind.safe_fall_distance,
            kind.fall_damage_multiplier,
            kind.water_efficiency
        );
        mob.extra.age = -1;
        let b = mob.dims();
        let _ = write!(out, "\tbaby {:?} {:?} {:?}", b.0, b.1, b.2);
        out.push('\n');
    }
    fs::write(args.get(2).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
