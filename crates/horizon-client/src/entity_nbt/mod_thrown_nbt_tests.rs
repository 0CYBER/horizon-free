use super::*;

#[test]
fn a_mod_thrown_type_saves_its_id_and_item_and_loads_back() {
    crate::projectiles::register_test_types();
    let bolt = crate::projectiles::thrown_type("ruby:bolt").unwrap();
    let mut p = Projectile::new(
        ProjectileKind::Thrown(bolt),
        Vec3::new(1.0, 80.0, 2.0),
        None,
    );
    p.velocity = Vec3::new(0.0, 0.5, 1.0);
    assert!(saved_with_chunk(&p));
    let tag = throwable_to_nbt(&p, [1; 4], None, None).unwrap();
    assert_eq!(super::kind(&tag).as_deref(), Some("ruby:bolt"));
    let item = tag.get("Item").and_then(item_from_nbt).unwrap();
    assert_eq!(&*item.item, "ruby:gem");
    let back = throwable_from_nbt(&tag, None).unwrap();
    assert_eq!((&back.kind, back.velocity), (&p.kind, p.velocity));
    // NoGravity comes back too, as for vanilla's (a summoned bolt that floats).
    p.no_gravity = true;
    let tag = throwable_to_nbt(&p, [1; 4], None, None).unwrap();
    assert!(throwable_from_nbt(&tag, None).unwrap().no_gravity);
    // a type no mod declares now is not a thrown kind.
    assert!(throwable_kind("gone:bolt").is_none());
}
fn summoned(id: &str, extra: &[(&str, Tag)]) -> Tag {
    let mut c = vec![
        ("id", Tag::String(id.into())),
        ("Pos", doubles(&[0.5, 110.0, 0.5])),
        ("Motion", doubles(&[0.0, 0.0, 0.0])),
    ];
    c.extend(extra.iter().cloned());
    Tag::compound(c)
}

/// Four summoned kinds had no loader and stayed inert (a spectral arrow,
/// a shulker bullet, a llama's spit and an experience bottle never fell).
#[test]
fn summoned_projectiles_of_every_kind_load_and_honour_no_gravity() {
    use crate::entity_rules::EntityRules;
    let flag = ("NoGravity", Tag::Byte(1));
    let arrow = arrow_from_nbt(&summoned("minecraft:spectral_arrow", &[]), None, |_| None)
        .unwrap()
        .0;
    assert!(arrow.is_spectral_arrow() && !arrow.no_gravity);
    let held = arrow_from_nbt(
        &summoned("minecraft:spectral_arrow", std::slice::from_ref(&flag)),
        None,
        |_| None,
    )
    .unwrap()
    .0;
    assert!(held.no_gravity);
    // ...and it is saved as the entity type it is.
    let saved = arrow_to_nbt(&arrow, [1; 4], None, None, None).unwrap();
    assert_eq!(kind(&saved).as_deref(), Some("minecraft:spectral_arrow"));
    let plain = arrow_from_nbt(&summoned("minecraft:arrow", &[]), None, |_| None)
        .unwrap()
        .0;
    let saved = arrow_to_nbt(&plain, [1; 4], None, None, None).unwrap();
    assert_eq!(kind(&saved).as_deref(), Some("minecraft:arrow"));

    for id in ["minecraft:llama_spit", "minecraft:shulker_bullet"] {
        let p = spit_or_bullet_from_nbt(&summoned(id, &[]), None, None).unwrap();
        assert_eq!(p.kind.id(), id);
        assert!(!p.no_gravity && p.position == Vec3::new(0.5, 110.0, 0.5));
        let held = spit_or_bullet_from_nbt(&summoned(id, std::slice::from_ref(&flag)), None, None)
            .unwrap();
        assert!(held.no_gravity);
    }
    assert!(spit_or_bullet_from_nbt(&summoned("minecraft:arrow", &[]), None, None).is_none());
    // a spit and a target-less bullet are saved and selectable (`/data`, `@e`).
    let spit = spit_or_bullet_from_nbt(&summoned("minecraft:llama_spit", &[]), None, None).unwrap();
    assert!(saved_with_chunk(&spit));
    let saved = spit_or_bullet_to_nbt(&spit, [1; 4], None, None, None).unwrap();
    assert_eq!(kind(&saved).as_deref(), Some("minecraft:llama_spit"));

    // A summoned experience bottle flies as the bottle item, with a bottle's gravity.
    let bottle = throwable_from_nbt(&summoned("minecraft:experience_bottle", &[]), None).unwrap();
    assert_eq!(bottle.kind.id(), "minecraft:experience_bottle");
    assert!(!bottle.no_gravity && (bottle.get_gravity() - 0.07).abs() < 1e-12);
    let held = throwable_from_nbt(&summoned("minecraft:experience_bottle", &[flag]), None).unwrap();
    assert!(held.no_gravity && held.get_gravity() == 0.0);
    let saved = throwable_to_nbt(&bottle, [1; 4], None, None).unwrap();
    assert_eq!(kind(&saved).as_deref(), Some("minecraft:experience_bottle"));
    let item = saved.get("Item").and_then(item_from_nbt).unwrap();
    assert_eq!(&*item.item, "minecraft:experience_bottle");
}

#[test]
fn no_gravity_survives_save_and_load_for_orbs() {
    // Entity.saveWithoutId / load: NoGravity round-trips on a kind that
    // fall by their own tick (orb, item, arrow, fireball).
    let mut orb = orbs_from_nbt(&Tag::compound([
        ("Pos", doubles(&[0.0, 10.0, 0.0])),
        ("NoGravity", Tag::Byte(1)),
    ]))
    .remove(0);
    assert!(orb.no_gravity);
    let back = orbs_from_nbt(&orb_to_nbt(&orb, [1; 4])).remove(0);
    assert!(back.no_gravity);
    orb.no_gravity = false;
    assert!(orb_to_nbt(&orb, [1; 4]).get("NoGravity").is_none());
}
