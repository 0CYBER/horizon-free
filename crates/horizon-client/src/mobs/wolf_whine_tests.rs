//! `Wolf.getAmbientSound` (#16865, `ai::play_ambient`): a tame wolf whines, one ambient sound in
//! three, while its health is under 20, a constant that does not follow its
//! max health; otherwise it pants.
use crate::mobs::finalize::{Attr, set_base};
use crate::mobs::world::with_test_world;
use crate::mobs::{Mob, MobEvent, MobWorld};
use horizon_core::geometry::Vec3;

/// Whether 60 ambient sounds of a tame wolf with this max health and health
/// hold a whine and a pant.
fn whines_and_pants(
    w: &MobWorld<'_>,
    types: &crate::mobs::MobTypes,
    max: f64,
    health: f32,
) -> (bool, bool) {
    let kind = std::sync::Arc::new(types.types["minecraft:wolf"].clone());
    let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    super::set_tame(&mut m, true);
    set_base(&mut m, Attr::MaxHealth, max);
    m.health = health;
    let mut events = Vec::new();
    for _ in 0..60 {
        crate::mobs::ai::ambient_sound(&mut m, w, &mut events);
    }
    let heard = |id: &str| {
        events
            .iter()
            .any(|e| matches!(e, MobEvent::Sound(s, ..) if *s == id))
    };
    (heard("entity.wolf.whine"), heard("entity.wolf.pant"))
}

#[test]
fn a_tame_wolf_whines_under_twenty_health_whatever_its_max() {
    with_test_world(&[], |w, types| {
        // A tamed wolf's own 40: full health pants, 19 whines.
        assert_eq!(whines_and_pants(w, types, 40.0, 40.0), (false, true));
        assert_eq!(whines_and_pants(w, types, 40.0, 19.0), (true, false));
        // A max of 100 (`/attribute`): 30 health is under half, yet it pants.
        assert_eq!(whines_and_pants(w, types, 100.0, 30.0), (false, true));
        // A tame wolf loaded with the wild 8 (`setTame(true, false)` keeps the
        // saved max): at full health it whines.
        assert_eq!(whines_and_pants(w, types, 8.0, 8.0), (true, false));
    });
}
