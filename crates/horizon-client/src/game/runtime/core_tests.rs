//! Core gameplay regression tests.
use mod_api::ServerHost;
#[test]
#[ignore = "daily"]
fn reload_resolves_parts_against_reloaded_tags() {
    use mod_api::{self, BlockPart, DataPackFilesEvent, Hooks, Registrar};
    struct TagPart;
    #[mod_api::horizon_part]
    impl BlockPart for TagPart {
        fn analog_output(&self, _: &dyn mod_api::LevelHost, _: &mut mod_api::AnalogOutputEvent) {}
    }
    struct ReloadMod;
    #[mod_api::__subscriptions]
    impl Hooks for ReloadMod {
        fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            registrar.block_part_tag("test:reload", TagPart);
        }
        fn data_pack_files(&self, _: &mut dyn ServerHost, event: &mut DataPackFilesEvent<'_>) {
            event.files.push((
                "tags/test:block/reload".into(),
                br#"{"values":["minecraft:stone"]}"#.to_vec(),
            ));
        }
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("reload", Box::new(ReloadMod))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::__private::install(mods);
    super::native_entity_refs::tests::with_game(|game| {
        let stone_key = mod_platform::__private::part_tests::block_key("stone");
        let name = stone_key.id(game);
        let dir = std::env::temp_dir().join(format!("horizon-key-tags-{}", std::process::id()));
        game.session_lock = Some(horizon_client::save::SessionLock::acquire(&dir).unwrap());
        assert_eq!(game.block_in_tag(stone_key, "test:reload"), Some(false));
        assert_eq!(
            game.block_in_tag(stone_key, "base_stone_overworld"),
            Some(true)
        );
        game.load_server_loot();
        let stack = super::functions::pack_stack(
            &game.world_root,
            &game.options.resources,
            &game.pack.data_source,
            &game.pack.data_changes,
            game.cmds.vanilla_disabled,
            &game.level.enabled_features,
            &game.pack.additional_data,
        );
        let root = game.options.resources.clone();
        let previous = std::sync::Arc::new(mod_api::DatapackRegistrySet::first(
            mod_api::DatapackCandidate::build(&[], &[]).unwrap(),
            horizon_client::datapack_net::next_session(),
        ));
        horizon_client::datapack_net::serve(Some(previous.clone()));
        let prepared = game.prepare_world_reload(&stack, &root).unwrap();
        // Preparation must not publish to joining clients.
        assert!(std::sync::Arc::ptr_eq(
            &previous,
            &horizon_client::datapack_net::served().unwrap(),
        ));
        game.install_world_tables(prepared);
        // A successful empty publication clears the previous world's served registries too.
        assert!(horizon_client::datapack_net::served().is_none());
        game.run_native_advancement_grants();
        assert_eq!(stone_key.id(game), name);
        assert_eq!(
            stone_key,
            mod_platform::__private::part_tests::block_key("minecraft:stone")
        );
        assert_eq!(game.block_in_tag(stone_key, "test:reload"), Some(true));
        assert_eq!(game.block_in_tag(stone_key, "minecraft:logs"), Some(false));
        assert_eq!(
            game.block_in_tag(stone_key, "base_stone_overworld"),
            Some(true)
        );
        let stone = game.block_state_default(stone_key).unwrap();
        assert!(api_values::block_state::block_listens(
            mod_api::HookId::AnalogOutput,
            stone
        ));
        game.session_lock = None;
        let _ = std::fs::remove_dir_all(dir);
    });
}
#[test]
#[ignore = "daily"]
fn client_option_snapshots_follow_settings() {
    super::native_entity_refs::tests::with_client_game(|game| {
        game.settings.fov = 90.0;
        game.settings.render_distance = 12;
        game.settings.gamma = 0.75;
        game.settings.gui_scale = 3;
        game.settings.show_subtitles = true;
        game.settings.chat_visibility = 1;
        game.settings.chat_delay = 1.5;
        game.settings.master_volume = 0.25;
        let host: &dyn mod_api::ClientHost = game;
        let (graphics, gui, chat, audio) = (
            host.graphics_options(),
            host.gui_options(),
            host.chat_options(),
            host.audio_options(),
        );
        assert_eq!(graphics.fov, 90.0);
        assert_eq!(graphics.render_distance, 12);
        assert_eq!(graphics.brightness, 0.75);
        assert_eq!(gui.gui_scale, 3);
        assert!(gui.show_subtitles);
        assert_eq!(chat.visibility, 1);
        assert_eq!(chat.delay, 1.5);
        assert_eq!(audio.master_volume, 0.25);

        game.settings.fov = 70.0;
        let host: &dyn mod_api::ClientHost = game;
        assert_eq!(host.graphics_options().fov, 70.0);
        assert_eq!(graphics.fov, 90.0, "an older snapshot stays unchanged");
    });
}

#[test]
#[ignore = "daily"]
fn client_input_options_follow_settings() {
    super::native_entity_refs::tests::with_client_game(|game| {
        game.settings.sensitivity = 0.25;
        game.settings.invert_x = true;
        game.settings.invert_y = false;
        game.settings.mouse_wheel_sensitivity = 0.75;
        game.settings.discrete_scroll = true;
        game.settings.toggle_crouch = false;
        game.settings.toggle_sprint = true;
        game.settings.auto_jump = false;
        let host: &dyn mod_api::ClientHost = game;
        let first = host.input_options();
        assert_eq!(first.sensitivity, 0.25);
        assert!(first.invert_x);
        assert!(!first.invert_y);
        assert_eq!(first.mouse_wheel_sensitivity, 0.75);
        assert!(first.discrete_scroll);
        assert!(!first.toggle_crouch);
        assert!(first.toggle_sprint);
        assert!(!first.auto_jump);

        game.settings.sensitivity = 0.5;
        game.settings.invert_x = false;
        game.settings.invert_y = true;
        game.settings.mouse_wheel_sensitivity = 1.5;
        game.settings.discrete_scroll = false;
        game.settings.toggle_crouch = true;
        game.settings.toggle_sprint = false;
        game.settings.auto_jump = true;
        let host: &dyn mod_api::ClientHost = game;
        let second = host.input_options();
        assert_eq!(second.sensitivity, 0.5);
        assert!(!second.invert_x);
        assert!(second.invert_y);
        assert_eq!(second.mouse_wheel_sensitivity, 1.5);
        assert!(!second.discrete_scroll);
        assert!(second.toggle_crouch);
        assert!(!second.toggle_sprint);
        assert!(second.auto_jump);
        assert_eq!(first.sensitivity, 0.25);
    });
}

