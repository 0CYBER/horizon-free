//! Disk regression for saves requested by native listeners during entity passes.
use super::*;
use mod_api::{Hooks, ItemPickedUpEvent, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct SaveOnPickup {
    line: &'static str,
    calls: Arc<AtomicUsize>,
}
#[mod_api::horizon_mod]
impl Hooks for SaveOnPickup {
    fn item_picked_up(&self, host: &mut dyn ServerHost, event: &mut ItemPickedUpEvent<'_>) {
        assert!(
            host.commands()
                .run_command("summon minecraft:armor_stand 5 64 5")
        );
        assert!(!host.commands().run_command(self.line));
        self.calls.fetch_add(1, Ordering::SeqCst);
        *event.can_pickup = mod_api::TriState::False;
    }
}

#[test]
#[ignore = "daily"]
fn save_requested_during_pickup_keeps_lifted_items_on_disk() {
    use horizon_client::{
        chunk::Chunk,
        motion::PlayerMotion,
        save::{LevelShape, SaveDir},
        server::user_lists::{BanList, IpBanList, OpList, WhiteList},
    };
    let mut missing = Vec::new();
    for (case, line) in ["save-all flush", "execute run save-all flush"]
        .into_iter()
        .enumerate()
    {
        let root = PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join(format!("save-pass-{case}"));
        std::fs::create_dir_all(&root).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        native_guest_turn_tests::with_guests(|game| {
            game.world_root = root.clone();
            game.save = Arc::new(SaveDir::new(root.clone(), LevelShape::OVERWORLD));
            game.options.dedicated = Some(Vec::new());
            game.server_lists = Some(Arc::new(Mutex::new(dedicated::Lists {
                ops: OpList::new(root.join("ops.json")),
                white: WhiteList::new(root.join("whitelist.json")),
                bans: BanList::new(root.join("bans.json")),
                ip_bans: IpBanList::new(root.join("ip-bans.json")),
                use_whitelist: false,
                enforce_whitelist: false,
                op_level: 4,
                player_idle_timeout: 0,
                stop: false,
                autosave: true,
                whitelist_bypass: Vec::new(),
            })));
            game.build_registries();
            game.ui.dispatcher = chat::SessionDispatcher(Arc::new(
                horizon_client::command::vanilla::build_for(false, true),
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.modified.insert((0, 0));
            // Each intrinsic family must refuse both full and single-level snapshots.
            macro_rules! refuses {
                ($($field:ident).+) => {{
                    let pass = game.$($field).+.lift();
                    assert!(game.save_game().is_err(), stringify!($($field).+));
                    assert!(game.save_level(true).is_err());
                    game.$($field).+.restore(pass, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
                }};
            }
            refuses!(mobs);
            refuses!(dropped);
            refuses!(boats);
            refuses!(carts);
            refuses!(armor_stands);
            refuses!(tnts);
            refuses!(falling);
            refuses!(orbs);
            refuses!(creatures.projectiles);
            refuses!(rockets);
            refuses!(tridents);
            game.with_entity_held_out(999, |game| assert!(game.save_game().is_err()));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.0, 64.0, 2.0)).unwrap());
            game.level.player_uuid = Some([0, 0, 0, 99]);
            game.spawn_item(
                ItemStack::new("minecraft:diamond", 7),
                [2.0, 64.0, 2.0],
                [0.0; 3],
                0,
            );
            game.spawn_item(
                ItemStack::new("minecraft:iron_ingot", 11),
                [8.0, 64.0, 8.0],
                [0.0; 3],
                20,
            );
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "save-pass",
                Box::new(SaveOnPickup {
                    line,
                    calls: calls.clone(),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                mod_api::__private::mark_game_thread();
                game.tick_items();
            });
            assert_eq!(calls.load(Ordering::SeqCst), 1, "listener must really run");
            assert_eq!(
                game.dropped.len(),
                2,
                "refused pickup and untouched row survive"
            );
            assert_eq!(game.dropped[0].stack.count, 7);
            game.save.flush().unwrap();
            assert!(
                game.save.load_entities(0, 0).is_empty(),
                "refused save wrote rows"
            );
            game.save_game().unwrap();
        });
        let disk = SaveDir::new(root.clone(), LevelShape::OVERWORLD);
        let tags = disk.load_entities(0, 0);
        std::fs::write(root.join("entity-evidence.txt"), format!("{tags:#?}")).unwrap();
        let kinds: Vec<_> = tags
            .iter()
            .filter_map(|t| match t.get("id") {
                Some(horizon_core::nbt::Tag::String(s)) => Some(s.to_string_lossy()),
                _ => None,
            })
            .collect();
        assert!(
            kinds.iter().any(|s| s == "minecraft:armor_stand"),
            "real nonlifted control missing: {tags:?}"
        );
        if kinds.iter().filter(|s| *s == "minecraft:item").count() != 2 {
            missing.push(format!(
                "{line}: lifted items missing from actual disk snapshot: {tags:?}"
            ));
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}
