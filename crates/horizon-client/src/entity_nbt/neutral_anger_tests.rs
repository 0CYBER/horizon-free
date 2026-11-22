use super::*;

/// #2784: a neutral mob's anger survives the save: the anger ends at an
/// absolute game time and the target comes back with it
/// (`NeutralMob.addPersistentAngerSaveData` / `readPersistentAngerSaveData`).
#[test]
fn a_neutral_mobs_anger_survives_the_save() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let mut m = Mob::new(
        1,
        types["minecraft:wolf"].clone(),
        Vec3::new(0.0, 64.0, 0.0),
        0.0,
    );
    m.extra.anger = 400;
    m.extra.anger_end = 3000;
    m.extra.angry_at = Some(crate::mobs::Who::Player);
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(number(tag.get("anger_end_time")), Some(3000));
    assert!(tag.get("angry_at").is_some());
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert_eq!(back.extra.anger_end, 3000);
    assert_eq!(back.extra.angry_at, Some(crate::mobs::Who::Player));
    // An iron golem angry at a monster: the target's UUID comes back as
    // anger at a mob (not at every player under universal_anger), and is
    // written back as it was read.
    let mut golem = Mob::new(4, types["minecraft:iron_golem"].clone(), Vec3::ZERO, 0.0);
    golem.extra.anger = 400;
    golem.extra.anger_end = 3000;
    let mut raw = mob_to_nbt(&golem, [1, 2, 3, 6], None, None, |_| None);
    if let Tag::Compound(map) = &mut raw {
        map.insert("angry_at".into(), Tag::IntArray(vec![9, 8, 7, 6]));
    }
    let back = mob_from_nbt(&raw, &types, 5, None).unwrap();
    assert_eq!(back.extra.angry_at, Some(crate::mobs::Who::Mob(0)));
    let again = mob_to_nbt(&back, [1, 2, 3, 6], Some(&raw), None, |_| None);
    assert_eq!(
        crate::save::uuid_from_tag(again.get("angry_at")),
        Some([9, 8, 7, 6])
    );
    // Once the anger is over (`stopBeingAngry`), the loaded target goes.
    let mut calm = back.clone();
    calm.extra.angry_at = None;
    let again = mob_to_nbt(&calm, [1, 2, 3, 6], Some(&raw), None, |_| None);
    assert!(again.get("angry_at").is_none());
    // A mob that is no NeutralMob writes no anger.
    let pig = Mob::new(3, types["minecraft:pig"].clone(), Vec3::ZERO, 0.0);
    let tag = mob_to_nbt(&pig, [1, 2, 3, 5], None, None, |_| None);
    assert!(tag.get("anger_end_time").is_none());
    // The anger that is left when the mob comes back.
    assert_eq!(crate::mobs::anger_left(3000, 1000), 2000);
    assert_eq!(crate::mobs::anger_left(3000, 4000), 0);
}