#[test]
fn weather_follows_the_dimension_type_not_the_id() {
    use super::level_can_have_weather as w;
    assert!(w(true, false, false)); // open sky
    assert!(!w(true, true, false)); // ceiling (Nether, overworld_caves)
    assert!(!w(false, false, false)); // no sky light
    assert!(!w(true, false, true)); // the End is named by id
}

use super::*;
use horizon_client::container_screen::{Button, Clicks, Release, Target};

#[test]
#[ignore = "daily"]
fn client_modifier_snapshot_tracks_focus_loss() {
    use mod_api::ClientHost;
    use winit::{event_loop::ControlFlow, keyboard::ModifiersState};
    struct TestLoop;
    impl EventLoopCtl for TestLoop {
        fn exit(&self) {}
        fn set_control_flow(&self, _: ControlFlow) {}
    }
    super::native_entity_refs::tests::with_client_game(|game| {
        game.modifiers = ModifiersState::SHIFT | ModifiersState::ALT;
        let snapshot = (game as &dyn ClientHost).modifiers();
        assert!(snapshot.shift());
        assert!(snapshot.alt());
        assert!(!snapshot.control());
        assert!(!snapshot.super_key());
        game.handle_window_event_inner(&TestLoop, WindowEvent::Focused(false));
        assert_eq!((game as &dyn ClientHost).modifiers().bits, 0);
        assert_eq!(snapshot.bits, 5);
    });
}

/// `ClipContext.Block.OUTLINE` for the SOURCE_ONLY ray (#8180): a ray
/// through the empty part of a torch's cell is not stopped by it.
#[test]
fn the_fluid_ray_passes_through_the_hollow_of_a_torch() {
    // A torch: a 2x10x2 pillar in the middle of its cell, at (0, 0, 0).
    let torch = || vec![[0.4375, 0.0, 0.4375, 0.5625, 0.625, 0.5625]];
    let dir = [0.0, 0.0, 1.0];
    // Beside the pillar: no hit, the ray goes on to the water behind.
    assert!(outline_stop([0.1, 0.3, -1.0], dir, 5.0, [0, 0, 0], [0, 0, -1], torch).is_none());
    // Above the pillar (a torch is 10/16 tall): no hit either.
    assert!(outline_stop([0.5, 0.8, -1.0], dir, 5.0, [0, 0, 0], [0, 0, -1], torch).is_none());
    // Through the pillar: stopped on its front face, not at the cell's edge.
    let hit = outline_stop([0.5, 0.3, -1.0], dir, 5.0, [0, 0, 0], [0, 0, -1], torch)
        .expect("the pillar stops the ray");
    assert_eq!(hit.face, [0, 0, -1]);
    assert!((hit.point[2] - 0.4375).abs() < 1e-9);
}

#[test]
#[ignore = "daily"]
fn right_click_block_controls_cauldron_use() {
    use mod_api::{self, BlockUpdateFlags, Flow, Hooks, RightClickBlockEvent, TriState};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    struct Observer(Arc<AtomicBool>, Arc<AtomicBool>, Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn right_click_block(
            &self,
            _: mod_api::HostSide,
            event: &mut RightClickBlockEvent<'_>,
        ) -> Flow {
            self.2.fetch_add(1, Ordering::SeqCst);
            if self.1.load(Ordering::SeqCst) {
                return Flow::Cancel;
            }
            *event.use_block = if self.0.load(Ordering::SeqCst) {
                TriState::True
            } else {
                TriState::False
            };
            *event.use_item = TriState::False;
            Flow::Continue
        }
    }
    let allowed = Arc::new(AtomicBool::new(false));
    let cancel = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = Arc::from([mod_api::ModEntry::new(
        "right-click-observer",
        Box::new(Observer(allowed.clone(), cancel.clone(), calls.clone())),
    )]);
    mod_platform::__private::install(mods);
    super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        let level = game.level("minecraft:overworld").unwrap();
        let empty = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cauldron")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], empty, BlockUpdateFlags::NEIGHBORS),
            Ok(true)
        );
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:water_bucket", 1));
        let hit = Hit {
            pos: [0, 64, 0],
            face: [0, 1, 0],
            point: [0.5, 65.0, 0.5],
            inside: false,
        };
        game.use_hand(Some(&hit)).unwrap();
        assert_eq!(game.world.block(0, 64, 0), empty);
        allowed.store(true, Ordering::SeqCst);
        game.use_hand(Some(&hit)).unwrap();
        assert_ne!(game.world.block(0, 64, 0), empty);
        assert_eq!(
            game.level_set_block(level, hit.pos, empty, BlockUpdateFlags::NEIGHBORS),
            Ok(true)
        );
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:water_bucket", 1));
        game.keys.insert(KeyCode::ShiftLeft);
        game.use_hand(Some(&hit)).unwrap();
        assert_ne!(game.world.block(0, 64, 0), empty);
        game.keys.remove(&KeyCode::ShiftLeft);

        let campfire = game
            .pack
            .assets
            .registry
            .default_state("minecraft:campfire")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, hit.pos, campfire, BlockUpdateFlags::NEIGHBORS),
            Ok(true)
        );
        let entity = game.new_block_entity("minecraft:campfire").unwrap();
        game.world.block_entities.insert((0, 64, 0), entity);
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:beef", 2));
        allowed.store(false, Ordering::SeqCst);
        game.use_hand(Some(&hit)).unwrap();
        assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 2);
        allowed.store(true, Ordering::SeqCst);
        game.use_hand(Some(&hit)).unwrap();
        assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 1);

        // A canceled click on a fence must not tie the player's mob.
        let fence = game
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_fence")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, hit.pos, fence, BlockUpdateFlags::NEIGHBORS),
            Ok(true)
        );
        let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
        let mut mob = horizon_client::mobs::Mob::new(
            12,
            kind,
            horizon_core::geometry::Vec3::new(1.0, 64.0, 1.0),
            0.0,
        );
        mob.extra.leash = Some(horizon_client::mobs::leash::Holder::Player);
        game.mobs.push(mob);
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:lead", 1));
        game.guest_use = Some(GuestUse::OnBlock(Hand::Main));
        game.forced_hit = Some(hit);
        cancel.store(true, Ordering::SeqCst);
        let before = calls.load(Ordering::SeqCst);
        game.use_item_as_player().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), before + 1);
        assert_eq!(
            game.mobs.last().unwrap().extra.leash,
            Some(horizon_client::mobs::leash::Holder::Player)
        );

        // A spectator's packet reaches the event, even when canceled.
        game.set_mode_id(3);
        game.guest_use = Some(GuestUse::OnBlock(Hand::Main));
        game.forced_hit = Some(hit);
        cancel.store(true, Ordering::SeqCst);
        let before = calls.load(Ordering::SeqCst);
        game.use_item_as_player().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), before + 1);
    });
}

