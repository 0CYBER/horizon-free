use super::*;
use horizon_client::recipe_book::{BookFlags, settings_codec, settings_sync::Message};
use std::sync::OnceLock;

static KEY: OnceLock<mod_api::RecipeBookTypeKey> = OnceLock::new();
struct BookMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for BookMod {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar.register_recipe_type("geometry:crafting");
        registrar.register_raw_recipe_serializer("geometry:crafting", "geometry:crafting");
        let key = registrar.recipe_book_type("geometry:book").unwrap();
        assert_eq!(KEY.get_or_init(|| key.clone()), &key);
    }
    fn recipe_book_categories(
        &self,
        _: mod_api::HostSide,
        event: &mut mod_api::RecipeBookCategoriesEvent<'_>,
    ) {
        event.categories.push(mod_api::RecipeBookCategory {
            id: "geometry:initial".into(),
            recipe_type: "geometry:crafting".into(),
            icon: "minecraft:stone".into(),
            target_menu: "geometry:menu".into(),
            book: KEY.get().unwrap().clone(),
            search_group: "geometry:search".into(),
            displays: Vec::new(),
        });
    }
}
fn receive(game: &mut Game<'_>, message: Message) {
    game.remote_book_settings(&settings_codec::encode(&message).unwrap());
}
struct BoundBookMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for BoundBookMod {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let key = registrar.recipe_book_type("bound:book").unwrap();
        registrar
            .menu_recipe_book(
                "bound:menu",
                &key,
                "bound:recipe",
                mod_api::RecipeBookLayout::Crafting {
                    width: 1,
                    height: 1,
                    slots: vec![0],
                    result: 1,
                },
                mod_api::Screen {
                    width: 176.0,
                    height: 166.0,
                    slots: vec![Some([8.0, 18.0]), Some([80.0, 18.0])],
                    recipe_book: Some(key.clone()),
                    recipe_book_toggle: Some([153.0, 60.0]),
                    ..Default::default()
                },
            )
            .unwrap();
    }
}
#[test]
#[ignore = "needs prepared resources and isolated homes"]
fn entering_inventory_requests_catalogue_for_preinstalled_bound_screen() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("bound", Box::new(BoundBookMod))].into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            mod_api::__private::resolve_menus(["bound:menu"]).unwrap();
            let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Bound").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.custom_books = true;
            remote.joined = true;
            remote.menu.set(Some((1, "bound:menu".into())));
            game.screen = Screen::Playing;
            game.container = Some((i32::MIN, 0, 0));
            game.mod_screen = Some(OpenNativeScreen {
                layout_revision: 4,
                key: (i32::MIN, 0, 0),
                window: 1,
                block: "bound:block".into(),
                data: Vec::new(),
                extra: None,
                screen: mod_api::RecipeBookLayout::registered_screen("bound:menu").unwrap(),
            });
            // Negotiation completed before the user opened the machine. As in
            // remote_open_screen, the declaration exists before Inventory is shown.
            use horizon_client::recipe_book::placement_sync;
            game.native_net.placement.client.hello(6);
            assert!(
                game.native_net
                    .placement
                    .client
                    .ready(6, placement_sync::schema())
            );
            game.ensure_mod_book_catalog();
            assert!(game.native_net.placement.client.window().is_none());
            game.set_screen(Screen::Inventory);
            let window = game
                .native_net
                .placement
                .client
                .window()
                .expect("catalogue requested on opening");
            assert_eq!(window.menu.as_str(), "bound:menu");
            assert_eq!(window.nonce, 4);
            assert!(game.native_net.placement.deadline.is_some());
        });
    });
}
fn geometry(game: &Game<'_>) -> ((f32, f32), Vec<(usize, [f32; 4])>) {
    let batch = horizon_client::hud::HudBatch::new([1280, 720], game.settings.gui_scale_inputs());
    let layout = game.built_container(&batch).expect("cached container");
    (layout.origin, layout.slots.clone())
}
#[test]
#[ignore = "needs prepared resources and isolated homes"]
fn authoritative_custom_book_rebuilds_cached_geometry_without_observe_screen() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("geometry", Box::new(BookMod))].into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let key = KEY.get().unwrap().clone();
            // The lightweight Game fixture does not run the launcher/content
            // bootstrap that publishes the negotiated menu registry.
            mod_api::__private::resolve_menus(["geometry:menu"]).unwrap();
            let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Geometry").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.joined = true;
            remote.menu.set(Some((1, "geometry:menu".into())));
            // A registered book alone must not authorize the private protocol.
            receive(game, Message::Ready);
            assert!(!game.native_net.book_ready);
            game.remote.as_mut().unwrap().custom_books = true;
            game.settings.gui_scale = 1;
            // The fixture has no renderer; layout still needs the same viewport
            // used by geometry() to evaluate toggle gutters and book placement.
            game.recipe_ui.test_viewport = Some([1280, 720]);
            game.screen = Screen::Inventory;
            game.title = None;
            game.container = Some((i32::MIN, 0, 0));
            game.mod_screen = Some(OpenNativeScreen {
                layout_revision: 0,
                key: (i32::MIN, 0, 0),
                window: 1,
                block: "minecraft:stone".into(),
                data: Vec::new(),
                extra: None,
                screen: mod_api::Screen {
                    width: 176.0,
                    height: 166.0,
                    slots: vec![Some([8.0, 18.0])],
                    inventory_y: 84.0,
                    inventory_x: 8.0,
                    own_labels: false,
                    own_background: false,
                    recipe_book: Some(key.clone()),
                    recipe_book_toggle: None,
                },
            });
            // A snapshot may precede the first draw. The build itself must
            // collect declarations, not wait for the render refresh path.
            let recipes = horizon_client::crafting::Recipes::from_sources(
                &game.options.resources,
                game.items(),
                &horizon_world::loot::PackOverlay::NONE,
                vec![(
                    "geometry:recipe".into(),
                    serde_json::json!({"type": "geometry:crafting"}),
                )],
                game.res.core.item_data.context(),
            )
            .unwrap();
            game.recipe_tables.install_test_recipes(recipes);
            game.recipe_tables.books = None;
            receive(game, Message::Ready);
            receive(
                game,
                Message::Snapshot {
                    epoch: 6,
                    revision: 0,
                    entries: vec![(
                        key.as_str().into(),
                        BookFlags {
                            open: true,
                            filtering: false,
                        },
                    )],
                },
            );
            game.build_screen();
            if let Err(error) = game
                .recipe_tables
                .books
                .as_ref()
                .expect("build collected declarations")
            {
                panic!("declaration refused: {error:?}");
            }
            assert_eq!(geometry(game).0, (629.0, 277.0));
            game.reset_book_view();
            receive(game, Message::Ready);
            receive(
                game,
                Message::Snapshot {
                    epoch: 7,
                    revision: 0,
                    entries: vec![(
                        key.as_str().into(),
                        BookFlags {
                            open: false,
                            filtering: false,
                        },
                    )],
                },
            );
            game.build_screen();
            let closed = geometry(game);
            assert_eq!(closed.0, (552.0, 277.0));
            assert!(closed.1.contains(&(300, [559.0, 294.0, 18.0, 18.0])));
            let saved = game.level.recipe_book.custom.clone();
            assert!(game.request_custom_book_settings(
                &key,
                mod_api::RecipeBookSettings {
                    open: true,
                    filtering: true
                }
            ));
            assert_eq!(geometry(game), closed);
            assert_eq!(game.level.recipe_book.custom, saved);
            receive(
                game,
                Message::Update {
                    epoch: 7,
                    revision: 1,
                    key: key.as_str().into(),
                    flags: BookFlags {
                        open: true,
                        filtering: true,
                    },
                },
            );
            let opened = geometry(game);
            assert_eq!(opened.0, (629.0, 277.0));
            assert!(opened.1.contains(&(300, [636.0, 294.0, 18.0, 18.0])));
            assert_ne!(opened.1, closed.1);
            assert_eq!(opened.1.len(), closed.1.len());
            let shift = opened.0.0 - closed.0.0;
            assert_eq!(opened.0.1, closed.0.1);
            for ((id, before), (after_id, after)) in closed.1.iter().zip(&opened.1) {
                assert_eq!(id, after_id);
                assert_eq!(*after, [before[0] + shift, before[1], before[2], before[3]]);
            }
            receive(
                game,
                Message::Update {
                    epoch: 7,
                    revision: 2,
                    key: key.as_str().into(),
                    flags: BookFlags {
                        open: false,
                        filtering: false,
                    },
                },
            );
            assert_eq!(geometry(game), closed);
            receive(
                game,
                Message::Update {
                    epoch: 7,
                    revision: 3,
                    key: key.as_str().into(),
                    flags: BookFlags {
                        open: true,
                        filtering: false,
                    },
                },
            );
            assert_eq!(geometry(game), opened);
            receive(game, Message::Busy);
            assert_eq!(geometry(game), closed);
            receive(
                game,
                Message::Snapshot {
                    epoch: 7,
                    revision: 4,
                    entries: vec![(
                        key.as_str().into(),
                        BookFlags {
                            open: true,
                            filtering: false,
                        },
                    )],
                },
            );
            assert_eq!(geometry(game), opened);
            game.reset_book_view();
            assert_eq!(geometry(game), closed);
        });
    });
}
