use super::*;

#[test]
fn arrow_family_belongs_to_the_actual_direct_entity_not_the_type() {
    let arrow = Damage::of(DamageKind::Arrow);
    let attack = Attack {
        projectile: Some(12),
        projectile_is_arrow: true,
        ..Default::default()
    };
    assert!(attack.direct_is_arrow(&arrow));
    assert!(!Attack::default().direct_is_arrow(&arrow));
    assert!(!attack.direct_is_arrow(&Damage::of(DamageKind::Magic)));
    assert!(!attack.direct_is_arrow(&Damage::of(DamageKind::Explosion)));
    assert!(!attack.direct_is_arrow(&Damage::of(DamageKind::MobAttack)));
    assert!(
        !Attack {
            direct: Some(0),
            ..attack.clone()
        }
        .direct_is_arrow(&arrow)
    );
    assert!(
        !Attack {
            direct: Some(99),
            ..attack.clone()
        }
        .direct_is_arrow(&arrow)
    );
    assert!(
        Attack {
            direct: Some(12),
            ..attack.clone()
        }
        .direct_is_arrow(&arrow)
    );
    assert!(
        !Attack {
            projectile_is_arrow: false,
            ..attack.clone()
        }
        .direct_is_arrow(&arrow)
    );
    assert!(
        !Attack {
            projectile: Some(0),
            ..attack
        }
        .direct_is_arrow(&arrow)
    );
}

#[test]
fn trident_source_identity_does_not_imply_an_abstract_arrow() {
    let trident = Damage::of(DamageKind::Named(crate::mobs::NamedDamage::new(
        "minecraft:trident",
        0,
    )));
    let attack = Attack {
        projectile: Some(12),
        projectile_is_arrow: true,
        ..Default::default()
    };
    assert!(attack.direct_is_arrow(&trident));
    assert_eq!(
        mob_source(&trident, &attack).damage_type,
        "minecraft:trident"
    );
    assert_eq!(mob_source(&trident, &attack).direct, 12);
    assert!(!Attack::default().direct_is_arrow(&trident));
    assert!(
        !Attack {
            direct: Some(0),
            ..attack
        }
        .direct_is_arrow(&trident)
    );
}