#[test]
#[ignore = "daily"]
fn fence_ties_without_a_lead_only_on_main_hand() {
    use horizon_client::mobs::leash::Holder;
    use mod_api::BlockUpdateFlags;
    super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        let level = game.level("minecraft:overworld").unwrap();
        let fence = game
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_fence")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], fence, BlockUpdateFlags::NEIGHBORS),
            Ok(true)
        );
        let hit = Hit {
            pos: [0, 64, 0],
            face: [0, 1, 0],
            point: [0.5, 65.0, 0.5],
            inside: false,
        };
        let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
        let mut mob = horizon_client::mobs::Mob::new(
            12,
            kind,
            horizon_core::geometry::Vec3::new(1.0, 64.0, 1.0),
            0.0,
        );
        mob.extra.leash = Some(Holder::Player);
        game.mobs.push(mob);
        game.forced_hit = Some(hit);
        game.guest_use = Some(GuestUse::OnBlock(Hand::Off));
        game.with_hand(Hand::Off, |game| game.use_hand(Some(&hit)))
            .unwrap();
        assert_eq!(game.mobs[0].extra.leash, Some(Holder::Player));
        game.offhand = Some(ItemStack::new("minecraft:lead", 1));
        game.with_hand(Hand::Off, |game| game.use_hand(Some(&hit)))
            .unwrap();
        assert_eq!(game.mobs[0].extra.leash, Some(Holder::Knot(hit.pos)));
        game.mobs[0].extra.leash = Some(Holder::Player);
        game.offhand = None;
        game.guest_use = Some(GuestUse::OnBlock(Hand::Main));
        game.with_hand(Hand::Main, |game| game.use_hand(Some(&hit)))
            .unwrap();
        assert_eq!(game.mobs[0].extra.leash, Some(Holder::Knot(hit.pos)));
    });
}

#[test]
fn tool_transform_keeps_hoe_and_shovel_results_with_scrape_particle() {
    use mod_api::ToolAbility::{AxeScrape, HoeTill, ShovelFlatten};
    let result = BlockStateId(42);
    for (transformer, ability) in [
        ("minecraft:hoe", HoeTill),
        ("minecraft:shovel", ShovelFlatten),
    ] {
        assert_eq!(
            vanilla_tool_state(transformer, "scrape", ability, result),
            Some(result)
        );
        assert_eq!(
            vanilla_tool_state(transformer, "scrape", AxeScrape, result),
            None
        );
    }
}
/// `MinecraftServer.computeNextAutosaveInterval`: three hundred seconds of
/// ticks at the rate the server runs at, never fewer than a hundred.
#[test]
fn the_autosave_waits_for_three_hundred_seconds_of_ticks() {
    // 20 ticks a second is the six thousand ticks a world autosaved on
    // before the rate could be changed (`/tick rate`).
    assert_eq!(autosave_interval(20.0), 6000);
    assert_eq!(autosave_interval(10.0), 3000);
    assert_eq!(autosave_interval(40.0), 12000);
    assert_eq!(autosave_interval(1000.0), 300_000);
    // `Math.max(100, ...)`: a very low rate still autosaves every hundred
    // ticks, and a frozen clock (0) does too.
    assert_eq!(autosave_interval(0.5), 150);
    assert_eq!(autosave_interval(0.0), 100);
}

/// `Minecraft.<init>`: only an explicit `startedCleanly: false` is an
/// unclean start, and it leaves fullscreen; the packs stay selected.
#[test]
fn an_unclean_start_leaves_fullscreen_and_keeps_the_packs() {
    let fullscreen = || Settings {
        fullscreen: true,
        resource_packs: vec!["pack".into()],
        ..Default::default()
    };
    // The key absent (a first boot) is a clean start.
    let mut settings = fullscreen();
    assert!(!unclean_start_recovery(&mut settings));
    assert!(settings.fullscreen);
    let mut settings = fullscreen();
    settings.file.fullscreen_resolution = Some("1920x1080@60:24".into());
    settings.file.set(
        "startedCleanly",
        horizon_client::options_file::V::Bool(false),
    );
    assert!(unclean_start_recovery(&mut settings));
    assert!(!settings.fullscreen);
    assert_eq!(settings.file.fullscreen_resolution, None);
    assert_eq!(settings.resource_packs, ["pack"]);
    let mut settings = fullscreen();
    settings.file.set(
        "startedCleanly",
        horizon_client::options_file::V::Bool(true),
    );
    assert!(!unclean_start_recovery(&mut settings));
    assert!(settings.fullscreen);
}

/// `ScreenView.id` (the `screen_render_post` hook) is a mod's
/// contract: every `Screen` has its own id, and it does not change.
/// `Screen::id`'s match makes a new variant give itself one; this list
/// makes the test cover it too.
#[test]
fn screen_ids_are_distinct_and_stable() {
    use std::collections::BTreeSet;
    let all = [
        Screen::Playing,
        Screen::GameModeSwitcher,
        Screen::Paused,
        Screen::Options,
        Screen::Chat,
        Screen::Creative,
        Screen::Inventory,
        Screen::Dead,
        Screen::Stats,
        Screen::Advancements,
        Screen::WorldOptions,
        Screen::Friends,
        Screen::Social,
        Screen::CommandBlock,
        Screen::BlockEditor,
        Screen::ServerLinks,
        Screen::ConfirmLink,
        Screen::DialogWarning,
        Screen::Win,
        Screen::Mod,
        Screen::HudEditor,
        Screen::Studio,
        Screen::DebugOptions,
    ];
    let ids: Vec<&'static str> = all.iter().map(|screen| screen.id()).collect();
    let unique: BTreeSet<&'static str> = ids.iter().copied().collect();
    assert_eq!(unique.len(), all.len(), "two screens share an id");
    for id in &ids {
        assert!(!id.is_empty(), "an empty screen id");
        assert!(
            id.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{id} is not a snake_case id"
        );
    }
    // The ones a mod can match on in the hook's docs.
    assert_eq!(Screen::Playing.id(), "playing");
    assert_eq!(Screen::Inventory.id(), "inventory");
    assert_eq!(Screen::Win.id(), "win");
    assert_eq!(Screen::Studio.id(), "studio");
    assert_ne!(Screen::Studio.id(), Screen::Chat.id());
}

