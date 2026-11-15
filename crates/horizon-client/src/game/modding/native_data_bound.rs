//! The notices a side's freshly bound data gives mods, in NeoForge's order.
use super::Game;

impl Game<'_> {
    /// Items' default components are bound (`DefaultDataComponentsBoundEvent`), then the
    /// tags (`TagsUpdatedEvent`), both for `cause`.
    pub(in crate::game) fn announce_data_bound(&mut self, cause: mod_api::TagsUpdateCause) {
        self.native_default_components_bound(cause);
        self.native_tags_updated(cause);
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_protocol::packets::{
        configuration::{RegistryTags, TagEntry, UpdateTags},
        play::{self, Clientbound as C},
    };
    use mod_api::{
        DataLoadContext, DefaultComponentsBoundEvent, Hooks, Host, HostSide, Identifier, ModEntry,
        ModifyDefaultComponentsEvent, TagsUpdateCause, TagsUpdatedEvent,
    };
    use std::sync::{Arc, Mutex};

    struct Probe(Arc<Mutex<Vec<(&'static str, TagsUpdateCause)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn default_components_bound(
            &self,
            _: HostSide<'_>,
            event: &mut DefaultComponentsBoundEvent,
        ) {
            self.0.lock().unwrap().push(("bound", event.cause));
        }
        fn tags_updated(&self, _: HostSide<'_>, event: &mut TagsUpdatedEvent) {
            self.0.lock().unwrap().push(("tags", event.cause));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn defaults_are_announced_bound_before_the_tags() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("bound_probe", Box::new(Probe(heard.clone())))].into();
        let reload = TagsUpdateCause::ServerDataLoad { reload: true };
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                heard.lock().unwrap().clear();
                game.announce_data_bound(reload);
            });
        });
        assert_eq!(
            *heard.lock().unwrap(),
            [("bound", reload), ("tags", reload)]
        );
    }

    /// The bound notice's default read: the value the mod's
    /// `modify_default_components` set, off the published defaults.
    fn dirt_stack_size(host: &dyn Host) -> Option<mod_api::Tag> {
        let key = host.item_key(&Identifier::parse("minecraft:dirt").unwrap())?;
        host.item_default_component(key, "minecraft:max_stack_size")
    }

    /// The load boundary a test's defaults edit flows through: the same
    /// wiring `entry.rs` installs for the game, first-installed process-wide.
    fn install_default_component_loads() {
        let _ = horizon_world::native_load_hooks::install(
            horizon_world::native_load_hooks::NativeLoadHooks {
                item_ability_default: horizon_items::item_abilities::vanilla_can_perform,
                loot_ability: |_, _, _| {},
                loot_table_load: |_, _, _| {},
                modify_default_components: |context, item, components| {
                    if !mod_api::__private::listened(mod_api::HookId::ModifyDefaultComponents) {
                        return;
                    }
                    let reader = horizon_client_render::api_context::LoadReader(context);
                    let context = reader.context();
                    mod_api::dispatch_modify_default_components(&context, item, components);
                },
                loot_function: |_, _, _, _, _| {},
                loot_condition: |_, _, _, _, _| {},
            },
        );
    }

    /// A mod that raises dirt's default stack size to 99 at its load, and
    /// records, per bound notice, the cause and the stack size it reads.
    struct DefaultsProbe(Arc<Mutex<Vec<(TagsUpdateCause, Option<mod_api::Tag>)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for DefaultsProbe {
        fn modify_default_components(
            &self,
            _: &dyn DataLoadContext,
            event: &mut ModifyDefaultComponentsEvent<'_>,
        ) {
            if event.item == "minecraft:dirt" {
                event
                    .components
                    .insert("minecraft:max_stack_size".into(), serde_json::json!(99));
            }
        }
        fn default_components_bound(
            &self,
            host: HostSide<'_>,
            event: &mut DefaultComponentsBoundEvent,
        ) {
            let stack = match host {
                HostSide::Server(host) => dirt_stack_size(host),
                HostSide::Client(host) => dirt_stack_size(host),
            };
            self.0.lock().unwrap().push((event.cause, stack));
        }
    }

    /// A single player's first open: the data load announces the defaults
    /// bound before the tags, `should_update_static_data()` holds for its
    /// cause, and a listener's read inside the notice returns the stack
    /// size its own `modify_default_components` published.
    #[test]
    #[ignore = "daily"]
    fn the_first_open_announces_bound_defaults_readable() {
        install_default_component_loads();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "defaults_probe",
            Box::new(DefaultsProbe(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let scratch = tempfile::tempdir().unwrap();
                let registry = Arc::new(horizon_client::gamerules::RuleRegistry::vanilla());
                let create = horizon_client::title::WorldOptions::default();
                let mut request = crate::game::world_loading::WorldRequest::new(
                    "defaults",
                    Some("defaults"),
                    Some(1),
                    true,
                    Some(&create),
                    registry.clone(),
                )
                .unwrap();
                request.save_root = scratch.path().to_owned();
                let loader = crate::game::world_loading::WorldLoader::start(
                    request,
                    game.res.clone(),
                    game.pack.clone(),
                    game.options.clone(),
                    game.settings.clone(),
                    Arc::new(horizon_world::datafix::progress::UpgradeProgress::new()),
                )
                .unwrap();
                let prepared = game.prepare_early_game_rules(loader).unwrap();
                game.finish_open_world(prepared).unwrap();
            });
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                game.announce_data_bound(TagsUpdateCause::ClientPacketReceived {
                    integrated_server: true,
                });
            });
        });
        let seen = seen.lock().unwrap();
        let server = TagsUpdateCause::ServerDataLoad { reload: false };
        let integrated = TagsUpdateCause::ClientPacketReceived {
            integrated_server: true,
        };
        assert_eq!(
            *seen,
            [
                (server, Some(mod_api::Tag::Int(99))),
                (integrated, Some(mod_api::Tag::Int(99)))
            ]
        );
        assert!(server.should_update_static_data());
        assert!(!integrated.should_update_static_data());
    }

    /// A remote login: the applied tags announce the defaults bound once,
    /// with the client cause, before the tags notice; a listener's read
    /// inside returns the published defaults. An update in play refires.
    #[test]
    #[ignore = "daily"]
    fn a_remote_login_announces_the_client_cause_before_the_tags() {
        install_default_component_loads();
        struct Both(Arc<Mutex<Vec<&'static str>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Both {
            fn modify_default_components(
                &self,
                _: &dyn DataLoadContext,
                event: &mut ModifyDefaultComponentsEvent<'_>,
            ) {
                if event.item == "minecraft:dirt" {
                    event
                        .components
                        .insert("minecraft:max_stack_size".into(), serde_json::json!(99));
                }
            }
            fn default_components_bound(
                &self,
                host: HostSide<'_>,
                event: &mut DefaultComponentsBoundEvent,
            ) {
                assert!(matches!(&host, HostSide::Client(_)));
                let stack = match host {
                    HostSide::Server(host) => dirt_stack_size(host),
                    HostSide::Client(host) => dirt_stack_size(host),
                };
                assert_eq!(
                    stack,
                    Some(mod_api::Tag::Int(99)),
                    "the defaults are published"
                );
                assert_eq!(
                    event.cause,
                    TagsUpdateCause::ClientPacketReceived {
                        integrated_server: false
                    }
                );
                assert!(event.cause.should_update_static_data());
                self.0.lock().unwrap().push("bound");
            }
            fn tags_updated(&self, _: HostSide<'_>, _: &mut TagsUpdatedEvent) {
                self.0.lock().unwrap().push("tags");
            }
        }

        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("login_probe", Box::new(Both(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                // A client's view of a server's tags: the login applies them.
                let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "BoundLogin").unwrap();
                let builtin = game.remote.as_ref().unwrap().builtin.clone();
                let id = |name: &str| builtin.get("minecraft:item").unwrap().id(name).unwrap();
                let registries = UpdateTags {
                    registries: vec![RegistryTags {
                        registry: "minecraft:item".into(),
                        tags: vec![TagEntry {
                            name: "test:items".into(),
                            ids: vec![id("minecraft:stick")],
                        }],
                    }],
                };
                heard.lock().unwrap().clear();
                game.handle_packet(login_packet()).unwrap();
                game.handle_packet(C::UpdateTags(registries)).unwrap();
            });
        });
        assert_eq!(*heard.lock().unwrap(), ["bound", "tags"]);
    }

    /// The login packet the remote-tags tests use.
    fn login_packet() -> C {
        C::Login(play::Login {
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
        })
    }
}
