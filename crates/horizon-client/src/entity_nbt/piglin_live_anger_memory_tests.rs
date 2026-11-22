use super::*;

fn mob() -> Mob {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.id = "minecraft:piglin".into();
    ty.species = crate::mobs::Species::Piglin;
    Mob::new(1, kind, Vec3::ZERO, 0.0)
}
fn raw() -> Tag {
    Tag::compound([(
        "Brain",
        Tag::compound([
            ("audit:root", Tag::Int(8)),
            (
                "memories",
                Tag::compound([
                    ("audit:keep", Tag::compound([("value", Tag::Int(7))])),
                    (
                        "minecraft:angry_at",
                        Tag::compound([
                            ("value", Tag::IntArray(vec![90, 91, 92, 93])),
                            ("ttl", Tag::Long(99)),
                        ]),
                    ),
                ]),
            ),
        ]),
    )])
}
fn memory(tag: &Tag) -> Option<&Tag> {
    tag.get("Brain")?.get("memories")?.get("minecraft:angry_at")
}
fn assert_unrelated(saved: &Tag) {
    let brain = saved.get("Brain").unwrap();
    assert_eq!(brain.get("audit:root"), Some(&Tag::Int(8)));
    assert_eq!(
        brain.get("memories").unwrap().get("audit:keep"),
        Some(&Tag::compound([("value", Tag::Int(7))]))
    );
}

#[test]
fn guarded_event_saves_actual_uuid_and_remaining_long_ttl() {
    let raw = raw();
    for actor in [[1, 2, 3, 4], [-5, 6, 7, 8]] {
        let mut m = mob();
        crate::mobs::piglin::set_guarded_anger_target(&mut m, Some(actor));
        assert_eq!(m.extra.anger, 600);
        assert_eq!(m.ai.target, Some(Who::Player));
        for remaining in [600, 520] {
            m.extra.anger = remaining;
            for base in [None, Some(&raw)] {
                let saved = mob_to_nbt(&m, [42, 43, 44, 45], base, None, |_| None);
                assert_eq!(
                    memory(&saved),
                    Some(&Tag::compound([
                        ("value", Tag::IntArray(actor.to_vec())),
                        ("ttl", Tag::Long(i64::from(remaining))),
                    ]))
                );
                if base.is_some() {
                    assert_unrelated(&saved);
                }
            }
        }
    }
}

/// `Piglin.readAdditionalSaveData` reads the brain's `ANGRY_AT` memory
/// back: the actor's UUID and the ticks left (`ExpirableValue`'s TTL).
/// Horizon wrote the memory but never read it, so a piglin that a player
/// had angered came back calm after its chunk reloaded.
#[test]
fn a_saved_guarded_anger_loads_its_uuid_and_ticks() {
    // A whole saved piglin: its id, a position and the brain's memory.
    let saved = |memory: Tag| {
        Tag::compound([
            ("id", Tag::String("minecraft:piglin".into())),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Double(64.0f64.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                ]),
            ),
            (
                "Brain",
                Tag::compound([("memories", Tag::compound([("minecraft:angry_at", memory)]))]),
            ),
        ])
    };
    let template = mob();
    let types = [(template.kind.id.clone(), template.kind.clone())].into();
    let uuid = Tag::IntArray(vec![90, 91, 92, 93]);
    let ticking = saved(Tag::compound([
        ("value", uuid.clone()),
        ("ttl", Tag::Long(99)),
    ]));
    let loaded = mob_from_nbt(&ticking, &types, 2, None).unwrap();
    assert_eq!(loaded.extra.anger, 99);
    assert_eq!(
        loaded.extra.piglin.guarded_angry_at,
        Some(Some([90, 91, 92, 93]))
    );
    // `ExpirableValue` leaves the TTL out when it never expires, and a
    // long past the counter's range holds as long as it can: the counter
    // keeps the longest it can while the goal ticks it down.
    for memory in [
        Tag::compound([("value", uuid.clone())]),
        Tag::compound([("value", uuid.clone()), ("ttl", Tag::Long(1 << 40))]),
    ] {
        let loaded = mob_from_nbt(&saved(memory), &types, 3, None).unwrap();
        assert_eq!(loaded.extra.anger, i32::MAX);
        assert_eq!(
            loaded.extra.piglin.guarded_angry_at,
            Some(Some([90, 91, 92, 93]))
        );
    }
}

#[test]
fn expired_owned_memory_cannot_resurrect_stale_raw_anger() {
    let mut m = mob();
    crate::mobs::piglin::set_guarded_anger_target(&mut m, Some([1, 2, 3, 4]));
    m.extra.anger = 0;
    m.ai.target = None;
    let saved = mob_to_nbt(&m, [42, 43, 44, 45], Some(&raw()), None, |_| None);
    assert!(memory(&saved).is_none());
    assert_unrelated(&saved);
}

#[test]
fn calm_loaded_and_missing_identity_states_are_distinct() {
    let mut m = mob();
    assert!(memory(&mob_to_nbt(&m, [42, 43, 44, 45], None, None, |_| None)).is_none());
    let raw = raw();
    // Loaded Brain semantics stay outside this live event projection.
    let untouched = mob_to_nbt(&m, [42, 43, 44, 45], Some(&raw), None, |_| None);
    assert_eq!(memory(&untouched), memory(&raw));
    crate::mobs::piglin::set_guarded_anger_target(&mut m, None);
    assert_eq!(m.extra.anger, 600);
    assert_eq!(m.ai.target, Some(Who::Player));
    let saved = mob_to_nbt(&m, [42, 43, 44, 45], Some(&raw), None, |_| None);
    assert!(memory(&saved).is_none());
    assert_unrelated(&saved);
}
