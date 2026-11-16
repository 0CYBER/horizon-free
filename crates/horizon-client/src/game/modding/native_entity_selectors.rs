//! Mods' selector tokens (`@allies`, NeoForge's `EntitySelectorManager`): the base selections
//! the command registries parse them with, and their types' filter over the candidates.
use super::Game;
use super::commands::Target;
use horizon_client::command::args::Selector;
use horizon_client_core::registries::SelectorToken;
use mod_api::SelectorSort;

/// The frozen base selections validated when the installed mods registered their tokens.
pub(super) fn selector_tokens() -> Vec<SelectorToken> {
    mod_api::__private::entity_selector_types()
        .map(|(name, base)| SelectorToken {
            name: name.to_owned(),
            tooltip: base.tooltip.clone(),
            players_only: base.players_only,
            limit: base.limit.map_or(usize::MAX, |limit| limit as usize),
            sort: match base.sort {
                SelectorSort::Nearest => "nearest",
                SelectorSort::Furthest => "furthest",
                SelectorSort::Random => "random",
                _ => "arbitrary",
            },
        })
        .collect()
}

impl Game<'_> {
    /// `select_in_level`, then, for a mod's token, only the candidates its type keeps: after the
    /// base selection and the `[...]` options, before sorting and the limit. A vanilla selector
    /// costs nothing more.
    pub(super) fn native_selector_kept(
        &mut self,
        s: &Selector,
        candidates: Vec<Target>,
        origin: [f64; 3],
    ) -> Vec<(Target, f64)> {
        let selected = self.select_in_level(candidates, s, origin);
        let Some(token) = &s.token else {
            return selected;
        };
        let Some(selector) = mod_api::__private::entity_selector_type(token) else {
            return Vec::new();
        };
        let Ok(source) = self.native_command_origin(self.ui.registries.permission) else {
            return Vec::new();
        };
        selected
            .into_iter()
            .filter(|(target, _)| {
                let Some(entity) = self.native_command_entity_target(*target) else {
                    return false;
                };
                let host: &mut dyn mod_api::ServerHost = self;
                // A panicking type keeps nothing, as an unanswered predicate.
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    selector.matches(host, &source, entity)
                }))
                .unwrap_or(false)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;
    use std::sync::{Arc, Mutex};

    struct Teammates(Arc<Mutex<Vec<(mod_api::CommandSource, u32)>>>);
    impl mod_api::EntitySelectorType for Teammates {
        fn base(&self) -> mod_api::SelectorBase {
            mod_api::SelectorBase::new(false, Some(1), SelectorSort::Nearest, "test.allies")
        }
        fn matches(
            &self,
            _: &mut dyn mod_api::ServerHost,
            source: &mod_api::CommandSource,
            entity: mod_api::EntityRef,
        ) -> bool {
            self.0.lock().unwrap().push((source.clone(), entity.id()));
            match entity.id() {
                112 | 115 => true, // This mod's allied roster, distinct from player ids.
                113 => panic!("fixture predicate failure"),
                _ => false,
            }
        }
    }
    struct TeamMod(Arc<Mutex<Vec<(mod_api::CommandSource, u32)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for TeamMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_entity_selector("allies", Teammates(self.0.clone()));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn synced_selector_tokens_survive_client_registry_refresh() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            game.options.dedicated = None;
            let (client, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "SelectorSyncTest").unwrap();
            let tokens = vec![SelectorToken {
                name: "remote_allies".into(),
                tooltip: "test.remote_allies".into(),
                players_only: true,
                limit: 1,
                sort: "nearest",
            }];
            game.apply_packet(horizon_protocol::packets::play::Clientbound::CustomPayload(
                horizon_protocol::packets::configuration::CustomPayload {
                    channel: horizon_client::lan::commands::selector_sync::CHANNEL.into(),
                    data: horizon_client::lan::commands::selector_sync::encode(&tokens),
                },
            ))
            .unwrap();
            assert_eq!(game.ui.registries.selector_tokens, tokens);
            game.build_registries();
            assert_eq!(game.ui.registries.selector_tokens, tokens);
            for data in [
                vec![2],
                vec![9],
                horizon_client::lan::commands::selector_sync::encode(&tokens),
            ] {
                game.apply_packet(horizon_protocol::packets::play::Clientbound::CustomPayload(
                    horizon_protocol::packets::configuration::CustomPayload {
                        channel: horizon_client::lan::commands::selector_sync::CHANNEL.into(),
                        data,
                    },
                ))
                .unwrap();
                assert!(game.ui.registries.selector_tokens.is_empty());
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn nearest_teammate_filters_before_limit_and_drops_false_and_panicking_candidates() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "selector_fixture",
            Box::new(TeamMod(calls.clone())),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_guest_turn_tests::with_guests(|game| {
                game.build_registries();
                // A body and level UUID alone do not identify a server command
                // source. Enter a real guest turn so its UUID/name/permission
                // and entity identity are installed by the production path.
                game.as_guest(12, |game| {
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    for (id, x) in [(111, 1.0), (112, 2.0), (113, 1.5), (114, 0.5), (115, 3.0)] {
                        game.mobs.push(horizon_client::mobs::Mob::new(
                            id,
                            kind.clone(),
                            Vec3::new(x, 0.0, 0.0),
                            0.0,
                        ));
                    }
                    let selector = Selector {
                        kind: 'e',
                        include_entities: true,
                        token: Some("allies".into()),
                        limit: 1,
                        sort: horizon_client::command::args::Sort::Nearest,
                        pos: [Some(0.0); 3],
                        ..Default::default()
                    };
                    assert_eq!(game.select(&selector), [Target::Mob(112)]);
                    let calls = calls.lock().unwrap();
                    for id in [111, 112, 113, 114, 115] {
                        let (source, _) = calls
                            .iter()
                            .find(|(_, candidate)| *candidate == id)
                            .expect("every mob must be filtered before limit");
                        let mod_api::CommandSource::Player { player, .. } = source else {
                            panic!("expected the acting player source");
                        };
                        assert_eq!(player.uuid(), 12);
                        assert_eq!(player.entity().id(), 12);
                    }
                });
            });
        });
    }
}
