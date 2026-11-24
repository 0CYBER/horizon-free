//! A client mod's screen control (`ClientHost::close_screen` and
//! `ClientHost::open_inventory`) and the sign it edits
//! (`ClientHost::sign_editor`): the same paths Escape and the inventory key
//! take, so the packets and follow-ups are vanilla's.
use super::Game;
use crate::game::Screen;
use mod_api::{BlockPos, SignEditView};
use winit::keyboard::KeyCode;

impl Game<'_> {
    /// `Minecraft.setScreen` through Escape for a container screen, and the
    /// screen's own close for any other: Escape closes a container with the
    /// menu's `ServerboundContainerClosePacket` first (a bare local close would
    /// leave the server's menu open), while a pause menu closes to the game.
    /// The sign editor is an overlay beside the screen, and Escape on it sends
    /// its `ServerboundSignUpdatePacket` like any other close. False when no
    /// screen is open, and for the death screen, which Escape does not close.
    pub(in crate::game) fn close_screen(&mut self) -> bool {
        if self.player.is_none() {
            return false;
        }
        if self.sign_edit.is_some() {
            self.close_sign_editor();
            return true;
        }
        if self.screen == Screen::Playing {
            return false;
        }
        let before = self.screen;
        // The Escape path itself, so every screen's own close and follow-ups
        // (and the screen hooks' closing notices) are exactly the key's.
        let _ = self.key_pressed(KeyCode::Escape, KeyCode::Escape);
        self.screen != before
    }
    /// `Minecraft.handleKeybinds` for `key.inventory`: the player's inventory
    /// opens locally and nothing is sent (the player menu is always open on the
    /// server), or, riding a vehicle with its own inventory, the key queues the
    /// open-inventory player command instead (`ServerboundPlayerCommandPacket`,
    /// sent at the player's next tick as the key's press is). False with a
    /// screen already open, for a spectator, in a portal transition, or with no
    /// world.
    pub(in crate::game) fn open_inventory(&mut self) -> bool {
        if self.player.is_none()
            || self.screen != Screen::Playing
            || self.sign_edit.is_some()
            || self.spectator()
            || self.in_portal_transition()
        {
            return false;
        }
        let vehicle = self.riding_mount_with_menu() || self.riding_chest_boat();
        let _ = self.key_pressed(KeyCode::KeyE, KeyCode::KeyE);
        vehicle || self.screen != Screen::Playing
    }
    /// `AbstractSignEditScreen`'s sign: its block position and the side the
    /// editor is on. None when no sign editor is open, or outside a world.
    pub(in crate::game) fn sign_editor(&self) -> Option<SignEditView> {
        self.player.as_ref()?;
        let edit = self.sign_edit.as_ref()?;
        Some(SignEditView {
            pos: BlockPos::new(edit.pos.0, edit.pos.1, edit.pos.2),
            front: edit.front,
        })
    }
}

