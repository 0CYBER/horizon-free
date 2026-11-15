//! The server's tags on this client (`ClientPacketListener.handleUpdateTags`, and configuration's
//! `RegistryDataCollector`): what native mods read of them, and their notice. The client's own
//! rules keep playing with its tag files.
use super::Game;
use horizon_client::items::{Items, TagRegistry};
use horizon_protocol::{item::Names, packets::configuration::UpdateTags, registry::SyncedNames};
use std::sync::Arc;

/// The registries whose tags the tag helpers read.
const READ_REGISTRIES: [(&str, TagRegistry); 3] = [
    ("minecraft:block", TagRegistry::Block),
    ("minecraft:item", TagRegistry::Item),
    ("minecraft:fluid", TagRegistry::Fluid),
];

impl Game<'_> {
    /// `ClientboundUpdateTagsPacket` in play.
    pub(super) fn remote_update_tags(&mut self, tags: &UpdateTags) {
        if let Some(r) = self.remote.as_mut() {
            Arc::make_mut(&mut r.registries).add_tags(tags);
        }
        if let Some(r) = self.remote.as_ref() {
            self.server_items = self.server_items(&r.registries);
        }
        self.invalidate_tagged_shader_ids();
        self.refresh_infiniburn();
        self.install_remote_tags();
    }

    /// Make the server's tags what native mods read, then tell them. Snapshot client
    /// enchant answers only after the tags, rebound parts and tag listeners are published.
    pub(super) fn install_remote_tags(&mut self) {
        if mod_api::__private::mods().is_empty() {
            self.resolve_enchant_particles();
            return;
        }
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        if r.registries.tags.is_empty() {
            r.tags = None;
            return;
        }
        let names = SyncedNames {
            builtin: &r.builtin,
            dynamic: &r.registries,
        };
        let mut items = Items::empty();
        for (key, registry) in READ_REGISTRIES {
            let Some(tags) = r.registries.tags.get(key) else {
                continue;
            };
            let members = |ids: &[i32]| ids.iter().filter_map(|&id| names.name(key, id)).collect();
            let tags = tags.iter().map(|(tag, ids)| (tag.clone(), members(ids)));
            items.set_tags(registry, tags.collect());
        }
        super::class_tags::install(&mut items, self.res.facts());
        // Rebuild scoped parts against the received tags before refreshing state-only facts.
        let mut part_items = self.items().clone();
        part_items.block_tags = items.block_tags.clone();
        part_items.item_tags = items.item_tags.clone();
        let parts = match self.prepare_parts(&part_items) {
            Ok(parts) => parts,
            Err(error) => {
                log::warn!("Horizon: client tag parts refused: {error}");
                return;
            }
        };
        self.install_parts(parts);
        if let Some(r) = self.remote.as_mut() {
            r.tags = Some(items);
        }
        self.announce_data_bound(mod_api::TagsUpdateCause::ClientPacketReceived {
            integrated_server: self.integrated.is_some(),
        });
        self.resolve_enchant_particles();
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_client_game;
    use horizon_protocol::packets::{
        configuration::{RegistryTags, TagEntry, UpdateTags},
        play::{self, Clientbound as C},
    };
    use horizon_protocol::registry::Builtin;
    use mod_api::{Hooks, HostSide, Identifier, ModEntry, TagsUpdateCause, TagsUpdatedEvent};
    use mod_platform::__private::part_tests::block_key;
    use std::sync::{Arc, Mutex};

    /// What a listener read of the tags during its notice.
    #[derive(Debug, PartialEq)]
    struct Seen {
        cause: TagsUpdateCause,
        items: Option<Vec<String>>,
        fluids: Option<usize>,
        stone_in_block_tag: Option<bool>,
        dirt_in_block_tag: Option<bool>,
        stick_in_item_tag: Option<bool>,
        lava_in_fluid_tag: Option<bool>,
    }

    struct Probe(Arc<Mutex<Vec<Seen>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn tags_updated(&self, host: HostSide<'_>, event: &mut TagsUpdatedEvent) {
            let HostSide::Client(host) = host else {
                panic!("the server's tags are a client's to apply");
            };
            let host: &dyn mod_api::Host = &*host;
            let tags = host.tags();
            let key = |tag| tags.key(&Identifier::parse(tag).unwrap()).unwrap();
            let stick = host.item_key(&Identifier::parse("minecraft:stick").unwrap());
            let items = tags.items(key("test:items"));
            self.0.lock().unwrap().push(Seen {
                cause: event.cause,
                items: items.map(|keys| keys.into_iter().map(|key| key.id(host)).collect()),
                fluids: tags.fluids(key("test:fluids")).map(|keys| keys.len()),
                stone_in_block_tag: host.block_in_tag(block_key("minecraft:stone"), "test:blocks"),
                dirt_in_block_tag: host.block_in_tag(block_key("minecraft:dirt"), "test:blocks"),
                stick_in_item_tag: tags.is_in_tag(stick.unwrap(), key("test:items")),
                lava_in_fluid_tag: host.fluid_in_tag("minecraft:lava", "test:fluids"),
            });
        }
    }

    fn update(builtin: &Builtin, tags: &[(&str, &str, &[&str])]) -> UpdateTags {
        let id = |registry: &str, name: &str| builtin.get(registry).unwrap().id(name).unwrap();
        UpdateTags {
            registries: tags
                .iter()
                .map(|(registry, tag, members)| RegistryTags {
                    registry: (*registry).into(),
                    tags: vec![TagEntry {
                        name: (*tag).into(),
                        ids: members.iter().map(|name| id(registry, name)).collect(),
                    }],
                })
                .collect(),
        }
    }

    fn login() -> C {
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

    fn names(names: &[&str]) -> Option<Vec<String>> {
        Some(names.iter().map(|name| (*name).to_owned()).collect())
    }

    #[test]
    #[ignore = "daily"]
    fn a_client_notice_reads_each_applied_set_of_server_tags() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("tags_probe", Box::new(Probe(seen.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "Steve").unwrap();
                let builtin = game.remote.as_ref().unwrap().builtin.clone();

                // Configuration's tags wait for the login, which binds the client's blocks.
                let registries = &mut game.remote.as_mut().unwrap().registries;
                Arc::make_mut(registries).add_tags(&update(
                    &builtin,
                    &[
                        (
                            "minecraft:block",
                            "test:blocks",
                            &["minecraft:stone", "minecraft:dirt"],
                        ),
                        ("minecraft:item", "test:items", &["minecraft:stick"]),
                        ("minecraft:fluid", "test:fluids", &["minecraft:lava"]),
                    ],
                ));
                assert!(seen.lock().unwrap().is_empty());
                assert!(game.native_tag_items().is_none());

                game.handle_packet(login()).unwrap();
                let received = TagsUpdateCause::ClientPacketReceived {
                    integrated_server: false,
                };
                assert_eq!(
                    *seen.lock().unwrap(),
                    [Seen {
                        cause: received,
                        items: names(&["minecraft:stick"]),
                        fluids: Some(1),
                        stone_in_block_tag: Some(true),
                        dirt_in_block_tag: Some(true),
                        stick_in_item_tag: Some(true),
                        lava_in_fluid_tag: Some(true),
                    }]
                );

                // An update in play replaces the registries it names and keeps the others.
                game.handle_packet(C::UpdateTags(update(
                    &builtin,
                    &[
                        ("minecraft:block", "test:blocks", &["minecraft:dirt"]),
                        ("minecraft:item", "test:other", &["minecraft:stick"]),
                    ],
                )))
                .unwrap();
                let seen = seen.lock().unwrap();
                assert_eq!(seen.len(), 2);
                assert_eq!(
                    seen[1],
                    Seen {
                        cause: received,
                        items: Some(Vec::new()),
                        fluids: Some(1),
                        stone_in_block_tag: Some(false),
                        dirt_in_block_tag: Some(true),
                        stick_in_item_tag: Some(false),
                        lava_in_fluid_tag: Some(true),
                    }
                );
            });
        });
    }

    fn power_update(builtin: &Builtin, tags: &[(&str, &[&str])]) -> UpdateTags {
        let blocks = builtin.get("minecraft:block").unwrap();
        UpdateTags {
            registries: vec![RegistryTags {
                registry: "minecraft:block".into(),
                tags: tags
                    .iter()
                    .map(|(name, members)| TagEntry {
                        name: (*name).into(),
                        ids: members.iter().map(|id| blocks.id(id).unwrap()).collect(),
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    #[ignore = "daily"]
    fn remote_tag_packets_rebuild_vanilla_enchant_providers_and_transmitters_without_mods() {
        use crate::game::enchant_power_beam_tests::{enchant_particles, place};
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_client_game(|game| {
                let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "PowerTags").unwrap();
                game.handle_packet(login()).unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                place(game, [0, 0, 0], "minecraft:enchanting_table");
                place(game, [-2, 0, -1], "minecraft:bookshelf");
                let builtin = game.remote.as_ref().unwrap().builtin.clone();
                let mut send = |providers: &[&str], transmitters: &[&str]| {
                    game.handle_packet(C::UpdateTags(power_update(
                        &builtin,
                        &[
                            ("minecraft:enchantment_power_provider", providers),
                            ("minecraft:enchantment_power_transmitter", transmitters),
                        ],
                    )))
                    .unwrap();
                    enchant_particles(game)
                };
                assert!(send(&["minecraft:bookshelf"], &["minecraft:air"]) > 0);
                assert_eq!(send(&[], &["minecraft:air"]), 0, "provider tag removed");
                assert!(send(&["minecraft:bookshelf"], &["minecraft:air"]) > 0);
                assert_eq!(
                    send(&["minecraft:bookshelf"], &[]),
                    0,
                    "transmitter tag removed"
                );
                assert!(send(&["minecraft:bookshelf"], &["minecraft:air"]) > 0);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn remote_enchant_snapshot_follows_tag_rebinding_and_tags_updated_callbacks() {
        use crate::game::enchant_power_beam_tests::{enchant_particles, place};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        static ENABLED: AtomicBool = AtomicBool::new(false);
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        static NOTICES: AtomicUsize = AtomicUsize::new(0);
        struct Provider;
        #[mod_api::horizon_part]
        impl mod_api::BlockPart for Provider {
            fn enchant_power_bonus(
                &self,
                host: &dyn mod_api::Host,
                event: &mut mod_api::EnchantPowerBonusEvent,
            ) {
                CALLS.fetch_add(1, Ordering::Relaxed);
                let received =
                    host.block_in_tag(event.state.block(), "power_test:providers") == Some(true);
                event.power = if received && ENABLED.load(Ordering::Relaxed) {
                    2.0
                } else {
                    0.0
                };
            }
        }
        struct PowerMod;
        #[mod_api::__subscriptions]
        impl Hooks for PowerMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.block_key("minecraft:glass");
                r.block_part_tag("power_test:providers", Provider);
            }
            fn tags_updated(&self, host: HostSide<'_>, _: &mut TagsUpdatedEvent) {
                let HostSide::Client(host) = host else { return };
                ENABLED.store(
                    host.block_in_tag(block_key("minecraft:glass"), "power_test:enabled")
                        == Some(true),
                    Ordering::Relaxed,
                );
                NOTICES.fetch_add(1, Ordering::Relaxed);
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("power_test", Box::new(PowerMod))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::entry::install_client_hooks();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "PowerCallbacks").unwrap();
                game.handle_packet(login()).unwrap();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                place(game, [0, 0, 0], "minecraft:enchanting_table");
                place(game, [-2, 0, -1], "minecraft:glass");
                let builtin = game.remote.as_ref().unwrap().builtin.clone();
                let mut send = |attached: bool, enabled: bool| {
                    let calls = CALLS.load(Ordering::Relaxed);
                    let notices = NOTICES.load(Ordering::Relaxed);
                    game.handle_packet(C::UpdateTags(power_update(
                        &builtin,
                        &[
                            ("minecraft:enchantment_power_provider", &[]),
                            (
                                "minecraft:enchantment_power_transmitter",
                                &["minecraft:air"],
                            ),
                            (
                                "power_test:providers",
                                if attached { &["minecraft:glass"] } else { &[] },
                            ),
                            (
                                "power_test:enabled",
                                if enabled { &["minecraft:glass"] } else { &[] },
                            ),
                        ],
                    )))
                    .unwrap();
                    assert_eq!(NOTICES.load(Ordering::Relaxed), notices + 1);
                    assert_eq!(
                        CALLS.load(Ordering::Relaxed),
                        calls + usize::from(attached),
                        "one snapshot after publication, never before the callback"
                    );
                    let particles = enchant_particles(game);
                    assert_eq!(
                        CALLS.load(Ordering::Relaxed),
                        calls + usize::from(attached),
                        "particle frames must not query parts"
                    );
                    particles
                };
                assert!(
                    send(true, true) > 0,
                    "new tags visible to both callback and part"
                );
                assert_eq!(
                    send(true, false),
                    0,
                    "callback answer changed before snapshot"
                );
                assert!(send(true, true) > 0);
                assert_eq!(
                    send(false, true),
                    0,
                    "tag-bound part detached before snapshot"
                );
            });
        });
    }
}
