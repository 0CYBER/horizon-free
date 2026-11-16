//! Disk regression for saves requested by native listeners during entity passes.
use super::*;
use horizon_client::{
    chunk::Chunk,
    motion::PlayerMotion,
    save::{LevelShape, SaveDir},
    server::user_lists::{BanList, IpBanList, OpList, WhiteList},
};
use mod_api::{Hooks, ItemPickedUpEvent, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

pub(super) fn with_save_game(root: &std::path::Path, test: impl FnOnce(&mut Game<'_>)) {
    native_guest_turn_tests::with_guests(|game| {
        game.world_root = root.to_path_buf();
        game.save = Arc::new(SaveDir::new(root.to_path_buf(), LevelShape::OVERWORLD));
        game.options.dedicated = Some(Vec::new());
        game.options.execution_role = execution_role::ExecutionRole::DedicatedServer;
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
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.0, 64.0, 2.0)).unwrap());
        game.level.player_uuid = Some([0, 0, 0, 99]);
        test(game);
    });
}

#[derive(Default)]
struct PanicPickup {
    visibility: horizon_client::join_seq::visibility::Visibility,
    calls: Arc<AtomicUsize>,
}
#[mod_api::horizon_mod]
impl Hooks for PanicPickup {
    fn item_picked_up(&self, _: &mut dyn ServerHost, _: &mut ItemPickedUpEvent<'_>) {
        assert!(
            !self.visibility.settled(),
            "actual callback must see the lifted owner"
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::panic::panic_any(77_u32);
    }
}
#[test]
#[ignore = "daily"]
fn visibility_listener_unwind_does_not_block_an_outside_save() {
    // Save admission is process-wide, even when this game's visibility is settled.
    // Other fixtures may legitimately hold leases; isolate the real save proof.
    crate::game::in_own_process(
        "game::visibility_unwind_tests::visibility_listener_unwind_does_not_block_an_outside_save",
        visibility_unwind_save,
    );
}

fn visibility_unwind_save() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
        .join(format!("visibility-unwind-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let visibility = horizon_client::join_seq::visibility::Visibility::default();
    let calls = Arc::new(AtomicUsize::new(0));
    with_save_game(&root, |game| {
        game.dropped.bind_visibility(&visibility);
        game.spawn_item(
            ItemStack::new("minecraft:diamond", 7),
            [2.0, 64.0, 2.0],
            [0.0; 3],
            0,
        );
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "visibility-unwind",
            Box::new(PanicPickup {
                visibility: visibility.clone(),
                calls: calls.clone(),
            }),
        )]
        .into();
        let panic = mod_platform::testing::with_mods(mods, || {
            mod_api::__private::mark_game_thread();
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| game.tick_items()))
                .unwrap_err()
        });
        assert_eq!(panic.downcast_ref::<u32>(), Some(&77));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            visibility.settled(),
            "unwind must release, not leave a stuck pass"
        );
        // The baseline item pass has no unwind restoration fence. This proves
        // lease drop/cleanup, NOT retained-row recovery or persisted entities.
        assert!(
            !root.join("level.dat").exists(),
            "no stale metadata can satisfy this proof"
        );
        assert!(game.native_run_command("save-all flush"));
        assert!(
            root.join("level.dat").exists(),
            "actual outside save must complete"
        );
    });
}