/// The `modifiers` a native mod's input hooks hand over are GLFW's bits
/// (`GLFW_MOD_SHIFT` 1, `GLFW_MOD_CONTROL` 2, `GLFW_MOD_ALT` 4,
/// `GLFW_MOD_SUPER` 8), each on its own and all together.
#[test]
fn glfw_modifier_bits_are_glfw_modifier_bits() {
    use winit::keyboard::ModifiersState;
    for (state, bits) in [
        (ModifiersState::empty(), 0),
        (ModifiersState::SHIFT, 1),
        (ModifiersState::CONTROL, 2),
        (ModifiersState::ALT, 4),
        (ModifiersState::SUPER, 8),
        (ModifiersState::SHIFT | ModifiersState::ALT, 5),
        (ModifiersState::CONTROL | ModifiersState::SUPER, 10),
        (ModifiersState::SHIFT | ModifiersState::CONTROL, 3),
        (
            ModifiersState::SHIFT
                | ModifiersState::CONTROL
                | ModifiersState::ALT
                | ModifiersState::SUPER,
            15,
        ),
    ] {
        assert_eq!(glfw_modifiers(state), bits, "{state:?}");
    }
    // Every modifier GLFW has a bit for, and no other.
    assert_eq!(glfw_modifiers(ModifiersState::all()), 15);
}

/// The `button` a native mod's mouse hooks hand over is GLFW's index (0
/// left, 1 right, 2 middle, 3 back, 4 forward, else the button's number),
/// the one `keybinds::mouse_name` names (`key.mouse.<index + 1>` from the
/// back button on).
#[test]
fn glfw_button_indices_are_glfw_indices() {
    for (button, index, name) in [
        (MouseButton::Left, 0, "key.mouse.left"),
        (MouseButton::Right, 1, "key.mouse.right"),
        (MouseButton::Middle, 2, "key.mouse.middle"),
        (MouseButton::Back, 3, "key.mouse.4"),
        (MouseButton::Forward, 4, "key.mouse.5"),
    ] {
        assert_eq!(glfw_button(button), index);
        assert_eq!(horizon_client::keybinds::mouse_name(button), name);
    }
    for n in 5..12u16 {
        assert_eq!(glfw_button(MouseButton::Other(n)), i32::from(n));
        assert_eq!(
            horizon_client::keybinds::mouse_name(MouseButton::Other(n)),
            format!("key.mouse.{}", n + 1)
        );
    }
}

/// F3 left on when leaving a world kept the axes on the title screen.
#[test]
fn the_debug_crosshair_is_gone_once_the_world_is() {
    assert!(debug_crosshair_drawn(true, true, false, false, false));
    assert!(!debug_crosshair_drawn(true, true, false, true, false));
}

/// Opening a world in front of a block left its outline over Loading terrain's panorama (#12872).
#[test]
fn the_block_outline_stays_off_the_panorama() {
    assert!(block_outline_drawn(false, false, true, false));
    assert!(!block_outline_drawn(false, false, true, true));
}

/// Snuffing a candle or a candle cake draws one smoke per
/// `getParticleOffsets`: by `candles` for a candle, a block up for a cake.
#[test]
fn a_snuffed_candle_and_candle_cake_smoke_at_vanillas_offsets() {
    assert_eq!(
        Game::candle_offsets("minecraft:candle", Some(1)),
        [[8u8, 8, 8]].as_slice()
    );
    assert_eq!(
        Game::candle_offsets("minecraft:red_candle", Some(3)),
        [[8u8, 5, 10], [6, 7, 8], [9, 8, 7]].as_slice()
    );
    assert_eq!(
        Game::candle_offsets("minecraft:candle_cake", None),
        [[8u8, 16, 8]].as_slice()
    );
}

use horizon_client::keymapping::{KeyAction, SNEAK, SPRINT, ToggleKeys};

/// A guest's interact packet names its entity and the server keeps it:
/// `ServerGamePacketListenerImpl.handleInteract` resolves
/// `packet.entityId()` once and `Player.interactOn` clicks that entity,
/// while a fresh pick would take the nearest one. Two cows stand one
/// behind the other; the packet names the far one.
#[test]
fn an_interact_packet_clicks_the_entity_it_names_not_the_nearest() {
    // (server id, distance from the guest) in the level's own order.
    let cows = [(17u32, 1.5f64), (23, 2.5)];
    assert_eq!(Game::named_entity(&cows, 23, |c| c.0), Some(1));
    // The near cow is reachable too, when that is the one named.
    assert_eq!(Game::named_entity(&cows, 17, |c| c.0), Some(0));
    // An id the level does not hold clicks nothing (Level.getEntity).
    assert_eq!(Game::named_entity(&cows, 31, |c| c.0), None);
}

/// The id path keeps the crosshair pick's filters, so a packet cannot name
/// a mob the pick would refuse: one in its death animation, the one the
/// player rides, or the camera entity. Without them a guest could shear,
/// feed or leash a dying mob that `mob_hit` never returns.
#[test]
fn an_interact_packet_cannot_name_a_mob_the_pick_would_refuse() {
    use super::creatures::named_mob_pickable;
    assert!(named_mob_pickable(true, 17, None, None));
    // Dead: `Mob.alive()` is health > 0.
    assert!(!named_mob_pickable(false, 17, None, None));
    // Ridden, and the camera entity (`remote_camera_id`).
    assert!(!named_mob_pickable(true, 17, Some(17), None));
    assert!(!named_mob_pickable(true, 17, None, Some(17)));
    // Another mob being ridden or watched does not block this one.
    assert!(named_mob_pickable(true, 17, Some(23), Some(29)));
}

#[path = "incoming_tests/pet_order.rs"]
mod pet_order_tests;
/// The packet's location is `EntityHitResult.getLocation() -
/// entity.position()`, the point the view ray met the box at measured
/// from the mob's feet (`MultiPlayerGameMode.interact`). The old fixed
/// [0, 1, 0] was a point inside every mob, which the server's
/// `interactAt` paths (armour stands, saddles, leashes) read as the click.
#[test]
fn an_interact_packet_carries_the_point_the_ray_hit() {
    let feet = Vec3::new(0.5, 0.0, 2.0);
    let eye = [0.5, 1.5, 1.0];
    let dir = [0.0, 0.0, 1.0];
    let box_at = Aabb::new(feet, Vec3::new(1.5, 2.0, 3.0));
    let t = ray_box(eye, dir, 3.0, box_at).unwrap();
    let point = Vec3::new(
        eye[0] + dir[0] * t,
        eye[1] + dir[1] * t,
        eye[2] + dir[2] * t,
    );
    // The ray enters the box's near face halfway up it, directly above
    // the feet: not the old centre point of the entity.
    assert_eq!(interact_location(point, feet), [0.0, 1.5, 0.0]);
}

/// A mod screen on a hopper or dispenser menu owns its slots: the stock
/// grid stands back so `slot_position` reaches the mod's own slots
/// (`native_screen_slot`) and its `inventory_y` (#5141). Without a mod
/// screen the stock layout answers exactly as before, which is what keeps
/// this invisible to players who run no native mods.
#[test]
fn a_mod_screen_on_a_machine_menu_takes_the_stock_layout_out() {
    let stock = Some((8.0, 109.0));
    // A mod screen is open: the stock grid does not answer.
    assert_eq!(Game::stock_machine_slot(stock, true), None);
    // No mod screen: the hopper and dispenser layouts are untouched.
    assert_eq!(Game::stock_machine_slot(stock, false), stock);
    // Other menus have no stock layout to stand back from.
    assert_eq!(Game::stock_machine_slot(None, true), None);
    assert_eq!(Game::stock_machine_slot(None, false), None);
}

