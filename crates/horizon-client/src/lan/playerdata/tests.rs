use super::*;
use crate::lan::{Joined, PlayerSender};
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

fn guest() -> Guest {
    let (tx, _rx) = mpsc::channel();
    Guest::new(
        Joined {
            conn: 2,
            uuid: 2,
            name: "Alex".into(),
            properties: Vec::new(),
            info: ClientInformation::new(8),
            brand: "test".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        2,
        [0.0, 64.0, 0.0],
        8,
    )
}

/// #9965: the owner's warden warning level and cooldown reach the file on
/// a save, instead of the ones the file was loaded with.
#[test]
fn a_stored_warden_tracker_replaces_the_loaded_one() {
    let mut g = guest();
    g.saved = Some(Tag::compound(vec![
        (
            "warden_spawn_tracker",
            Tag::compound(vec![
                ("warning_level", Tag::Int(2)),
                ("cooldown_ticks", Tag::Int(77)),
                ("ticks_since_last_warning", Tag::Int(5)),
            ]),
        ),
        ("unrelated_key", Tag::Int(9)),
    ]));
    let tracker = crate::sculk::WardenSpawnTracker {
        ticks_since_last_warning: 0,
        warning_level: 3,
        cooldown_ticks: 200,
    };
    with_warden(&mut g, Some(tracker.to_nbt()));
    let (tag, _) = round_trip(&g);
    let mut back = guest();
    let mut level = Level::default();
    level.read_player(&tag);
    level.player_extra = Some(tag);
    apply(&mut back, &level);
    let saved = back.saved.as_ref().unwrap();
    let read =
        crate::sculk::WardenSpawnTracker::from_nbt(saved.get("warden_spawn_tracker").unwrap());
    assert_eq!((read.warning_level, read.cooldown_ticks), (3, 200));
    assert_eq!(read.ticks_since_last_warning, 0);
    assert!(matches!(saved.get("unrelated_key"), Some(Tag::Int(9))));
}

#[test]
fn a_new_player_faces_the_world_spawn_angle() {
    let mut g = guest();
    face_world_spawn(&mut g, false, [90.0, -15.0]);
    assert_eq!((g.yaw, g.pitch, g.head_yaw), (90.0, -15.0, 90.0));
}

#[test]
fn a_saved_rotation_beats_the_world_spawn_angle() {
    let mut g = guest();
    let level = Level {
        rotation: [-135.0, 30.0],
        ..Level::default()
    };
    apply(&mut g, &level);
    face_world_spawn(&mut g, true, [90.0, -15.0]);
    assert_eq!((g.yaw, g.pitch, g.head_yaw), (-135.0, 30.0, -135.0));
}

#[test]
fn body_and_saddle_survive_a_save_in_vanillas_equipment_compound() {
    let dir = world("body-saddle");
    let mut g = holding(7, "minecraft:diamond", 1);
    let mut saddle = ItemStack::new("minecraft:saddle", 1);
    saddle.damage = 3;
    g.equipment.set(
        horizon_items::equipment::BODY,
        Some(ItemStack::new("minecraft:leather_horse_armor", 1)),
    );
    g.equipment
        .set(horizon_items::equipment::SADDLE, Some(saddle));
    g.armor[0] = Some(ItemStack::new("minecraft:iron_helmet", 1));
    save(&g, &dir);
    let tag = read_dat(&path(&dir, 7)).unwrap();
    let equipment = tag.get("equipment").unwrap();
    for key in ["body", "saddle", "head"] {
        assert!(equipment.get(key).is_some(), "{key} is in equipment");
    }
    assert!(tag.get("Inventory").is_some());
    let mut loaded = guest();
    loaded.uuid = 7;
    assert!(load(&mut loaded, &dir));
    assert_eq!(loaded.equipment, g.equipment);
    assert_eq!(loaded.armor, g.armor);
    // A vanilla file with only a saddle keeps it and has no body.
    let mut level = Level::default();
    level.read_player(&Tag::compound([(
        "equipment",
        Tag::compound([(
            "saddle",
            crate::save::item_to_nbt(&ItemStack::new("minecraft:saddle", 1)),
        )]),
    )]));
    let mut vanilla = guest();
    apply(&mut vanilla, &level);
    assert_eq!(vanilla.equipment[horizon_items::equipment::BODY], None);
    assert_eq!(
        vanilla.equipment[horizon_items::equipment::SADDLE]
            .as_ref()
            .map(|s| &*s.item),
        Some("minecraft:saddle")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

fn holding(uuid: u128, item: &str, count: u32) -> Guest {
    let mut g = guest();
    g.uuid = uuid;
    g.position = [10.5, 70.0, -3.5];
    g.inventory[0] = Some(ItemStack {
        item: Arc::from(item),
        count,
        damage: 0,
        extra: None,
    });
    g
}

fn world(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("horizon-owner-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn autosaves_keep_snapshots_and_order_before_logout() {
    let dir = world("async-batch");
    let mut saves = Autosaves::default();
    for uuid in 1..=150 {
        let mut g = holding(uuid, "minecraft:diamond", 5);
        saves.enqueue(&g, &dir);
        // Subsequent live changes cannot alter the queued snapshot.
        g.inventory[0] = None;
    }
    let completed = saves.completed(true);
    assert!(saves.writes_finished().unwrap());
    assert!(completed.iter().map(|p| p.uuid).eq(1..=150));
    for uuid in 1..=150 {
        let mut loaded = guest();
        loaded.uuid = uuid;
        assert!(load(&mut loaded, &dir));
        assert_eq!(first_item(&loaded), Some(("minecraft:diamond", 5)));
    }
    saves.enqueue(&holding(1, "minecraft:gold_ingot", 7), &dir);
    saves.enqueue(&holding(1, "minecraft:emerald", 9), &dir);
    assert_eq!(saves.completed(true).len(), 2);
    assert!(saves.writes_finished().unwrap());
    let mut loaded = guest();
    loaded.uuid = 1;
    assert!(load(&mut loaded, &dir));
    assert_eq!(first_item(&loaded), Some(("minecraft:emerald", 9)));
    // The barrier protects the synchronous, newer logout snapshot.
    for uuid in 1..=150 {
        saves.enqueue(&holding(uuid, "minecraft:diamond", 5), &dir);
    }
    assert!(
        saves
            .save_after(&holding(1, "minecraft:iron_ingot", 11), &dir)
            .1
    );
    drop(saves);
    assert!(load(&mut loaded, &dir));
    assert_eq!(first_item(&loaded), Some(("minecraft:iron_ingot", 11)));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn autosave_failures_recover_and_drop_drains_writes() {
    let dir = world("async-error");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("players"), b"not a directory").unwrap();
    let mut saves = Autosaves::default();
    saves.enqueue(&holding(1, "minecraft:diamond", 5), &dir);
    assert!(saves.completed(true).is_empty());
    assert!(saves.writes_finished().is_err());
    assert!(saves.completed(false).is_empty());
    assert!(saves.writes_finished().unwrap());
    drop(saves);
    std::fs::remove_file(dir.join("players")).unwrap();
    let mut saves = Autosaves::default();
    saves.enqueue(&holding(1, "minecraft:diamond", 5), &dir);
    for uuid in 2..=150 {
        saves.enqueue(&holding(uuid, "minecraft:diamond", 5), &dir);
    }
    drop(saves);
    for uuid in 1..=150 {
        let mut g = guest();
        g.uuid = uuid;
        assert!(load(&mut g, &dir));
        assert_eq!(first_item(&g), Some(("minecraft:diamond", 5)));
    }
    let mut loaded = guest();
    loaded.uuid = 1;
    assert!(load(&mut loaded, &dir));
    assert_eq!(first_item(&loaded), Some(("minecraft:diamond", 5)));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn mixed_autosaves_keep_successes_and_match_logout_files() {
    let bad = world("mixed-bad");
    let good = world("mixed-good");
    let reference = world("mixed-reference");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("players"), b"not a directory").unwrap();
    let mut saves = Autosaves::default();
    let g = holding(2, "minecraft:emerald", 9);
    saves.enqueue(&holding(1, "minecraft:diamond", 5), &bad);
    saves.enqueue(&g, &good);
    saves.enqueue(&holding(3, "minecraft:diamond", 5), &bad);
    let saved = saves.completed(true);
    assert_eq!(saved.iter().map(|p| p.uuid).collect::<Vec<_>>(), vec![2]);
    assert!(saves.writes_finished().is_err());
    assert!(saves.writes_finished().unwrap());
    save(&g, &reference);
    assert_eq!(
        read_dat(&path(&good, 2)).unwrap(),
        read_dat(&path(&reference, 2)).unwrap()
    );
    std::fs::remove_file(bad.join("players")).unwrap();
    saves.enqueue(&g, &bad);
    assert_eq!(saves.completed(true).len(), 1);
    assert!(saves.writes_finished().unwrap());
    for dir in [bad, good, reference] {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn a_disconnected_writer_is_restarted() {
    let dir = world("restart");
    let (send, receive) = std::sync::mpsc::channel();
    drop(receive);
    let (complete, done) = std::sync::mpsc::channel();
    drop(complete);
    let mut saves = Autosaves::default();
    saves.writer = Some(PlayerWriter {
        send,
        done,
        thread: std::thread::spawn(|| {}),
    });
    saves.pending = 1;
    saves.enqueue(&holding(1, "minecraft:diamond", 5), &dir);
    assert_eq!(saves.completed(true).len(), 1);
    assert!(saves.writes_finished().is_err());
    saves.enqueue(&holding(2, "minecraft:emerald", 9), &dir);
    assert!(saves.writer.is_some());
    assert_eq!(saves.completed(true).len(), 1);
    assert!(saves.writes_finished().unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}

fn first_item(g: &Guest) -> Option<(&str, u32)> {
    g.inventory[0].as_ref().map(|s| (&*s.item, s.count))
}

#[test]
fn the_owner_loads_the_file_level_dat_names_over_an_older_own_file() {
    // #5351: a newer build saved the owner to both files, then 0.2.93 saved
    // it again to the file level.dat names alone. The owner's own file is
    // older; vanilla's loadPlayerData reads the named one for the owner.
    let owner = horizon_protocol::crypto::offline_uuid("Player46606");
    let dir = world("named");
    save(&holding(owner, "minecraft:diamond", 5), &dir);
    let mut level = Level::default();
    store(&holding(owner, "minecraft:gold_ingot", 7), &mut level, None);
    level.write(&dir).unwrap();
    let named = ints_uuid(level.player_uuid.unwrap());
    assert_ne!(
        named, owner,
        "level.dat names offline `Player`, not the owner"
    );
    let saved = Level::read(&dir).unwrap();
    let mut g = guest();
    g.uuid = owner;
    assert!(load_owner(&mut g, &dir, Some(&saved)));
    assert_eq!(first_item(&g), Some(("minecraft:gold_ingot", 7)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_owner_loads_its_own_file_when_the_named_one_is_missing() {
    let owner = horizon_protocol::crypto::offline_uuid("Player46606");
    let dir = world("own");
    let mut level = Level::default();
    level.write_without_player(&dir).unwrap();
    save(&holding(owner, "minecraft:diamond", 5), &dir);
    let saved = Level::read(&dir).unwrap();
    let mut g = guest();
    g.uuid = owner;
    assert!(load_owner(&mut g, &dir, Some(&saved)));
    assert_eq!(first_item(&g), Some(("minecraft:diamond", 5)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn player_attachments_survive_playerdata_and_respawn() {
    use horizon_mod_runtime::attachments::{AttachmentCodec, AttachmentError, AttachmentType};
    let keep = AttachmentType {
        namespace: "test",
        name: "keep",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n: &i32| Ok([("count".into(), Tag::Int(*n))].into()),
            decode: |c| match c.get(&"count".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: true,
        sync: false,
    };
    let dir = world("attachments");
    let mut original = guest();
    keep.set(
        &mut original.attachments,
        horizon_mod_runtime::attachments::Holder::Level,
        31,
    )
    .unwrap();
    save(&original, &dir);
    let mut loaded = guest();
    loaded.uuid = original.uuid;
    let index = std::sync::Arc::new(std::sync::Mutex::new(
        horizon_mod_runtime::attachments::index::AttachmentIndex::default(),
    ));
    let binding = loaded.attachments.bind_index(&index, loaded.uuid);
    assert!(load(&mut loaded, &dir));
    let copy = loaded.attachments.copy_on_death().unwrap();
    loaded.attachments.replace_contents(copy);
    assert_eq!(
        keep.get(
            &mut loaded.attachments,
            horizon_mod_runtime::attachments::Holder::Level
        ),
        Ok(31)
    );
    let rows = index
        .lock()
        .unwrap()
        .snapshot(&("test".into(), "keep".into()));
    assert_eq!(rows[0].entry.read(&keep), Ok(&31));
    assert!(binding.unregister(), "load and respawn preserve authority");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_save_cut_off_between_its_renames_loads_the_dat_old() {
    // PlayerDataStorage.load: a file `write_dat` moved to `.dat_old` and
    // did not replace (the game stopped between the two renames) is read
    // instead, for the owner's named file and for anyone's own file.
    let owner = horizon_protocol::crypto::offline_uuid("Player46606");
    let dir = world("dat-old");
    let mut level = Level::default();
    for (item, count) in [("minecraft:diamond", 5), ("minecraft:gold_ingot", 7)] {
        store(&holding(owner, item, count), &mut level, None);
        level.write(&dir).unwrap();
        save(&holding(owner, item, count), &dir);
    }
    let named = path(&dir, ints_uuid(level.player_uuid.unwrap()));
    for file in [&named, &path(&dir, owner)] {
        std::fs::remove_file(file).unwrap();
        assert!(file.with_extension("dat_old").is_file());
    }
    let saved = Level::read(&dir).unwrap();
    assert!(saved.singleplayer_file, "the named file's .dat_old counts");
    let mut g = guest();
    g.uuid = owner;
    assert!(load_owner(&mut g, &dir, Some(&saved)));
    assert_eq!(first_item(&g), Some(("minecraft:diamond", 5)));
    let mut g = guest();
    g.uuid = owner;
    assert!(load(&mut g, &dir), "a guest's own file falls back too");
    assert_eq!(first_item(&g), Some(("minecraft:diamond", 5)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_owner_leaving_saves_the_file_it_loads() {
    // The last autosave wrote both files with a diamond; the owner leaves
    // with gold and the game stops before the world's own save. The file
    // the owner loads holds the gold, as its own file does.
    let owner = horizon_protocol::crypto::offline_uuid("Player46606");
    let dir = world("leave");
    let mut level = Level::default();
    store(&holding(owner, "minecraft:diamond", 5), &mut level, None);
    level.write(&dir).unwrap();
    save(&holding(owner, "minecraft:diamond", 5), &dir);
    save_owner(&holding(owner, "minecraft:gold_ingot", 7), &mut level, &dir);
    let saved = Level::read(&dir).unwrap();
    let mut g = guest();
    g.uuid = owner;
    assert!(load_owner(&mut g, &dir, Some(&saved)));
    assert_eq!(first_item(&g), Some(("minecraft:gold_ingot", 7)));
    let mut g = guest();
    g.uuid = owner;
    assert!(load(&mut g, &dir));
    assert_eq!(first_item(&g), Some(("minecraft:gold_ingot", 7)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn opening_under_the_account_moves_an_offline_owners_files() {
    // A world older builds made names its player by the offline UUID: level.dat's
    // `singleplayer_uuid` and `players/data/<offline>.dat`. The first open under
    // the account moves the file, its `UUID` and level.dat's name.
    let old = horizon_protocol::crypto::offline_uuid("Player46606");
    let new = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
    let dir = world("migrate");
    let mut level = Level {
        player_uuid: Some(uuid_ints(old)),
        ..Level::default()
    };
    store(&holding(old, "minecraft:diamond", 5), &mut level, None);
    level.write(&dir).unwrap();

    let mut read = Level::read(&dir).unwrap();
    assert!(migrate_owner(&dir, &mut read, old, new));
    let moved = read_dat(&path(&dir, new)).expect("the account's player file");
    assert_eq!(
        uuid_from_tag(moved.get("UUID")),
        Some(uuid_ints(new)),
        "the file is named after its UUID"
    );
    assert!(!path(&dir, old).exists(), "the offline file is gone");
    assert!(
        path(&dir, old).with_extension("dat_offline").exists(),
        "the offline file is kept as a backup"
    );
    let named = read_dat(&dir.join("level.dat")).unwrap();
    assert_eq!(
        uuid_from_tag(named.get("Data").and_then(|d| d.get("singleplayer_uuid"))),
        Some(uuid_ints(new))
    );
    // The owner finds the data as its own; the world opening's copy has it too.
    let saved = Level::read(&dir).unwrap();
    assert!(saved.singleplayer_file);
    let mut g = guest();
    g.uuid = new;
    assert!(load_owner(&mut g, &dir, Some(&saved)));
    assert_eq!(first_item(&g), Some(("minecraft:diamond", 5)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_migration_keeps_the_newer_of_the_two_player_files() {
    // The account already played this world (its own file is newer than the
    // offline one level.dat still names): the newer file is the player's data.
    let old = horizon_protocol::crypto::offline_uuid("Player46606");
    let new = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
    let dir = world("migrate-newer");
    let mut level = Level {
        player_uuid: Some(uuid_ints(old)),
        ..Level::default()
    };
    store(&holding(old, "minecraft:diamond", 5), &mut level, None);
    level.write(&dir).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    save(&holding(new, "minecraft:emerald", 9), &dir);

    let mut read = Level::read(&dir).unwrap();
    assert!(migrate_owner(&dir, &mut read, old, new));
    let moved = read_dat(&path(&dir, new)).unwrap();
    assert_eq!(uuid_from_tag(moved.get("UUID")), Some(uuid_ints(new)));
    assert!(!path(&dir, old).exists());
    let mut g = guest();
    g.uuid = new;
    assert!(load_owner(&mut g, &dir, Some(&read)));
    assert_eq!(first_item(&g), Some(("minecraft:emerald", 9)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_world_without_an_owner_still_moves_the_offline_file() {
    // A server world or a Realms download has no `singleplayer_uuid`; its owner's
    // player file is still named by the offline UUID and is the owner's.
    let old = horizon_protocol::crypto::offline_uuid("Player46606");
    let new = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
    let dir = world("migrate-ownerless");
    let mut level = Level {
        player_uuid: Some(uuid_ints(old)),
        ..Level::default()
    };
    store(&holding(old, "minecraft:diamond", 5), &mut level, None);
    level.write(&dir).unwrap();
    let mut root = read_dat(&dir.join("level.dat")).unwrap();
    if let Tag::Compound(root) = &mut root
        && let Some(Tag::Compound(data)) = root.get_mut(&"Data".into())
    {
        data.remove(&"singleplayer_uuid".into());
    }
    write_dat(&dir.join("level.dat"), root).unwrap();

    let mut read = Level::read(&dir).unwrap();
    assert_eq!(read.player_uuid, None);
    assert!(migrate_owner(&dir, &mut read, old, new));
    assert!(path(&dir, new).is_file());
    assert!(!path(&dir, old).exists());
    assert_eq!(read.player_uuid, Some(uuid_ints(new)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_migration_leaves_another_accounts_world_alone() {
    let other = horizon_protocol::crypto::offline_uuid("Player46606");
    let old = horizon_protocol::crypto::offline_uuid("Alex");
    let new = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
    let dir = world("migrate-foreign");
    let mut level = Level {
        player_uuid: Some(uuid_ints(other)),
        ..Level::default()
    };
    store(&holding(other, "minecraft:diamond", 5), &mut level, None);
    level.write(&dir).unwrap();
    let before = read_dat(&dir.join("level.dat")).unwrap();

    let mut read = Level::read(&dir).unwrap();
    assert!(!migrate_owner(&dir, &mut read, old, new));
    assert_eq!(read_dat(&dir.join("level.dat")).unwrap(), before);
    assert!(!path(&dir, new).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_saved_burn_and_fall_come_back() {
    // A player saved burning in mid-fall (#1323, #1327): Entity.saveWithoutId's
    // Fire, Air and fall_distance, read back for the next body.
    let mut g = guest();
    g.loaded_body = Some(SavedBody {
        fire_ticks: 96,
        air: 250,
        fall_distance: 9.5,
        velocity: [0.0; 3],
        food_exhaustion: 0.0,
        food_tick_timer: 0,
        ticks_frozen: 0,
        invulnerable: false,
        invulnerable_time: 0,
    });
    let mut level = Level::default();
    store(&g, &mut level, None);
    let tag = level.player_tag();
    let mut read = Level::default();
    read.read_player(&tag);
    read.player_extra = Some(tag);
    let mut back = guest();
    apply(&mut back, &read);
    assert_eq!(back.loaded_body, g.loaded_body);
}

/// arismoko/horizon#916, coderabbit: a guest's `Invulnerable` flag and its window ride
/// the file as the host's own do — saved with the body, read back for
/// the next one; a window closed since goes.
#[test]
fn a_saved_flag_and_window_come_back() {
    let mut g = guest();
    g.loaded_body = Some(SavedBody {
        fire_ticks: -20,
        air: 300,
        fall_distance: 0.0,
        velocity: [0.0; 3],
        food_exhaustion: 0.0,
        food_tick_timer: 0,
        ticks_frozen: 0,
        invulnerable: true,
        invulnerable_time: 41,
    });
    let (tag, back) = round_trip(&g);
    assert_eq!(tag.get("Invulnerable"), Some(&Tag::Byte(1)));
    assert_eq!(tag.get("invulnerable_time"), Some(&Tag::Int(41)));
    assert_eq!(back.loaded_body, g.loaded_body);
    // Closed since the file was written: the window's key goes, the
    // flag stays.
    let mut g = back;
    g.loaded_body.as_mut().unwrap().invulnerable_time = 0;
    let (tag, back) = round_trip(&g);
    assert_eq!(tag.get("invulnerable_time"), None);
    assert_eq!(tag.get("Invulnerable"), Some(&Tag::Byte(1)));
    assert_eq!(back.loaded_body, g.loaded_body);
}

/// Store, write, read and apply a guest's file, as a server does between a
/// leave and the next join.
fn round_trip(g: &Guest) -> (Tag, Guest) {
    let mut level = Level::default();
    store(g, &mut level, None);
    let tag = level.player_tag();
    let mut read = Level::default();
    read.read_player(&tag);
    read.player_extra = Some(tag.clone());
    let mut back = guest();
    apply(&mut back, &read);
    (tag, back)
}

/// #11193: a player who saved while gliding comes back gliding, as
/// LivingEntity.addAdditionalSaveData's FallFlying and the flag
/// readAdditionalSaveData sets from it have it.
#[test]
fn a_saved_glide_comes_back() {
    let mut g = guest();
    g.fall_flying = true;
    let (tag, back) = round_trip(&g);
    assert_eq!(tag.get("FallFlying"), Some(&Tag::Byte(1)));
    assert!(back.fall_flying);
    let (tag, back) = round_trip(&guest());
    assert_eq!(tag.get("FallFlying"), Some(&Tag::Byte(0)));
    assert!(!back.fall_flying);
}

/// `getBooleanOr` takes any numeric tag: an int `FallFlying` from
/// another writer still starts the glide.
#[test]
fn a_numeric_fall_flying_reads_as_a_glide() {
    let mut level = Level::default();
    store(&guest(), &mut level, None);
    let mut tag = level.player_tag();
    put(&mut tag, "FallFlying", Tag::Int(1));
    let mut read = Level::default();
    read.read_player(&tag);
    assert!(read.fall_flying);
}

#[test]
fn a_saved_motion_comes_back() {
    // Entity.saveWithoutId writes Motion, the delta movement, as it is;
    // Entity.load reads each component back, as 0 when it is over 10.
    let mut g = guest();
    g.loaded_body = Some(SavedBody {
        fire_ticks: -20,
        air: 300,
        fall_distance: 0.0,
        velocity: [0.25, -0.0784, 12.5],
        food_exhaustion: 0.0,
        food_tick_timer: 0,
        ticks_frozen: 0,
        invulnerable: false,
        invulnerable_time: 0,
    });
    let (tag, back) = round_trip(&g);
    let mut read = Level::default();
    read.read_player(&tag);
    assert_eq!(read.motion, [0.25, -0.0784, 12.5]);
    assert_eq!(
        back.loaded_body.expect("a body").velocity,
        [0.25, -0.0784, 0.0]
    );
}

/// `Entity.load` reads a player's `Fire` with `getShortOr` too: an int
/// 40000 in the player file is -25536, not half an hour of burning.
#[test]
fn a_players_saved_fire_is_narrowed_to_a_short() {
    for (saved, expected) in [(Tag::Int(40000), -25536), (Tag::Short(60), 60)] {
        let level = Level {
            player_extra: Some(Tag::compound([("Fire", saved)])),
            ..Level::default()
        };
        let mut g = guest();
        apply(&mut g, &level);
        assert_eq!(g.loaded_body.expect("a body").fire_ticks, expected);
    }
}

#[test]
fn a_saved_exhaustion_and_food_timer_come_back() {
    // FoodData.addAdditionalSaveData writes foodExhaustionLevel (a float)
    // and foodTickTimer (an int) with the food level and saturation.
    let mut g = guest();
    g.loaded_body = Some(SavedBody {
        fire_ticks: -20,
        air: 300,
        fall_distance: 0.0,
        velocity: [0.0; 3],
        food_exhaustion: 2.75,
        food_tick_timer: 37,
        ticks_frozen: 0,
        invulnerable: false,
        invulnerable_time: 0,
    });
    let (tag, back) = round_trip(&g);
    assert_eq!(
        tag.get("foodExhaustionLevel"),
        Some(&Tag::Float(2.75f32.to_bits()))
    );
    assert_eq!(tag.get("foodTickTimer"), Some(&Tag::Int(37)));
    let body = back.loaded_body.expect("a body");
    assert_eq!((body.food_exhaustion, body.food_tick_timer), (2.75, 37));
}

#[test]
fn the_previous_game_mode_is_kept_and_saved() {
    // ServerPlayerGameMode.changeGameModeForPlayer: the mode left becomes
    // the previous one, and the mode in use changes nothing.
    // ServerPlayer.addAdditionalSaveData writes previousPlayerGameType (an
    // int) only when there is one.
    let mut g = guest();
    assert_eq!(g.previous_game_mode, None);
    let (tag, _) = round_trip(&g);
    assert_eq!(tag.get("previousPlayerGameType"), None);
    g.change_game_mode(1);
    assert_eq!((g.game_mode, g.previous_game_mode), (1, Some(0)));
    g.change_game_mode(1);
    assert_eq!((g.game_mode, g.previous_game_mode), (1, Some(0)));
    g.change_game_mode(3);
    assert_eq!((g.game_mode, g.previous_game_mode), (3, Some(1)));
    let (tag, back) = round_trip(&g);
    assert_eq!(tag.get("previousPlayerGameType"), Some(&Tag::Int(1)));
    assert_eq!((back.game_mode, back.previous_game_mode), (3, Some(1)));
}

#[test]
fn a_saved_freeze_comes_back_and_a_thaw_clears_it() {
    // Entity.saveWithoutId writes TicksFrozen (an int) only when it is
    // above 0; Entity.load reads it with 0 for a missing key.
    let frozen = |ticks| SavedBody {
        fire_ticks: -20,
        air: 300,
        fall_distance: 0.0,
        velocity: [0.0; 3],
        food_exhaustion: 0.0,
        food_tick_timer: 0,
        ticks_frozen: ticks,
        invulnerable: false,
        invulnerable_time: 0,
    };
    let mut g = guest();
    g.loaded_body = Some(frozen(120));
    let (tag, back) = round_trip(&g);
    assert_eq!(tag.get("TicksFrozen"), Some(&Tag::Int(120)));
    assert_eq!(back.loaded_body.expect("a body").ticks_frozen, 120);
    // Thawed since: the file the guest came from still says 120, and the
    // key goes.
    let mut g = back;
    g.loaded_body = Some(frozen(0));
    let (tag, back) = round_trip(&g);
    assert_eq!(tag.get("TicksFrozen"), None);
    assert_eq!(back.loaded_body.expect("a body").ticks_frozen, 0);
}

#[test]
fn post_effects_are_saved_with_the_player() {
    // ServerPlayer.addAdditionalSaveData's `post_effects`, in order; none
    // is an empty list, and a cleared list does not come back from an
    // older file's copy.
    let mut g = guest();
    g.post_effects = vec!["minecraft:invert".into(), "pack:glow".into()];
    let mut level = Level::default();
    store(&g, &mut level, None);
    let tag = level.player_tag();
    let mut read = Level::default();
    read.read_player(&tag);
    read.player_extra = Some(tag);
    let mut back = guest();
    apply(&mut back, &read);
    assert_eq!(back.post_effects, g.post_effects);
    // The cleared list is written over the loaded file.
    g.post_effects.clear();
    store(&g, &mut read, None);
    let tag = read.player_tag();
    assert!(matches!(
        tag.get("post_effects"),
        Some(horizon_core::nbt::Tag::List(l)) if l.is_empty()
    ));
}

/// The resources the prepared game data holds, for a body to breathe with.
fn resources() -> Option<(crate::assets::Assets, crate::motion::MotionResources)> {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client")
                .join(horizon_core::VANILLA_VERSION)
        });
    if !root.join("movement-properties.nbt").exists() {
        return None;
    }
    let assets = crate::assets::Assets::load(&root).ok()?;
    let resources = crate::motion::MotionResources::load(&root, &assets).ok()?;
    Some((assets, resources))
}

/// A body at scale 0.1 with a permanent +0.4 on its step height, such as
/// a command or a datapack gives.
fn small_body(resources: &crate::motion::MotionResources) -> crate::motion::PlayerMotion {
    use horizon_world::attributes::{Modifier, Operation};
    let mut m = crate::motion::PlayerMotion::new(resources, horizon_core::geometry::Vec3::ZERO)
        .expect("a body");
    let key = m.attributes.key("minecraft:scale").unwrap();
    let handle = m.attributes.get_instance(Some(key)).unwrap().unwrap();
    m.attributes
        .edit(&handle, |a, o| a.set_base_value(0.1, o))
        .unwrap();
    let key = m.attributes.key("minecraft:step_height").unwrap();
    let handle = m.attributes.get_instance(Some(key)).unwrap().unwrap();
    let step = Modifier::new("test:step", 0.4, Operation::AddValue).unwrap();
    m.attributes
        .edit(&handle, |a, o| a.add_permanent(step, o))
        .unwrap();
    m
}

#[test]
fn a_new_body_takes_the_scale_of_the_old_one() {
    // #3684: ServerPlayer.restoreFrom copies the base values to the body
    // that follows a death (assignBaseValues) and, for the same player in
    // another level, the permanent modifiers too (assignPermanentModifiers).
    // The body Horizon made on a respawn or a level change started from
    // the defaults: a player at /attribute scale 0.1 was 1.0 again.
    let Some((_assets, resources)) = resources() else {
        eprintln!("skipped: prepared resources unavailable");
        return;
    };
    let next = |g: &Guest| {
        let mut m =
            crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
                .expect("a body");
        m.apply_saved_attributes(g.carried_attributes.as_ref())
            .unwrap();
        m
    };
    for (restore_all, step) in [(false, 0.6), (true, 1.0)] {
        let mut g = guest();
        g.motion = Some(Box::new(small_body(&resources)));
        g.new_body(restore_all);
        assert!(g.motion.is_none());
        let mut m = next(&g);
        assert_eq!(m.attribute("minecraft:scale").unwrap(), 0.1);
        assert!((m.attribute("minecraft:step_height").unwrap() - step).abs() < 1e-9);
        // The tick that follows rebuilds the box at that scale.
        m.refresh_attributes().unwrap();
        assert_eq!(m.dimensions.width, 0.6 * 0.1);
        assert_eq!(m.dimensions.height, 1.8 * 0.1);
    }
    // No body before: nothing to hand over.
    let mut g = guest();
    g.new_body(true);
    assert!(g.carried_attributes.is_none());
}

#[test]
fn a_player_between_two_bodies_saves_the_attributes_the_next_one_takes() {
    // #3684: logging out after a respawn, before the new body has ticked,
    // must not write the file's older list over the scale.
    let Some((_assets, resources)) = resources() else {
        eprintln!("skipped: prepared resources unavailable");
        return;
    };
    let mut g = guest();
    g.motion = Some(Box::new(small_body(&resources)));
    g.new_body(false);
    let mut level = Level::default();
    store(&g, &mut level, None);
    let tag = level.player_tag();
    let mut read = Level::default();
    read.read_player(&tag);
    read.player_extra = Some(tag);
    let mut back = crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
        .expect("a body");
    back.apply_saved_attributes(read.player_extra.as_ref().and_then(|t| t.get("attributes")))
        .unwrap();
    assert_eq!(back.attribute("minecraft:scale").unwrap(), 0.1);
}

#[test]
fn a_saved_air_comes_back() {
    // A player saved underwater (#1321): the file keeps the remaining air
    // (Entity.saveWithoutId's Air), and the body that comes back breathes
    // on it instead of starting with a full supply.
    let Some((_assets, resources)) = resources() else {
        eprintln!("skipped the air round trip: prepared resources unavailable");
        return;
    };
    let mut g = guest();
    let mut motion =
        crate::motion::PlayerMotion::new(&resources, horizon_core::geometry::Vec3::ZERO)
            .expect("a body");
    motion.air = 99;
    g.motion = Some(Box::new(motion));
    let mut level = Level::default();
    store(&g, &mut level, None);
    let tag = level.player_tag();
    let mut read = Level::default();
    read.read_player(&tag);
    read.player_extra = Some(tag);
    let mut back = guest();
    apply(&mut back, &read);
    assert_eq!(back.loaded_body.expect("a body").air, 99);
}

#[test]
fn a_saved_root_vehicle_comes_back() {
    // A player saved riding a horse (#1474): the file holds the vehicle
    // (ServerPlayer.saveParentVehicle's RootVehicle {Attach, Entity}),
    // and the next login spawns it and rides it again.
    let vehicle = [1, 2, 3, 4];
    let horse = Tag::compound([
        ("id", Tag::String("minecraft:horse".into())),
        ("UUID", Tag::IntArray(vehicle.to_vec())),
    ]);
    let root = Tag::compound([
        ("Attach", Tag::IntArray(vehicle.to_vec())),
        ("Entity", horse),
    ]);
    let g = guest();
    let mut level = Level::default();
    store(&g, &mut level, Some(root.clone()));
    let tag = level.player_tag();
    assert_eq!(tag.get("RootVehicle"), Some(&root));
    let mut read = Level::default();
    read.read_player(&tag);
    read.player_extra = Some(tag);
    let mut back = guest();
    apply(&mut back, &read);
    assert_eq!(back.saved_vehicle, Some(vehicle));
    // Saved again before the horse is back, the file keeps it whole.
    let mut again = Level::default();
    store(&back, &mut again, None);
    assert_eq!(again.player_tag().get("RootVehicle"), Some(&root));
}

fn abilities(g: &Guest) -> (i8, f32) {
    match g.abilities_packet() {
        horizon_protocol::packets::play::Clientbound::PlayerAbilities(p) => {
            (p.flags, p.flying_speed)
        }
        other => panic!("not the abilities: {other:?}"),
    }
}

/// The stored abilities go out in every abilities packet (a game mode
/// change's, a respawn's), are saved as vanilla's `abilities` and come back
/// when the player does; the mode's flags are set again as they are placed,
/// the speeds and creative flight are kept; a file without them is vanilla's.
#[test]
fn the_stored_abilities_are_sent_kept_and_saved() {
    let dir = world("fly-speed");
    let mut g = guest();
    assert_eq!(abilities(&g).1, 0.05);
    g.abilities.flying_speed = 2.5;
    g.abilities.walking_speed = 0.3;
    g.game_mode = 1;
    g.abilities.update_for_mode(1);
    g.abilities.flying = true;
    assert_eq!(abilities(&g), (1 | 2 | 4 | 8, 2.5));
    g.game_mode = 0;
    g.abilities.update_for_mode(0);
    assert_eq!(abilities(&g), (0, 2.5));
    g.abilities.may_build = false;
    g.abilities.instabuild = true;
    save(&g, &dir);
    let tag = read_dat(&path(&dir, g.uuid)).unwrap();
    let saved = |key: &str| tag.get("abilities").and_then(|a| a.get(key)).cloned();
    assert_eq!(saved("flySpeed"), Some(Tag::Float(2.5f32.to_bits())));
    assert_eq!(saved("walkSpeed"), Some(Tag::Float(0.3f32.to_bits())));
    assert_eq!(saved("mayBuild"), Some(Tag::Byte(0)));
    assert_eq!(saved("instabuild"), Some(Tag::Byte(1)));
    let mut back = guest();
    assert!(load(&mut back, &dir));
    assert_eq!(
        (back.abilities.flying_speed, back.abilities.walking_speed),
        (2.5, 0.3)
    );
    // Placed in survival: the mode's flags win over the file's.
    assert!(back.abilities.may_build && !back.abilities.instabuild);
    // A vanilla file's own speed is read, and one without is 0.05.
    let mut level = Level::default();
    level.read_player(&Tag::compound([(
        "abilities",
        Tag::compound([("flySpeed", Tag::Float(0.1f32.to_bits()))]),
    )]));
    let mut other = guest();
    apply(&mut other, &level);
    assert_eq!(other.abilities.flying_speed, 0.1);
    level.read_player(&Tag::compound([(
        "abilities",
        Tag::Compound(Default::default()),
    )]));
    apply(&mut other, &level);
    assert_eq!(other.abilities.flying_speed, 0.05);
    assert_eq!(other.abilities.walking_speed, 0.1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_world_without_singleplayer_uuid_gives_the_owner_a_new_player_not_anothers() {
    // A world from a vanilla server: no `singleplayer_uuid`, and the players' files are other
    // people's. The owner has none: vanilla starts them new (PlayerList.loadPlayerData).
    let owner = horizon_protocol::crypto::offline_uuid("Owner");
    let friend = horizon_protocol::crypto::offline_uuid("Friend");
    let dir = world("server-world");
    save(&holding(friend, "minecraft:netherite_ingot", 9), &dir);
    let mut level = Level::default();
    level.write(&dir).unwrap();
    let _ = std::fs::remove_file(dir.join("level.dat_old"));
    let mut tag = read_dat(&dir.join("level.dat")).unwrap();
    if let Some(Tag::Compound(data)) = tag.get_mut("Data") {
        data.remove(&"singleplayer_uuid".into());
    }
    write_dat(&dir.join("level.dat"), tag).unwrap();
    let read = Level::read(&dir).unwrap();
    let mut g = holding(owner, "minecraft:dirt", 1);
    g.inventory[0] = None;
    let known = load_owner(&mut g, &dir, Some(&read));
    assert!(!known, "the owner has not been here before");
    assert_eq!(first_item(&g), None, "no inventory of the friend's");
    // The owner's save names the owner, and leaves the friend's file alone.
    let before = std::fs::read(path(&dir, friend)).unwrap();
    let mut level = read;
    save_owner(&g, &mut level, &dir);
    assert_eq!(level.player_uuid, Some(uuid_ints(owner)));
    assert_eq!(std::fs::read(path(&dir, friend)).unwrap(), before);
    assert!(exists(&dir, owner));
}

#[test]
fn a_downloaded_realms_world_finds_the_accounts_player_file() {
    // `FileDownload.untarGzipArchive` drops `singleplayer_uuid`; the Realm's player
    // file is named by the account's UUID (the Realm authenticated them). The owner
    // loads it instead of starting a new player.
    let account = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
    let dir = world("download");
    save(&holding(account, "minecraft:diamond", 5), &dir);
    let mut level = Level::default();
    level.write_without_player(&dir).unwrap();
    let mut tag = read_dat(&dir.join("level.dat")).unwrap();
    if let Some(Tag::Compound(data)) = tag.get_mut("Data") {
        data.remove(&"singleplayer_uuid".into());
    }
    write_dat(&dir.join("level.dat"), tag).unwrap();
    let read = Level::read(&dir).unwrap();
    assert_eq!(read.player_uuid, None);
    let mut g = guest();
    g.uuid = account;
    assert!(load_owner(&mut g, &dir, Some(&read)));
    assert_eq!(first_item(&g), Some(("minecraft:diamond", 5)));
    let _ = std::fs::remove_dir_all(&dir);
}
