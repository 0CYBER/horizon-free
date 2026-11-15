//! Independent 26.3 Projectile.shootFromRotation sequence fixtures.
use super::{Projectile, ProjectileKind};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};

#[test]
fn rotation_shot_consumes_six_doubles_in_axis_order_and_inherits_motion() {
    // java.util.Random(1234).nextDouble():
    // .6465821602909256, .9513577109193919, .8575884598068334,
    // .45823330506267057, .3359524025416939, .20387478195313158.
    // Vanilla Projectile.shoot: normalize [0,0,1], add x/y/z triangles
    // scaled by .0172275, multiply by 1.5, then add known movement.
    for grounded in [false, true] {
        let mut shot = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None)
            .with_entity_random(LegacyRandom::new(1234));
        shot.shoot_from_rotation(0.0, 0.0, 0.0, 1.5, 1.0, Vec3::new(0.2, 0.3, -0.4), grounded);
        let expected_y = if grounded {
            0.0103198363925326
        } else {
            0.3103198363925326
        };
        for (actual, expected) in [
            (shot.velocity.x, 0.19212421880232217),
            (shot.velocity.y, expected_y),
            (shot.velocity.z, 1.1034130508130342),
        ] {
            assert!((actual - expected).abs() < 1e-14, "{actual} != {expected}");
        }
        assert_eq!(shot.entity_random().next_double(), 0.34690742873967684);
        assert!(shot.needs_sync);
    }
}

#[test]
fn rotation_shot_applies_vanilla_near_zero_normalization_without_skipping_draws() {
    let mut shot = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None)
        .with_entity_random(LegacyRandom::new(1234));
    // At pitch 90 with a -90 vertical offset all three direction components
    // are below Vec3.normalize's widened 1.0E-5F threshold. Spread is zero,
    // but Projectile.getMovementToShoot still takes all six double draws.
    shot.shoot_from_rotation(90.0, 0.0, -90.0, 1.5, 0.0, Vec3::new(0.2, 0.3, -0.4), false);
    assert_eq!(shot.velocity, Vec3::new(0.2, 0.3, -0.4));
    assert_eq!(shot.entity_random().next_double(), 0.34690742873967684);
    assert!(shot.needs_sync);
}