/// A falling block is drawn moved by its column's model offset, as
/// `ModelBlockRenderer.tesselateBlock` does with `BlockState.getOffset`
/// of `BlockPos.containing(x, maxY, z)`: a pointed dripstone that breaks
/// off stays where it hung instead of jumping to the block's centre (arismoko/horizon#4102).
#[test]
fn a_falling_block_keeps_its_model_offset() {
    let at = Vec3::new(3.5, 70.25, -1.5);
    let falling = FallingBlock {
        on_ground: false,
        attachment_lifetime: Default::default(),
        rotation: [0.0, 0.0],
        join: Default::default(),
        id: 0,
        state: BlockStateId(7),
        start_pos: BlockPos::new(3, 72, -2),
        position: at,
        previous: Vec3::new(3.5, 71.0, -1.5),
        velocity: Vec3::ZERO,
        age: 5,
        no_gravity: false,
        portal: Default::default(),
        drop_item: true,
        fall_distance: 0.0,
        stuck: Vec3::ZERO,
        damage: horizon_client::falling_block::Damage::default(),
        saved: None,
    };
    // OffsetType.XZ: one offset per column (Mth.getSeed(x, 0, z)).
    let offset = |state: BlockStateId, x: i32, z: i32| {
        assert_eq!(state, BlockStateId(7));
        ((x, z) == (3, -2)).then_some([0.125, 0.0, -0.0625])
    };
    let origin = falling.model_origin(0.5, offset);
    assert_eq!(origin, glam::DVec3::new(3.125, 70.625, -2.0625));
    // Blocks without a random offset are drawn centred on the entity.
    let origin = falling.model_origin(1.0, |_, _, _| None);
    assert_eq!(origin, glam::DVec3::new(3.0, 70.25, -2.0));
}

/// `FallingBlockRenderer.shouldRender` emits nothing for a falling block whose
/// own state is the state of the cell it is in (a `NoGravity` block, or one
/// that has just landed), where Horizon drew a z-fighting copy; the same block
/// draws its model again once that cell is cleared.
#[test]
fn a_falling_block_in_its_own_state_emits_nothing() {
    let state = BlockStateId(7);
    assert!(!falling_block_visible(state, state));
    assert!(falling_block_visible(state, BlockStateId(0)));
}

/// A falling block does not sneak, so `Entity.isDescending()` is false.
/// Scaffolding top surface collides solidly with non-descending entities
/// so falling scaffolding lands on top of existing scaffolding instead
/// of falling through and breaking (#7553).
#[test]
fn falling_block_does_not_descend_through_scaffolding() {
    let pos = BlockPos::new(0, 10, 0);
    let scaffold = horizon_world::block_collision::ScaffoldingCollision::from_state(
        &horizon_world::chunk::BlockState {
            name: "minecraft:scaffolding".to_string(),
            properties: [
                ("bottom".to_string(), "false".to_string()),
                ("distance".to_string(), "0".to_string()),
                ("waterlogged".to_string(), "false".to_string()),
            ]
            .into_iter()
            .collect(),
        },
    )
    .unwrap()
    .expect("scaffolding collision rule");

    // When a falling block is falling downward (wanted.y < 0) above the scaffolding top (y = 11.0):
    let falling_y = 11.0;
    let buggy_descending_context = CollisionContext::for_entity(
        CollisionEntity {
            kind: horizon_world::collision_context::EntityContextKind::Other,
            identity: EntityIdentity(0),
            y: falling_y,
            descending: true, // the old buggy behavior: wanted.y < 0.0
            main_hand: None,
        },
        false,
    );
    let fixed_context = CollisionContext::for_entity(
        CollisionEntity {
            kind: horizon_world::collision_context::EntityContextKind::Other,
            identity: EntityIdentity(0),
            y: falling_y,
            descending: false, // fixed: FallingBlockEntity does not sneak
            main_hand: None,
        },
        false,
    );

    // Before the fix, descending=true caused scaffolding collision to be empty:
    assert!(buggy_descending_context.is_descending());
    let buggy_shape = scaffold.shape(&buggy_descending_context, pos);
    assert!(
        buggy_shape.is_empty(),
        "descending entities fall through scaffolding"
    );

    // With descending=false, scaffolding provides stable top collision:
    assert!(!fixed_context.is_descending());
    let solid_shape = scaffold.shape(&fixed_context, pos);
    assert!(
        !solid_shape.is_empty(),
        "falling block must collide with scaffolding top"
    );
}

/// A key bound to key.attack keeps attacking while it is held: each
/// repeat clicks like the press (arismoko/horizon#3243).
#[test]
fn a_held_key_bound_to_a_button_repeats_its_clicks() {
    assert_eq!(keyboard_button(true, KeyAction::Press), Some(true));
    assert_eq!(keyboard_button(true, KeyAction::Repeat), Some(true));
    assert_eq!(keyboard_button(true, KeyAction::Release), Some(false));
    for action in [KeyAction::Press, KeyAction::Repeat, KeyAction::Release] {
        assert_eq!(keyboard_button(false, action), None);
    }
}

