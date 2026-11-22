use super::*;

/// The level's side of a bullet's `Target`: the id -> UUID map
/// (`EntityMemory.uuids`), the mobs it currently holds, and the player.
/// `save_who_uuid` and `bullet_target` are the game layer's two helpers,
/// kept here as the same two calls so both halves of the bridge are what the
/// round trips below go through.
struct Level {
    uuids: HashMap<u32, [i32; 4]>,
    mobs: Vec<Mob>,
    player: Option<[i32; 4]>,
}
impl Level {
    /// `Game::save_who_uuid`.
    fn save_who_uuid(&self, who: Who) -> Option<[i32; 4]> {
        who_uuid(who, &mut |id| self.uuids.get(&id).copied())
    }
    /// `Game::bullet_target`: the `Who` a saved `Target` names, while that
    /// entity is in the level.
    fn bullet_target(&self, tag: &Tag) -> Option<Who> {
        who_by_uuid(&self.uuids, &self.mobs, self.player, bullet_target(tag)?)
    }
    /// A shulker in the level: its id and the UUID it is saved with.
    fn with_mob(mut self, id: u32, uuid: [i32; 4]) -> Self {
        self.uuids.insert(id, uuid);
        self.mobs.push(Mob::new(
            id,
            crate::mobs::sensor_scan_tests::kind(),
            Vec3::ZERO,
            0.0,
        ));
        self
    }
}
fn no_entities() -> Level {
    Level {
        uuids: HashMap::new(),
        mobs: Vec::new(),
        player: None,
    }
}
/// The bullet the tests start from: fired by the player at mob 31 along +X.
fn homing(target: Option<Who>) -> Projectile {
    Projectile::new(
        ProjectileKind::ShulkerBullet {
            target,
            dir: Some(5),
            steps: 23,
            delta: [0.15, -0.15, 0.075],
            seed: 7,
        },
        Vec3::new(4.5, 80.0, 10.5),
        Some(Who::Player),
    )
}
fn bullet_kind(p: &Projectile) -> (Option<Who>, Option<u8>, i32, [f64; 3]) {
    let ProjectileKind::ShulkerBullet {
        target,
        dir,
        steps,
        delta,
        ..
    } = p.kind
    else {
        panic!("not a bullet: {:?}", p.kind);
    };
    (target, dir, steps, delta)
}

/// `ShulkerBullet.addAdditionalSaveData` then `readAdditionalSaveData`, both
/// over the UUID bridge: a bullet in flight keeps the target it homes on, the
/// step it is on, the steps left and its per-axis target deltas, and the
/// target comes back as the `Who::Mob` of the shulker it was fired at.
///
/// Horizon wrote none of it and `saved_with_chunk` turned the kind away while
/// it had a target, so every bullet a shulker fired was dropped by the chunk
/// save.
#[test]
fn a_homing_bullet_round_trips_its_target_step_and_deltas() {
    let shulker = [9, 8, 7, 6];
    let level = no_entities().with_mob(31, shulker);
    let p = homing(Some(Who::Mob(31)));
    assert!(saved_with_chunk(&p), "a homing bullet is saved now");
    // The writer `Game::save_projectile` calls, given the UUID
    // `Game::save_who_uuid` answers for the target.
    let target = level.save_who_uuid(Who::Mob(31));
    assert_eq!(target, Some(shulker));
    let tag = spit_or_bullet_to_nbt(&p, [1; 4], None, target, None).unwrap();
    assert_eq!(kind(&tag).as_deref(), Some("minecraft:shulker_bullet"));
    assert_eq!(
        bullet_target(&tag),
        Some(shulker),
        "`Target` is the shulker's UUID"
    );
    assert_eq!(number(tag.get("Dir")), Some(5));
    assert_eq!(number(tag.get("Steps")), Some(23));
    assert_eq!(float(tag.get("TXD")), Some(0.15));
    assert_eq!(float(tag.get("TYD")), Some(-0.15));
    assert_eq!(float(tag.get("TZD")), Some(0.075));
    // Read back the way `Game::load_spit_or_bullet` does: the compound's
    // `Target` resolved against the level, then the bullet's own fields.
    let target = level.bullet_target(&tag);
    assert_eq!(target, Some(Who::Mob(31)), "back on the same shulker");
    let p = spit_or_bullet_from_nbt(&tag, None, target).unwrap();
    assert_eq!(
        bullet_kind(&p),
        (Some(Who::Mob(31)), Some(5), 23, [0.15, -0.15, 0.075])
    );
}

/// `ShulkerBullet.readAdditionalSaveData`: `Dir` goes through
/// `Direction.from3DDataValue`, which wraps any byte, and `Steps` is read
/// as saved, negative or not.
#[test]
fn a_loaded_dir_wraps_and_steps_keep_their_sign() {
    // `Direction.from3DDataValue(b)` is `BY_3D_DATA[abs(b % 6)]`, and
    // `getIntOr` keeps a negative step count.
    for (dir, steps, expected) in [(7, -3, (1, -3)), (-2, 4, (2, 4)), (5, 0, (5, 0))] {
        let tag = Tag::compound([
            ("id", Tag::String("minecraft:shulker_bullet".into())),
            ("Pos", doubles(&[4.5, 80.0, 10.5])),
            ("Dir", Tag::Byte(dir)),
            ("Steps", Tag::Int(steps)),
        ]);
        let p = spit_or_bullet_from_nbt(&tag, None, None).unwrap();
        let (_, d, s, _) = bullet_kind(&p);
        assert_eq!(
            (d, s),
            (Some(expected.0), expected.1),
            "Dir {dir}, Steps {steps}"
        );
    }
}

