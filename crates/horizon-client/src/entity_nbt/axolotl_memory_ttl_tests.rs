use super::*;

fn memory<'a>(tag: &'a Tag, key: &str) -> Option<&'a Tag> {
    tag.get("Brain")?.get("memories")?.get(key)
}

#[test]
fn integer_memory_saves_keep_finite_ttl_and_drop_it_after_a_permanent_write() {
    for key in [
        "minecraft:play_dead_ticks",
        "minecraft:temptation_cooldown_ticks",
    ] {
        for ttl in [None, Some(0), Some(1), Some(5), Some(-1), Some(i64::MAX)] {
            let mut entry = Tag::compound([("value", Tag::Int(100))]);
            if let Some(ttl) = ttl {
                put(&mut entry, "ttl", Tag::Long(ttl));
            }
            let raw = Tag::compound([(
                "Brain",
                Tag::compound([("memories", Tag::compound([(key, entry)]))]),
            )]);
            let mut kind = crate::mobs::sensor_scan_tests::kind();
            let ty = Arc::make_mut(&mut kind);
            ty.id = "minecraft:axolotl".into();
            ty.species = crate::mobs::Species::Axolotl;
            let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
            restore_axolotl_memories(&mut mob, &raw);
            let saved = mob_to_nbt(&mob, [0; 4], Some(&raw), None, |_| None);
            let entry = memory(&saved, key).expect("loaded value must be present");
            assert_eq!(entry.get("value"), Some(&Tag::Int(100)));
            let expected = ttl.filter(|v| *v != i64::MAX).map(Tag::Long);
            assert_eq!(entry.get("ttl"), expected.as_ref(), "{key}: {ttl:?}");

            // CountDownCooldownTicks/ValidatePlayDead write ordinary permanent memories.
            let brain = mob.extra.animal_brain.as_mut().unwrap();
            if key == "minecraft:play_dead_ticks" {
                brain.mem.play_dead_ticks.set(99);
            } else {
                brain.mem.temptation_cooldown_ticks.set(99);
            }
            let saved = mob_to_nbt(&mob, [0; 4], Some(&raw), None, |_| None);
            let entry = memory(&saved, key).unwrap();
            assert_eq!(entry.get("value"), Some(&Tag::Int(99)));
            assert!(entry.get("ttl").is_none(), "stale raw TTL must not return");
        }
    }
}