/// A server dialog, a book, a sign editor or a prompt over the world is a
/// screen: held keys and the mouse buttons are released
/// (`KeyMapping.releaseAll`), and toggled sneak comes back when it closes
/// (#4974, #9729).
#[test]
fn a_dialog_releases_held_keys_like_a_screen() {
    assert!(gameplay_has_keyboard(Screen::Playing, false, false));
    assert!(!gameplay_has_keyboard(Screen::Playing, false, true));
    assert!(!gameplay_has_keyboard(Screen::Playing, true, false));
    assert!(!gameplay_has_keyboard(Screen::Chat, false, false));

    let mut keys = horizon_client::held::HeldKeys::from_iter([KeyCode::KeyW]);
    let mut left = true;
    let mut right = true;
    drop_held_behind_dialog(false, &mut keys, &mut left, &mut right);
    assert!(keys.raw(&KeyCode::KeyW) && left && right);
    drop_held_behind_dialog(true, &mut keys, &mut left, &mut right);
    assert!(keys.is_empty() && !left && !right);

    let options = [true, false, false, false];
    let mut toggles = ToggleKeys::default();
    toggles.key(SNEAK, KeyAction::Press, false, true);
    toggles.key(SNEAK, KeyAction::Release, false, true);
    assert!(toggles.is_down(SNEAK));
    toggles.screen_opened(options);
    assert!(!toggles.is_down(SNEAK));
    toggles.screen_closed(options);
    assert!(toggles.is_down(SNEAK));
}
/// A key bound to key.pickItem clones over a container screen, as the
/// middle button does; a key bound to attack or use does nothing there (#5378).
#[test]
fn a_key_bound_to_pick_block_clones_behind_a_container_screen() {
    assert!(container_key_clones(MouseButton::Middle, KeyAction::Press));
    assert!(container_key_clones(MouseButton::Middle, KeyAction::Repeat));
    assert!(!container_key_clones(
        MouseButton::Middle,
        KeyAction::Release
    ));
    for button in [MouseButton::Left, MouseButton::Right] {
        assert!(!container_key_clones(button, KeyAction::Press));
    }
}
#[test]
fn sleep_fade_lasts_ten_ticks_after_waking() {
    let alpha = |timer| match hud_overlays::sleep_overlay(320, 240, timer).first() {
        Some(hud_layout::GuiDraw::Fill { color, .. }) => color >> 24,
        None => 0,
        _ => panic!("sleep overlay must be a full-screen fill"),
    };
    assert_eq!(tick_sleep_timer(0, false), 0);
    let mut timer = 0;
    for _ in 0..50 {
        timer = tick_sleep_timer(timer, true);
    }
    assert_eq!(alpha(timer as i32), 110);
    for _ in 0..100 {
        timer = tick_sleep_timer(timer, true);
    }
    assert_eq!(timer, 100);
    assert_eq!(alpha(timer as i32), 220);
    for _ in 0..5 {
        timer = tick_sleep_timer(timer, false);
    }
    assert_eq!(alpha(timer as i32), 110);
    for _ in 0..5 {
        timer = tick_sleep_timer(timer, false);
    }
    assert_eq!(timer, 0);
    assert!(hud_overlays::sleep_overlay(320, 240, timer as i32).is_empty());
}

#[test]
fn world_rest_poll_does_not_wait_for_a_blocked_worker() {
    let (release, wait) = std::sync::mpsc::channel();
    let loader = WorldRestLoader {
        progress: Progress::new(),
        handle: std::thread::spawn(move || {
            wait.recv().unwrap();
            Ok(())
        }),
    };
    assert!(!loader.ready().unwrap());
    release.send(()).unwrap();
    loader.finish().unwrap();
}

#[test]
fn world_rest_poll_waits_for_a_slow_worker() {
    let (release, wait) = std::sync::mpsc::channel();
    let loader = WorldRestLoader {
        progress: Progress::new(),
        handle: std::thread::spawn(move || {
            wait.recv().unwrap();
            Ok(())
        }),
    };
    // However long it takes, a worker still loading is not an error.
    assert!(!loader.ready().unwrap());
    release.send(()).unwrap();
    while !loader.ready().unwrap() {
        std::thread::sleep(Duration::from_millis(1));
    }
    loader.finish().unwrap();
}

#[test]
fn world_rest_worker_failure_reaches_the_client() {
    let loader = WorldRestLoader {
        progress: Progress::new(),
        handle: std::thread::spawn(|| Err("missing world table".to_owned())),
    };
    assert_eq!(
        loader.finish().unwrap_err().to_string(),
        "missing world table"
    );
}

#[test]
fn world_rest_worker_panic_reaches_the_client() {
    let loader = WorldRestLoader {
        progress: Progress::new(),
        handle: std::thread::spawn(|| panic!("broken loader")),
    };
    assert_eq!(
        loader.finish().unwrap_err().to_string(),
        "world resource loader panicked"
    );
}

/// An integrated server thread shares the loaded resources with the client.
#[test]
fn shared_resources_can_cross_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<Resources>();
    shared::<PackResources>();
}

#[test]
fn selecting_creative_tabs_does_not_drop_the_carried_stack_on_release() {
    let mut creative = Creative::from_json(&serde_json::json!({
        "schema": 2,
        "tabs": [
            {"type": "CATEGORY", "items": ["{count:1,id:\"minecraft:stone\"}"]},
            {"type": "INVENTORY"}
        ]
    }))
    .unwrap();
    for button in [Button::Left, Button::Right] {
        let mut clicks = Clicks::default();
        for tab in [1, 1, 0, 1] {
            select_creative_tab(&mut creative, &mut clicks, tab);
            assert_eq!(creative.selected, tab);
            assert_eq!(creative.tab().inventory, tab == 1);
            assert_eq!(
                clicks.release(
                    &Target::<usize>::Outside,
                    button,
                    false,
                    false,
                    false,
                    false
                ),
                Release::Nothing
            );
        }
    }
}

/// Eyes under water: the ray hits the source the eyes are in, at the eyes
/// (arismoko/horizon#3787), whichever way it looks.
#[test]
fn a_ray_starting_inside_a_source_hits_it_at_the_start() {
    let eye = [0.5, 65.62, 0.5];
    let down = [0.0, -1.0, 0.0];
    assert_eq!(
        clip_start_source(eye, down, 4.5, [0, 65, 0], true),
        Some(([0, 1, 0], eye))
    );
    // Without water above, 0.62 is still below the surface (8/9).
    assert_eq!(
        clip_start_source(eye, [0.0, 1.0, 0.0], 4.5, [0, 65, 0], false),
        Some(([0, -1, 0], eye))
    );
    // Looking ahead, slightly down: SOUTH's dot product is largest.
    let ahead = [0.0, -0.2f64, 0.98f64.sqrt()];
    assert_eq!(
        clip_start_source(eye, ahead, 4.5, [0, 65, 0], true),
        Some(([0, 0, -1], eye))
    );
}

/// Eyes above a surface source's 8/9: a ray going down enters it through
/// the top inside the block; level or rising rays miss it.
#[test]
fn a_ray_starting_above_a_source_surface_enters_through_the_top() {
    let eye = [0.5, 65.95, 0.5];
    let hit = clip_start_source(eye, [0.0, -1.0, 0.0], 4.5, [0, 65, 0], false);
    let (face, point) = hit.unwrap();
    assert_eq!(face, [0, 1, 0]);
    assert!((point[1] - (65.0 + f64::from(8.0f32 / 9.0f32))).abs() < 1e-12);
    assert_eq!(
        clip_start_source(eye, [1.0, 0.0, 0.0], 4.5, [0, 65, 0], false),
        None
    );
    // Leaves the block sideways before it comes down to the surface.
    let shallow = [0.995f64.sqrt(), -0.0707, 0.0];
    assert_eq!(
        clip_start_source(eye, shallow, 4.5, [0, 65, 0], false),
        None
    );
}