/// A bullet with no target — a `/summon`ed one, or one loaded without its
/// target — writes no `Target` key at all, as vanilla's
/// `if (this.finalTarget != null)` does, and no `Dir` until it has chosen a
/// step. `Steps` and the deltas are always written, zero or not.
#[test]
fn a_bullet_without_a_target_writes_no_target_key() {
    let bare = Tag::compound([
        ("id", Tag::String("minecraft:shulker_bullet".into())),
        ("Pos", doubles(&[4.5, 80.0, 10.5])),
    ]);
    let level = no_entities().with_mob(31, [9, 8, 7, 6]);
    let p = spit_or_bullet_from_nbt(&bare, None, level.bullet_target(&bare)).unwrap();
    assert_eq!(
        bullet_kind(&p),
        (None, None, 0, [0.0; 3]),
        "a bare summon has no target and no step"
    );
    let saved = spit_or_bullet_to_nbt(&p, [1; 4], None, None, None).unwrap();
    assert!(saved.get("Target").is_none(), "no `Target`");
    assert!(saved.get("Dir").is_none(), "no `Dir` before a step");
    assert_eq!(number(saved.get("Steps")), Some(0));
    assert_eq!(float(saved.get("TXD")), Some(0.0));
    // The same for a bullet whose target the level has no UUID for: the
    // bridge writes nothing rather than a made-up UUID, and a re-read
    // bullet has no target.
    let p = homing(Some(Who::Mob(31)));
    let nowhere = no_entities();
    assert_eq!(nowhere.save_who_uuid(Who::Mob(31)), None);
    let saved = spit_or_bullet_to_nbt(&p, [1; 4], None, None, None).unwrap();
    assert!(saved.get("Target").is_none());
    assert_eq!(nowhere.bullet_target(&saved), None);
}

/// The stored `Target` names a mob that is no longer loaded: vanilla keeps
/// the UUID and re-resolves it every tick, and Horizon's `Who` can only name
/// an entity in the level, so the bullet comes back with no target and falls
/// rather than homing on whatever the id now means. Here the id the mob had
/// is reused by a projectile the same save holds, which is the case a bare
/// id would get wrong.
#[test]
fn a_target_that_is_not_loaded_comes_back_with_none() {
    let shulker = [9, 8, 7, 6];
    let p = homing(Some(Who::Mob(31)));
    let tag = spit_or_bullet_to_nbt(&p, [1; 4], None, Some(shulker), None).unwrap();
    // id 31 is a projectile now, and 32 is the only mob left.
    let mut uuids = HashMap::from([(31, shulker), (32, [1, 1, 1, 1])]);
    let mut mobs = vec![Mob::new(
        32,
        crate::mobs::sensor_scan_tests::kind(),
        Vec3::ZERO,
        0.0,
    )];
    let gone = Level {
        uuids: uuids.clone(),
        mobs: mobs.clone(),
        player: None,
    };
    assert_eq!(gone.bullet_target(&tag), None, "not the id-reusing entity");
    let p = spit_or_bullet_from_nbt(&tag, None, gone.bullet_target(&tag)).unwrap();
    let (target, dir, steps, delta) = bullet_kind(&p);
    assert_eq!(target, None, "targetless, and it falls");
    assert_eq!(
        (dir, steps, delta),
        (Some(5), 23, [0.15, -0.15, 0.075]),
        "the rest of the bullet is still its own"
    );
    // The mob that is here still resolves, so the `None` above is the mob
    // being gone and not the read half refusing every mob.
    uuids.insert(33, [2, 2, 2, 2]);
    mobs.push(Mob::new(
        33,
        crate::mobs::sensor_scan_tests::kind(),
        Vec3::ZERO,
        0.0,
    ));
    let here = Level {
        uuids,
        mobs,
        player: None,
    };
    assert_eq!(here.bullet_target(&tag), None);
    let mut moved = tag.clone();
    put(&mut moved, "Target", uuid_tag([2, 2, 2, 2]));
    assert_eq!(here.bullet_target(&moved), Some(Who::Mob(33)));
}

/// The player is the other `Who` a `Target` can name: the same bridge, the
/// player's own UUID.
#[test]
fn a_target_naming_the_player_comes_back_as_the_player() {
    let player = player_uuid();
    let level = Level {
        uuids: HashMap::new(),
        mobs: Vec::new(),
        player: Some(player),
    };
    let p = homing(Some(Who::Player));
    let target = level.save_who_uuid(Who::Player);
    assert_eq!(target, Some(player));
    let tag = spit_or_bullet_to_nbt(&p, [1; 4], None, target, None).unwrap();
    assert_eq!(level.bullet_target(&tag), Some(Who::Player));
    // With no player in the level the UUID names nobody.
    let empty = Level {
        uuids: HashMap::new(),
        mobs: Vec::new(),
        player: None,
    };
    assert_eq!(empty.bullet_target(&tag), None);
}
