// Same logical test module, kept in its own file.
use super::Target;
use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};
use horizon_core::nbt::Tag;

#[test]
#[ignore = "daily"]
fn saved_orb_uuid_comes_from_registered_identity_without_or_despite_raw() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();
    let uuid = [0, 0, 0, 2];
    let tag = Tag::compound([
        ("UUID", Tag::IntArray(uuid.to_vec())),
        (
            "Pos",
            Tag::List(vec![
                Tag::Double(4.5_f64.to_bits()),
                Tag::Double(200.0_f64.to_bits()),
                Tag::Double(4.5_f64.to_bits()),
            ]),
        ),
        ("Value", Tag::Short(1)),
        ("Age", Tag::Short(0)),
        ("Health", Tag::Short(5)),
        ("NoGravity", Tag::Byte(1)),
    ]);
    game.load_orb(tag, Some(uuid));
    assert_eq!(game.orbs.len(), 1);
    let id = game.orbs[0].id;
    assert_eq!(game.persist.uuids.get(&id), Some(&uuid));
    let time = game.world.game_time;
    for raw in [
        None,
        Some(Tag::compound([("UUID", Tag::IntArray(vec![9, 9, 9, 9]))])),
    ] {
        if let Some(raw) = &raw {
            game.persist.raw.insert(id, raw.clone());
        } else {
            game.persist.raw.remove(&id);
        }
        for _ in 0..3 {
            let saved = game.entity_nbt_in_level(Target::Orb(id)).unwrap();
            assert_eq!(saved.get("UUID"), Some(&Tag::IntArray(uuid.to_vec())));
            assert_eq!(game.persist.uuids.get(&id), Some(&uuid));
            assert_eq!(game.persist.raw.get(&id), raw.as_ref());
            assert_eq!(game.world.game_time, time);
        }
    }
}