#[test]
fn stack_drop_requires_ctrl_even_with_sprint_latched() {
    for sprint in [false, true] {
        let mut toggles = ToggleKeys::default();
        if sprint {
            toggles.key(SPRINT, KeyAction::Press, false, true);
            toggles.key(SPRINT, KeyAction::Release, false, true);
        }
        assert_eq!(toggles.is_down(SPRINT), sprint);

        for held in 0..4 {
            let mut modifiers = winit::keyboard::ModifiersState::empty();
            let mut physical_keys = HashSet::from([KeyCode::ShiftLeft, KeyCode::KeyQ]);
            let sides = [KeyCode::ControlLeft, KeyCode::ControlRight];
            for (bit, key) in sides.into_iter().enumerate() {
                if held & (1 << bit) != 0 {
                    horizon_client_input::modifiers::set_key(
                        &mut modifiers,
                        &mut physical_keys,
                        key,
                        true,
                    );
                }
            }
            assert_eq!(drop_whole_stack(modifiers), held != 0);
            for key in sides {
                horizon_client_input::modifiers::set_key(
                    &mut modifiers,
                    &mut physical_keys,
                    key,
                    false,
                );
            }
            assert!(!drop_whole_stack(modifiers));
            assert_eq!(toggles.is_down(SPRINT), sprint);
        }
    }
}

#[test]
fn quit_shortcut_blocks_window_close_when_option_disabled() {
    use winit::keyboard::ModifiersState;

    let mut keys = HashSet::new();
    let alt = ModifiersState::ALT;
    let super_key = ModifiersState::SUPER;

    let quit_shortcuts_enabled = true;
    let quit_shortcuts_disabled = false;

    keys.insert(if cfg!(target_os = "macos") {
        KeyCode::KeyQ
    } else {
        KeyCode::F4
    });
    let mods = if cfg!(target_os = "macos") {
        super_key
    } else {
        alt
    };

    let should_ignore =
        |quit_shortcuts: bool, modifiers: ModifiersState, keys: &HashSet<KeyCode>| {
            !quit_shortcuts && horizon_client::input::is_quit_shortcut_down(modifiers, keys)
        };

    // When enabled, close requests from shortcuts proceed normally.
    assert!(!should_ignore(quit_shortcuts_enabled, mods, &keys));

    // When disabled and the quit shortcut is held, close requests are ignored.
    assert!(should_ignore(quit_shortcuts_disabled, mods, &keys));

    // When disabled but shortcut NOT held (e.g. 'X' button clicked), close request proceeds.
    assert!(!should_ignore(
        quit_shortcuts_disabled,
        ModifiersState::empty(),
        &HashSet::new()
    ));
}