#[cfg(test)]
mod client_screen_tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_client_game;
    use crate::game::remote::tests::connect_menu_test;
    use horizon_client::{
        containers::BlockEntity,
        mobs::Mob,
        signs::{SignData, SignText},
    };
    use horizon_core::geometry::Vec3;
    use horizon_protocol::connection::ConnReader;
    use horizon_protocol::packets::play::{self, Clientbound as C, Serverbound as S};

    /// What the window sent since the last call, as play packets.
    fn sent(reader: &mut ConnReader) -> Vec<S> {
        reader
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let mut packets = Vec::new();
        while let Ok((id, body)) = reader.recv() {
            packets.extend(S::decode(id, &body).ok());
        }
        packets
    }

    /// A window logged in to the overworld at (0.5, 65, 0.5), its player made
    /// by the server's position packet, with everything the login sent drained.
    fn logged_in(game: &mut Game<'_>) -> ConnReader {
        let (mut reader, _writer) = connect_menu_test(game);
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 1,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: false,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [0.5, 65.0, 0.5],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: 0,
        }))
        .unwrap();
        sent(&mut reader);
        reader
    }

    /// A menu's network id, from the prepared registries report.
    fn menu_type(name: &str) -> i32 {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES")
                .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
        );
        let report = horizon_protocol::registry::Builtin::find(&root)
            .expect("the prepared resources carry registries.json");
        let builtin = horizon_protocol::registry::Builtin::load(&report).unwrap();
        builtin.get("minecraft:menu").unwrap().id(name).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn closing_a_chest_sends_its_menu_id_and_closes_the_client_menu() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            let menu = menu_type("minecraft:generic_9x3");
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 7,
                menu,
                title: horizon_core::nbt::Tag::String("Chest".into()),
            }))
            .unwrap();
            assert_eq!(game.screen, Screen::Inventory);
            assert!(game.screen_title().is_some(), "the server's menu is open");
            sent(&mut reader);

            assert!(game.close_screen(), "a chest screen is open");
            assert_eq!(game.screen, Screen::Playing);
            assert!(game.screen_title().is_none(), "the client menu is closed");
            let Some(S::ContainerClose(close)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::ContainerClose(_)))
            else {
                panic!("the container close with the menu's id goes out");
            };
            assert_eq!(close.container, 7);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_tooltip_reads_the_live_topmost_screen() {
        use horizon_core::nbt::Tag;
        use mod_api::{ClientHost, Hooks, ItemTooltipEvent, ModEntry};
        use std::sync::{Arc, Mutex};

        type Seen = Arc<Mutex<Vec<Option<mod_api::ScreenIdentity>>>>;
        struct Probe(Seen);
        #[mod_api::__subscriptions]
        impl Hooks for Probe {
            fn item_tooltip(&self, host: &dyn ClientHost, _: &mut ItemTooltipEvent<'_>) {
                self.0.lock().unwrap().push(host.current_screen());
            }
        }

        let seen = Seen::default();
        let mods = vec![ModEntry::new("screen_probe", Box::new(Probe(seen.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let _reader = logged_in(game);
                game.remote.as_mut().unwrap().level_load.finish();
                assert!(
                    game.renderer.is_none(),
                    "identity does not need a render view"
                );
                let stack = horizon_client::items::ItemStack::new("minecraft:enchanted_book", 1);
                let sample = |game: &Game<'_>| {
                    drop(game.stack_tooltip(&stack));
                };
                let open_menu = |game: &mut Game<'_>, menu: &str| {
                    game.handle_packet(C::OpenScreen(play::OpenScreen {
                        container: 7,
                        menu: menu_type(menu),
                        // Deliberately identical: a title-based gate would not
                        // distinguish the chest and enchanting table.
                        title: Tag::String("Same title".into()),
                    }))
                    .unwrap();
                };

                sample(game);
                assert!(game.open_inventory());
                sample(game);
                assert!(game.close_screen());
                open_menu(game, "minecraft:generic_9x3");
                sample(game);
                assert!(game.close_screen());
                open_menu(game, "minecraft:enchantment");
                sample(game);
                game.handle_packet(C::ShowDialog(play::ShowDialog {
                    dialog: play::DialogHolder::Direct(Tag::compound([
                        ("type", Tag::String("minecraft:notice".into())),
                        ("title", Tag::String("Notice".into())),
                    ])),
                }))
                .unwrap();
                sample(game);
                game.link_prompt = Some("https://example.com".into());
                sample(game);
                game.link_prompt = None;
                sample(game);
                game.handle_packet(C::ClearDialog(horizon_protocol::packets::Empty))
                    .unwrap();
                sample(game);
                assert!(game.close_screen());
                sample(game);
            });
        });
        let seen = seen.lock().unwrap();
        let seen: Vec<_> = seen
            .iter()
            .map(|screen| {
                screen
                    .as_ref()
                    .map(|screen| (screen.id.as_str(), screen.menu.as_deref()))
            })
            .collect();
        assert_eq!(
            seen,
            [
                None,
                Some(("inventory", None)),
                Some(("inventory", Some("minecraft:generic_9x3"))),
                Some(("inventory", Some("minecraft:enchantment"))),
                Some(("dialog", None)),
                Some(("link_prompt", None)),
                Some(("dialog", None)),
                Some(("inventory", Some("minecraft:enchantment"))),
                None,
            ]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn closing_the_inventory_sends_the_close_for_id_zero() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.set_screen(Screen::Inventory);
            assert!(game.close_screen());
            assert_eq!(game.screen, Screen::Playing);
            let Some(S::ContainerClose(close)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::ContainerClose(_)))
            else {
                panic!("the player container close goes out");
            };
            assert_eq!(close.container, 0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn closing_with_no_screen_reports_false_and_sends_nothing() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert_eq!(game.screen, Screen::Playing);
            assert!(!game.close_screen());
            assert!(sent(&mut reader).is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn opening_the_inventory_is_local_and_needs_no_screen() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert!(game.open_inventory());
            assert_eq!(game.screen, Screen::Inventory);
            assert!(
                sent(&mut reader).is_empty(),
                "the inventory opens without telling the server"
            );
            assert!(!game.open_inventory(), "a screen is already open");
            assert_eq!(game.screen, Screen::Inventory);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn opening_the_inventory_riding_a_horse_sends_the_player_command() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            // The player's keys act once the client told the server it loaded.
            game.streaming
                .insert_remote(&mut game.world, horizon_client::chunk::Chunk::empty(0, 0));
            game.poll_stationary_client().unwrap();
            sent(&mut reader); // PlayerLoaded precedes the inventory action.
            let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            let mut horse = Mob::new(42, kind, Vec3::new(0.5, 65.0, 0.5), 0.0);
            horse.extra.tame = true;
            game.mobs.push(horse);
            game.riding = Some(42);
            // An overlay counts as an open screen even when a mount would otherwise
            // make open_inventory report that its command was queued.
            let at = (3, 64, -2);
            game.world
                .block_entities
                .insert(at, BlockEntity::Sign(SignData::default()));
            game.remote_open_sign_editor(play::OpenSignEditor {
                pos: [3, 64, -2],
                front: true,
            });
            assert!(!game.open_inventory(), "the sign editor is already open");
            assert!(game.sign_editor().is_some());
            assert_eq!(game.screen, Screen::Playing);
            assert!(
                sent(&mut reader).is_empty(),
                "no vehicle menu command was sent"
            );
            assert!(game.close_screen());
            sent(&mut reader);
            assert!(game.open_inventory());
            // The key's press is replayed at the player's next tick, so the
            // command goes out there, as the key's does.
            game.tick_remote_level().unwrap();
            let Some(S::PlayerCommand(command)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::PlayerCommand(_)))
            else {
                panic!("the open-inventory command goes out");
            };
            assert_eq!((command.id, command.action), (7, 5));
            assert_eq!(command.data, 0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_sign_editor_reports_its_sign_and_closing_sends_the_update() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            let at = (3, 64, -2);
            let sign = SignData {
                front: SignText {
                    lines: [
                        "first".into(),
                        "second".into(),
                        "third".into(),
                        "fourth".into(),
                    ],
                    ..SignText::default()
                },
                ..SignData::default()
            };
            game.world
                .block_entities
                .insert(at, BlockEntity::Sign(sign));
            game.remote_open_sign_editor(play::OpenSignEditor {
                pos: [at.0, at.1, at.2],
                front: true,
            });
            assert_eq!(
                game.sign_editor(),
                Some(SignEditView {
                    pos: BlockPos::new(3, 64, -2),
                    front: true,
                })
            );

            assert!(game.close_screen());
            assert_eq!(game.sign_editor(), None);
            let Some(S::SignUpdate(update)) = sent(&mut reader)
                .into_iter()
                .find(|p| matches!(p, S::SignUpdate(_)))
            else {
                panic!("the sign update goes out with the close");
            };
            assert_eq!(update.pos, [3, 64, -2]);
            assert!(update.front);
            assert_eq!(
                update.lines,
                [
                    "first".to_owned(),
                    "second".to_owned(),
                    "third".to_owned(),
                    "fourth".to_owned()
                ]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn no_sign_editor_reports_no_sign() {
        with_client_game(|game| {
            logged_in(game);
            assert_eq!(game.sign_editor(), None);
        });
    }
}
