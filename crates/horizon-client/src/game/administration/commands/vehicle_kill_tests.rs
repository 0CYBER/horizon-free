// Test module extracted without changing its logical path.
use super::*;
use crate::game::BlockEntity;
use crate::game::{Audio, Options, Progress, Settings, load_resources};
use horizon_core::nbt::parse_snbt;

#[test]
#[ignore = "daily"]
fn kill_spills_explicit_vehicle_contents_after_menu_writeback() {
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
    game.set_allow_commands(true);
    let clock = game.world.game_time;
    let mut differences = Vec::new();
    for (index, kind) in [
        "oak_chest_boat",
        "bamboo_chest_raft",
        "chest_minecart",
        "hopper_minecart",
        "oak_boat",
    ]
    .iter()
    .enumerate()
    {
        let uuid = [51, 0, 0, index as i32 + 1];
        let source = r#"{Pos:[5.25d,200d,4.75d],NoGravity:1b,Items:[{Slot:0b,id:"minecraft:diamond",count:3,components:{"minecraft:custom_data":{h051:1}}},{Slot:1b,id:"minecraft:dirt",count:5},{Slot:2b,id:"minecraft:stick",count:7}]}"#.to_string();
        let (tag, _) = parse_snbt(&source).unwrap();
        let boat = kind.contains("boat") || kind.contains("raft");
        let (id, mut slots) = if boat {
            game.load_boat(tag, Some(uuid), &format!("minecraft:{kind}"));
            let b = game.boats.last().unwrap();
            assert_eq!(b.position, Vec3::new(5.25, 200.0, 4.75));
            (b.id, b.items.clone())
        } else {
            game.load_cart(tag, Some(uuid), &format!("minecraft:{kind}"));
            let c = game.carts.last().unwrap();
            (c.id, c.items.clone())
        };
        let container = index < 4;
        assert_eq!(
            slots.iter().flatten().map(|s| s.count).sum::<u32>(),
            if container { 15 } else { 0 }
        );
        let target = if boat {
            Target::Boat(id)
        } else {
            Target::Cart(id)
        };
        assert_eq!(
            game.command_target_nbt(target).unwrap().get("UUID"),
            Some(&horizon_core::nbt::Tag::IntArray(uuid.to_vec()))
        );
        // The active menu's actual edited slot must win over the older model.
        let key = (i32::MAX - 15, crate::game::minecarts::CART_KEY_Y, id as i32);
        if container {
            slots[0].as_mut().unwrap().count = 4;
            game.world
                .block_entities
                .insert(key, BlockEntity::Chest(slots.clone()));
            game.container = Some(key);
        }
        let old_len = game.dropped.len();
        let actor = format!("00000033-0000-0000-0000-{:012x}", index + 1);
        assert!(
            game.run_command(&format!("kill {actor}")),
            "{kind}: actual command admitted"
        );
        assert!(!game.boats.iter().any(|b| b.id == id));
        assert!(!game.carts.iter().any(|c| c.id == id));
        assert!(!game.world.block_entities.contains_key(&key));
        assert_ne!(game.container, Some(key));
        let dropped = &game.dropped[old_len..];
        let expected: Vec<_> = slots.into_iter().flatten().collect();
        if dropped.len() != expected.len()
            || expected
                .iter()
                .any(|s| !dropped.iter().any(|d| &d.stack == s))
        {
            differences.push(format!(
                "{kind}: expected {expected:?}, actual {:?}",
                dropped.iter().map(|d| &d.stack).collect::<Vec<_>>()
            ));
        }
    }
    assert_eq!(game.world.game_time, clock);
    assert!(
        differences.is_empty(),
        "kill must spill authoritative explicit contents: {differences:#?}"
    );
}