#[test]
fn integrated_server_save_persists_owner_player_data() {
    let dir = std::env::temp_dir().join(format!(
        "horizon-integrated-player-save-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    // Initial world creation writes `level.dat` before the player spawns.
    let mut level = Level {
        name: "Creative".into(),
        creative: true,
        game_mode: 1,
        ..Level::default()
    };
    level.write(&dir).unwrap();

    // The integrated server stores the owner's state in `level` and saves the world.
    level.position = Some([128.5, 72.0, -64.25]);
    level.inventory = vec![(0, "minecraft:diamond_pickaxe".into(), 1, 0)];
    write_level_for_role(
        &mut level,
        &dir,
        execution_role::ExecutionRole::IntegratedServer,
    )
    .unwrap();

    let reopened = Level::read(&dir).unwrap();
    assert_eq!(reopened.position, Some([128.5, 72.0, -64.25]));
    assert_eq!(
        reopened.inventory,
        vec![(0, "minecraft:diamond_pickaxe".into(), 1, 0)]
    );

    // A dedicated server save updates world metadata without overwriting the player file.
    let mut dedicated = Level {
        player_uuid: level.player_uuid,
        day_time: 6000,
        ..Level::default()
    };
    write_level_for_role(
        &mut dedicated,
        &dir,
        execution_role::ExecutionRole::DedicatedServer,
    )
    .unwrap();

    let reopened = Level::read(&dir).unwrap();
    assert_eq!(reopened.day_time, 6000);
    assert_eq!(reopened.position, Some([128.5, 72.0, -64.25]));
    assert_eq!(
        reopened.inventory,
        vec![(0, "minecraft:diamond_pickaxe".into(), 1, 0)]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `Screen::Studio` is the Studio script slot: it never draws the chat screen
/// (`Screen::Studio => {}` in `game.rs`'s draw match, unlike
/// `Screen::Chat => draw_chat_screen`), it never blurs the background, and
/// while it is up gameplay does not own the keyboard (WASD cannot walk
/// through it, and movement keys are cleared on open).
#[test]
fn studio_screen_stays_off_the_chat_slot() {
    assert!(Screen::Studio != Screen::Chat);
    assert_eq!(Screen::Studio.id(), "studio");
    assert!(!Screen::Studio.blurs_background(false));
    assert!(!Screen::Studio.blurs_background(true));
    assert!(!gameplay_has_keyboard(Screen::Studio, false, false));
    // The pure gate behind `Game::open_studio_screen`, as
    // `native_mod_screen_open`'s `mod_screen_opens` is gated.
    use super::studio::studio_screen_opens;
    assert_eq!(studio_screen_opens(Screen::Playing, false), None);
    assert_eq!(studio_screen_opens(Screen::Playing, true), Some(false));
    assert_eq!(studio_screen_opens(Screen::Chat, false), Some(false));
    assert_eq!(studio_screen_opens(Screen::Studio, false), Some(true));
    assert_eq!(studio_screen_opens(Screen::Studio, true), Some(true));
}

#[test]
fn eager_saves_keep_dirty_order_cooldowns_and_writer_budget() {
    use horizon_client::world::ModifiedChunks;
    use std::cell::Cell;
    let now = Instant::now();
    let mut dirty = ModifiedChunks::default();
    dirty.extend((0..27).map(|x| (x, 0)));
    dirty.autosaved(&[(0, 0), (2, 0)], now);
    dirty.remove(&(1, 0)); // already clean
    let writes = Cell::new(127);
    let mut chosen = Vec::new();
    dirty.save_eagerly(
        now,
        || writes.get() < 128,
        |k| k != (3, 0),
        |k| {
            chosen.push(k);
            writes.set(writes.get() + 1);
            true
        },
    );
    assert_eq!(chosen, [(4, 0)]); // cooldown, clean, absent, then budget exhausted
    chosen.clear();
    dirty.save_eagerly(
        now,
        || true,
        |_| true,
        |k| {
            chosen.push(k);
            true
        },
    );
    assert_eq!(chosen, (5..25).map(|x| (x, 0)).collect::<Vec<_>>()); // exactly 20
    dirty.insert((4, 0)); // a later edit goes AFTER the still-cooled entries
    chosen.clear();
    dirty.save_eagerly(
        now + Duration::from_secs(10),
        || true,
        |_| true,
        |k| {
            chosen.push(k);
            true
        },
    );
    assert_eq!(chosen, [(0, 0), (2, 0), (25, 0), (26, 0), (4, 0)]);
    dirty.insert((4, 0));
    dirty.save_eagerly(
        now + Duration::from_secs(11),
        || false,
        |_| true,
        |_| panic!("no time"),
    );
    assert!(dirty.contains(&(4, 0)));
    dirty.unloaded(&(4, 0)); // unloading removes the cooldown even if reloaded immediately
    chosen.clear();
    dirty.save_eagerly(
        now + Duration::from_secs(11),
        || true,
        |_| true,
        |k| {
            chosen.push(k);
            true
        },
    );
    assert_eq!(chosen, [(4, 0)]);
}

#[test]
#[ignore = "daily"]
fn eager_saves_wait_for_an_open_world_and_for_light() {
    let mut resources = None;
    let mut game = crate::game::game_tests::tests::server_game(&mut resources);
    let dir = std::env::temp_dir().join(format!("horizon-eager-game-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    game.save = Arc::new(horizon_client::save::SaveDir::new(
        dir.clone(),
        horizon_client::save::LevelShape::OVERWORLD,
    ));
    Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
    game.world
        .chunks
        .insert((1, 0), Arc::new(horizon_client::chunk::Chunk::empty(1, 0)));
    game.world.modified.extend([(0, 0), (1, 0)]);
    game.save_chunks_eagerly(); // no world open: nothing to save into
    assert!(game.world.modified.contains(&(0, 0)));
    game.world_root = dir.clone();
    game.save_chunks_eagerly();
    assert!(!game.world.modified.contains(&(0, 0)));
    assert!(game.world.modified.contains(&(1, 0))); // not lit yet
    Arc::make_mut(game.world.chunks.get_mut(&(1, 0)).unwrap()).lit = true;
    game.save_chunks_eagerly(); // still queued: no new dirty transition needed
    assert!(!game.world.modified.contains(&(1, 0)));
    game.save.flush().unwrap();
    assert!(dir.join("region/r.0.0.mca").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[path = "incoming_tests/knot_command_view.rs"]
mod knot_command_view_tests;

#[test]
fn the_debug_crosshair_gives_way_to_the_radial_menu() {
    assert!(!debug_crosshair_drawn(true, true, false, false, true));
}

/// Wind charges: where they burst, and what their hit deals.
mod wind_charge_burst_tests {
    use super::*;
    use crate::game::wind::above_the_top;
    use horizon_client::{
        chunk::{Chunk, LevelShape},
        mobs::Who,
        projectiles::{Hit, Projectile, ProjectileKind},
    };
    use horizon_core::position::ChunkPos;
    use std::sync::Arc;

    fn with_game(test: impl FnOnce(&mut Game<'_>)) {
        let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.game_dir =
            Some(std::env::temp_dir().join(format!("horizon-wind-charge-{}", std::process::id())));
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
        game.creative = true;
        assert!(game.run_command("forceload add 0 0"));
        assert!(game.run_command("tick freeze"));
        assert!(game.run_command("fill 0 198 0 15 199 15 stone"));
        test(&mut game);
    }

    /// `AbstractWindCharge.tick`: `getBlockY() > level.getMaxY() + 30`, i.e. block
    /// Y 350 in the Overworld and 286 in the Nether and the End.
    #[test]
    fn a_charge_bursts_thirty_blocks_above_its_dimensions_top() {
        for (shape, top) in [
            (LevelShape::OVERWORLD, 350.0),
            (LevelShape::NETHER, 286.0),
            (LevelShape::END, 286.0),
        ] {
            assert!(!above_the_top(shape, top - 1.0), "{shape:?} just below");
            assert!(!above_the_top(shape, top - 0.5), "{shape:?} below");
            assert!(above_the_top(shape, top), "{shape:?} at the top");
            assert!(above_the_top(shape, top + 0.5), "{shape:?} above");
        }
    }

    /// The same at play: a charge 44 blocks above the Nether's top, but below the
    /// Overworld's, bursts where it is and is spent.
    #[test]
    #[ignore = "daily"]
    fn a_nether_charge_bursts_above_the_nethers_top() {
        with_game(|game| {
            game.world.shape = LevelShape::NETHER;
            game.audio.capture = Some(Vec::new());
            let at = Vec3::new(2.5, 300.5, 2.5);
            game.add_projectile(Projectile::new(
                ProjectileKind::WindCharge,
                at,
                Some(Who::Player),
            ));
            game.ticked_entities_through = game.next_entity;
            let id = game.creatures.projectiles.last().unwrap().id;
            game.tick_projectiles().unwrap();
            let bursts: Vec<_> = game
                .audio
                .capture
                .take()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| s.event == "minecraft:entity.wind_charge.wind_burst")
                .collect();
            assert_eq!(bursts.len(), 1, "the charge bursts once");
            assert_eq!(bursts[0].position, [at.x, at.y, at.z]);
            assert!(
                !game.creatures.projectiles.iter().any(|p| p.id == id),
                "the charge is spent"
            );
        });
    }

    /// `AbstractWindCharge.onHitEntity` hurts with `damageSources().windCharge`,
    /// so a mob it kills records `minecraft:wind_charge` (message id `mob`), not
    /// `minecraft:thrown`.
    #[test]
    #[ignore = "daily"]
    fn a_wind_charge_kill_of_a_mob_deals_the_wind_charge_damage_type() {
        with_game(|game| {
            assert!(game.run_command(
                "summon pig 2.5 200.5 2.5 {UUID:[I;116,0,0,97],NoAI:1b,NoGravity:1b,Health:1f}"
            ));
            let id = game.mobs.last().expect("the pig").id;
            let at = Vec3::new(2.5, 200.5, 2.5);
            let mut p = Projectile::new(ProjectileKind::WindCharge, at, Some(Who::Player));
            p.previous = at;
            game.wind_charge_hit(
                &p,
                Hit::Entity {
                    who: Who::Mob(id),
                    at,
                },
            )
            .unwrap();
            let pig = game.mobs.iter().find(|m| m.id == id).expect("the pig");
            assert!(!pig.alive(), "a point of damage kills 1 health");
            let hit = pig.combat.entries.last().expect("the hit").source;
            assert_eq!(
                crate::game::creatures::source_damage_type(hit),
                "minecraft:wind_charge"
            );
            let def = game
                .health_resources
                .damage
                .id("minecraft:wind_charge")
                .and_then(|id| game.health_resources.damage.definition(id))
                .expect("the wind charge damage type");
            assert_eq!(def.message_id, "mob", "death.attack.mob");
        });
    }
}
