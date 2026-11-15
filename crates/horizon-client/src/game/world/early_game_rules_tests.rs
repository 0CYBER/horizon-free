#![cfg(test)]
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::gamerules::Value;
use mod_api::{Hooks, ModEntry, ServerHost};
use std::sync::{Arc, Mutex};

struct Observe(Arc<Mutex<Vec<&'static str>>>);
fn assert_rules(host: &dyn ServerHost) {
    assert_eq!(
        host.game_rule("minecraft:keep_inventory"),
        Ok(mod_api::GameRuleValue::Bool(true))
    );
    assert_eq!(
        host.game_rule("minecraft:random_tick_speed"),
        Ok(mod_api::GameRuleValue::Int(7))
    );
    assert_eq!(
        host.game_rule("minecraft:not_a_rule"),
        Err(mod_api::GameRuleReadError::UnknownRule)
    );
    let rules = host.game_rules();
    assert!(rules.contains(&(
        "minecraft:keep_inventory".into(),
        mod_api::GameRuleValue::Bool(true)
    )));
    assert!(rules.contains(&(
        "minecraft:random_tick_speed".into(),
        mod_api::GameRuleValue::Int(7)
    )));
}
#[mod_api::__subscriptions]
impl Hooks for Observe {
    fn server_about_to_start(
        &self,
        host: &mut dyn ServerHost,
        _: &mut mod_api::ServerAboutToStartEvent,
    ) {
        assert_rules(host);
        self.0.lock().unwrap().push("about");
    }
    fn server_starting(&self, host: &mut dyn ServerHost, _: &mut mod_api::ServerStartingEvent) {
        assert_rules(host);
        self.0.lock().unwrap().push("starting");
    }
    fn game_rule_changed(&self, _: &mut dyn ServerHost, _: &mut mod_api::GameRuleChangedEvent<'_>) {
        self.0.lock().unwrap().push("changed");
    }
}

#[test]
#[ignore = "daily"]
fn early_server_game_rules_saved_and_creation_values() {
    for dedicated in [false, true] {
        for saved in [false, true] {
            let scratch = tempfile::tempdir().unwrap();
            let registry = Arc::new(horizon_client::gamerules::RuleRegistry::vanilla());
            let mut create = horizon_client::title::WorldOptions::default();
            assert!(create.rules.set("keep_inventory", Value::Bool(true)));
            assert!(create.rules.set("random_tick_speed", Value::Int(7)));
            if saved {
                let mut level = horizon_client::save::Level {
                    game_rules: create.rules.by_name(),
                    ..Default::default()
                };
                level.write(scratch.path()).unwrap();
                create.rules.save(scratch.path()).unwrap();
            }
            let observed = Arc::new(Mutex::new(Vec::new()));
            mod_platform::testing::with_mods(
                vec![ModEntry::new(
                    "rule_read",
                    Box::new(Observe(observed.clone())),
                )]
                .into(),
                || {
                    with_game(|game| {
                        let mut request = crate::game::world_loading::WorldRequest::new(
                            "rules",
                            Some("rules"),
                            Some(1),
                            true,
                            (!saved).then_some(&create),
                            registry.clone(),
                        )
                        .unwrap();
                        request.save_root = scratch.path().to_owned();
                        let prepared = if dedicated {
                            game.prepare_world_and_rules(request).unwrap()
                        } else {
                            let loader = crate::game::world_loading::WorldLoader::start(
                                request,
                                game.res.clone(),
                                game.pack.clone(),
                                game.options.clone(),
                                game.settings.clone(),
                                Arc::new(horizon_world::datafix::progress::UpgradeProgress::new()),
                            )
                            .unwrap();
                            game.prepare_early_game_rules(loader).unwrap()
                        };
                        game.native_server_about_to_start(dedicated);
                        game.native_server_starting(dedicated);
                        game.finish_open_world(prepared).unwrap();
                        assert_rules(game);
                        let level = game.level(game.dim.id()).unwrap();
                        assert_eq!(
                            level.game_rule(game, "keep_inventory"),
                            Ok(Some(mod_api::GameRuleValue::Bool(true)))
                        );
                        assert_eq!(
                            level.game_rule(game, "random_tick_speed"),
                            Ok(Some(mod_api::GameRuleValue::Int(7)))
                        );
                        assert_eq!(
                            level.game_rules(game).unwrap(),
                            <dyn ServerHost>::game_rules(game)
                        );
                    });
                },
            );
            assert_eq!(*observed.lock().unwrap(), ["about", "starting"]);
        }
    }
}
